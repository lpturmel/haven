//! File operations with progress, conflict policies and undo records.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use regex::Regex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Conflict {
    Replace,
    KeepBoth,
    Skip,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpKind {
    Copy,
    Move,
    Trash,
}

impl OpKind {
    pub fn verb(self) -> &'static str {
        match self {
            OpKind::Copy => "Copying",
            OpKind::Move => "Moving",
            OpKind::Trash => "Moving to Trash",
        }
    }
}

/// Shared progress for a running job.
#[derive(Default, Debug)]
pub struct Progress {
    pub total_bytes: AtomicU64,
    pub done_bytes: AtomicU64,
    pub total_items: AtomicU64,
    pub done_items: AtomicU64,
    pub cancel: AtomicBool,
    pub finished: AtomicBool,
    pub current: Mutex<String>,
    pub error: Mutex<Option<String>>,
}

impl Progress {
    pub fn new() -> Arc<Progress> {
        Arc::new(Progress::default())
    }
    pub fn fraction(&self) -> f32 {
        let total = self.total_bytes.load(Ordering::Relaxed);
        if total == 0 {
            let items = self.total_items.load(Ordering::Relaxed).max(1);
            return self.done_items.load(Ordering::Relaxed) as f32 / items as f32;
        }
        self.done_bytes.load(Ordering::Relaxed) as f32 / total as f32
    }
    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
    fn set_current(&self, p: &Path) {
        if let Ok(mut c) = self.current.lock() {
            *c = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
        }
    }
}

/// What to do to reverse a completed operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Undo {
    /// Items were created (copy, new folder): undo moves them to the Trash.
    Created(Vec<PathBuf>),
    /// Items were moved/renamed `from -> to`: undo moves them back.
    Moved(Vec<(PathBuf, PathBuf)>),
    /// Items were trashed `original -> location in trash`: undo restores them.
    Trashed(Vec<(PathBuf, PathBuf)>),
}

impl Undo {
    pub fn describe(&self) -> String {
        match self {
            Undo::Created(v) => format!("Undo create ({} items)", v.len()),
            Undo::Moved(v) => format!("Undo move ({} items)", v.len()),
            Undo::Trashed(v) => format!("Undo trash ({} items)", v.len()),
        }
    }

    /// Reverses the operation. Returns the inverse so it can be redone.
    pub fn apply(&self) -> io::Result<Undo> {
        match self {
            Undo::Created(paths) => {
                let mut trashed = Vec::new();
                for p in paths {
                    let landed = haven_macos::trash(p).map_err(io::Error::other)?;
                    trashed.push((p.clone(), landed));
                }
                Ok(Undo::Trashed(trashed))
            }
            Undo::Moved(pairs) => {
                let mut back = Vec::new();
                for (from, to) in pairs.iter().rev() {
                    if from.exists() {
                        return Err(io::Error::new(
                            io::ErrorKind::AlreadyExists,
                            format!("{} already exists", from.display()),
                        ));
                    }
                    move_path(to, from)?;
                    back.push((to.clone(), from.clone()));
                }
                Ok(Undo::Moved(back))
            }
            Undo::Trashed(pairs) => {
                let mut restored = Vec::new();
                for (orig, landed) in pairs {
                    if orig.exists() {
                        return Err(io::Error::new(
                            io::ErrorKind::AlreadyExists,
                            format!("{} already exists", orig.display()),
                        ));
                    }
                    fs::rename(landed, orig)?;
                    restored.push(orig.clone());
                }
                Ok(Undo::Created(restored))
            }
        }
    }
}

/// Finder-style "keep both" name: `name copy.ext`, `name copy 2.ext`, ...
pub fn unique_name(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if fs::symlink_metadata(&candidate).is_err() {
        return candidate;
    }
    let p = Path::new(name);
    let (stem, ext) = match (p.file_stem(), p.extension()) {
        (Some(s), Some(e)) if !name.starts_with('.') || name.matches('.').count() > 1 => (
            s.to_string_lossy().into_owned(),
            format!(".{}", e.to_string_lossy()),
        ),
        _ => (name.to_string(), String::new()),
    };
    for n in 1.. {
        let suffix = if n == 1 {
            " copy".to_string()
        } else {
            format!(" copy {n}")
        };
        let c = dir.join(format!("{stem}{suffix}{ext}"));
        if fs::symlink_metadata(&c).is_err() {
            return c;
        }
    }
    unreachable!()
}

