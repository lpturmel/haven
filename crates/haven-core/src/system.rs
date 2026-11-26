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
