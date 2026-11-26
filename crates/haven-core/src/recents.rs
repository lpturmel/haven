//! Recently used files: Haven's own open history merged with Spotlight's
//! `kMDItemLastUsedDate`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::home_dir;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recent {
    pub path: PathBuf,
    pub used: SystemTime,
}

/// Parses `mdfind -0 -attr kMDItemLastUsedDate` output.
pub fn parse_mdfind(out: &[u8]) -> Vec<Recent> {
    const SEP: &str = "   kMDItemLastUsedDate = ";
    out.split(|b| *b == 0)
        .filter_map(|rec| {
            let rec = String::from_utf8_lossy(rec);
            let (path, date) = rec.rsplit_once(SEP)?;
            let dt = chrono::DateTime::parse_from_str(date.trim(), "%Y-%m-%d %H:%M:%S %z").ok()?;
            let secs = u64::try_from(dt.timestamp()).ok()?;
            Some(Recent {
                path: PathBuf::from(path),
                used: UNIX_EPOCH + Duration::from_secs(secs),
            })
        })
        .collect()
}

fn is_interesting(path: &Path, home: &Path) -> bool {
    let s = path.to_string_lossy();
    if s.contains("/.") || s.contains(".app/") || s.ends_with(".app") {
        return false;
    }
    if path.starts_with(home.join("Library")) {
        return false;
    }
    true
}

/// Queries Spotlight for files used in the last `days` days under the home folder.
pub fn spotlight_recents(days: u32) -> Vec<Recent> {
    let home = home_dir();
    let query = format!(
        "kMDItemLastUsedDate >= $time.today(-{days}) && kMDItemContentTypeTree != public.folder"
    );
    let Ok(out) = Command::new("/usr/bin/mdfind")
        .arg("-0")
        .arg("-onlyin")
        .arg(&home)
        .arg("-attr")
        .arg("kMDItemLastUsedDate")
        .arg(query)
        .output()
    else {
        return Vec::new();
    };
    parse_mdfind(&out.stdout)
        .into_iter()
        .filter(|r| is_interesting(&r.path, &home))
        .collect()
}

/// Merges history and Spotlight results, newest first, deduplicated, existing only.
pub fn merge(mut a: Vec<Recent>, b: Vec<Recent>, limit: usize) -> Vec<Recent> {
    a.extend(b);
    a.sort_by_key(|r| std::cmp::Reverse(r.used));
    let mut seen = std::collections::HashSet::new();
    a.into_iter()
        .filter(|r| seen.insert(r.path.clone()))
        .filter(|r| r.path.exists())
        .take(limit)
        .collect()
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mdfind_records() {
        let raw = b"/Users/x/a.txt   kMDItemLastUsedDate = 2026-11-25 16:05:51 +0000\0/Users/x/b c.md   kMDItemLastUsedDate = 2026-11-24 01:00:00 +0000\0garbage\0";
        let got = parse_mdfind(raw);
        assert_eq!(got.len(), 2);
        assert_eq!(got[1].path, PathBuf::from("/Users/x/b c.md"));
        assert!(got[0].used > got[1].used);
    }

    #[test]
    fn filters_noise() {
        let home = PathBuf::from("/Users/x");
        assert!(is_interesting(Path::new("/Users/x/Documents/a.pdf"), &home));
        assert!(!is_interesting(
            Path::new("/Users/x/Library/Caches/a"),
            &home
        ));
        assert!(!is_interesting(Path::new("/Users/x/.cache/a"), &home));
        assert!(!is_interesting(
            Path::new("/Users/x/Applications/Foo.app"),
            &home
        ));
    }

    #[test]
    fn merge_dedups_and_orders() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("f");
        let g = dir.path().join("g");
        std::fs::write(&f, b"").unwrap();
        std::fs::write(&g, b"").unwrap();
        let t = |s| UNIX_EPOCH + Duration::from_secs(s);
        let merged = merge(
            vec![Recent {
                path: f.clone(),
                used: t(10),
            }],
            vec![
                Recent {
                    path: g.clone(),
                    used: t(20),
                },
                Recent {
                    path: f.clone(),
                    used: t(5),
                },
                Recent {
                    path: dir.path().join("missing"),
                    used: t(30),
                },
            ],
            10,
        );
        assert_eq!(merged.iter().map(|r| &r.path).collect::<Vec<_>>(), [&g, &f]);
        assert_eq!(merged[1].used, t(10));
    }
}
