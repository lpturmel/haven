//! Native Finder icons, rendered off the main thread and cached by type.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::Arc;

use gpui::{App, Global, Image, ImageFormat, ImageSource, RenderImage};
use haven_core::Entry;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum IconSize {
    /// 16–20pt rows.
    Small,
    /// Grid tiles and start page.
    Large,
    /// The preview pane.
    Preview,
}

impl IconSize {
    fn px(self) -> f64 {
        match self {
            IconSize::Small => 64.0,
            IconSize::Large => 256.0,
            IconSize::Preview => 1024.0,
        }
    }
}

#[derive(Default)]
pub struct IconCache {
    map: HashMap<(String, IconSize), Option<ImageSource>>,
    pending: HashSet<(String, IconSize)>,
    /// Per-file thumbnails in insertion order with their decoded size,
    /// evicted beyond `THUMB_BUDGET` bytes (type icons are few and stay).
    thumbs: VecDeque<((String, IconSize), usize)>,
    thumb_bytes: usize,
}

/// Decoded per-file thumbnails kept in memory (CPU + GPU atlas).
const THUMB_BUDGET: usize = 192 * 1024 * 1024;
/// Budget charged for a cached "no thumbnail" entry.
const FAILED_ENTRY_COST: usize = 4096;

/// What the background job produced.
enum Loaded {
    /// A shared type/path icon: gpui decodes and caches it by content.
    Encoded(Arc<Image>),
    /// A per-file thumbnail decoded here, so its atlas tile can be freed
    /// with `drop_image` on eviction (gpui's own asset cache can't: a
    /// re-decode gets a new id and orphans the old tile).
    Decoded(Arc<RenderImage>, usize),
}

/// PNG -> gpui's BGRA frame.
fn decode_png(png: &[u8]) -> Option<(Arc<RenderImage>, usize)> {
    let mut rgba = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .ok()?
        .into_rgba8();
    for px in rgba.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
    }
    let bytes = rgba.len();
    let frame = image::Frame::new(rgba);
    Some((
        Arc::new(RenderImage::new(smallvec::smallvec![frame])),
        bytes,
    ))
}

impl Global for IconCache {}

/// Where the icon comes from: a type (shared by many files) or a specific path.
enum Source {
    Type(String),
    Path(String),
    /// A real thumbnail of an image GPUI can't decode (HEIC, RAW, ...).
    Thumb(String),
}

fn source_for(path: &Path, is_dir: bool, is_package: bool, ext: &str) -> Source {
    let p = path.to_string_lossy();
    if !is_dir && haven_core::kind::is_native_image_ext(ext) {
        return Source::Thumb(p.into_owned());
    }
    if is_package || matches!(ext, "icns" | "app") {
        return Source::Path(p.into_owned());
    }
    if is_dir {
        // Special folders and volume roots carry custom icons.
        let home = haven_core::home_dir();
        let special = path.parent() == Some(home.as_path())
            || path == home
            || path == Path::new("/")
            || path.parent() == Some(Path::new("/Volumes"))
            || path == Path::new("/Applications");
        return if special {
            Source::Path(p.into_owned())
        } else {
            Source::Type("public.folder".into())
        };
    }
    if ext.is_empty() {
        Source::Type("public.data".into())
    } else {
        Source::Type(ext.to_string())
    }
}

impl IconCache {
    pub fn init(cx: &mut App) {
        cx.set_global(IconCache::default());
    }
}

pub fn entry_icon(entry: &Entry, size: IconSize, cx: &mut App) -> Option<ImageSource> {
    // Thumbnails are keyed by size + mtime too, so a file changed in place
    // (or read mid-copy) gets a fresh one.
    let mtime = entry
        .modified
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    icon(
        &entry.path,
        entry.is_dir,
        entry.is_package,
        &entry.ext,
        size,
        &format!("{}:{mtime}", entry.size),
        cx,
    )
}

