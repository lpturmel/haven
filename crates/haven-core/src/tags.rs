//! Native Finder tags, stored in the `com.apple.metadata:_kMDItemUserTags`
//! extended attribute as a binary plist array of `"Name\n<color index>"`.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

pub const TAGS_XATTR: &str = "com.apple.metadata:_kMDItemUserTags";

/// Finder label colors, in Finder's on-disk index order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum TagColor {
    #[default]
    None = 0,
    Gray = 1,
    Green = 2,
    Purple = 3,
    Blue = 4,
    Yellow = 5,
    Red = 6,
    Orange = 7,
}

impl TagColor {
    pub const ALL: [TagColor; 8] = [
        TagColor::None,
        TagColor::Red,
        TagColor::Orange,
        TagColor::Yellow,
        TagColor::Green,
        TagColor::Blue,
        TagColor::Purple,
        TagColor::Gray,
    ];

    pub fn from_index(i: u8) -> TagColor {
        match i {
            1 => TagColor::Gray,
            2 => TagColor::Green,
            3 => TagColor::Purple,
            4 => TagColor::Blue,
            5 => TagColor::Yellow,
            6 => TagColor::Red,
            7 => TagColor::Orange,
            _ => TagColor::None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            TagColor::None => "None",
            TagColor::Gray => "Gray",
            TagColor::Green => "Green",
            TagColor::Purple => "Purple",
            TagColor::Blue => "Blue",
            TagColor::Yellow => "Yellow",
            TagColor::Red => "Red",
            TagColor::Orange => "Orange",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Tag {
    pub name: String,
    pub color: TagColor,
}

impl Tag {
    pub fn new(name: impl Into<String>, color: TagColor) -> Tag {
        Tag {
            name: name.into(),
            color,
        }
    }

    fn encode(&self) -> String {
        format!("{}\n{}", self.name, self.color as u8)
    }

    fn decode(raw: &str) -> Tag {
        match raw.rsplit_once('\n') {
            Some((name, idx)) => Tag {
                name: name.to_string(),
                color: idx
                    .trim()
                    .parse()
                    .map(TagColor::from_index)
                    .unwrap_or_default(),
            },
            None => Tag {
                name: raw.to_string(),
                color: TagColor::None,
            },
        }
    }
}

/// Decodes the raw xattr payload.
pub fn decode_tags(bytes: &[u8]) -> Vec<Tag> {
    match plist::from_bytes::<Vec<String>>(bytes) {
        Ok(list) => list.iter().map(|s| Tag::decode(s)).collect(),
        Err(_) => Vec::new(),
    }
}

/// Encodes tags as a binary plist payload.
pub fn encode_tags(tags: &[Tag]) -> Vec<u8> {
    let list: Vec<String> = tags.iter().map(Tag::encode).collect();
    let mut out = Vec::new();
    plist::to_writer_binary(&mut out, &list).expect("plist encoding to memory cannot fail");
    out
}

/// Reads tags for `path` (does not follow a final symlink, like Finder).
pub fn read_tags(path: &Path) -> Vec<Tag> {
    match xattr::get(path, TAGS_XATTR) {
        Ok(Some(bytes)) => decode_tags(&bytes),
        _ => Vec::new(),
    }
}

/// Replaces the tag list on `path`. An empty list removes the attribute.
pub fn write_tags(path: &Path, tags: &[Tag]) -> io::Result<()> {
    if tags.is_empty() {
        match xattr::remove(path, TAGS_XATTR) {
            Ok(()) => Ok(()),
            Err(e) if e.raw_os_error() == Some(libc::ENOATTR) => Ok(()),
            Err(e) => Err(e),
        }
    } else {
        xattr::set(path, TAGS_XATTR, &encode_tags(tags))
    }
}

/// Adds `tag` to every path (keeping other tags), or removes it when `on` is false.
pub fn set_tag(paths: &[PathBuf], tag: &Tag, on: bool) -> io::Result<()> {
    for path in paths {
        let mut tags = read_tags(path);
        let pos = tags
            .iter()
            .position(|t| t.name.eq_ignore_ascii_case(&tag.name));
        match (on, pos) {
            (true, Some(i)) => tags[i].color = tag.color,
            (true, None) => tags.push(tag.clone()),
            (false, Some(i)) => {
                tags.remove(i);
            }
            (false, None) => continue,
        }
        write_tags(path, &tags)?;
    }
    Ok(())
}

/// Finds every file carrying `name` using Spotlight.
pub fn search_tag(name: &str) -> Vec<PathBuf> {
    let escaped = name.replace('\\', "\\\\").replace('\'', "\\'");
    let query = format!("kMDItemUserTags == '{escaped}'");
    let Ok(out) = Command::new("/usr/bin/mdfind")
        .arg("-0")
        .arg(query)
        .output()
    else {
        return Vec::new();
    };
    out.stdout
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| PathBuf::from(String::from_utf8_lossy(s).into_owned()))
        .collect()
}

/// The default Finder tags, used when Haven has no saved tag definitions.
pub fn default_tags() -> Vec<Tag> {
    [
        ("Red", TagColor::Red),
        ("Orange", TagColor::Orange),
        ("Yellow", TagColor::Yellow),
        ("Green", TagColor::Green),
        ("Blue", TagColor::Blue),
        ("Purple", TagColor::Purple),
        ("Gray", TagColor::Gray),
    ]
    .into_iter()
    .map(|(n, c)| Tag::new(n, c))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plist_round_trip() {
        let tags = vec![
            Tag::new("Design", TagColor::Red),
            Tag::new("Plain", TagColor::None),
        ];
        let bytes = encode_tags(&tags);
        assert!(bytes.starts_with(b"bplist00"));
        assert_eq!(decode_tags(&bytes), tags);
    }

    #[test]
    fn decodes_finder_style_entries() {
        assert_eq!(Tag::decode("Work\n4"), Tag::new("Work", TagColor::Blue));
        assert_eq!(Tag::decode("Bare"), Tag::new("Bare", TagColor::None));
        assert_eq!(Tag::decode("Odd\nzz"), Tag::new("Odd", TagColor::None));
    }

    #[test]
    fn xattr_round_trip_on_real_file() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("x.txt");
        std::fs::write(&f, b"").unwrap();
        assert!(read_tags(&f).is_empty());

        set_tag(
            std::slice::from_ref(&f),
            &Tag::new("Dev", TagColor::Yellow),
            true,
        )
        .unwrap();
        set_tag(
            std::slice::from_ref(&f),
            &Tag::new("School", TagColor::Blue),
            true,
        )
        .unwrap();
        assert_eq!(read_tags(&f).len(), 2);

        set_tag(
            std::slice::from_ref(&f),
            &Tag::new("dev", TagColor::Yellow),
            false,
        )
        .unwrap();
        assert_eq!(read_tags(&f), vec![Tag::new("School", TagColor::Blue)]);

        write_tags(&f, &[]).unwrap();
        assert!(read_tags(&f).is_empty());
        write_tags(&f, &[]).unwrap();
    }
}
