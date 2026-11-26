//! Path-bar completion.

use std::path::Path;

use crate::pathres::expand;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// What the input should become if this candidate is accepted.
    pub replacement: String,
    /// The entry name shown in the dropdown.
    pub name: String,
    pub is_dir: bool,
}

/// Completes the last component of `input`. The typed directory prefix is kept
/// verbatim (so `~/de` → `~/dev/`). Prefix matches rank above substring and
/// subsequence matches; directories rank above files.
pub fn complete(input: &str, cwd: &Path, show_hidden: bool, limit: usize) -> Vec<Candidate> {
    let (head, partial) = match input.rfind('/') {
        Some(i) => (&input[..=i], &input[i + 1..]),
        None if input == "~" => {
            return vec![Candidate {
                replacement: "~/".into(),
                name: "~".into(),
                is_dir: true,
            }];
        }
        None => ("", input),
    };
    let dir = if head.is_empty() {
        cwd.to_path_buf()
    } else {
        match expand(head, cwd) {
            Ok(p) => p,
            Err(_) => return Vec::new(),
        }
    };
    let Ok(read) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let needle = partial.to_lowercase();
    let mut scored: Vec<(u8, bool, String)> = read
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') && !show_hidden && !partial.starts_with('.') {
                return None;
            }
            let lower = name.to_lowercase();
            let rank = if lower.starts_with(&needle) {
                0
            } else if needle.is_empty() {
                return None;
            } else if lower.contains(&needle) {
                1
            } else if is_subsequence(&needle, &lower) {
                2
            } else {
                return None;
            };
            let is_dir = std::fs::metadata(e.path())
                .map(|m| m.is_dir())
                .unwrap_or(false);
            Some((rank, is_dir, name))
        })
        .collect();
    scored.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(b.1.cmp(&a.1))
            .then_with(|| a.2.to_lowercase().cmp(&b.2.to_lowercase()))
    });
    scored
        .into_iter()
        .take(limit)
        .map(|(_, is_dir, name)| Candidate {
            replacement: format!("{head}{name}{}", if is_dir { "/" } else { "" }),
            name,
            is_dir,
        })
        .collect()
}

fn is_subsequence(needle: &str, hay: &str) -> bool {
    let mut it = hay.chars();
    needle.chars().all(|c| it.any(|h| h == c))
}

/// Longest common prefix of candidate replacements (for shell-style Tab).
pub fn common_prefix(cands: &[Candidate]) -> Option<String> {
    let first = &cands.first()?.replacement;
    let mut len = first.len();
    for c in &cands[1..] {
        len = len.min(
            first
                .char_indices()
                .zip(c.replacement.chars())
                .take_while(|((_, a), b)| a == b)
                .last()
                .map(|((i, ch), _)| i + ch.len_utf8())
                .unwrap_or(0),
        );
    }
    Some(first[..len].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completes_prefix_substring_and_keeps_head() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir(root.join("Documents")).unwrap();
        std::fs::create_dir(root.join("dev")).unwrap();
        std::fs::write(root.join("deploy.sh"), b"").unwrap();
        std::fs::create_dir(root.join("mydesk")).unwrap();
        std::fs::create_dir(root.join(".dotdir")).unwrap();

        let got = complete("de", root, false, 10);
        let names: Vec<_> = got.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["dev", "deploy.sh", "mydesk", "Documents"]);
        assert_eq!(got[0].replacement, "dev/");
        assert_eq!(got[1].replacement, "deploy.sh");

        let head = format!("{}/", root.display());
        let got = complete(&format!("{head}Doc"), Path::new("/"), false, 10);
        assert_eq!(got[0].replacement, format!("{head}Documents/"));

        assert!(complete("dotd", root, false, 10).is_empty());
        assert_eq!(complete(".do", root, false, 10)[0].name, ".dotdir");
        assert_eq!(complete("dvd", root, false, 10).len(), 0);
        assert_eq!(complete("mdk", root, false, 10)[0].name, "mydesk");
    }

    #[test]
    fn common_prefix_of_candidates() {
        let c = |s: &str| Candidate {
            replacement: s.into(),
            name: s.into(),
            is_dir: true,
        };
        assert_eq!(
            common_prefix(&[c("~/dev/"), c("~/deploy")]).unwrap(),
            "~/de"
        );
        assert_eq!(common_prefix(&[c("abc")]).unwrap(), "abc");
        assert_eq!(common_prefix(&[]), None);
    }
}
