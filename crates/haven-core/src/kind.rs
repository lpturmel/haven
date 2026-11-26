use crate::listing::Entry;

/// Human-readable kind, Finder style.
pub fn kind_label(entry: &Entry) -> String {
    if entry.is_dir {
        return match entry.ext.as_str() {
            "app" => "Application".into(),
            _ if entry.is_package => format!("{} package", entry.ext.to_uppercase()),
            _ => "Folder".into(),
        };
    }
    kind_for_ext(&entry.ext)
        .map(str::to_string)
        .unwrap_or_else(|| {
            if entry.ext.is_empty() {
                "Document".into()
            } else {
                format!("{} file", entry.ext.to_uppercase())
            }
        })
}

pub fn kind_for_ext(ext: &str) -> Option<&'static str> {
    Some(match ext {
        "rs" => "Rust source",
        "py" => "Python script",
        "js" | "mjs" | "cjs" => "JavaScript",
        "ts" | "mts" => "TypeScript",
        "tsx" | "jsx" => "React component",
        "json" => "JSON",
        "toml" => "TOML",
        "yaml" | "yml" => "YAML",
        "md" | "markdown" => "Markdown",
        "txt" => "Plain text",
        "sh" | "bash" | "zsh" => "Shell script",
        "go" => "Go source",
        "c" => "C source",
        "h" => "C header",
        "cpp" | "cc" | "cxx" => "C++ source",
        "hpp" => "C++ header",
        "swift" => "Swift source",
        "lua" => "Lua script",
        "html" | "htm" => "HTML document",
        "css" => "CSS stylesheet",
        "xml" => "XML document",
        "csv" => "CSV document",
        "pdf" => "PDF document",
        "png" => "PNG image",
        "jpg" | "jpeg" => "JPEG image",
        "gif" => "GIF image",
        "webp" => "WebP image",
        "heic" => "HEIC image",
        "svg" => "SVG image",
        "tiff" | "tif" => "TIFF image",
        "bmp" => "BMP image",
        "ico" | "icns" => "Icon",
        "mp3" => "MP3 audio",
        "wav" => "WAV audio",
        "flac" => "FLAC audio",
        "m4a" => "MPEG-4 audio",
        "mp4" | "m4v" => "MPEG-4 movie",
        "mov" => "QuickTime movie",
        "mkv" => "Matroska video",
        "zip" => "ZIP archive",
        "gz" | "tgz" => "Gzip archive",
        "tar" => "Tar archive",
        "dmg" => "Disk image",
        "docx" | "doc" => "Word document",
        "xlsx" | "xls" => "Excel spreadsheet",
        "pptx" | "ppt" => "PowerPoint presentation",
        "xaml" => "XAML document",
        "lock" => "Lock file",
        "log" => "Log file",
        _ => return None,
    })
}

const IMAGE_EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "tiff", "tif", "svg", "ico",
];

/// Images GPUI can decode directly.
pub fn is_image_ext(ext: &str) -> bool {
    IMAGE_EXTS.contains(&ext)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, dir: bool, package: bool) -> Entry {
        let mut e = Entry::from_path(std::path::Path::new("/")).unwrap();
        e.name = name.into();
        e.ext = std::path::Path::new(name)
            .extension()
            .map(|x| x.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        e.is_dir = dir;
        e.is_package = package;
        e
    }

    #[test]
    fn labels() {
        assert_eq!(kind_label(&entry("src", true, false)), "Folder");
        assert_eq!(kind_label(&entry("Haven.app", true, true)), "Application");
        assert_eq!(
            kind_label(&entry("x.xcodeproj", true, true)),
            "XCODEPROJ package"
        );
        assert_eq!(kind_label(&entry("main.rs", false, false)), "Rust source");
        assert_eq!(kind_label(&entry("data.weird", false, false)), "WEIRD file");
        assert_eq!(kind_label(&entry("Makefile", false, false)), "Document");
        assert!(is_image_ext("png") && !is_image_ext("rs"));
    }
}
