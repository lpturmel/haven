//! Small shared building blocks.

use gpui::{
    AnyElement, App, Div, ElementId, Hsla, InteractiveElement, IntoElement, ParentElement, Pixels,
    SharedString, StatefulInteractiveElement, Styled, Svg, div, px, svg,
};

use crate::theme::theme;

pub fn icon(name: &str) -> Svg {
    svg()
        .path(SharedString::from(format!("icons/{name}.svg")))
        .flex_none()
}

pub fn icon_sized(name: &str, size: Pixels, color: Hsla) -> Svg {
    icon(name).size(size).text_color(color)
}

/// A square icon button with hover state.
pub fn icon_button(id: impl Into<ElementId>, name: &str, cx: &App) -> gpui::Stateful<Div> {
    let t = theme(cx);
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .size(px(26.))
        .rounded(px(6.))
        .text_color(t.text_muted)
        .hover(|s| s.bg(t.hover))
        .active(|s| s.bg(t.active))
        .cursor_pointer()
        .child(icon(name).size(px(15.)).text_color(t.text_muted))
}

pub fn text_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    primary: bool,
    cx: &App,
) -> gpui::Stateful<Div> {
    let t = theme(cx);
    let base = div()
        .id(id)
        .px(px(12.))
        .h(px(28.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.))
        .text_size(px(13.))
        .cursor_pointer()
        .child(label.into());
    if primary {
        base.bg(t.accent)
            .text_color(gpui::white())
            .hover(|s| s.opacity(0.9))
    } else {
        base.bg(t.elevated)
            .border_1()
            .border_color(t.field_border)
            .text_color(t.text)
            .hover(|s| s.bg(t.active))
    }
}

pub fn kbd(label: impl Into<SharedString>, cx: &App) -> Div {
    let t = theme(cx);
    div()
        .px(px(5.))
        .py(px(1.))
        .rounded(px(4.))
        .bg(t.hover)
        .text_size(px(11.))
        .text_color(t.text_faint)
        .child(label.into())
}

pub fn section_label(label: impl Into<SharedString>, cx: &App) -> Div {
    let t = theme(cx);
    div()
        .text_size(px(11.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(t.text_muted)
        .child(label.into())
}

pub fn tag_dot(color: Hsla, size: Pixels) -> Div {
    div().flex_none().size(size).rounded_full().bg(color)
}

pub fn empty_state(icon_name: &str, title: impl Into<SharedString>, cx: &App) -> AnyElement {
    let t = theme(cx);
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(10.))
        .child(icon_sized(icon_name, px(28.), t.text_faint))
        .child(
            div()
                .text_size(px(13.))
                .text_color(t.text_faint)
                .child(title.into()),
        )
        .into_any_element()
}

/// Icon name for an entry when no native icon is available yet.
pub fn fallback_icon(is_dir: bool, ext: &str) -> &'static str {
    if is_dir {
        return "folder";
    }
    match ext {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "heic" | "svg" | "bmp" | "tiff" => "file-image",
        "mp4" | "mov" | "mkv" | "avi" | "m4v" => "file-video",
        "mp3" | "wav" | "flac" | "m4a" | "aac" => "file-audio",
        "zip" | "gz" | "tgz" | "tar" | "7z" | "rar" | "dmg" => "file-archive",
        "json" => "file-json",
        "md" | "txt" | "pdf" | "doc" | "docx" | "rtf" => "file-text",
        "rs" | "py" | "js" | "ts" | "tsx" | "go" | "c" | "cpp" | "h" | "swift" | "lua" | "sh"
        | "html" | "css" | "toml" | "yaml" | "yml" => "file-code",
        _ => "file",
    }
}

/// Single-line text that ends in "…" when it doesn't fit.
///
/// Use instead of gpui's `truncate()`: that sets no-wrap, and gpui 0.2.2
/// caches a no-wrap text's first measurement, which usually has no definite
/// width yet, so the ellipsis is never applied and long names are just cut
/// off. A one-line clamp with normal wrapping is re-measured at the real
/// width.
pub trait Ellipsis: gpui::Styled + Sized {
    fn ellipsis(self) -> Self {
        self.overflow_hidden().line_clamp(1).text_ellipsis()
    }
}

impl<T: gpui::Styled> Ellipsis for T {}

/// A small hover tooltip (use with `.tooltip(|_, cx| tooltip("…", cx))`).
pub struct Tooltip(SharedString);

impl gpui::Render for Tooltip {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        div()
            .px(px(8.))
            .py(px(3.))
            .rounded(px(5.))
            .bg(t.popover_bg)
            .border_1()
            .border_color(t.card_border)
            .font_family(crate::theme::UI_FONT)
            .text_size(px(12.))
            .text_color(t.text)
            .child(self.0.clone())
    }
}

pub fn tooltip(text: impl Into<SharedString>, cx: &mut App) -> gpui::AnyView {
    use gpui::AppContext as _;
    let text = text.into();
    cx.new(|_| Tooltip(text)).into()
}
