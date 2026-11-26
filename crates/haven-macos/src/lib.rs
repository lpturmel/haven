//! Thin AppKit / Foundation glue for Haven. Every function degrades to `None`
//! or an error on non-macOS targets so the rest of the workspace stays portable.

use std::path::{Path, PathBuf};

/// Renders the Finder icon for an existing path as PNG bytes at `px` pixels.
pub fn file_icon_png(path: &Path, px: f64) -> Option<Vec<u8>> {
    #[cfg(target_os = "macos")]
    {
        imp::file_icon_png(path, px)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (path, px);
        None
    }
}

/// Decodes an image with ImageIO (HEIC, AVIF, RAW, PSD, ... anything macOS
/// can read) into a PNG thumbnail whose longer side is at most `max_px`,
/// honouring EXIF orientation. Returns `None` if the file isn't an image.
pub fn image_thumbnail_png(path: &Path, max_px: u32) -> Option<Vec<u8>> {
    #[cfg(target_os = "macos")]
    {
        imp::image_thumbnail_png(path, max_px)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (path, max_px);
        None
    }
}

/// Renders the generic icon for a file type (UTI such as `public.folder`, or
/// a bare extension such as `rs`) as PNG bytes at `px` pixels.
pub fn type_icon_png(file_type: &str, px: f64) -> Option<Vec<u8>> {
    #[cfg(target_os = "macos")]
    {
        imp::type_icon_png(file_type, px)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (file_type, px);
        None
    }
}

/// Installs a native behind-window blur (the Finder sidebar material) under the
/// app's content. `ns_view` is the window's content `NSView` (from the window's
/// AppKit raw handle). Content drawn with transparency, such as the sidebar,
/// then shows a real frosted backdrop; opaque content hides it. Returns false
/// when it could not be installed (not on the main thread, or not macOS).
///
/// # Safety
/// `ns_view` must be a valid `NSView` pointer that outlives this call.
pub unsafe fn install_sidebar_material(ns_view: *mut std::ffi::c_void) -> bool {
    #[cfg(target_os = "macos")]
    {
        // SAFETY: forwarded caller contract.
        unsafe { imp::install_sidebar_material(ns_view) }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = ns_view;
        false
    }
}

/// Moves `path` to the Trash and returns where it ended up (so it can be undone).
pub fn trash(path: &Path) -> Result<PathBuf, String> {
    #[cfg(target_os = "macos")]
    {
        imp::trash(path)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        Err("trash is only supported on macOS".into())
    }
}

/// Applications that can open `path`, default first.
pub fn apps_for(path: &Path) -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        imp::apps_for(path)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        Vec::new()
    }
}

/// Forces the dark Aqua appearance app-wide (so vibrancy renders dark). Must be
/// called on the main thread; does nothing elsewhere.
pub fn force_dark_appearance() {
    #[cfg(target_os = "macos")]
    imp::force_dark_appearance();
}

#[cfg(target_os = "macos")]
mod imp {
    use std::path::{Path, PathBuf};