/// Top-level names in `sources` that already exist in `dest`.
pub fn conflicts(sources: &[PathBuf], dest: &Path) -> Vec<PathBuf> {
    sources
        .iter()
        .filter_map(|s| {
            let target = dest.join(s.file_name()?);
            (fs::symlink_metadata(&target).is_ok() && target != *s).then_some(target)
        })
        .collect()
}

fn tree_size(path: &Path) -> (u64, u64) {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return (0, 0);
    };
    if !meta.is_dir() {
        return (meta.len(), 1);
    }
    let mut bytes = 0;
    let mut items = 1;
    for e in jwalk::WalkDir::new(path)
        .skip_hidden(false)
        .follow_links(false)
        .into_iter()
        .flatten()
    {
        if let Ok(m) = e.metadata()
            && m.is_file()
        {
            bytes += m.len();
        }
        items += 1;
    }
    (bytes, items)
}

fn copy_tree(src: &Path, dst: &Path, progress: &Progress) -> io::Result<()> {
    if progress.cancelled() {
        return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
    }
    let meta = fs::symlink_metadata(src)?;
    progress.set_current(src);
    if meta.file_type().is_symlink() {
        let target = fs::read_link(src)?;
        std::os::unix::fs::symlink(target, dst)?;
    } else if meta.is_dir() {
        fs::create_dir(dst)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            copy_tree(&entry.path(), &dst.join(entry.file_name()), progress)?;
        }
        fs::set_permissions(dst, meta.permissions())?;
    } else {
        // std::fs::copy uses clonefile/fcopyfile on macOS: instant on APFS.
        fs::copy(src, dst)?;
        progress.done_bytes.fetch_add(meta.len(), Ordering::Relaxed);
    }
    progress.done_items.fetch_add(1, Ordering::Relaxed);
    Ok(())
}

fn remove_any(path: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

/// Moves `from` to `to`, falling back to copy+delete across volumes.
pub fn move_path(from: &Path, to: &Path) -> io::Result<()> {
    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(e) if e.raw_os_error() == Some(libc::EXDEV) => {
            copy_tree(from, to, &Progress::default())?;
            remove_any(from)
        }
        Err(e) => Err(e),
    }
}

/// Copies or moves `sources` into `dest`. Returns an undo record.
pub fn transfer(
    kind: OpKind,
    sources: &[PathBuf],
    dest: &Path,
    policy: Conflict,
    progress: &Progress,
) -> io::Result<Undo> {
    let (mut bytes, mut items) = (0, 0);
    for s in sources {
        let (b, i) = tree_size(s);
        bytes += b;
        items += i;
    }
    progress.total_bytes.store(bytes, Ordering::Relaxed);
    progress.total_items.store(items, Ordering::Relaxed);

    let mut created = Vec::new();
    let mut moved = Vec::new();
    let result = (|| {
        for src in sources {
            let Some(name) = src.file_name() else {
                continue;
            };
            let mut target = dest.join(name);
            let same = target == *src;
            if same && kind == OpKind::Move {
                continue;
            }
            if fs::symlink_metadata(&target).is_ok() {
                match policy {
                    Conflict::Skip if !same => continue,
                    Conflict::Replace if !same => {
                        if dest.starts_with(src) {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidInput,
                                "cannot replace a folder with itself",
                            ));
                        }
                        let landed = haven_macos::trash(&target).map_err(io::Error::other)?;
                        let _ = landed;
                    }
                    _ => target = unique_name(dest, &name.to_string_lossy()),
                }
            }
            if dest.starts_with(src) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "cannot {} a folder into itself",
                        if kind == OpKind::Copy { "copy" } else { "move" }
                    ),
                ));
            }
            match kind {
                OpKind::Copy => {
                    copy_tree(src, &target, progress)?;
                    created.push(target);
                }
                OpKind::Move => {
                    progress.set_current(src);
                    move_path(src, &target)?;
                    let (b, i) = tree_size(&target);
                    progress.done_bytes.fetch_add(b, Ordering::Relaxed);
                    progress.done_items.fetch_add(i, Ordering::Relaxed);
                    moved.push((src.clone(), target));
                }
                OpKind::Trash => unreachable!(),
            }
        }
        Ok(())
    })();
    progress.finished.store(true, Ordering::Relaxed);
    if let Err(e) = &result
        && let Ok(mut err) = progress.error.lock()
    {
        *err = Some(e.to_string());
    }
    let undo = if kind == OpKind::Copy {
        Undo::Created(created)
    } else {
        Undo::Moved(moved)
    };
    result.map(|_| undo)
}

