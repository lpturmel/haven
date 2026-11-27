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
    /// Several steps, undone last-first (e.g. "trash the replaced item, then copy").
    Batch(Vec<Undo>),
}

/// An operation that failed part-way. `undo` reverses the part that succeeded.
#[derive(Debug)]
pub struct OpError {
    pub error: io::Error,
    pub undo: Undo,
}

impl std::fmt::Display for OpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.error.fmt(f)
    }
}

impl std::error::Error for OpError {}

pub type OpResult = Result<Undo, OpError>;

fn plural(n: usize) -> String {
    if n == 1 {
        "1 item".into()
    } else {
        format!("{n} items")
    }
}

impl Undo {
    pub fn describe(&self) -> String {
        match self {
            Undo::Created(v) => format!("Undid create ({})", plural(v.len())),
            Undo::Moved(v) => format!("Undid move ({})", plural(v.len())),
            Undo::Trashed(v) => format!("Restored from Trash ({})", plural(v.len())),
            // A Replace conflict is recorded as [trashed originals, new items].
            Undo::Batch(steps) => match steps.as_slice() {
                [Undo::Trashed(old), Undo::Created(_) | Undo::Moved(_)] if !old.is_empty() => {
                    format!("Undid replace ({})", plural(old.len()))
                }
                _ => steps
                    .iter()
                    .rev()
                    .find(|s| !s.is_empty())
                    .map(Undo::describe)
                    .unwrap_or_else(|| "Undid".into()),
            },
        }
    }

    /// Whether undoing would do nothing.
    pub fn is_empty(&self) -> bool {
        match self {
            Undo::Created(v) => v.is_empty(),
            Undo::Moved(v) => v.is_empty(),
            Undo::Trashed(v) => v.is_empty(),
            Undo::Batch(steps) => steps.iter().all(Undo::is_empty),
        }
    }

    /// Reverses the operation. Returns the inverse so it can be redone.
    pub fn apply(&self) -> io::Result<Undo> {
        match self {
            // Every variant is all-or-nothing: on failure, whatever was already
            // reversed is put back, so the record stays valid to retry.
            Undo::Created(paths) => {
                let mut trashed: Vec<(PathBuf, PathBuf)> = Vec::new();
                for p in paths {
                    match haven_macos::trash(p) {
                        Ok(landed) => trashed.push((p.clone(), landed)),
                        Err(e) => {
                            for (orig, landed) in trashed.iter().rev() {
                                let _ = fs::rename(landed, orig);
                            }
                            return Err(io::Error::other(e));
                        }
                    }
                }
                Ok(Undo::Trashed(trashed))
            }
            Undo::Moved(pairs) => undo_moves(pairs),
            Undo::Trashed(pairs) => {
                if let Some((orig, _)) = pairs.iter().find(|(orig, _)| orig.exists()) {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        format!("{} already exists", orig.display()),
                    ));
                }
                let mut restored: Vec<(PathBuf, PathBuf)> = Vec::new();
                for (orig, landed) in pairs {
                    if let Err(e) = fs::rename(landed, orig) {
                        for (orig, landed) in restored.iter().rev() {
                            let _ = fs::rename(orig, landed);
                        }
                        return Err(e);
                    }
                    restored.push((orig.clone(), landed.clone()));
                }
                Ok(Undo::Created(
                    restored.into_iter().map(|(o, _)| o).collect(),
                ))
            }
            Undo::Batch(steps) => {
                let mut inverse: Vec<Undo> = Vec::new();
                for step in steps.iter().rev() {
                    match step.apply() {
                        Ok(inv) => inverse.push(inv),
                        Err(e) => {
                            // Redo the steps already undone, newest first.
                            for inv in inverse.iter().rev() {
                                let _ = inv.apply();
                            }
                            return Err(e);
                        }
                    }
                }
                Ok(Undo::Batch(inverse))
            }
        }
    }
}