    use objc2::AnyThread;
    use objc2::rc::{Retained, autoreleasepool};
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImage, NSWorkspace};
    use objc2_foundation::{NSDictionary, NSFileManager, NSPoint, NSRect, NSSize, NSString, NSURL};

    fn png_from_image(image: &NSImage, px: f64) -> Option<Vec<u8>> {
        let mut rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(px, px));
        // SAFETY: rect is valid and writable; no context or hints are passed.
        let cg = unsafe { image.CGImageForProposedRect_context_hints(&mut rect, None, None) }?;
        let rep = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), &cg);
        // SAFETY: an empty, correctly typed property dictionary.
        let data = unsafe {
            rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
        }?;
        Some(data.to_vec())
    }

    pub fn image_thumbnail_png(path: &Path, max_px: u32) -> Option<Vec<u8>> {
        use objc2_core_foundation::{CFBoolean, CFDictionary, CFNumber, CFString, CFType, CFURL};
        use objc2_image_io::{
            CGImageSource, kCGImageSourceCreateThumbnailFromImageAlways,
            kCGImageSourceCreateThumbnailWithTransform, kCGImageSourceThumbnailMaxPixelSize,
        };
        autoreleasepool(|_| {
            let url = CFURL::from_file_path(path)?;
            // SAFETY: a valid file URL and no options.
            let source = unsafe { CGImageSource::with_url(&url, None) }?;
            let yes: &CFType = CFBoolean::new(true);
            let size = CFNumber::new_i64(i64::from(max_px));
            // SAFETY: reading ImageIO's constant option keys.
            let keys: [&CFString; 3] = unsafe {
                [
                    kCGImageSourceCreateThumbnailFromImageAlways,
                    kCGImageSourceCreateThumbnailWithTransform,
                    kCGImageSourceThumbnailMaxPixelSize,
                ]
            };
            let values: [&CFType; 3] = [yes, yes, &size];
            let options = CFDictionary::<CFString, CFType>::from_slices(&keys, &values);
            // SAFETY: the options dictionary holds the documented key/value types.
            let cg = unsafe { source.thumbnail_at_index(0, Some(options.as_opaque())) }?;
            let rep = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), &cg);
            // SAFETY: an empty, correctly typed property dictionary.
            let data = unsafe {
                rep.representationUsingType_properties(
                    NSBitmapImageFileType::PNG,
                    &NSDictionary::new(),
                )
            }?;
            Some(data.to_vec())
        })
    }

    pub fn file_icon_png(path: &Path, px: f64) -> Option<Vec<u8>> {
        autoreleasepool(|_| {
            let icon =
                NSWorkspace::sharedWorkspace().iconForFile(&NSString::from_str(path.to_str()?));
            png_from_image(&icon, px)
        })
    }

    #[allow(deprecated)]
    pub fn type_icon_png(file_type: &str, px: f64) -> Option<Vec<u8>> {
        autoreleasepool(|_| {
            let icon =
                NSWorkspace::sharedWorkspace().iconForFileType(&NSString::from_str(file_type));
            png_from_image(&icon, px)
        })
    }

    pub fn force_dark_appearance() {
        use objc2::MainThreadMarker;
        use objc2_app_kit::{NSAppearance, NSAppearanceNameDarkAqua, NSApplication};
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        // SAFETY: reading an AppKit-provided static appearance name.
        let name = unsafe { NSAppearanceNameDarkAqua };
        let appearance = NSAppearance::appearanceNamed(name);
        NSApplication::sharedApplication(mtm).setAppearance(appearance.as_deref());
    }

    pub unsafe fn install_sidebar_material(ns_view: *mut std::ffi::c_void) -> bool {
        use objc2::MainThreadMarker;
        use objc2_app_kit::{
            NSAutoresizingMaskOptions, NSView, NSVisualEffectBlendingMode, NSVisualEffectMaterial,
            NSVisualEffectState, NSVisualEffectView, NSWindowOrderingMode,
        };
        let Some(mtm) = MainThreadMarker::new() else {
            return false;
        };
        if ns_view.is_null() {
            return false;
        }
        // SAFETY: the caller guarantees a live NSView.
        let content: &NSView = unsafe { &*(ns_view as *const NSView) };
        // The content view renders through its own (Metal) layer, which subviews
        // would cover. Put the material behind it, as a sibling in the window's
        // frame view, so it only shows where the content is transparent.
        // SAFETY: reading the superview of a live view on the main thread.
        let Some(frame_view) = (unsafe { content.superview() }) else {
            return false;
        };
        let effect = NSVisualEffectView::initWithFrame(mtm.alloc(), frame_view.bounds());
        effect.setMaterial(NSVisualEffectMaterial::Sidebar);
        effect.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
        // Like Finder: vibrant while the window is key, flatter when inactive.
        effect.setState(NSVisualEffectState::FollowsWindowActiveState);
        effect.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        frame_view.addSubview_positioned_relativeTo(
            &effect,
            NSWindowOrderingMode::Below,
            Some(content),
        );
        true
    }

    pub fn trash(path: &Path) -> Result<PathBuf, String> {
        autoreleasepool(|_| {
            let s = path.to_str().ok_or("path is not valid UTF-8")?;
            let url = NSURL::fileURLWithPath(&NSString::from_str(s));
            let mut out: Option<Retained<NSURL>> = None;
            NSFileManager::defaultManager()
                .trashItemAtURL_resultingItemURL_error(&url, Some(&mut out))
                .map_err(|e| e.localizedDescription().to_string())?;
            let landed = out
                .and_then(|u| u.path())
                .map(|p| PathBuf::from(p.to_string()))
                .unwrap_or_default();
            Ok(landed)
        })
    }

    pub fn apps_for(path: &Path) -> Vec<PathBuf> {
        autoreleasepool(|_| {
            let Some(s) = path.to_str() else {
                return Vec::new();
            };
            let url = NSURL::fileURLWithPath(&NSString::from_str(s));
            let ws = NSWorkspace::sharedWorkspace();
            let default = ws
                .URLForApplicationToOpenURL(&url)
                .and_then(|u| u.path())
                .map(|p| PathBuf::from(p.to_string()));
            let mut apps: Vec<PathBuf> = default.iter().cloned().collect();
            let all = ws.URLsForApplicationsToOpenURL(&url);
            for i in 0..all.count() {
                if let Some(p) = all.objectAtIndex(i).path() {
                    let p = PathBuf::from(p.to_string());
                    if !apps.contains(&p) {
                        apps.push(p);
                    }
                }
            }
            apps
        })
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn thumbnails_heic_and_rejects_non_images() {
        // macOS ships HEIC wallpapers; use one if present.
        let dir = std::path::Path::new("/System/Library/Desktop Pictures");
        let heic = std::fs::read_dir(dir).ok().and_then(|r| {
            r.flatten()
                .map(|e| e.path())
                .find(|p| p.extension().is_some_and(|e| e == "heic"))
        });
        if let Some(heic) = heic {
            let png = image_thumbnail_png(&heic, 256).expect("HEIC decodes");
            assert_eq!(&png[1..4], b"PNG");
        }
        let txt = std::env::temp_dir().join(format!("haven-not-image-{}.txt", std::process::id()));
        std::fs::write(&txt, b"hello").unwrap();
        assert!(image_thumbnail_png(&txt, 256).is_none());
        let _ = std::fs::remove_file(&txt);
    }

    #[test]
    fn renders_folder_type_icon() {
        let png = type_icon_png("public.folder", 64.0).expect("folder icon");
        assert_eq!(&png[1..4], b"PNG");
    }

    #[test]
    fn renders_path_icon() {
        let png = file_icon_png(Path::new("/Applications"), 32.0).expect("icon");
        assert_eq!(&png[1..4], b"PNG");
    }

    #[test]
    fn trash_returns_location() {
        let dir = std::env::temp_dir().join(format!("haven-trash-test-{}", std::process::id()));
        std::fs::write(&dir, b"x").unwrap();
        match trash(&dir) {
            Ok(landed) => {
                assert!(!dir.exists());
                if landed.exists() {
                    std::fs::remove_file(&landed).ok();
                }
            }
            // Temp volumes without a Trash (sandboxed CI) are acceptable.
            Err(_) => {
                std::fs::remove_file(&dir).ok();
            }
        }
    }
}
