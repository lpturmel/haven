//! Directory watching for open tabs (non-recursive; FSEvents on macOS).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

/// Watches a set of directories and reports which one changed.
pub struct DirWatcher {
    watcher: RecommendedWatcher,
    counts: HashMap<PathBuf, usize>,
}

impl DirWatcher {
    /// `on_change` receives the watched directory whose contents changed. It is
    /// called from a background thread and should be cheap (e.g. send on a channel).
    pub fn new(on_change: impl Fn(PathBuf) + Send + 'static) -> notify::Result<DirWatcher> {
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
            dirs.dedup();
            for d in dirs {
                on_change(d);
            }
        })?;
        Ok(DirWatcher {
            watcher,
            counts: HashMap::new(),
        })
    }

    /// Reference-counted watch so several tabs can share a directory.
    pub fn watch(&mut self, dir: &Path) {
        let count = self.counts.entry(dir.to_path_buf()).or_insert(0);
        if *count == 0 {
            let _ = self.watcher.watch(dir, RecursiveMode::NonRecursive);
        }
        *count += 1;
    }

    pub fn unwatch(&mut self, dir: &Path) {
        if let Some(count) = self.counts.get_mut(dir) {
            *count -= 1;
            if *count == 0 {
                self.counts.remove(dir);
                let _ = self.watcher.unwatch(dir);
            }
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
}