/// Moves items to the Trash.
pub fn trash(paths: &[PathBuf], progress: &Progress) -> io::Result<Undo> {
    progress
        .total_items
        .store(paths.len() as u64, Ordering::Relaxed);
    let mut done = Vec::new();
    for p in paths {
        if progress.cancelled() {
            break;
        }
        progress.set_current(p);
        let landed = haven_macos::trash(p).map_err(io::Error::other)?;
        done.push((p.clone(), landed));
        progress.done_items.fetch_add(1, Ordering::Relaxed);
    }
    progress.finished.store(true, Ordering::Relaxed);
    Ok(Undo::Trashed(done))
}

/// Creates "untitled folder" (or a numbered variant) in `dir`.
pub fn new_folder(dir: &Path) -> io::Result<PathBuf> {
    let mut path = dir.join("untitled folder");
    let mut n = 2;
    while fs::symlink_metadata(&path).is_ok() {
        path = dir.join(format!("untitled folder {n}"));
        n += 1;
    }
    fs::create_dir(&path)?;
    Ok(path)
}

pub fn validate_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("Name can't be empty".into());
    }
    if name.contains('/') {
        return Err("Name can't contain “/”".into());
    }
    if name == "." || name == ".." {
        return Err("That name is reserved".into());
    }
    Ok(())
}

pub fn rename(path: &Path, new_name: &str) -> io::Result<Undo> {
    validate_name(new_name).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let target = path.with_file_name(new_name);
    if target == path {
        return Ok(Undo::Moved(vec![]));
    }
    let case_only =
        target.to_string_lossy().to_lowercase() == path.to_string_lossy().to_lowercase();
    if !case_only && fs::symlink_metadata(&target).is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("“{new_name}” already exists"),
        ));
    }
    fs::rename(path, &target)?;
    Ok(Undo::Moved(vec![(path.to_path_buf(), target)]))
}