/// A hidden temporary name unique across this process: concurrent operations
/// (two undos, an undo and a bulk rename) must never pick the same one, since
/// `rename` silently replaces an existing file.
fn temp_name(kind: &str) -> String {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    format!(".haven-{kind}-{}-{n}", std::process::id())
}

/// Reverses `(from, to)` moves: everything at `to` goes back to `from`.
///
/// Two-phase through temporary names, so overlapping records (a bulk rename
/// `f 1 -> f 2, f 2 -> f 3`, or a swap) undo cleanly. A `from` is only a
/// conflict if something else occupies it: another entry's `to` that is about
/// to move away, or the same file (case-only rename), is fine.
fn undo_moves(pairs: &[(PathBuf, PathBuf)]) -> io::Result<Undo> {
    let leaving: std::collections::HashSet<&PathBuf> = pairs.iter().map(|(_, to)| to).collect();
    for (from, to) in pairs {
        let same = match (fs::symlink_metadata(from), fs::symlink_metadata(to)) {
            (Ok(a), Ok(b)) => same_file(&a, &b),
            _ => false,
        };
        if fs::symlink_metadata(from).is_ok() && !same && !leaving.contains(from) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{} already exists", from.display()),
            ));
        }
    }
    let temps: Vec<PathBuf> = pairs
        .iter()
        .map(|(_, to)| to.with_file_name(temp_name("undo")))
        .collect();
    for (i, ((_, to), tmp)) in pairs.iter().zip(&temps).enumerate() {
        if let Err(e) = fs::rename(to, tmp) {
            for j in (0..i).rev() {
                let _ = fs::rename(&temps[j], &pairs[j].1);
            }
            return Err(e);
        }
    }
    for (i, ((from, _), tmp)) in pairs.iter().zip(&temps).enumerate() {
        if let Err(e) = move_path(tmp, from, &Progress::default()) {
            for j in (0..i).rev() {
                let _ = move_path(&pairs[j].0, &temps[j], &Progress::default());
            }
            for j in 0..pairs.len() {
                let _ = fs::rename(&temps[j], &pairs[j].1);
            }
            return Err(e);
        }
    }
    Ok(Undo::Moved(
        pairs
            .iter()
            .map(|(from, to)| (to.clone(), from.clone()))
            .collect(),
    ))
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
    // jwalk yields the root itself, so it is counted by the loop.
    let mut items = 0;
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

/// Moves `from` to `to`, falling back to copy+delete across volumes. A failed
/// or cancelled cross-volume copy removes its partial output and leaves
/// `from` untouched.
pub fn move_path(from: &Path, to: &Path, progress: &Progress) -> io::Result<()> {
    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(e) if e.raw_os_error() == Some(libc::EXDEV) => {
            if let Err(e) = copy_tree(from, to, progress) {
                let _ = remove_any(to);
                return Err(e);
            }
            remove_any(from)
        }
        Err(e) => Err(e),
    }
}

/// Resolves symlinks (`/tmp` → `/private/tmp`) so two spellings of one
/// folder compare equal. Falls back to the path as given when it can't be
/// resolved (it may not exist yet).
fn resolved(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Like `resolved`, but a symlink stays itself: only its parent folder is
/// resolved. Moving or copying a link moves the link, not what it points to.
fn resolved_item(path: &Path) -> PathBuf {
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => resolved(parent).join(name),
        _ => resolved(path),
    }
}

/// The first of `sources` that `dest` is, or lies inside, once symlinks are
/// resolved. Moving or copying it into `dest` would put a folder inside
/// itself. A dragged symlink counts as the link, not its target. Touches the
/// filesystem: call it off the UI thread.
pub fn source_containing(sources: &[PathBuf], dest: &Path) -> Option<PathBuf> {
    let dest = resolved(dest);
    sources
        .iter()
        .find(|s| dest.starts_with(resolved_item(s)))
        .cloned()
}

