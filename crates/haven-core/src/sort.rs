use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

use crate::kind::kind_label;
use crate::listing::Entry;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SortKey {
    Name,
    Kind,
    Size,
    Modified,
    Created,
    Extension,
    Tags,
}

impl SortKey {
    pub const ALL: [SortKey; 7] = [
        SortKey::Name,
        SortKey::Kind,
        SortKey::Size,
        SortKey::Modified,
        SortKey::Created,
        SortKey::Extension,
        SortKey::Tags,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SortKey::Name => "Name",
            SortKey::Kind => "Kind",
            SortKey::Size => "Size",
            SortKey::Modified => "Date Modified",
            SortKey::Created => "Date Created",
            SortKey::Extension => "Extension",
            SortKey::Tags => "Tags",
        }
    }

    /// Whether a freshly chosen key should start descending (newest/biggest first).
    pub fn default_descending(self) -> bool {
        matches!(self, SortKey::Size | SortKey::Modified | SortKey::Created)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SortSpec {
    /// Keys in priority order with their `descending` flag.
    pub keys: Vec<(SortKey, bool)>,
    pub folders_first: bool,
}

impl Default for SortSpec {
    fn default() -> Self {
        SortSpec {
            keys: vec![(SortKey::Name, false)],
            folders_first: true,
        }
    }
}

impl SortSpec {
    pub fn primary(&self) -> (SortKey, bool) {
        self.keys.first().copied().unwrap_or((SortKey::Name, false))
    }

    /// Header click: make `key` the only key, or flip it if it already is primary.
    pub fn click(&mut self, key: SortKey) {
        match self.keys.first() {
            Some(&(k, desc)) if k == key => self.keys = vec![(key, !desc)],
            _ => self.keys = vec![(key, key.default_descending())],
        }
    }

    /// Shift+click: append `key` as a secondary key, flip it if present,
    /// or remove it on the third click.
    pub fn shift_click(&mut self, key: SortKey) {
        if let Some(pos) = self.keys.iter().position(|(k, _)| *k == key) {
            let (_, desc) = self.keys[pos];
            if desc == key.default_descending() && self.keys.len() > 1 {
                self.keys[pos].1 = !desc;
            } else if self.keys.len() > 1 {
                self.keys.remove(pos);
            } else {
                self.keys[pos].1 = !desc;
            }
        } else {
            self.keys.push((key, key.default_descending()));
        }
    }

    /// 1-based position of `key` and its direction, if it participates.
    pub fn rank(&self, key: SortKey) -> Option<(usize, bool)> {
        self.keys
            .iter()
            .position(|(k, _)| *k == key)
            .map(|i| (i + 1, self.keys[i].1))
    }
}

/// Builds a key that sorts naturally ("file2" < "file10") and case-insensitively
/// under plain byte comparison: digit runs are left-padded to 20 characters.
pub fn natural_key(name: &str) -> String {
    let lower = name.to_lowercase();
    let mut out = String::with_capacity(lower.len() + 8);
    let mut digits = String::new();
    let flush = |digits: &mut String, out: &mut String| {
        if !digits.is_empty() {
            let trimmed = digits.trim_start_matches('0');
            let trimmed = if trimmed.is_empty() { "0" } else { trimmed };
            for _ in trimmed.len()..20 {
                out.push('0');
            }
            out.push_str(trimmed);
            digits.clear();
        }
    };
    for ch in lower.chars() {
        if ch.is_ascii_digit() {
            digits.push(ch);
        } else {
            flush(&mut digits, &mut out);
            out.push(ch);
        }
    }
    flush(&mut digits, &mut out);
    out
}

fn cmp_key(a: &Entry, b: &Entry, key: SortKey) -> Ordering {
    match key {
        SortKey::Name => a.sort_key.cmp(&b.sort_key),
        SortKey::Kind => kind_label(a).cmp(&kind_label(b)),
        SortKey::Size => a
            .effective_size()
            .unwrap_or(0)
            .cmp(&b.effective_size().unwrap_or(0)),
        SortKey::Modified => a.modified.cmp(&b.modified),
        SortKey::Created => a.created.cmp(&b.created),
        SortKey::Extension => a.ext.cmp(&b.ext),
        SortKey::Tags => {
            // Tagged items first, then by first tag name.
            let ta = a.tags.first().map(|t| t.name.to_lowercase());
            let tb = b.tags.first().map(|t| t.name.to_lowercase());
            match (ta, tb) {
                (Some(x), Some(y)) => x.cmp(&y),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            }
        }
    }
}

pub fn compare(a: &Entry, b: &Entry, spec: &SortSpec) -> Ordering {
    if spec.folders_first {
        let fa = a.is_navigable();
        let fb = b.is_navigable();
        if fa != fb {
            return fb.cmp(&fa);
        }
    }
    for &(key, desc) in &spec.keys {
        let ord = cmp_key(a, b, key);
        let ord = if desc { ord.reverse() } else { ord };
        if ord != Ordering::Equal {
            return ord;
        }
    }
    a.sort_key.cmp(&b.sort_key)
}

pub fn sort_entries(entries: &mut [Entry], spec: &SortSpec) {
    if entries.len() > 20_000 {
        use rayon::prelude::*;
        entries.par_sort_by(|a, b| compare(a, b, spec));
    } else {
        entries.sort_by(|a, b| compare(a, b, spec));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn entry(name: &str, dir: bool, size: u64, age: u64) -> Entry {
        Entry {
            path: name.into(),
            name: name.into(),
            sort_key: natural_key(name),
            ext: std::path::Path::new(name)
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default(),
            is_dir: dir,
            is_package: false,
            is_symlink: false,
            hidden: false,
            size,
            dir_size: None,
            modified: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000 - age)),
            created: None,
            tags: vec![],
        }
    }

    fn names(v: &[Entry]) -> Vec<&str> {
        v.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn natural_order() {
        let mut v = vec![
            entry("file10.txt", false, 0, 0),
            entry("File2.txt", false, 0, 0),
            entry("file1.txt", false, 0, 0),
            entry("a", false, 0, 0),
        ];
        sort_entries(&mut v, &SortSpec::default());
        assert_eq!(names(&v), ["a", "file1.txt", "File2.txt", "file10.txt"]);
    }

    #[test]
    fn folders_first_and_toggle() {
        let mut v = vec![
            entry("b", false, 0, 0),
            entry("z", true, 0, 0),
            entry("a", false, 0, 0),
        ];
        let mut spec = SortSpec::default();
        sort_entries(&mut v, &spec);
        assert_eq!(names(&v), ["z", "a", "b"]);
        spec.folders_first = false;
        sort_entries(&mut v, &spec);
        assert_eq!(names(&v), ["a", "b", "z"]);
    }

    #[test]
    fn multi_key_sort() {
        let mut v = vec![
            entry("b.rs", false, 10, 5),
            entry("a.rs", false, 10, 1),
            entry("c.md", false, 99, 3),
            entry("d.md", false, 10, 2),
        ];
        let mut spec = SortSpec::default();
        spec.click(SortKey::Size); // size desc
        spec.shift_click(SortKey::Modified); // then newest first
        assert_eq!(
            spec.keys,
            vec![(SortKey::Size, true), (SortKey::Modified, true)]
        );
        sort_entries(&mut v, &spec);
        assert_eq!(names(&v), ["c.md", "a.rs", "d.md", "b.rs"]);

        spec.click(SortKey::Extension);
        spec.shift_click(SortKey::Name);
        sort_entries(&mut v, &spec);
        assert_eq!(names(&v), ["c.md", "d.md", "a.rs", "b.rs"]);
    }

    #[test]
    fn click_cycles() {
        let mut spec = SortSpec::default();
        spec.click(SortKey::Name);
        assert_eq!(spec.keys, vec![(SortKey::Name, true)]);
        spec.shift_click(SortKey::Size);
        assert_eq!(spec.rank(SortKey::Size), Some((2, true)));
        spec.shift_click(SortKey::Size);
        assert_eq!(spec.rank(SortKey::Size), Some((2, false)));
        spec.shift_click(SortKey::Size);
        assert_eq!(spec.rank(SortKey::Size), None);
    }

    #[test]
    fn natural_key_pads_digits() {
        assert!(natural_key("v2") < natural_key("v10"));
        assert!(natural_key("img007") == natural_key("IMG7"));
    }
}
