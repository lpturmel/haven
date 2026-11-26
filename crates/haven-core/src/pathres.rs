//! Resolves what the user typed in the path bar into a real location.

use std::path::{Component, Path, PathBuf};

use crate::home_dir;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    /// Navigate into this (canonical) directory.
    Dir(PathBuf),
    /// Navigate to `parent` and select `file`.
    File { parent: PathBuf, file: PathBuf },
}

/// Expands `~`, `$VAR` / `${VAR}`, `file://` URLs and relative paths (against
/// `cwd`) without touching the filesystem. `..` and `.` are folded lexically.
pub fn expand(input: &str, cwd: &Path) -> Result<PathBuf, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("Enter a path".into());
    }
    let input = match input.strip_prefix("file://") {
        Some(rest) => percent_decode(rest.strip_prefix("localhost").unwrap_or(rest)),
        None => input.to_string(),
    };
    let expanded = expand_vars(&input)?;
    let expanded = if expanded == "~" {
        home_dir().display().to_string()
    } else if let Some(rest) = expanded.strip_prefix("~/") {
        format!("{}/{}", home_dir().display(), rest)
    } else {
        expanded
    };
    let path = Path::new(&expanded);
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    Ok(normalize(&joined))
}

/// Resolves to a canonical directory or file target.
pub fn resolve(input: &str, cwd: &Path) -> Result<Resolved, String> {
    let path = expand(input, cwd)?;
    let canon = std::fs::canonicalize(&path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => format!("No such file or folder: {}", path.display()),
        std::io::ErrorKind::PermissionDenied => format!("Permission denied: {}", path.display()),
        _ => format!("{}: {e}", path.display()),
    })?;
    if canon.is_dir() {
        Ok(Resolved::Dir(canon))
    } else {
        let parent = canon.parent().unwrap_or(Path::new("/")).to_path_buf();
        Ok(Resolved::File {
            parent,
            file: canon,
        })
    }
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        PathBuf::from("/")
    } else {
        out
    }
}

fn expand_vars(input: &str) -> Result<String, String> {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '$' {
            out.push(ch);
            continue;
        }
        let braced = chars.peek() == Some(&'{');
        if braced {
            chars.next();
        }
        let mut name = String::new();
        while let Some(&c) = chars.peek() {
            if c.is_ascii_alphanumeric() || c == '_' {
                name.push(c);
                chars.next();
            } else {
                break;
            }
        }
        let closed = !braced || chars.peek() == Some(&'}');
        if braced && closed {
            chars.next();
        }
        // `$` is legal in file names (`$RECYCLE.BIN`, `C$`): anything that is
        // not a set variable stays literal and resolves as a path.
        match std::env::var(&name) {
            Ok(value) if !name.is_empty() && closed => out.push_str(&value),
            _ => {
                out.push('$');
                if braced {
                    out.push('{');
                }
                out.push_str(&name);
                if braced && closed {
                    out.push('}');
                }
            }
        }
    }
    Ok(out)
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        // Work on bytes: slicing the `&str` could split a UTF-8 character
        // (e.g. `%aé`) and panic.
        if bytes[i] == b'%'
            && let (Some(hi), Some(lo)) = (
                bytes.get(i + 1).and_then(|b| (*b as char).to_digit(16)),
                bytes.get(i + 2).and_then(|b| (*b as char).to_digit(16)),
            )
        {
            out.push((hi * 16 + lo) as u8);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_home_vars_relative_and_dots() {
        let home = home_dir();
        assert_eq!(expand("~", Path::new("/x")).unwrap(), home);
        assert_eq!(expand("~/a/b", Path::new("/x")).unwrap(), home.join("a/b"));
        assert_eq!(expand("$HOME/a", Path::new("/x")).unwrap(), home.join("a"));
        assert_eq!(
            expand("${HOME}/a", Path::new("/x")).unwrap(),
            home.join("a")
        );
        assert_eq!(
            expand("sub/../c", Path::new("/x/y")).unwrap(),
            PathBuf::from("/x/y/c")
        );
        assert_eq!(
            expand("..", Path::new("/x/y")).unwrap(),
            PathBuf::from("/x")
        );
        assert_eq!(
            expand("/../..", Path::new("/")).unwrap(),
            PathBuf::from("/")
        );
        // Unset variables and stray `$` stay literal: `$` is legal in names.
        assert_eq!(
            expand("/Volumes/USB/$RECYCLE.BIN", Path::new("/")).unwrap(),
            PathBuf::from("/Volumes/USB/$RECYCLE.BIN")
        );
        assert_eq!(
            expand("/x/${HAVEN_DEFINITELY_UNSET_VAR}/C$", Path::new("/")).unwrap(),
            PathBuf::from("/x/${HAVEN_DEFINITELY_UNSET_VAR}/C$")
        );
        assert_eq!(
            expand("/x/${unclosed", Path::new("/")).unwrap(),
            PathBuf::from("/x/${unclosed")
        );
        assert!(expand("   ", Path::new("/")).is_err());
    }

    #[test]
    fn file_urls_are_decoded() {
        assert_eq!(
            expand("file:///tmp/My%20Folder", Path::new("/")).unwrap(),
            PathBuf::from("/tmp/My Folder")
        );
        // A `%` followed by a multi-byte character must not panic.
        assert_eq!(
            expand("file:///tmp/%aé/x", Path::new("/")).unwrap(),
            PathBuf::from("/tmp/%aé/x")
        );
        assert_eq!(
            expand("file:///tmp/caf%C3%A9%", Path::new("/")).unwrap(),
            PathBuf::from("/tmp/café%")
        );
    }

    #[test]
    fn resolves_dirs_files_symlinks_and_errors() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::create_dir(root.join("real")).unwrap();
        std::fs::write(root.join("real/f.txt"), b"").unwrap();
        std::os::unix::fs::symlink(root.join("real"), root.join("link")).unwrap();

        assert_eq!(
            resolve("real", &root).unwrap(),
            Resolved::Dir(root.join("real"))
        );
        assert_eq!(
            resolve("link", &root).unwrap(),
            Resolved::Dir(root.join("real"))
        );
        assert_eq!(
            resolve("link/f.txt", &root).unwrap(),
            Resolved::File {
                parent: root.join("real"),
                file: root.join("real/f.txt")
            }
        );
        let err = resolve("nope", &root).unwrap_err();
        assert!(err.starts_with("No such file or folder"), "{err}");
    }
}
