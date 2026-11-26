//! Recursive directory sizes, computed in parallel with cancellation.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Shared cancel flag + progress counters for long-running scans.
#[derive(Default, Debug)]
pub struct Control {
    pub cancel: AtomicBool,
    pub files: AtomicU64,
    pub bytes: AtomicU64,
}

impl Control {
    pub fn new() -> Arc<Control> {
        Arc::new(Control::default())
    }
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// Total logical size of all files below `root` (symlinks are not followed).
/// Returns `None` when cancelled.
pub fn dir_size(root: &Path, control: &Control) -> Option<u64> {
    let meta = std::fs::symlink_metadata(root).ok()?;
    if !meta.is_dir() {
        return Some(meta.len());
    }
    let mut total = 0u64;
    for entry in jwalk::WalkDir::new(root)
        .skip_hidden(false)
        .follow_links(false)
    {
        if control.is_cancelled() {
            return None;
        }
        let Ok(entry) = entry else { continue };
        if entry.file_type().is_file()
            && let Ok(m) = entry.metadata()
        {
            total += m.len();
            control.files.fetch_add(1, Ordering::Relaxed);
            control.bytes.fetch_add(m.len(), Ordering::Relaxed);
        }
    }
    Some(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sums_nested_files_and_cancels() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("a/b")).unwrap();
        std::fs::write(dir.path().join("x"), vec![0u8; 100]).unwrap();
        std::fs::write(dir.path().join("a/b/y"), vec![0u8; 23]).unwrap();
        std::fs::write(dir.path().join("a/.z"), vec![0u8; 7]).unwrap();
        let c = Control::new();
        assert_eq!(dir_size(dir.path(), &c), Some(130));
        assert_eq!(c.files.load(Ordering::Relaxed), 3);

        let c = Control::new();
        c.cancel();
        assert_eq!(dir_size(dir.path(), &c), None);
    }
}