#[derive(Clone, Debug, Default)]
pub struct BulkRename {
    pub find: String,
    pub replace: String,
    pub regex: bool,
    pub prefix: String,
    pub suffix: String,
    /// When set, appends ` {n}` numbering starting at this value.
    pub number_from: Option<u32>,
    pub case: CaseChange,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CaseChange {
    #[default]
    Keep,
    Lower,
    Upper,
}

impl BulkRename {
    /// Computes new names (stem transformations keep the extension).
    pub fn preview(&self, names: &[String]) -> Result<Vec<String>, String> {
        let re = if self.find.is_empty() {
            None
        } else if self.regex {
            Some(Regex::new(&self.find).map_err(|e| e.to_string())?)
        } else {
            Some(Regex::new(&regex::escape(&self.find)).unwrap())
        };
        Ok(names
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let p = Path::new(name);
                let (stem, ext) = match (p.file_stem(), p.extension()) {
                    (Some(s), Some(e)) if !name.starts_with('.') => (
                        s.to_string_lossy().into_owned(),
                        format!(".{}", e.to_string_lossy()),
                    ),
                    _ => (name.clone(), String::new()),
                };
                let mut stem = match &re {
                    Some(re) => re.replace_all(&stem, self.replace.as_str()).into_owned(),
                    None => stem,
                };
                stem = match self.case {
                    CaseChange::Keep => stem,
                    CaseChange::Lower => stem.to_lowercase(),
                    CaseChange::Upper => stem.to_uppercase(),
                };
                let num = self
                    .number_from
                    .map(|n| format!(" {}", n as usize + i))
                    .unwrap_or_default();
                format!("{}{stem}{}{num}{ext}", self.prefix, self.suffix)
            })
            .collect())
    }

    /// Applies the rename to `paths`, refusing duplicate or clashing targets.
    pub fn apply(&self, paths: &[PathBuf]) -> io::Result<Undo> {
        let names: Vec<String> = paths
            .iter()
            .map(|p| {
                p.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            })
            .collect();
        let new = self
            .preview(&names)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        let mut seen = std::collections::HashSet::new();
        for (p, n) in paths.iter().zip(&new) {
            validate_name(n).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
            let t = p.with_file_name(n);
            if !seen.insert(t.clone()) {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("two items would be named “{n}”"),
                ));
            }
            if t != *p && !paths.contains(&t) && fs::symlink_metadata(&t).is_ok() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("“{n}” already exists"),
                ));
            }
        }
        // Two-phase via temporary names so swaps (a->b, b->a) work.
        let mut temps = Vec::new();
        for (i, p) in paths.iter().enumerate() {
            let tmp = p.with_file_name(format!(".haven-rename-{}-{i}", std::process::id()));
            fs::rename(p, &tmp)?;
            temps.push(tmp);
        }
        let mut moved = Vec::new();
        for ((orig, tmp), n) in paths.iter().zip(&temps).zip(&new) {
            let t = orig.with_file_name(n);
            fs::rename(tmp, &t)?;
            moved.push((orig.clone(), t));
        }
        Ok(Undo::Moved(moved))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        (dir, root)
    }

    #[test]
    fn unique_names_follow_finder() {
        let (_d, root) = setup();
        fs::write(root.join("a.txt"), b"").unwrap();
        assert_eq!(unique_name(&root, "a.txt"), root.join("a copy.txt"));
        fs::write(root.join("a copy.txt"), b"").unwrap();
        assert_eq!(unique_name(&root, "a.txt"), root.join("a copy 2.txt"));
        fs::create_dir(root.join("dir")).unwrap();
        assert_eq!(unique_name(&root, "dir"), root.join("dir copy"));
        assert_eq!(unique_name(&root, "new"), root.join("new"));
    }

    #[test]
    fn copy_tree_with_conflicts_and_undo() {
        let (_d, root) = setup();
        fs::create_dir_all(root.join("src/sub")).unwrap();
        fs::write(root.join("src/sub/f.txt"), b"hello").unwrap();
        fs::write(root.join("x.txt"), b"x").unwrap();
        fs::create_dir(root.join("dst")).unwrap();
        fs::write(root.join("dst/x.txt"), b"old").unwrap();

        let p = Progress::new();
        let sources = vec![root.join("src"), root.join("x.txt")];
        assert_eq!(
            conflicts(&sources, &root.join("dst")),
            vec![root.join("dst/x.txt")]
        );
        let undo = transfer(
            OpKind::Copy,
            &sources,
            &root.join("dst"),
            Conflict::KeepBoth,
            &p,
        )
        .unwrap();
        assert_eq!(fs::read(root.join("dst/src/sub/f.txt")).unwrap(), b"hello");
        assert_eq!(fs::read(root.join("dst/x.txt")).unwrap(), b"old");
        assert_eq!(fs::read(root.join("dst/x copy.txt")).unwrap(), b"x");
        assert!((p.fraction() - 1.0).abs() < 1e-6);
        assert_eq!(
            undo,
            Undo::Created(vec![root.join("dst/src"), root.join("dst/x copy.txt")])
        );

        let p = Progress::new();
        transfer(
            OpKind::Copy,
            &[root.join("x.txt")],
            &root.join("dst"),
            Conflict::Skip,
            &p,
        )
        .unwrap();
        assert!(!root.join("dst/x copy 2.txt").exists());
    }

    #[test]
    fn duplicate_in_same_dir_keeps_both() {
        let (_d, root) = setup();
        fs::write(root.join("a.txt"), b"a").unwrap();
        transfer(
            OpKind::Copy,
            &[root.join("a.txt")],
            &root,
            Conflict::Replace,
            &Progress::default(),
        )
        .unwrap();
        assert!(root.join("a copy.txt").exists());
    }

    #[test]
    fn move_and_undo_move() {
        let (_d, root) = setup();
        fs::write(root.join("a.txt"), b"a").unwrap();
        fs::create_dir(root.join("d")).unwrap();
        let undo = transfer(
            OpKind::Move,
            &[root.join("a.txt")],
            &root.join("d"),
            Conflict::KeepBoth,
            &Progress::default(),
        )
        .unwrap();
        assert!(root.join("d/a.txt").exists() && !root.join("a.txt").exists());
        let redo = undo.apply().unwrap();
        assert!(root.join("a.txt").exists() && !root.join("d/a.txt").exists());
        redo.apply().unwrap();
        assert!(root.join("d/a.txt").exists());
    }

    #[test]
    fn refuses_copy_into_itself() {
        let (_d, root) = setup();
        fs::create_dir_all(root.join("a/b")).unwrap();
        let err = transfer(
            OpKind::Copy,
            &[root.join("a")],
            &root.join("a/b"),
            Conflict::KeepBoth,
            &Progress::default(),
        );
        assert!(err.is_err());
    }

    #[test]
    fn rename_validates_and_undoes() {
        let (_d, root) = setup();
        fs::write(root.join("a.txt"), b"").unwrap();
        fs::write(root.join("b.txt"), b"").unwrap();
        assert!(rename(&root.join("a.txt"), "b.txt").is_err());
        assert!(rename(&root.join("a.txt"), "x/y").is_err());
        let undo = rename(&root.join("a.txt"), "c.txt").unwrap();
        assert!(root.join("c.txt").exists());
        undo.apply().unwrap();
        assert!(root.join("a.txt").exists());
        rename(&root.join("a.txt"), "A.txt").unwrap();
    }

    #[test]
    fn bulk_rename_preview_and_apply() {
        let (_d, root) = setup();
        for n in ["IMG_001.jpg", "IMG_002.jpg"] {
            fs::write(root.join(n), b"").unwrap();
        }
        let br = BulkRename {
            find: r"IMG_(\d+)".into(),
            replace: "photo-$1".into(),
            regex: true,
            prefix: "trip ".into(),
            ..Default::default()
        };
        let names = vec!["IMG_001.jpg".to_string(), "IMG_002.jpg".to_string()];
        assert_eq!(
            br.preview(&names).unwrap(),
            ["trip photo-001.jpg", "trip photo-002.jpg"]
        );

        let numbered = BulkRename {
            number_from: Some(1),
            case: CaseChange::Lower,
            ..Default::default()
        };
        assert_eq!(
            numbered.preview(&names).unwrap(),
            ["img_001 1.jpg", "img_002 2.jpg"]
        );

        let paths = vec![root.join("IMG_001.jpg"), root.join("IMG_002.jpg")];
        let undo = br.apply(&paths).unwrap();
        assert!(root.join("trip photo-001.jpg").exists());
        undo.apply().unwrap();
        assert!(root.join("IMG_001.jpg").exists());

        let clash = BulkRename {
            find: r"\d+".into(),
            replace: "x".into(),
            regex: true,
            ..Default::default()
        };
        assert!(clash.apply(&paths).is_err());
        assert!(root.join("IMG_001.jpg").exists());
    }

    #[test]
    fn new_folder_numbers() {
        let (_d, root) = setup();
        assert_eq!(new_folder(&root).unwrap(), root.join("untitled folder"));
        assert_eq!(new_folder(&root).unwrap(), root.join("untitled folder 2"));
    }
}
