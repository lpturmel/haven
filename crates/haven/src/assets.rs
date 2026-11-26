//! Embedded Lucide icons (ISC licensed, see assets/icons/LICENSE-lucide).

use std::borrow::Cow;

use anyhow::Result;
use gpui::{AssetSource, SharedString};

macro_rules! icons {
    ($($name:literal),* $(,)?) => {
        const ICONS: &[(&str, &[u8])] = &[
            $((concat!("icons/", $name, ".svg"),
               include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/icons/", $name, ".svg")))),*
        ];
    };
}

icons![
    "app-window",
    "arrow-down",
    "arrow-down-wide-narrow",
    "arrow-left",
    "arrow-right",
    "arrow-up",
    "arrow-up-down",
    "arrow-up-narrow-wide",
    "binary",
    "check",
    "chevron-down",
    "chevron-left",
    "chevron-right",
    "chevron-up",
    "circle",
    "clipboard-paste",
    "clock",
    "cloud",
    "code",
    "columns-3",
    "command",
    "copy",
    "download",
    "ellipsis",
    "external-link",
    "eye",
    "eye-off",
    "file",
    "file-archive",
    "file-audio",
    "file-code",
    "file-image",
    "file-json",
    "file-text",
    "file-video",
    "folder",
    "folder-open",
    "folder-plus",
    "hard-drive",
    "hash",
    "history",
    "house",
    "image",
    "info",
    "layout-grid",
    "link",
    "list",
    "loader-circle",
    "monitor",
    "music",
    "package",
    "panel-right",
    "pencil",
    "pin",
    "pin-off",
    "plus",
    "refresh-cw",
    "scissors",
    "search",
    "settings",
    "square-terminal",
    "star",
    "tag",
    "terminal",
    "text-cursor-input",
    "trash-2",
    "triangle-alert",
    "undo-2",
    "usb",
    "user",
    "video",
    "x",
];

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(ICONS
            .iter()
            .find(|(p, _)| *p == path)
            .map(|(_, b)| Cow::Borrowed(*b)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(ICONS
            .iter()
            .filter(|(p, _)| p.starts_with(path))
            .map(|(p, _)| SharedString::from(*p))
            .collect())
    }
}
