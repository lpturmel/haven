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
