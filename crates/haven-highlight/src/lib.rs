//! Tree-sitter powered syntax highlighting for Haven's preview pane.
//!
//! The output is a per-line list of `(byte range, Style)` spans so the UI can
//! build text runs for a virtualized line list without re-scanning the text.

use std::cell::RefCell;
use std::ops::Range;
use std::path::Path;
use std::sync::OnceLock;
use std::sync::atomic::AtomicUsize;

use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

/// Semantic style classes. The UI maps these to colors.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Style {
    Keyword,
    String,
    Comment,
    Function,
    Type,
    Constant,
    Number,
    Operator,
    Punctuation,
    Property,
    Variable,
    VariableBuiltin,
    Tag,
    Attribute,
    Label,
    Escape,
    Constructor,
    Module,
    Heading,
    Link,
    Emphasis,
    Strong,
    Literal,
}

/// Capture names we recognize, paired with their style. tree-sitter-highlight
/// matches hierarchically, so `function.method` falls back to `function`.
const CAPTURES: &[(&str, Style)] = &[
    ("attribute", Style::Attribute),
    ("boolean", Style::Constant),
    ("character", Style::String),
    ("comment", Style::Comment),
    ("constant", Style::Constant),
    ("constant.builtin", Style::Constant),
    ("constructor", Style::Constructor),
    ("embedded", Style::Variable),
    ("escape", Style::Escape),
    ("function", Style::Function),
    ("function.builtin", Style::Function),
    ("function.macro", Style::Function),
    ("keyword", Style::Keyword),
    ("label", Style::Label),
    ("module", Style::Module),
    ("namespace", Style::Module),
    ("number", Style::Number),
    ("float", Style::Number),
    ("operator", Style::Operator),
    ("property", Style::Property),
    ("field", Style::Property),
    ("punctuation", Style::Punctuation),
    ("string", Style::String),
    ("string.special", Style::Escape),
    ("string.escape", Style::Escape),
    ("tag", Style::Tag),
    ("type", Style::Type),
    ("type.builtin", Style::Type),
    ("variable", Style::Variable),
    ("variable.builtin", Style::VariableBuiltin),
    ("variable.parameter", Style::Variable),
    ("text.title", Style::Heading),
    ("markup.heading", Style::Heading),
    ("text.uri", Style::Link),
    ("text.reference", Style::Link),
    ("markup.link", Style::Link),
    ("text.emphasis", Style::Emphasis),
    ("markup.italic", Style::Emphasis),
    ("text.strong", Style::Strong),
    ("markup.bold", Style::Strong),
    ("text.literal", Style::Literal),
    ("markup.raw", Style::Literal),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lang {
    Rust,
    Python,
    JavaScript,
    TypeScript,
    Tsx,
    Json,
    Toml,
    Yaml,
    Markdown,
    MarkdownInline,
    Bash,
    Go,
    C,
    Cpp,
    Html,
    Css,
    Lua,
    Swift,
}

impl Lang {
    pub fn name(self) -> &'static str {
        match self {
            Lang::Rust => "Rust",
            Lang::Python => "Python",
            Lang::JavaScript => "JavaScript",
            Lang::TypeScript => "TypeScript",
            Lang::Tsx => "TSX",
            Lang::Json => "JSON",
            Lang::Toml => "TOML",
            Lang::Yaml => "YAML",
            Lang::Markdown => "Markdown",
            Lang::MarkdownInline => "Markdown",
            Lang::Bash => "Shell",
            Lang::Go => "Go",
            Lang::C => "C",
            Lang::Cpp => "C++",
            Lang::Html => "HTML",
            Lang::Css => "CSS",
            Lang::Lua => "Lua",
            Lang::Swift => "Swift",
        }
    }

    fn from_injection_name(name: &str) -> Option<Lang> {
        Some(match name.to_ascii_lowercase().as_str() {
            "rust" | "rs" => Lang::Rust,
            "python" | "py" => Lang::Python,
            "javascript" | "js" | "jsx" => Lang::JavaScript,
            "typescript" | "ts" => Lang::TypeScript,
            "tsx" => Lang::Tsx,
            "json" => Lang::Json,
            "toml" => Lang::Toml,
            "yaml" | "yml" => Lang::Yaml,
            "markdown" | "md" => Lang::Markdown,
            "markdown_inline" => Lang::MarkdownInline,
            "bash" | "sh" | "shell" | "zsh" => Lang::Bash,
            "go" => Lang::Go,
            "c" => Lang::C,
            "cpp" | "c++" => Lang::Cpp,
            "html" => Lang::Html,
            "css" => Lang::Css,
            "lua" => Lang::Lua,
            "swift" => Lang::Swift,
            _ => return None,
        })
    }

    fn build(self) -> Option<HighlightConfiguration> {
        let (language, name, highlights, injections, locals): (
            tree_sitter::Language,
            &str,
            String,
            &str,
            &str,
        ) = match self {
            Lang::Rust => (
                tree_sitter_rust::LANGUAGE.into(),
                "rust",
                tree_sitter_rust::HIGHLIGHTS_QUERY.into(),
                tree_sitter_rust::INJECTIONS_QUERY,
                "",
            ),
            Lang::Python => (
                tree_sitter_python::LANGUAGE.into(),
                "python",
                tree_sitter_python::HIGHLIGHTS_QUERY.into(),
                "",
                "",
            ),
            Lang::JavaScript => (
                tree_sitter_javascript::LANGUAGE.into(),
                "javascript",
                format!(
                    "{}\n{}",
                    tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
                    tree_sitter_javascript::HIGHLIGHT_QUERY
                ),
                tree_sitter_javascript::INJECTIONS_QUERY,
                tree_sitter_javascript::LOCALS_QUERY,
            ),
            Lang::TypeScript => (
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
                "typescript",
                format!(
                    "{}\n{}",
                    tree_sitter_typescript::HIGHLIGHTS_QUERY,
                    tree_sitter_javascript::HIGHLIGHT_QUERY
                ),
                tree_sitter_javascript::INJECTIONS_QUERY,
                tree_sitter_typescript::LOCALS_QUERY,
            ),
            Lang::Tsx => (
                tree_sitter_typescript::LANGUAGE_TSX.into(),
                "tsx",
                format!(
                    "{}\n{}\n{}",
                    tree_sitter_typescript::HIGHLIGHTS_QUERY,
                    tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
                    tree_sitter_javascript::HIGHLIGHT_QUERY
                ),
                tree_sitter_javascript::INJECTIONS_QUERY,
                tree_sitter_typescript::LOCALS_QUERY,
            ),
            Lang::Json => (
                tree_sitter_json::LANGUAGE.into(),
                "json",
                tree_sitter_json::HIGHLIGHTS_QUERY.into(),
                "",
                "",
            ),
            Lang::Toml => (
                tree_sitter_toml_ng::LANGUAGE.into(),
                "toml",
                tree_sitter_toml_ng::HIGHLIGHTS_QUERY.into(),
                "",
                "",
            ),
            Lang::Yaml => (
                tree_sitter_yaml::LANGUAGE.into(),
                "yaml",
                tree_sitter_yaml::HIGHLIGHTS_QUERY.into(),
                "",
                "",
            ),
            Lang::Markdown => (
                tree_sitter_md::LANGUAGE.into(),
                "markdown",
                tree_sitter_md::HIGHLIGHT_QUERY_BLOCK.into(),
                tree_sitter_md::INJECTION_QUERY_BLOCK,
                "",
            ),
            Lang::MarkdownInline => (
                tree_sitter_md::INLINE_LANGUAGE.into(),
                "markdown_inline",
                tree_sitter_md::HIGHLIGHT_QUERY_INLINE.into(),
                tree_sitter_md::INJECTION_QUERY_INLINE,
                "",
            ),
            Lang::Bash => (
                tree_sitter_bash::LANGUAGE.into(),
                "bash",
                tree_sitter_bash::HIGHLIGHT_QUERY.into(),
                "",
                "",
            ),
            Lang::Go => (
                tree_sitter_go::LANGUAGE.into(),
                "go",
                tree_sitter_go::HIGHLIGHTS_QUERY.into(),
                "",
                "",
            ),
            Lang::C => (
                tree_sitter_c::LANGUAGE.into(),
                "c",
                tree_sitter_c::HIGHLIGHT_QUERY.into(),
                "",
                "",
            ),
            Lang::Cpp => (
                tree_sitter_cpp::LANGUAGE.into(),
                "cpp",
                format!(
                    "{}\n{}",
                    tree_sitter_cpp::HIGHLIGHT_QUERY,
                    tree_sitter_c::HIGHLIGHT_QUERY
                ),
                "",
                "",
            ),
            Lang::Html => (
                tree_sitter_html::LANGUAGE.into(),
                "html",
                tree_sitter_html::HIGHLIGHTS_QUERY.into(),
                tree_sitter_html::INJECTIONS_QUERY,
                "",
            ),
            Lang::Css => (
                tree_sitter_css::LANGUAGE.into(),
                "css",
                tree_sitter_css::HIGHLIGHTS_QUERY.into(),
                "",
                "",
            ),
            Lang::Lua => (
                tree_sitter_lua::LANGUAGE.into(),
                "lua",
                tree_sitter_lua::HIGHLIGHTS_QUERY.into(),
                tree_sitter_lua::INJECTIONS_QUERY,
                tree_sitter_lua::LOCALS_QUERY,
            ),
            Lang::Swift => (
                tree_sitter_swift::LANGUAGE.into(),
                "swift",
                tree_sitter_swift::HIGHLIGHTS_QUERY.into(),
                tree_sitter_swift::INJECTIONS_QUERY,
                tree_sitter_swift::LOCALS_QUERY,
            ),
        };
        let mut config =
            HighlightConfiguration::new(language, name, &highlights, injections, locals).ok()?;
        let names: Vec<&str> = CAPTURES.iter().map(|(n, _)| *n).collect();
        config.configure(&names);
        Some(config)
    }
}

