//! Directory watching for open tabs (non-recursive; FSEvents on macOS).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

/// Canonical directory -> the paths tabs asked to watch for it.
type Aliases = Arc<Mutex<HashMap<PathBuf, Vec<PathBuf>>>>;

/// Watches a set of directories and reports which one changed.
///
/// FSEvents reports canonical paths (`/private/tmp/...` for `/tmp/...`, the
/// target of a symlinked folder, ...). Directories are watched by their
/// canonical path and events are translated back to the path each tab uses.
pub struct DirWatcher {
    watcher: RecommendedWatcher,
    /// Requested dir -> (ref count, canonical path resolved at watch time).
    /// `unwatch` must use the same canonical path even if the folder has since
    /// been deleted or a symlink retargeted.
    counts: HashMap<PathBuf, (usize, PathBuf)>,
    aliases: Aliases,
}

impl DirWatcher {
    /// `on_change` receives the watched directory whose contents changed. It is
    /// called from a background thread and should be cheap (e.g. send on a channel).
    pub fn new(on_change: impl Fn(PathBuf) + Send + 'static) -> notify::Result<DirWatcher> {
        let aliases: Aliases = Arc::default();
        let lookup = aliases.clone();
        let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let Ok(event) = res else { return };
            if matches!(event.kind, EventKind::Access(_)) {
                return;
            }
            let mut dirs: Vec<PathBuf> = event
                .paths
                .iter()
                .filter_map(|p| p.parent().map(Path::to_path_buf))
                .collect();
            dirs.sort();
            dirs.dedup();
            let map = lookup.lock().map(|m| m.clone()).unwrap_or_default();
            for d in dirs {
                match map.get(&d) {
                    Some(requested) => requested.iter().cloned().for_each(&on_change),
                    None => on_change(d),
                }
            }
        })?;
        Ok(DirWatcher {
            watcher,
            counts: HashMap::new(),
            aliases,
        })
    }

    fn canonical(dir: &Path) -> PathBuf {
        std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf())
    }

    /// Reference-counted watch so several tabs can share a directory.
    pub fn watch(&mut self, dir: &Path) {
        if let Some((count, _)) = self.counts.get_mut(dir) {
            *count += 1;
            return;
        }
        let canon = Self::canonical(dir);
        self.counts.insert(dir.to_path_buf(), (1, canon.clone()));
        let first = {
            let mut map = self.aliases.lock().unwrap_or_else(|e| e.into_inner());
            let list = map.entry(canon.clone()).or_default();
            list.push(dir.to_path_buf());
            list.len() == 1
        };
        if first {
            let _ = self.watcher.watch(&canon, RecursiveMode::NonRecursive);
        }
    }

    pub fn unwatch(&mut self, dir: &Path) {
        let Some((count, _)) = self.counts.get_mut(dir) else {
            return;
        };
        *count -= 1;
        if *count > 0 {
            return;
        }
        let Some((_, canon)) = self.counts.remove(dir) else {
            return;
        };
        let last = {
            let mut map = self.aliases.lock().unwrap_or_else(|e| e.into_inner());
            match map.get_mut(&canon) {
                Some(list) => {
                    list.retain(|p| p != dir);
                    let empty = list.is_empty();
                    if empty {
                        map.remove(&canon);
                    }
                    empty
                }
                None => true,
            }
        };
        if last {
            let _ = self.watcher.unwatch(&canon);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn reports_changes_in_watched_dir() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let (tx, rx) = mpsc::channel();
        let mut w = DirWatcher::new(move |d| {
            let _ = tx.send(d);
        })
        .unwrap();
        w.watch(&root);
        std::thread::sleep(Duration::from_millis(200));
        std::fs::write(root.join("new.txt"), b"x").unwrap();
        let got = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("change event");
        assert_eq!(got, root);
        w.unwatch(&root);
        assert!(w.counts.is_empty());
    }

    #[test]
    fn reports_changes_under_the_path_the_tab_used() {
        // Watch through a symlink: events arrive for the canonical path but
        // must be reported for the requested one.
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::create_dir(root.join("real")).unwrap();
        std::os::unix::fs::symlink(root.join("real"), root.join("link")).unwrap();
        let (tx, rx) = mpsc::channel();
        let mut w = DirWatcher::new(move |d| {
            let _ = tx.send(d);
        })
        .unwrap();
        w.watch(&root.join("link"));
        std::thread::sleep(Duration::from_millis(200));
        std::fs::write(root.join("real/new.txt"), b"x").unwrap();
        let got = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("change event");
        assert_eq!(got, root.join("link"));
        w.unwatch(&root.join("link"));
        assert!(w.aliases.lock().unwrap().is_empty());
    }

    #[test]
    fn unwatch_after_the_folder_is_gone_cleans_up() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::create_dir(root.join("real")).unwrap();
        std::os::unix::fs::symlink(root.join("real"), root.join("link")).unwrap();
        let mut w = DirWatcher::new(|_| {}).unwrap();
        w.watch(&root.join("link"));
        // The symlink disappears before the tab stops watching it.
        std::fs::remove_file(root.join("link")).unwrap();
        w.unwatch(&root.join("link"));
        assert!(w.counts.is_empty());
        assert!(w.aliases.lock().unwrap().is_empty());
        // Watching the real folder later is a fresh, working watch.
        w.watch(&root.join("real"));
        assert_eq!(
            w.aliases.lock().unwrap().get(&root.join("real")),
            Some(&vec![root.join("real")])
        );
    }
}
