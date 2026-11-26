//! System integration: default file viewer registration.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::home_dir;

pub const BUNDLE_ID: &str = "dev.haven.app";

fn ls_plist() -> PathBuf {
    home_dir()
        .join("Library/Preferences/com.apple.LaunchServices/com.apple.launchservices.secure.plist")
}

fn run(cmd: &str, args: &[&str]) -> Result<(), String> {
    let out = Command::new(cmd)
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Whether Haven is registered as the file viewer (`NSFileViewer`).
pub fn is_default_viewer() -> bool {
    Command::new("/usr/bin/defaults")
        .args(["read", "-g", "NSFileViewer"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == BUNDLE_ID)
        .unwrap_or(false)
}

/// Makes Haven the default file viewer and `public.folder` handler, or restores Finder.
pub fn set_default_viewer(enable: bool) -> Result<(), String> {
    if enable {
        run(
            "/usr/bin/defaults",
            &["write", "-g", "NSFileViewer", "-string", BUNDLE_ID],
        )?;
    } else {
        let _ = run("/usr/bin/defaults", &["delete", "-g", "NSFileViewer"]);
    }
    set_folder_handler(&ls_plist(), enable)
}

/// Adds or removes Haven as the `public.folder` handler in the LaunchServices
/// plist at `path`, keeping every other association.
fn set_folder_handler(path: &Path, enable: bool) -> Result<(), String> {
    let path = path.to_path_buf();
    // Start empty only if the file doesn't exist yet. Any other read or parse
    // error must abort: rewriting from an empty dict would wipe every other
    // file-type association the user has.
    let mut root: plist::Dictionary = if path.exists() {
        plist::from_file(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?
    } else {
        plist::Dictionary::new()
    };
    let mut handlers: Vec<plist::Value> = root
        .get("LSHandlers")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    handlers.retain(|h| {
        h.as_dictionary()
            .and_then(|d| d.get("LSHandlerContentType"))
            .and_then(|v| v.as_string())
            != Some("public.folder")
    });
    if enable {
        let mut d = plist::Dictionary::new();
        d.insert("LSHandlerContentType".into(), "public.folder".into());
        d.insert("LSHandlerRoleAll".into(), BUNDLE_ID.into());
        handlers.push(plist::Value::Dictionary(d));
    }
    root.insert("LSHandlers".into(), plist::Value::Array(handlers));
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension(format!("plist.haven-{}", std::process::id()));
    plist::to_file_binary(&tmp, &root).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    // LaunchServices picks up handler changes on its next refresh (or at login).
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handlers(path: &Path) -> Vec<(String, String)> {
        let root: plist::Dictionary = plist::from_file(path).unwrap();
        root.get("LSHandlers")
            .and_then(|v| v.as_array())
            .unwrap()
            .iter()
            .filter_map(|h| {
                let d = h.as_dictionary()?;
                let ty = d.get("LSHandlerContentType")?.as_string()?.to_string();
                let app = d
                    .get("LSHandlerRoleAll")
                    .and_then(|v| v.as_string())
                    .unwrap_or("")
                    .to_string();
                Some((ty, app))
            })
            .collect()
    }

    #[test]
    fn folder_handler_keeps_other_associations() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ls.plist");
        let mut other = plist::Dictionary::new();
        other.insert("LSHandlerContentType".into(), "public.html".into());
        other.insert("LSHandlerRoleAll".into(), "com.example.browser".into());
        let mut root = plist::Dictionary::new();
        root.insert(
            "LSHandlers".into(),
            plist::Value::Array(vec![plist::Value::Dictionary(other)]),
        );
        plist::to_file_binary(&path, &root).unwrap();

        set_folder_handler(&path, true).unwrap();
        let h = handlers(&path);
        assert!(h.contains(&("public.html".into(), "com.example.browser".into())));
        assert!(h.contains(&("public.folder".into(), BUNDLE_ID.into())));
        set_folder_handler(&path, false).unwrap();
        assert_eq!(
            handlers(&path),
            vec![("public.html".into(), "com.example.browser".into())]
        );
    }

    #[test]
    fn unreadable_plist_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ls.plist");
        std::fs::write(&path, b"not a plist at all").unwrap();
        assert!(set_folder_handler(&path, true).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"not a plist at all");
        // A missing file is fine: it is created with just our handler.
        let fresh = dir.path().join("new.plist");
        set_folder_handler(&fresh, true).unwrap();
        assert_eq!(
            handlers(&fresh),
            vec![("public.folder".into(), BUNDLE_ID.into())]
        );
    }
}
