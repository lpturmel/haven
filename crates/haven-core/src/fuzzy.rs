//! Recursive fuzzy file finder: a parallel, gitignore-aware walk streaming
//! into a nucleo matcher.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use ignore::{WalkBuilder, WalkState};
use nucleo::pattern::{CaseMatching, Normalization};
use nucleo::{Config, Matcher, Nucleo, Utf32Str};
use unicode_segmentation::UnicodeSegmentation;

const SKIP_DIRS: &[&str] = &[
    "node_modules",
    ".git",
    "target",
    ".cache",
    ".Trash",
    "Library",
];

pub struct FuzzyMatch {
    pub path: PathBuf,
    /// Path relative to the root, as displayed.
    pub rel: String,
    /// Char indices into `rel` that matched.
    pub indices: Vec<u32>,
    pub is_dir: bool,
}

pub struct FuzzyFinder {
    /// (path, is_dir, relative path as displayed). The display string is
    /// kept: nucleo's haystack keeps only the first char of each grapheme, so
    /// rebuilding names from it drops accents (`café` -> `cafe`).
    nucleo: Nucleo<(PathBuf, bool, String)>,
    root: PathBuf,
    cancel: Arc<AtomicBool>,
    walking: Arc<AtomicBool>,
    found: Arc<AtomicUsize>,
    matcher: Matcher,
    query: String,
}

