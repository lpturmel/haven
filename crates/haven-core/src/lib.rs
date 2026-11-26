//! Haven's filesystem core. No UI dependencies: everything here is plain Rust
//! so it can be unit tested and benchmarked in isolation.

pub mod complete;
pub mod dirsize;
pub mod fmt;
pub mod fuzzy;
pub mod kind;
pub mod listing;
pub mod ops;
pub mod pathres;
pub mod recents;
pub mod sort;
pub mod state;
pub mod system;
pub mod tags;
pub mod volumes;
pub mod watch;

pub use listing::{Entry, list_dir};
pub use sort::{SortKey, SortSpec};
pub use tags::{Tag, TagColor};

/// The user's home directory.
pub fn home_dir() -> std::path::PathBuf {
    std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("/"))
}

/// Replaces a leading home directory with `~` for display.
pub fn tildify(path: &std::path::Path) -> String {
    let home = home_dir();
    match path.strip_prefix(&home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}
