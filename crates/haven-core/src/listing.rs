use std::fs::{self, Metadata};
use std::io;
use std::os::macos::fs::MetadataExt as _;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use rayon::prelude::*;

use crate::sort::natural_key;
use crate::tags::{self, Tag};

/// `UF_HIDDEN` from `<sys/stat.h>`: the Finder "hidden" flag.
const UF_HIDDEN: u32 = 0x8000;

/// Directory extensions that Finder treats as opaque documents/apps.
const PACKAGE_EXTS: &[&str] = &[
    "app",
    "bundle",
    "framework",
    "plugin",
    "kext",
    "appex",
    "prefpane",
    "qlgenerator",
    "mdimporter",
    "photoslibrary",
    "musiclibrary",
    "tvlibrary",
    "xcodeproj",
    "xcworkspace",
    "playground",
    "rtfd",
    "pages",
    "numbers",
    "key",
    "scptd",
    "band",
    "logicx",
    "fcpbundle",
    "sparsebundle",
    "pkg",
    "mpkg",
    "lproj_bundle",
];

#[derive(Clone, Debug)]
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    /// Precomputed natural-sort key (lowercased, digit runs padded).
    pub sort_key: String,
    /// Lowercased extension, empty if none.
    pub ext: String,
    /// True for directories and symlinks pointing at directories.
    pub is_dir: bool,
    /// Directory that should behave like a file (e.g. `.app`).
    pub is_package: bool,
    pub is_symlink: bool,
    pub hidden: bool,
    /// Logical size in bytes (0 for directories until computed).
    pub size: u64,
    /// Recursive size for directories, once computed.
    pub dir_size: Option<u64>,
    pub modified: Option<SystemTime>,
    pub created: Option<SystemTime>,
    pub tags: Vec<Tag>,
}

impl Entry {
    /// Builds an entry from a path, reading its metadata (does not follow the
    /// final symlink except to learn whether it targets a directory).
    pub fn from_path(path: &Path) -> io::Result<Entry> {
        let meta = fs::symlink_metadata(path)?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        Ok(Self::from_meta(path.to_path_buf(), name, &meta, true))
    }

    fn from_meta(path: PathBuf, name: String, meta: &Metadata, with_tags: bool) -> Entry {
        let is_symlink = meta.file_type().is_symlink();
        let target_meta = if is_symlink {
            fs::metadata(&path).ok()
        } else {
            None
        };
        let effective = target_meta.as_ref().unwrap_or(meta);
        let is_dir = effective.is_dir();
        let ext = Path::new(&name)
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        let is_package = is_dir && PACKAGE_EXTS.contains(&ext.as_str());
        let hidden = name.starts_with('.') || meta.st_flags() & UF_HIDDEN != 0;
        let tags = if with_tags { tags::read_tags(&path) } else { Vec::new() };
        Entry {
            sort_key: natural_key(&name),
            size: if is_dir { 0 } else { effective.len() },
            dir_size: None,
            modified: effective.modified().ok(),
            created: effective.created().ok(),
            path,
            name,
            ext,
            is_dir,
            is_package,
            is_symlink,
            hidden,
            tags,
        }
    }

    /// Whether activating this entry should navigate into it.
    pub fn is_navigable(&self) -> bool {
        self.is_dir && !self.is_package
    }

    /// Size used for display and sorting: recursive size for directories when known.
    pub fn effective_size(&self) -> Option<u64> {
        if self.is_dir {
            self.dir_size
        } else {
            Some(self.size)
        }
    }
}

/// Directories with more entries than this get their tags in a second pass
/// (`getxattr` costs ~15µs per file, which dominates listing time).
pub const INLINE_TAG_LIMIT: usize = 400;

/// Whether `list_dir` skipped tags for this listing.
pub fn needs_tag_pass(len: usize) -> bool {
    len > INLINE_TAG_LIMIT
}

/// Reads tags for many paths (sequential: getxattr contends when parallel).
pub fn read_tags_bulk(paths: &[PathBuf]) -> Vec<(PathBuf, Vec<tags::Tag>)> {
    paths
        .iter()
        .filter_map(|p| {
            let t = tags::read_tags(p);
            (!t.is_empty()).then(|| (p.clone(), t))
        })
        .collect()
}

/// Lists a directory. Metadata is read in parallel for large directories;
/// tags are read inline only for small ones (see [`needs_tag_pass`]).
/// Hidden entries are included only when `show_hidden`.
pub fn list_dir(dir: &Path, show_hidden: bool) -> io::Result<Vec<Entry>> {
    let names: Vec<(PathBuf, String)> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| (e.path(), e.file_name().to_string_lossy().into_owned()))
        .filter(|(_, name)| show_hidden || !name.starts_with('.'))
        .collect();

    let with_tags = !needs_tag_pass(names.len());
    let build = |(path, name): (PathBuf, String)| -> Option<Entry> {
        let meta = fs::symlink_metadata(&path).ok()?;
        let entry = Entry::from_meta(path, name, &meta, with_tags);
        (show_hidden || !entry.hidden).then_some(entry)
    };

    let entries = if names.len() > 256 {
        names.into_par_iter().filter_map(build).collect()
    } else {
        names.into_iter().filter_map(build).collect()
    };
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_files_dirs_and_hides_dotfiles() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), b"hello").unwrap();
        fs::write(dir.path().join(".hidden"), b"").unwrap();
        fs::create_dir(dir.path().join("Sub")).unwrap();
        fs::create_dir(dir.path().join("Thing.app")).unwrap();

        let mut entries = list_dir(dir.path(), false).unwrap();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["Sub", "Thing.app", "a.txt"]);
        assert!(entries[0].is_navigable());
        assert!(entries[1].is_package && !entries[1].is_navigable());
        assert_eq!(entries[2].size, 5);
        assert_eq!(entries[2].ext, "txt");

        let all = list_dir(dir.path(), true).unwrap();
        assert_eq!(all.len(), 4);
    }

    #[test]
    fn symlink_to_dir_is_dir() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("real")).unwrap();
        std::os::unix::fs::symlink(dir.path().join("real"), dir.path().join("link")).unwrap();
        let e = Entry::from_path(&dir.path().join("link")).unwrap();
        assert!(e.is_symlink && e.is_dir);
    }

    #[test]
    fn large_dirs_defer_tags_to_bulk_pass() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..(INLINE_TAG_LIMIT + 10) {
            fs::write(dir.path().join(format!("f{i}")), b"").unwrap();
        }
        let tagged = dir.path().join("f3");
        tags::write_tags(&tagged, &[tags::Tag::new("Dev", tags::TagColor::Yellow)]).unwrap();
        let entries = list_dir(dir.path(), false).unwrap();
        assert!(needs_tag_pass(entries.len()));
        assert!(entries.iter().all(|e| e.tags.is_empty()));
        let paths: Vec<PathBuf> = entries.iter().map(|e| e.path.clone()).collect();
        let bulk = read_tags_bulk(&paths);
        assert_eq!(bulk.len(), 1);
        assert_eq!(bulk[0].0, tagged);
    }

    #[test]
    fn lists_large_dirs_in_parallel() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..600 {
            fs::write(dir.path().join(format!("f{i}")), b"").unwrap();
        }
        assert_eq!(list_dir(dir.path(), false).unwrap().len(), 600);
    }
}