/// Copies or moves `sources` into `dest`. Returns an undo record; on failure
/// the error carries an undo record for the items that were already done.
pub fn transfer(
    kind: OpKind,
    sources: &[PathBuf],
    dest: &Path,
    policy: Conflict,
    progress: &Progress,
) -> OpResult {
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
    let mut replaced = Vec::new();
    let result = (|| {
        // Checked for every source before anything moves, so a bad drop
        // fails whole instead of half-done.
        if source_containing(sources, dest).is_some() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "cannot {} a folder into itself",
                    if kind == OpKind::Copy { "copy" } else { "move" }
                ),
            ));
        }
        for src in sources {
            if progress.cancelled() {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
            }
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
                        // Never trash the source itself or a folder holding it
                        // (moving `/x/a/a` into `/x` would trash `/x/a`).
                        if dest.starts_with(src) || src.starts_with(&target) {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidInput,
                                "cannot replace a folder with itself",
                            ));
                        }
                        let landed = haven_macos::trash(&target).map_err(io::Error::other)?;
                        replaced.push((target.clone(), landed));
                    }
                    _ => target = unique_name(dest, &name.to_string_lossy()),
                }
            }
            match kind {
                OpKind::Copy => {
                    if let Err(e) = copy_tree(src, &target, progress) {
                        // The partial copy is ours (the target was free or
                        // already trashed): don't leave it behind.
                        let _ = remove_any(&target);
                        return Err(e);
                    }
                    created.push(target);
                }
                OpKind::Move => {
                    progress.set_current(src);
                    move_path(src, &target, progress)?;
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
    let main = if kind == OpKind::Copy {
        Undo::Created(created)
    } else {
        Undo::Moved(moved)
    };
    // Undo runs last-first: remove the new items, then restore what they replaced.
    let undo = if replaced.is_empty() {
        main
    } else {
        Undo::Batch(vec![Undo::Trashed(replaced), main])
    };
    match result {
        Ok(()) => Ok(undo),
        Err(error) => Err(OpError { error, undo }),
    }
}

/// Moves items to the Trash. On failure the error still carries the undo
/// record for the items that were trashed.
pub fn trash(paths: &[PathBuf], progress: &Progress) -> OpResult {
    progress
        .total_items
        .store(paths.len() as u64, Ordering::Relaxed);
    let mut done = Vec::new();
    for p in paths {
        if progress.cancelled() {
            break;
        }
        progress.set_current(p);
        match haven_macos::trash(p) {
            Ok(landed) => done.push((p.clone(), landed)),
            Err(e) => {
                progress.finished.store(true, Ordering::Relaxed);
                return Err(OpError {
                    error: io::Error::other(e),
                    undo: Undo::Trashed(done),
                });
            }
        }
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
    // On case-insensitive volumes `a` → `A` resolves to the same file: allow it.
    // Anything else that already exists (including a different file that only
    // differs by case on a case-sensitive volume) is refused.
    if let Ok(existing) = fs::symlink_metadata(&target)
        && !same_file(&existing, &fs::symlink_metadata(path)?)
    {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("“{new_name}” already exists"),
        ));
    }
    fs::rename(path, &target)?;
    Ok(Undo::Moved(vec![(path.to_path_buf(), target)]))
}

