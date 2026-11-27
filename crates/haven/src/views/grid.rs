//! Icon grid with native icons and image thumbnails.

use gpui::{
    AnyElement, App, Context, InteractiveElement, IntoElement, LineFragment, MouseButton,
    MouseDownEvent, ObjectFit, ParentElement, Pixels, SharedString, StatefulInteractiveElement,
    Styled, StyledImage, Window, canvas, div, img, prelude::FluentBuilder, px, uniform_list,
};
use haven_core::kind::is_image_ext;

use super::{drag_payload, drag_preview, entry_icon_el, file_drop_target};
use crate::store::Store;
use crate::tab::Tab;
use crate::theme::{UI_FONT, theme};
use crate::ui::{empty_state, tag_dot};

const LABEL_SIZE: f32 = 12.;

/// Splits a tile label into at most two lines that each fit `width`, the
/// second ending in "…" when the name is longer. Done here rather than with
/// `line_clamp(2)`: gpui truncates a clamped text as one line twice as wide,
/// so when word wrapping leaves room on line 1, line 2 overflows the tile.
fn label_lines(name: &str, width: Pixels, cx: &App) -> (SharedString, Option<SharedString>) {
    let mut wrapper = cx
        .text_system()
        .line_wrapper(gpui::font(UI_FONT), px(LABEL_SIZE));
    let fragments = [LineFragment::text(name)];
    let Some(split) = wrapper.wrap_line(&fragments, width).next().map(|b| b.ix) else {
        return (SharedString::from(name.to_string()), None);
    };
    let (first, rest) = name.split_at(split);
    let mut runs = Vec::new();
    let second = wrapper.truncate_line(rest.trim_start().to_string().into(), width, "…", &mut runs);
    (
        first.trim_end().to_string().into(),
        Some(tidy_ellipsis(second)),
    )
}

/// "long name …" → "long name…": a cut that lands on a space keeps it before
/// the ellipsis, which reads like a missing word. Trimming only makes the
/// line narrower, so it still fits.
fn tidy_ellipsis(line: SharedString) -> SharedString {
    match line.strip_suffix('…') {
        Some(head) if head.ends_with(char::is_whitespace) => format!("{}…", head.trim_end()).into(),
        _ => line,
    }
}

pub fn tile_width(icon: f32) -> f32 {
    icon + 48.
}