const LANG_COUNT: usize = 18;

fn lang_index(lang: Lang) -> usize {
    lang as usize
}

fn config_for(lang: Lang) -> Option<&'static HighlightConfiguration> {
    static CONFIGS: [OnceLock<Option<HighlightConfiguration>>; LANG_COUNT] =
        [const { OnceLock::new() }; LANG_COUNT];
    CONFIGS[lang_index(lang)]
        .get_or_init(|| lang.build())
        .as_ref()
}

/// Detects the language for a file from its name and (optionally) its first line.
pub fn detect(path: &Path, first_line: Option<&str>) -> Option<Lang> {
    let name = path.file_name()?.to_str()?.to_ascii_lowercase();
    let by_name = match name.as_str() {
        "cargo.lock" | "pipfile" | "poetry.lock" => Some(Lang::Toml),
        ".bashrc" | ".zshrc" | ".bash_profile" | ".profile" | ".zprofile" | ".zshenv"
        | "pkgbuild" | ".envrc" => Some(Lang::Bash),
        "package-lock.json" | ".prettierrc" | ".eslintrc" | "tsconfig.json" => Some(Lang::Json),
        _ => None,
    };
    if by_name.is_some() {
        return by_name;
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    let by_ext = ext.as_deref().and_then(|ext| {
        Some(match ext {
            "rs" => Lang::Rust,
            "py" | "pyi" | "pyw" => Lang::Python,
            "js" | "mjs" | "cjs" | "jsx" => Lang::JavaScript,
            "ts" | "mts" | "cts" => Lang::TypeScript,
            "tsx" => Lang::Tsx,
            "json" | "jsonc" | "json5" | "geojson" | "webmanifest" => Lang::Json,
            "toml" => Lang::Toml,
            "yaml" | "yml" => Lang::Yaml,
            "md" | "markdown" | "mdx" => Lang::Markdown,
            "sh" | "bash" | "zsh" | "ksh" | "command" => Lang::Bash,
            "go" => Lang::Go,
            "c" | "h" => Lang::C,
            "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" | "ino" | "m" | "mm" => Lang::Cpp,
            "html" | "htm" | "xhtml" | "svelte" | "vue" => Lang::Html,
            "css" | "scss" | "less" => Lang::Css,
            "lua" => Lang::Lua,
            "swift" => Lang::Swift,
            _ => return None,
        })
    });
    if by_ext.is_some() {
        return by_ext;
    }
    let line = first_line?;
    if let Some(shebang) = line.strip_prefix("#!") {
        let interp = shebang
            .split_whitespace()
            .flat_map(|s| s.rsplit('/').next())
            .find(|s| *s != "env")
            .unwrap_or("");
        return match interp {
            s if s.starts_with("python") => Some(Lang::Python),
            "node" | "deno" | "bun" => Some(Lang::JavaScript),
            "bash" | "sh" | "zsh" | "ksh" | "dash" => Some(Lang::Bash),
            "lua" | "luajit" => Some(Lang::Lua),
            _ => None,
        };
    }
    None
}

/// Per-line highlight spans. `spans[i]` covers line `i` (lines split on `\n`),
/// with ranges relative to the start of that line.
pub type LineSpans = Vec<Vec<(Range<usize>, Style)>>;

thread_local! {
    static HIGHLIGHTER: RefCell<Highlighter> = RefCell::new(Highlighter::new());
}

/// Highlights `text` (which should already have `\r` stripped). Returns `None`
/// if the grammar fails to load or `cancel` becomes non-zero.
pub fn highlight(lang: Lang, text: &str, cancel: Option<&AtomicUsize>) -> Option<LineSpans> {
    let config = config_for(lang)?;
    let line_starts: Vec<usize> = std::iter::once(0)
        .chain(text.match_indices('\n').map(|(i, _)| i + 1))
        .collect();
    let mut lines: LineSpans = vec![Vec::new(); line_starts.len()];

    HIGHLIGHTER.with(|h| {
        let mut h = h.borrow_mut();
        let events = h
            .highlight(config, text.as_bytes(), None, cancel, |name| {
                Lang::from_injection_name(name).and_then(config_for)
            })
            .ok()?;
        let mut stack: Vec<Style> = Vec::new();
        for event in events {
            match event.ok()? {
                HighlightEvent::HighlightStart(hl) => stack.push(CAPTURES[hl.0].1),
                HighlightEvent::HighlightEnd => {
                    stack.pop();
                }
                HighlightEvent::Source { start, end } => {
                    let Some(&style) = stack.last() else { continue };
                    push_span(&mut lines, &line_starts, text, start..end, style);
                }
            }
        }
        Some(())
    })?;
    Some(lines)
}

fn push_span(
    lines: &mut LineSpans,
    line_starts: &[usize],
    text: &str,
    range: Range<usize>,
    style: Style,
) {
    let mut line = match line_starts.binary_search(&range.start) {
        Ok(i) => i,
        Err(i) => i - 1,
    };
    let mut start = range.start;
    while start < range.end && line < line_starts.len() {
        let line_start = line_starts[line];
        let line_end = line_starts
            .get(line + 1)
            .map(|s| s - 1)
            .unwrap_or(text.len());
        let end = range.end.min(line_end);
        if end > start {
            lines[line].push((start - line_start..end - line_start, style));
        }
        line += 1;
        start = line_starts.get(line).copied().unwrap_or(range.end);
    }
}

/// Normalizes text for display: strips `\r` and expands tabs to 4 spaces.
pub fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.split('\n') {
        if !out.is_empty() || line.as_ptr() != text.as_ptr() {
            out.push('\n');
        }
        let mut col = 0usize;
        for ch in line.chars() {
            match ch {
                '\r' => {}
                '\t' => {
                    let n = 4 - (col % 4);
                    out.extend(std::iter::repeat_n(' ', n));
                    col += n;
                }
                c => {
                    out.push(c);
                    col += 1;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_by_extension_name_and_shebang() {
        assert_eq!(detect(Path::new("/a/main.rs"), None), Some(Lang::Rust));
        assert_eq!(detect(Path::new("x.TSX"), None), Some(Lang::Tsx));
        assert_eq!(detect(Path::new("Cargo.lock"), None), Some(Lang::Toml));
        assert_eq!(detect(Path::new(".zshrc"), None), Some(Lang::Bash));
        assert_eq!(
            detect(Path::new("script"), Some("#!/usr/bin/env python3")),
            Some(Lang::Python)
        );
        assert_eq!(detect(Path::new("blob.bin"), None), None);
    }

    #[test]
    fn every_language_builds() {
        for lang in [
            Lang::Rust,
            Lang::Python,
            Lang::JavaScript,
            Lang::TypeScript,
            Lang::Tsx,
            Lang::Json,
            Lang::Toml,
            Lang::Yaml,
            Lang::Markdown,
            Lang::MarkdownInline,
            Lang::Bash,
            Lang::Go,
            Lang::C,
            Lang::Cpp,
            Lang::Html,
            Lang::Css,
            Lang::Lua,
            Lang::Swift,
        ] {
            assert!(config_for(lang).is_some(), "{lang:?} failed to build");
        }
    }

    #[test]
    fn highlights_rust_per_line() {
        let src = "fn main() {\n    let s = \"hi\";\n}\n";
        let lines = highlight(Lang::Rust, src, None).unwrap();
        assert_eq!(lines.len(), 4);
        assert!(lines[0].contains(&(0..2, Style::Keyword)));
        assert!(
            lines[1]
                .iter()
                .any(|(r, s)| *s == Style::String && &src[12 + r.start..12 + r.end] == "\"hi\"")
        );
    }

    #[test]
    fn multiline_spans_are_split() {
        let src = "/* a\nb */\nfn x() {}";
        let lines = highlight(Lang::Rust, src, None).unwrap();
        assert_eq!(lines[0], vec![(0..4, Style::Comment)]);
        assert_eq!(lines[1], vec![(0..4, Style::Comment)]);
    }

    #[test]
    fn normalize_expands_tabs_and_strips_cr() {
        assert_eq!(normalize("a\tb\r\n\tc"), "a   b\n    c");
        assert_eq!(normalize(""), "");
    }
}