pub fn path_icon(
    path: &Path,
    is_dir: bool,
    is_package: bool,
    ext: &str,
    size: IconSize,
    cx: &mut App,
) -> Option<ImageSource> {
    icon(path, is_dir, is_package, ext, size, "", cx)
}

fn icon(
    path: &Path,
    is_dir: bool,
    is_package: bool,
    ext: &str,
    size: IconSize,
    stamp: &str,
    cx: &mut App,
) -> Option<ImageSource> {
    let source = source_for(path, is_dir, is_package, ext);
    let key = match &source {
        Source::Type(t) => (format!("t:{t}"), size),
        Source::Path(p) => (format!("p:{p}"), size),
        Source::Thumb(p) => (format!("i:{p}@{stamp}"), size),
    };
    let cache = cx.global::<IconCache>();
    if let Some(hit) = cache.map.get(&key) {
        return hit.clone();
    }
    if cache.pending.contains(&key) {
        return None;
    }
    cx.global_mut::<IconCache>().pending.insert(key.clone());
    let px = size.px();
    let task = cx.background_executor().spawn(async move {
        match source {
            Source::Type(t) => haven_macos::type_icon_png(&t, px)
                .map(|b| Loaded::Encoded(Arc::new(Image::from_bytes(ImageFormat::Png, b)))),
            Source::Path(p) => haven_macos::file_icon_png(Path::new(&p), px)
                .map(|b| Loaded::Encoded(Arc::new(Image::from_bytes(ImageFormat::Png, b)))),
            Source::Thumb(p) => haven_macos::image_thumbnail_png(Path::new(&p), px as u32)
                .or_else(|| haven_macos::file_icon_png(Path::new(&p), px))
                .and_then(|png| decode_png(&png))
                .map(|(img, bytes)| Loaded::Decoded(img, bytes)),
        }
    });
    cx.spawn(async move |cx| {
        let loaded = task.await;
        let _ = cx.update(|cx| {
            let mut evicted: Vec<Arc<RenderImage>> = Vec::new();
            let cache = cx.global_mut::<IconCache>();
            cache.pending.remove(&key);
            let thumb = key.0.starts_with("i:");
            let source = match loaded {
                // A failed thumbnail is cached too (no retry every frame) but
                // still counts against the budget, so such entries are evicted.
                None if thumb => {
                    cache.thumbs.push_back((key.clone(), FAILED_ENTRY_COST));
                    cache.thumb_bytes += FAILED_ENTRY_COST;
                    None
                }
                None => None,
                Some(Loaded::Encoded(img)) => Some(ImageSource::Image(img)),
                Some(Loaded::Decoded(img, bytes)) => {
                    cache.thumbs.push_back((key.clone(), bytes));
                    cache.thumb_bytes += bytes;
                    while cache.thumb_bytes > THUMB_BUDGET && cache.thumbs.len() > 1 {
                        let Some((old, old_bytes)) = cache.thumbs.pop_front() else {
                            break;
                        };
                        cache.thumb_bytes -= old_bytes;
                        if let Some(Some(ImageSource::Render(r))) = cache.map.remove(&old) {
                            evicted.push(r);
                        }
                    }
                    Some(ImageSource::Render(img))
                }
            };
            cache.map.insert(key, source);
            // Free the evicted thumbnails' GPU atlas tiles in every window.
            for r in evicted {
                cx.drop_image(r, None);
            }
            cx.refresh_windows();
        });
    })
    .detach();
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoded_thumbnails_are_bgra() {
        let mut png = Vec::new();
        let red = image::RgbaImage::from_pixel(2, 1, image::Rgba([255, 0, 0, 255]));
        red.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let (img, bytes) = decode_png(&png).expect("decodes");
        assert_eq!(bytes, 2 * 4);
        let frame = img.as_bytes(0).unwrap();
        assert_eq!(&frame[..4], &[0, 0, 255, 255]); // B, G, R, A
        assert!(decode_png(b"not a png").is_none());
    }
}
