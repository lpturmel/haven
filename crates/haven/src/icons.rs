//! Native Finder icons, rendered off the main thread and cached by type.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use gpui::{App, Global, Image, ImageFormat};
use haven_core::Entry;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum IconSize {
    /// 16–20pt rows.
    Small,
    /// Grid tiles and start page.
    Large,
}

impl IconSize {
    fn px(self) -> f64 {
        match self {
            IconSize::Small => 64.0,
            IconSize::Large => 256.0,
        }
    }
}

#[derive(Default)]
pub struct IconCache {
    map: HashMap<(String, IconSize), Option<Arc<Image>>>,
    pending: HashSet<(String, IconSize)>,
}

impl Global for IconCache {}

/// Where the icon comes from: a type (shared by many files) or a specific path.
enum Source {
    Type(String),
    Path(String),
}

fn source_for(path: &Path, is_dir: bool, is_package: bool, ext: &str) -> Source {
    let p = path.to_string_lossy();
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

pub fn entry_icon(entry: &Entry, size: IconSize, cx: &mut App) -> Option<Arc<Image>> {
    path_icon(
        &entry.path,
        entry.is_dir,
        entry.is_package,
        &entry.ext,
        size,
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
) -> Option<Arc<Image>> {
    let source = source_for(path, is_dir, is_package, ext);
    let key = match &source {
        Source::Type(t) => (format!("t:{t}"), size),
        Source::Path(p) => (format!("p:{p}"), size),
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
        let png = match source {
            Source::Type(t) => haven_macos::type_icon_png(&t, px),
            Source::Path(p) => haven_macos::file_icon_png(Path::new(&p), px),
        };
        png.map(|bytes| Arc::new(Image::from_bytes(ImageFormat::Png, bytes)))
    });
    cx.spawn(async move |cx| {
        let image = task.await;
        let _ = cx.update(|cx| {
            let cache = cx.global_mut::<IconCache>();
            cache.pending.remove(&key);
            cache.map.insert(key, image);
            cx.refresh_windows();
        });
    })
    .detach();
    None
}