pub fn render(tab: &mut Tab, _window: &mut Window, cx: &mut Context<Tab>) -> AnyElement {
    let icon_size = Store::get(cx).grid_icon_size;
    let tile_w = tile_width(icon_size);
    let tile_h = icon_size + 52.;
    let cols = tab.grid_cols.max(1);
    let rows = tab.entries.len().div_ceil(cols);
    let entity = cx.entity();

    let body: AnyElement = if let Some(err) = &tab.error {
        empty_state("triangle-alert", err.clone(), cx)
    } else if tab.entries.is_empty() && !tab.loading {
        empty_state("folder-open", "This folder is empty", cx)
    } else {
        uniform_list(
            "grid-rows",
            rows,
            cx.processor(move |tab, range: std::ops::Range<usize>, _window, cx| {
                range
                    .map(|r| {
                        let start = r * cols;
                        let end = (start + cols).min(tab.entries.len());
                        div()
                            .w_full()
                            .flex()
                            .h(px(tile_h))
                            .px(px(10.))
                            .children((start..end).map(|ix| tile(tab, ix, icon_size, tile_w, cx)))
                            .into_any_element()
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(tab.scroll.clone())
        .size_full()
        .pt(px(8.))
        .into_any_element()
    };

    let dest = tab.dir().map(|d| d.to_path_buf());
    div()
        .id("grid-view")
        .size_full()
        .relative()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|tab, _: &MouseDownEvent, window, cx| {
                window.focus(&tab.focus);
                tab.clear_selection(cx);
            }),
        )
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(|tab, ev: &MouseDownEvent, window, cx| {
                tab.context_background(ev.position, window, cx);
            }),
        )
        .when_some(dest, |d, dest| {
            file_drop_target(
                d,
                gpui::transparent_black(),
                cx,
                move |tab, dropped, _, cx| tab.drop_on(dest.clone(), dropped, cx),
            )
        })
        .child(
            // Measures the available width to compute the column count.
            canvas(
                move |bounds, _, cx| {
                    let cols =
                        (((f32::from(bounds.size.width) - 20.) / tile_w).floor() as usize).max(1);
                    entity.update(cx, |tab, cx| {
                        if tab.grid_cols != cols {
                            tab.grid_cols = cols;
                            cx.notify();
                        }
                    });
                },
                |_, _, _, _| {},
            )
            .absolute()
            .size_full(),
        )
        .child(body)
        .into_any_element()
}

fn tile(
    tab: &mut Tab,
    ix: usize,
    icon_size: f32,
    tile_w: f32,
    cx: &mut Context<Tab>,
) -> AnyElement {
    let t = theme(cx).clone();
    let entry = tab.entries[ix].clone();
    let selected = tab.selection.contains(&ix);
    let store_tags = Store::get(cx).tags.clone();
    let visual: AnyElement = if is_image_ext(&entry.ext) && entry.size < 40 * 1024 * 1024 {
        img(entry.path.clone())
            .size(px(icon_size))
            .object_fit(ObjectFit::Contain)
            .rounded(px(4.))
            .with_fallback({
                let e = entry.clone();
                move || div().child(e.name.clone()).into_any_element()
            })
            .into_any_element()
    } else {
        entry_icon_el(&entry, icon_size, cx)
    };
    let payload = drag_payload(tab, ix);
    let navigable = entry.is_navigable();
    let path = entry.path.clone();
    let muted = if entry.hidden { 0.55 } else { 1.0 };

    div()
        .id(ix)
        .w(px(tile_w))
        .h_full()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(4.))
        .p(px(6.))
        .rounded(px(8.))
        .when(selected, |d| d.bg(t.selection))
        .when(!selected, |d| d.hover(|s| s.bg(t.hover)))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |tab, ev: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                tab.mouse_down_item(ix, ev.modifiers, ev.click_count, window, cx);
            }),
        )
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |tab, ev: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                tab.context_item(ix, ev.position, window, cx);
            }),
        )
        .on_click(cx.listener(move |tab, _, _, cx| tab.click_item(ix, cx)))
        .on_drag(payload, |files, _, _, cx| drag_preview(files, cx))
        .when(navigable, |d| {
            file_drop_target(d, t.selection, cx, move |tab, dropped, _, cx| {
                if !dropped.paths.contains(&path) {
                    tab.drop_on(path.clone(), dropped, cx);
                }
            })
        })
        .child(
            div()
                .size(px(icon_size))
                .flex()
                .items_center()
                .justify_center()
                // Hidden items fade their content, not the hover/selection fill.
                .opacity(muted)
                .child(visual),
        )
        .child(
            div()
                .w_full()
                .opacity(muted)
                .flex()
                .justify_center()
                .items_center()
                .gap(px(4.))
                .children(entry.tags.iter().take(3).map(|tag| {
                    let color = store_tags
                        .iter()
                        .find(|s| s.name == tag.name)
                        .map(|s| s.color)
                        .unwrap_or(tag.color);
                    tag_dot(t.tag(color), px(7.))
                }))
                .child({
                    let tags = entry.tags.len().min(3) as f32;
                    // Tile padding (2 × 6) and the tag dots with their gaps.
                    let width = tile_w - 12. - tags * 11.;
                    let (first, second) = label_lines(&entry.name, px(width), cx);
                    div()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .items_center()
                        .text_size(px(LABEL_SIZE))
                        .line_height(px(15.))
                        .text_color(t.text)
                        .child(div().whitespace_nowrap().child(first))
                        .children(second.map(|l| div().whitespace_nowrap().child(l)))
                }),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ellipsis_hugs_the_last_word() {
        let t = |s: &str| tidy_ellipsis(SharedString::from(s.to_string())).to_string();
        assert_eq!(t("long name …"), "long name…");
        assert_eq!(t("long name  \t…"), "long name…");
        assert_eq!(t("long name…"), "long name…");
        // Not truncated: left alone, spaces and all.
        assert_eq!(t("short name "), "short name ");
    }

    #[gpui::test]
    fn long_labels_fit_two_lines(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let width = px(tile_width(72.) - 12.);
            let name = "A folder with a really long name that should ellipsize nicely";
            let (first, second) = label_lines(name, width, cx);
            let second = second.expect("wraps to a second line");
            assert!(second.ends_with('…'), "{second}");
            assert!(!second.ends_with(" …"), "{second}");
            assert!(name.starts_with(first.as_ref()));
            let mut w = cx
                .text_system()
                .line_wrapper(gpui::font(UI_FONT), px(LABEL_SIZE));
            for line in [first.as_ref(), second.as_ref()] {
                // Each line fits on its own (no further wrap point).
                let frags = [LineFragment::text(line)];
                assert!(
                    w.wrap_line(&frags, width).next().is_none(),
                    "{line:?} overflows"
                );
            }
            assert_eq!(label_lines("docs", width, cx), ("docs".into(), None));
        });
    }
}
