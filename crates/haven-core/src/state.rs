//! Persisted application state (`~/Library/Application Support/Haven/state.json`).

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::home_dir;
use crate::recents::{Recent, now_secs};
use crate::sort::SortSpec;
use crate::tags::{Tag, default_tags};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum ViewMode {
    #[default]
    List,
    Grid,
    Columns,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderPrefs {
    pub view: ViewMode,
    pub sort: SortSpec,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Session {
    /// `None` is the start page.
    pub tabs: Vec<Option<PathBuf>>,
    pub active: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppState {
    pub version: u32,
    /// Folders pinned in the sidebar, in order.
    pub sidebar_pins: Vec<PathBuf>,
    /// Files and folders pinned on the start page.
    pub start_pins: Vec<PathBuf>,
    /// Known tags with their colors (sidebar order).
    pub tags: Vec<Tag>,
    pub folder_prefs: HashMap<PathBuf, FolderPrefs>,
    pub default_view: ViewMode,
    pub default_sort: SortSpec,
    pub show_hidden: bool,
    pub preview_visible: bool,
    pub sidebar_width: f32,
    pub preview_width: f32,
    pub grid_icon_size: f32,
    pub collapsed_sections: Vec<String>,
    /// Files opened through Haven: (path, unix seconds).
    pub history: Vec<(PathBuf, u64)>,
    pub session: Session,
    /// Application used for "Open in Terminal" (bundle name for `open -a`).
    pub terminal_app: String,
    /// Command used for "Open in Editor".
    pub editor_command: String,
    pub respect_gitignore: bool,
}

impl Default for AppState {
    fn default() -> Self {
        let home = home_dir();
        let terminal_app = if Path::new("/Applications/Ghostty.app").exists() {
            "Ghostty"
        } else {
            "Terminal"
        }
        .to_string();
        AppState {
            version: 1,
            sidebar_pins: Vec::new(),
            start_pins: ["Desktop", "Documents", "Downloads"]
                .iter()
                .map(|d| home.join(d))
                .filter(|p| p.exists())
                .collect(),
            tags: default_tags(),
            folder_prefs: HashMap::new(),
            default_view: ViewMode::List,
            default_sort: SortSpec::default(),
            show_hidden: false,
            preview_visible: true,
            sidebar_width: 220.0,
            preview_width: 360.0,
            grid_icon_size: 72.0,
            collapsed_sections: Vec::new(),
            history: Vec::new(),
            session: Session::default(),
            terminal_app,
            editor_command: "code".into(),
            respect_gitignore: true,
        }
    }
}

impl AppState {
    pub fn default_path() -> PathBuf {
        if let Some(dir) = std::env::var_os("HAVEN_STATE_DIR") {
            return PathBuf::from(dir).join("state.json");
        }
        home_dir().join("Library/Application Support/Haven/state.json")
    }

    pub fn load() -> AppState {
        Self::load_from(&Self::default_path()).unwrap_or_default()
    }

    pub fn load_from(path: &Path) -> io::Result<AppState> {
        let bytes = std::fs::read(path)?;
        serde_json::from_slice(&bytes).map_err(io::Error::other)
    }

    pub fn save(&self) -> io::Result<()> {
        self.save_to(&Self::default_path())
    }

    /// Atomic write: temp file + rename.
    pub fn save_to(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        let json = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        std::fs::write(&tmp, json)?;
        std::fs::rename(tmp, path)
    }

    pub fn prefs_for(&self, dir: &Path) -> FolderPrefs {
        self.folder_prefs.get(dir).cloned().unwrap_or(FolderPrefs {
            view: self.default_view,
            sort: self.default_sort.clone(),
        })
    }

    pub fn set_prefs(&mut self, dir: &Path, prefs: FolderPrefs) {
        self.default_view = prefs.view;
        self.folder_prefs.insert(dir.to_path_buf(), prefs);
    }

    pub fn toggle_sidebar_pin(&mut self, path: &Path) {
        toggle(&mut self.sidebar_pins, path);
    }

    pub fn toggle_start_pin(&mut self, path: &Path) {
        toggle(&mut self.start_pins, path);
    }

    /// Moves a sidebar pin to `to` (index in the list).
    pub fn reorder_sidebar_pin(&mut self, path: &Path, to: usize) {
        if let Some(from) = self.sidebar_pins.iter().position(|p| p == path) {
            let item = self.sidebar_pins.remove(from);
            let to = to.min(self.sidebar_pins.len());
            self.sidebar_pins.insert(to, item);
        }
    }

    pub fn record_opened(&mut self, path: &Path) {
        self.history.retain(|(p, _)| p != path);
        self.history.insert(0, (path.to_path_buf(), now_secs()));
        self.history.truncate(100);
    }

    pub fn history_recents(&self) -> Vec<Recent> {
        self.history
            .iter()
            .map(|(p, t)| Recent {
                path: p.clone(),
                used: std::time::UNIX_EPOCH + std::time::Duration::from_secs(*t),
            })
            .collect()
    }

    pub fn tag_color(&self, name: &str) -> Option<crate::tags::TagColor> {
        self.tags
            .iter()
            .find(|t| t.name.eq_ignore_ascii_case(name))
            .map(|t| t.color)
    }

    /// Registers a tag seen on disk so it appears in the sidebar.
    pub fn learn_tag(&mut self, tag: &Tag) -> bool {
        if self
            .tags
            .iter()
            .any(|t| t.name.eq_ignore_ascii_case(&tag.name))
        {
            return false;
        }
        self.tags.push(tag.clone());
        true
    }

    pub fn is_collapsed(&self, section: &str) -> bool {
        self.collapsed_sections.iter().any(|s| s == section)
    }

    pub fn toggle_section(&mut self, section: &str) {
        if let Some(i) = self.collapsed_sections.iter().position(|s| s == section) {
            self.collapsed_sections.remove(i);
        } else {
            self.collapsed_sections.push(section.to_string());
        }
    }
}

fn toggle(list: &mut Vec<PathBuf>, path: &Path) {
    if let Some(i) = list.iter().position(|p| p == path) {
        list.remove(i);
    } else {
        list.push(path.to_path_buf());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sort::SortKey;
    use crate::tags::TagColor;

    #[test]
    fn round_trip_and_defaults_for_missing_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/state.json");
        let mut s = AppState::default();
        s.toggle_sidebar_pin(Path::new("/tmp"));
        s.set_prefs(
            Path::new("/tmp"),
            FolderPrefs {
                view: ViewMode::Grid,
                sort: SortSpec {
                    keys: vec![(SortKey::Size, true)],
                    folders_first: false,
                },
            },
        );
        s.save_to(&path).unwrap();
        let loaded = AppState::load_from(&path).unwrap();
        assert_eq!(loaded, s);
        assert_eq!(loaded.prefs_for(Path::new("/tmp")).view, ViewMode::Grid);
        assert_eq!(loaded.prefs_for(Path::new("/other")).view, ViewMode::Grid);

        std::fs::write(&path, br#"{"show_hidden": true}"#).unwrap();
        let partial = AppState::load_from(&path).unwrap();
        assert!(partial.show_hidden);
        assert_eq!(partial.tags.len(), 7);
        assert_eq!(partial.sidebar_width, 220.0);
    }

    #[test]
    fn pins_toggle_and_reorder() {
        let mut s = AppState {
            sidebar_pins: vec![],
            ..Default::default()
        };
        for p in ["/a", "/b", "/c"] {
            s.toggle_sidebar_pin(Path::new(p));
        }
        s.reorder_sidebar_pin(Path::new("/c"), 0);
        assert_eq!(
            s.sidebar_pins,
            [PathBuf::from("/c"), "/a".into(), "/b".into()]
        );
        s.toggle_sidebar_pin(Path::new("/a"));
        assert_eq!(s.sidebar_pins.len(), 2);
    }

    #[test]
    fn history_dedups_and_learns_tags() {
        let mut s = AppState::default();
        s.record_opened(Path::new("/x"));
        s.record_opened(Path::new("/y"));
        s.record_opened(Path::new("/x"));
        assert_eq!(s.history.len(), 2);
        assert_eq!(s.history[0].0, PathBuf::from("/x"));
        assert!(s.learn_tag(&Tag::new("Design", TagColor::Red)));
        assert!(!s.learn_tag(&Tag::new("design", TagColor::Blue)));
        assert_eq!(s.tag_color("DESIGN"), Some(TagColor::Red));
    }
}