fn same_file(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    a.dev() == b.dev() && a.ino() == b.ino()
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
            // On a case-insensitive volume a case change "exists" as the
            // same file: that is not a clash.
            let same = match (fs::symlink_metadata(&t), fs::symlink_metadata(p)) {
                (Ok(a), Ok(b)) => same_file(&a, &b),
                _ => false,
            };
            if t != *p && !same && !paths.contains(&t) && fs::symlink_metadata(&t).is_ok() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("“{n}” already exists"),
                ));
            }
        }
        // Two-phase via temporary names so swaps (a->b, b->a) work. Any
        // failure rolls everything back to the original names.
        let targets: Vec<PathBuf> = paths
            .iter()
            .zip(&new)
            .map(|(p, n)| p.with_file_name(n))
            .collect();
        self.apply_two_phase(paths, &targets, &mut |from, to| fs::rename(from, to))
    }

    fn apply_two_phase(
        &self,
        paths: &[PathBuf],
        targets: &[PathBuf],
        rename: &mut dyn FnMut(&Path, &Path) -> io::Result<()>,
    ) -> io::Result<Undo> {
        let temps: Vec<PathBuf> = paths
            .iter()
            .map(|p| p.with_file_name(temp_name("rename")))
            .collect();
        for (i, (p, tmp)) in paths.iter().zip(&temps).enumerate() {
            if let Err(e) = rename(p, tmp) {
                for j in (0..i).rev() {
                    let _ = fs::rename(&temps[j], &paths[j]);
                }
                return Err(e);
            }
        }
        for (i, (tmp, t)) in temps.iter().zip(targets).enumerate() {
            if let Err(e) = rename(tmp, t) {
                for j in (0..i).rev() {
                    let _ = fs::rename(&targets[j], &temps[j]);
                }
                for (tmp, orig) in temps.iter().zip(paths) {
                    let _ = fs::rename(tmp, orig);
                }
                return Err(e);
            }
        }
        Ok(Undo::Moved(
            paths.iter().cloned().zip(targets.iter().cloned()).collect(),
        ))
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
    fn finds_a_source_holding_the_destination_through_symlinks() {
        let (_d, root) = setup();
        fs::create_dir_all(root.join("a/b")).unwrap();
        fs::create_dir(root.join("c")).unwrap();
        // `alias` plays `/tmp`, `root` plays `/private/tmp`.
        std::os::unix::fs::symlink(&root, root.join("alias")).unwrap();
        let a = root.join("alias/a");
        // Equal, under another spelling.
        assert_eq!(
            source_containing(std::slice::from_ref(&a), &root.join("a")),
            Some(a.clone())
        );
        // Inside it, under another spelling, either way round.
        assert_eq!(
            source_containing(std::slice::from_ref(&a), &root.join("a/b")),
            Some(a.clone())
        );
        assert_eq!(
            source_containing(&[root.join("a")], &root.join("alias/a/b")),
            Some(root.join("a"))
        );
        // Not just the first source.
        assert_eq!(
            source_containing(&[root.join("c"), a.clone()], &root.join("a/b")),
            Some(a.clone())
        );
        // A sibling, or a name that merely shares a prefix, is fine.
        fs::create_dir(root.join("ab")).unwrap();
        assert_eq!(
            source_containing(&[root.join("c")], &root.join("a/b")),
            None
        );
        assert_eq!(source_containing(&[a], &root.join("ab")), None);
        // A destination that doesn't exist yet is compared as given.
        assert_eq!(
            source_containing(&[root.join("a")], &root.join("a/new")),
            Some(root.join("a"))
        );
    }

    #[test]
    fn a_symlink_moves_into_the_folder_it_points_to() {
        let (_d, root) = setup();
        fs::create_dir_all(root.join("a/b")).unwrap();
        fs::create_dir(root.join("links")).unwrap();
        std::os::unix::fs::symlink(root.join("a"), root.join("links/to_a")).unwrap();
        let link = root.join("links/to_a");
        for dest in [root.join("a"), root.join("a/b")] {
            assert_eq!(source_containing(std::slice::from_ref(&link), &dest), None);
        }
        transfer(
            OpKind::Move,
            std::slice::from_ref(&link),
            &root.join("a/b"),
            Conflict::KeepBoth,
            &Progress::default(),
        )
        .unwrap();
        let moved = root.join("a/b/to_a");
        assert!(
            fs::symlink_metadata(&moved)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(fs::symlink_metadata(&link).is_err());
        // Parents still resolve: a real folder dragged by an aliased path is
        // still caught.
        std::os::unix::fs::symlink(&root, root.join("alias")).unwrap();
        assert_eq!(
            source_containing(&[root.join("alias/a")], &root.join("a/b")),
            Some(root.join("alias/a"))
        );
    }

    #[test]
    fn moving_a_folder_into_itself_moves_nothing() {
        let (_d, root) = setup();
        fs::create_dir_all(root.join("pinA/sub")).unwrap();
        fs::write(root.join("five.txt"), b"5").unwrap();
        std::os::unix::fs::symlink(&root, root.join("alias")).unwrap();
        // The file comes first, so a per-item check would move it before
        // failing on the folder.
        let r = transfer(
            OpKind::Move,
            &[root.join("alias/five.txt"), root.join("alias/pinA")],
            &root.join("pinA/sub"),
            Conflict::KeepBoth,
            &Progress::default(),
        );
        assert!(r.is_err());
        assert!(root.join("five.txt").exists());
        assert!(!root.join("pinA/sub/five.txt").exists());
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
    fn replace_is_undoable_and_restores_the_overwritten_item() {
        let (_d, root) = setup();
        fs::write(root.join("a.txt"), b"new").unwrap();
        fs::create_dir(root.join("d")).unwrap();
        fs::write(root.join("d/a.txt"), b"old").unwrap();
        let undo = match transfer(
            OpKind::Copy,
            &[root.join("a.txt")],
            &root.join("d"),
            Conflict::Replace,
            &Progress::default(),
        ) {
            Ok(u) => u,
            // Volumes without a Trash (sandboxed CI) cannot replace.
            Err(_) => return,
        };
        assert_eq!(fs::read(root.join("d/a.txt")).unwrap(), b"new");
        assert!(matches!(undo, Undo::Batch(_)));
        assert_eq!(undo.describe(), "Undid replace (1 item)");
        undo.apply().unwrap();
        assert_eq!(fs::read(root.join("d/a.txt")).unwrap(), b"old");
    }

    #[test]
    fn partial_failure_keeps_undo_for_completed_items() {
        let (_d, root) = setup();
        fs::write(root.join("ok.txt"), b"x").unwrap();
        fs::create_dir(root.join("d")).unwrap();
        let missing = root.join("missing.txt");
        let err = transfer(
            OpKind::Move,
            &[root.join("ok.txt"), missing],
            &root.join("d"),
            Conflict::KeepBoth,
            &Progress::default(),
        )
        .unwrap_err();
        assert_eq!(
            err.undo,
            Undo::Moved(vec![(root.join("ok.txt"), root.join("d/ok.txt"))])
        );
        err.undo.apply().unwrap();
        assert!(root.join("ok.txt").exists());
    }

    #[test]
    fn trash_then_undo_restores() {
        let (_d, root) = setup();
        fs::write(root.join("t.txt"), b"x").unwrap();
        let undo = match trash(&[root.join("t.txt")], &Progress::default()) {
            Ok(u) => u,
            Err(_) => return,
        };
        assert!(!root.join("t.txt").exists());
        undo.apply().unwrap();
        assert!(root.join("t.txt").exists());
        assert_eq!(undo.describe(), "Restored from Trash (1 item)");
    }

    #[test]
    fn rename_refuses_to_clobber_a_different_file() {
        let (_d, root) = setup();
        fs::write(root.join("a"), b"1").unwrap();
        fs::write(root.join("b"), b"2").unwrap();
        assert!(rename(&root.join("a"), "b").is_err());
        assert_eq!(fs::read(root.join("b")).unwrap(), b"2");
        // Case-only rename of the same file is fine.
        rename(&root.join("a"), "A").unwrap();
    }

    #[test]
    fn bulk_rename_rolls_back_on_failure() {
        let (_d, root) = setup();
        for n in ["one", "two", "three"] {
            fs::write(root.join(n), n.as_bytes()).unwrap();
        }
        let paths: Vec<PathBuf> = ["one", "two", "three"]
            .iter()
            .map(|n| root.join(n))
            .collect();
        let targets: Vec<PathBuf> = ["1", "2", "3"].iter().map(|n| root.join(n)).collect();
        let br = BulkRename::default();
        let mut calls = 0;
        // Fail on the second rename of phase 2 (call index 4 of 6).
        let res = br.apply_two_phase(&paths, &targets, &mut |from, to| {
            calls += 1;
            if calls == 5 {
                Err(io::Error::other("boom"))
            } else {
                fs::rename(from, to)
            }
        });
        assert!(res.is_err());
        for (p, n) in paths.iter().zip(["one", "two", "three"]) {
            assert_eq!(fs::read(p).unwrap(), n.as_bytes(), "{p:?} restored");
        }
        assert_eq!(
            fs::read_dir(&root).unwrap().count(),
            3,
            "no temp files left"
        );
    }

    #[test]
    fn new_folder_numbers() {
        let (_d, root) = setup();
        assert_eq!(new_folder(&root).unwrap(), root.join("untitled folder"));
        assert_eq!(new_folder(&root).unwrap(), root.join("untitled folder 2"));
    }

    #[test]
    fn case_only_rename_and_its_undo() {
        let (_d, root) = setup();
        fs::write(root.join("a.txt"), b"a").unwrap();
        let undo = rename(&root.join("a.txt"), "A.txt").unwrap();
        let names: Vec<String> = fs::read_dir(&root)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["A.txt"]);
        undo.apply().unwrap();
        let names: Vec<String> = fs::read_dir(&root)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["a.txt"]);
    }

    #[test]
    fn undo_of_swapped_and_chained_moves() {
        let (_d, root) = setup();
        let p = |n: &str| root.join(n);
        // A swap a <-> b already happened: b holds A, a holds B.
        fs::write(p("b.txt"), b"A").unwrap();
        fs::write(p("a.txt"), b"B").unwrap();
        let swap = Undo::Moved(vec![(p("a.txt"), p("b.txt")), (p("b.txt"), p("a.txt"))]);
        let redo = swap.apply().unwrap();
        assert_eq!(fs::read(p("a.txt")).unwrap(), b"A");
        assert_eq!(fs::read(p("b.txt")).unwrap(), b"B");
        redo.apply().unwrap();
        assert_eq!(fs::read(p("a.txt")).unwrap(), b"B");
        // A shifting rename f1 -> f2, f2 -> f3.
        fs::write(p("f2"), b"1").unwrap();
        fs::write(p("f3"), b"2").unwrap();
        Undo::Moved(vec![(p("f1"), p("f2")), (p("f2"), p("f3"))])
            .apply()
            .unwrap();
        assert_eq!(fs::read(p("f1")).unwrap(), b"1");
        assert_eq!(fs::read(p("f2")).unwrap(), b"2");
        assert!(!p("f3").exists());
    }

    #[test]
    fn failed_undo_changes_nothing() {
        let (_d, root) = setup();
        let p = |n: &str| root.join(n);
        fs::write(p("moved-1"), b"1").unwrap();
        fs::write(p("moved-2"), b"2").unwrap();
        fs::write(p("orig-2"), b"someone else").unwrap();
        let undo = Undo::Moved(vec![
            (p("orig-1"), p("moved-1")),
            (p("orig-2"), p("moved-2")),
        ]);
        assert!(undo.apply().is_err());
        assert!(p("moved-1").exists() && !p("orig-1").exists());
        assert_eq!(fs::read(p("orig-2")).unwrap(), b"someone else");
        // Trashed: one original is occupied -> nothing is restored.
        fs::write(p("t1"), b"t1").unwrap();
        fs::write(p("t2"), b"t2").unwrap();
        let trashed = Undo::Trashed(vec![(p("orig-a"), p("t1")), (p("orig-2"), p("t2"))]);
        assert!(trashed.apply().is_err());
        assert!(p("t1").exists() && !p("orig-a").exists());
    }

    #[test]
    fn bulk_rename_can_change_case() {
        let (_d, root) = setup();
        fs::write(root.join("photo.jpg"), b"").unwrap();
        let spec = BulkRename {
            case: CaseChange::Upper,
            ..Default::default()
        };
        let undo = spec.apply(&[root.join("photo.jpg")]).unwrap();
        let names: Vec<String> = fs::read_dir(&root)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["PHOTO.jpg"]);
        undo.apply().unwrap();
        assert!(
            fs::read_dir(&root)
                .unwrap()
                .any(|e| e.unwrap().file_name() == "photo.jpg")
        );
    }

    #[test]
    fn replace_never_trashes_a_folder_holding_the_source() {
        let (_d, root) = setup();
        fs::create_dir_all(root.join("a/a")).unwrap();
        fs::write(root.join("a/a/keep.txt"), b"k").unwrap();
        let out = transfer(
            OpKind::Move,
            &[root.join("a/a")],
            &root,
            Conflict::Replace,
            &Progress::default(),
        );
        assert!(out.is_err());
        assert_eq!(fs::read(root.join("a/a/keep.txt")).unwrap(), b"k");
    }

    #[test]
    fn failed_copy_leaves_no_partial_output() {
        let (_d, root) = setup();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/ok.txt"), b"ok").unwrap();
        fs::write(root.join("src/locked.txt"), b"no").unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            root.join("src/locked.txt"),
            fs::Permissions::from_mode(0o000),
        )
        .unwrap();
        fs::create_dir(root.join("dst")).unwrap();
        let out = transfer(
            OpKind::Copy,
            &[root.join("src")],
            &root.join("dst"),
            Conflict::KeepBoth,
            &Progress::default(),
        );
        fs::set_permissions(
            root.join("src/locked.txt"),
            fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        assert!(out.is_err());
        assert!(!root.join("dst/src").exists());
    }

    #[test]
    fn cancelled_transfer_does_nothing_and_progress_totals_match() {
        let (_d, root) = setup();
        fs::create_dir_all(root.join("tree/sub")).unwrap();
        for n in ["tree/a", "tree/b", "tree/sub/c"] {
            fs::write(root.join(n), b"").unwrap();
        }
        fs::create_dir(root.join("dst")).unwrap();
        let p = Progress::new();
        p.cancel.store(true, Ordering::Relaxed);
        let out = transfer(
            OpKind::Move,
            &[root.join("tree")],
            &root.join("dst"),
            Conflict::KeepBoth,
            &p,
        );
        assert!(out.is_err());
        assert!(root.join("tree").exists() && !root.join("dst/tree").exists());
        // Empty files: progress is item-based and must reach 100%.
        let p = Progress::new();
        transfer(
            OpKind::Copy,
            &[root.join("tree")],
            &root.join("dst"),
            Conflict::KeepBoth,
            &p,
        )
        .unwrap();
        assert_eq!(p.total_items.load(Ordering::Relaxed), 5);
        assert!((p.fraction() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn concurrent_undos_in_one_folder_never_clobber_each_other() {
        let (_d, root) = setup();
        let mut records = Vec::new();
        for i in 0..16 {
            fs::write(root.join(format!("moved-{i}")), format!("{i}")).unwrap();
            records.push(Undo::Moved(vec![(
                root.join(format!("orig-{i}")),
                root.join(format!("moved-{i}")),
            )]));
        }
        std::thread::scope(|s| {
            for r in &records {
                s.spawn(move || r.apply().unwrap());
            }
        });
        for i in 0..16 {
            assert_eq!(
                fs::read_to_string(root.join(format!("orig-{i}"))).unwrap(),
                format!("{i}")
            );
        }
    }
}
