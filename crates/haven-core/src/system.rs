//! System integration: default file viewer registration.

use std::path::PathBuf;
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
    set_folder_handler(enable)
}

fn set_folder_handler(enable: bool) -> Result<(), String> {
    let path = ls_plist();
    let mut root: plist::Dictionary = plist::from_file(&path).unwrap_or_default();
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
    plist::to_file_binary(&path, &root).map_err(|e| e.to_string())?;
    // LaunchServices picks up handler changes on its next refresh (or at login).
    Ok(())
}