impl FuzzyFinder {
    /// Starts walking `root` immediately on background threads. `notify` is
    /// called whenever new results may be available.
    pub fn new(
        root: &Path,
        respect_gitignore: bool,
        show_hidden: bool,
        notify: Arc<dyn Fn() + Send + Sync>,
    ) -> FuzzyFinder {
        let nucleo = Nucleo::new(Config::DEFAULT.match_paths(), notify, None, 1);
        let injector = nucleo.injector();
        let cancel = Arc::new(AtomicBool::new(false));
        let walking = Arc::new(AtomicBool::new(true));
        let found = Arc::new(AtomicUsize::new(0));
        let root = root.to_path_buf();

        let (c, w, f, r) = (cancel.clone(), walking.clone(), found.clone(), root.clone());
        std::thread::spawn(move || {
            let walker = WalkBuilder::new(&r)
                .hidden(!show_hidden)
                .git_ignore(respect_gitignore)
                .git_global(respect_gitignore)
                .git_exclude(respect_gitignore)
                .ignore(respect_gitignore)
                .parents(respect_gitignore)
                .require_git(false)
                .follow_links(false)
                .filter_entry(|e| {
                    let name = e.file_name().to_string_lossy();
                    !(e.depth() > 0
                        && e.file_type().is_some_and(|t| t.is_dir())
                        && SKIP_DIRS.contains(&name.as_ref()))
                })
                .build_parallel();
            walker.run(|| {
                let injector = injector.clone();
                let root = r.clone();
                let c = c.clone();
                let f = f.clone();
                Box::new(move |entry| {
                    if c.load(Ordering::Relaxed) {
                        return WalkState::Quit;
                    }
                    let Ok(entry) = entry else {
                        return WalkState::Continue;
                    };
                    if entry.depth() == 0 {
                        return WalkState::Continue;
                    }
                    let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
                    let path = entry.into_path();
                    let rel = path
                        .strip_prefix(&root)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .into_owned();
                    injector.push((path, is_dir, rel), |item, cols| {
                        cols[0] = item.2.as_str().into()
                    });
                    f.fetch_add(1, Ordering::Relaxed);
                    WalkState::Continue
                })
            });
            w.store(false, Ordering::Relaxed);
        });

        FuzzyFinder {
            nucleo,
            root,
            cancel,
            walking,
            found,
            matcher: Matcher::new(Config::DEFAULT.match_paths()),
            query: String::new(),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn set_query(&mut self, query: &str) {
        let append = query.starts_with(&self.query) && !self.query.is_empty();
        self.nucleo
            .pattern
            .reparse(0, query, CaseMatching::Smart, Normalization::Smart, append);
        self.query = query.to_string();
    }

    /// Advances matching. Returns true if results changed or work is ongoing.
    pub fn tick(&mut self) -> bool {
        let status = self.nucleo.tick(10);
        status.changed || status.running || self.is_walking()
    }

    pub fn is_walking(&self) -> bool {
        self.walking.load(Ordering::Relaxed)
    }

    pub fn total(&self) -> usize {
        self.found.load(Ordering::Relaxed)
    }

    pub fn matched_count(&self) -> usize {
        self.nucleo.snapshot().matched_item_count() as usize
    }

    pub fn results(&mut self, limit: usize) -> Vec<FuzzyMatch> {
        let snap = self.nucleo.snapshot();
        let n = (snap.matched_item_count() as usize).min(limit) as u32;
        let pattern = snap.pattern().column_pattern(0);
        let mut out = Vec::with_capacity(n as usize);
        let mut buf = Vec::new();
        for item in snap.matched_items(0..n) {
            let hay = &item.matcher_columns[0];
            let mut indices = Vec::new();
            if !self.query.is_empty() {
                let s = hay.to_string();
                let utf = Utf32Str::new(&s, &mut buf);
                pattern.indices(utf, &mut self.matcher, &mut indices);
                indices.sort_unstable();
                indices.dedup();
            }
            let rel = item.data.2.clone();
            out.push(FuzzyMatch {
                indices: grapheme_to_char_indices(&rel, &indices),
                path: item.data.0.clone(),
                rel,
                is_dir: item.data.1,
            });
        }
        out
    }
}

/// nucleo reports matches per grapheme; the UI highlights chars. Expands each
/// matched grapheme to all of its chars (e.g. `e` + combining accent).
fn grapheme_to_char_indices(text: &str, graphemes: &[u32]) -> Vec<u32> {
    let mut out = Vec::new();
    let mut char_ix = 0u32;
    for (g, grapheme) in text.graphemes(true).enumerate() {
        let n = grapheme.chars().count() as u32;
        if graphemes.binary_search(&(g as u32)).is_ok() {
            out.extend(char_ix..char_ix + n);
        }
        char_ix += n;
    }
    out
}

impl Drop for FuzzyFinder {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn finds_and_ranks_files_respecting_gitignore() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src/ui")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
        std::fs::write(root.join("src/ui/workspace.rs"), b"").unwrap();
        std::fs::write(root.join("src/main.rs"), b"").unwrap();
        std::fs::write(root.join("node_modules/pkg/workspace.rs"), b"").unwrap();
        std::fs::write(root.join("ignored.log"), b"").unwrap();
        std::fs::write(root.join(".gitignore"), b"*.log\n").unwrap();

        let mut f = FuzzyFinder::new(root, true, false, Arc::new(|| {}));
        f.set_query("wsrs");
        let deadline = Instant::now() + Duration::from_secs(5);
        while (f.is_walking() || f.tick()) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        f.tick();
        let res = f.results(10);
        assert_eq!(
            res.len(),
            1,
            "{:?}",
            res.iter().map(|r| &r.rel).collect::<Vec<_>>()
        );
        assert_eq!(res[0].rel, "src/ui/workspace.rs");
        assert!(!res[0].indices.is_empty());

        f.set_query("log");
        f.tick();
        std::thread::sleep(Duration::from_millis(20));
        f.tick();
        assert!(f.results(10).iter().all(|r| !r.rel.ends_with(".log")));
    }

    #[test]
    fn keeps_combining_characters_in_names() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let nfd = "cafe\u{301}.txt"; // "café" with a combining accent
        std::fs::write(root.join(nfd), b"").unwrap();
        let mut f = FuzzyFinder::new(root, false, false, Arc::new(|| {}));
        f.set_query("caf");
        let deadline = Instant::now() + Duration::from_secs(5);
        while (f.is_walking() || f.tick()) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        let got = f.results(10);
        assert_eq!(got.len(), 1);
        // APFS may store the name in either normalization; it must not lose
        // the accent either way.
        assert!(got[0].rel.contains('\u{301}') || got[0].rel.contains('é'));
        assert_eq!(got[0].indices, [0, 1, 2]);
    }

    #[test]
    fn grapheme_indices_expand_to_all_chars() {
        assert_eq!(grapheme_to_char_indices("e\u{301}x", &[0]), [0, 1]);
        assert_eq!(grapheme_to_char_indices("e\u{301}x", &[1]), [2]);
    }
}
