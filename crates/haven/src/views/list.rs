//! Detail list view with sortable, multi-key column headers.

use gpui::{
    AnyElement, Context, ExternalPaths, FontWeight, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, ParentElement, SharedString, StatefulInteractiveElement, Styled, Window, div,
    prelude::FluentBuilder, px, uniform_list,
};
use haven_core::kind::kind_label;
use haven_core::{SortKey, fmt};

use super::{drag_payload, drag_preview, entry_icon_el};
use crate::store::Store;
use crate::tab::{DraggedFiles, Location, Tab};
use crate::theme::theme;
use crate::ui::Ellipsis as _;
use crate::ui::{empty_state, icon, tag_dot};

pub const ROW_H: f32 = 26.;
const DATE_W: f32 = 168.;
const SIZE_W: f32 = 84.;
const KIND_W: f32 = 136.;

fn header_cell(
    tab: &Tab,
    key: SortKey,
    label: &'static str,
    width: Option<f32>,
    right: bool,
    cx: &mut Context<Tab>,
) -> AnyElement {
    let t = theme(cx).clone();
    let rank = tab.sort.rank(key);
    let multi = tab.sort.keys.len() > 1;
    let mut cell = div()
        .id(SharedString::from(format!("hdr-{label}")))
        .flex()
        .items_center()
        .gap(px(4.))
        .h_full()
        .px(px(6.))
        .rounded(px(4.))
        .cursor_pointer()
        .hover(|s| s.bg(t.hover))
        .text_color(if rank.is_some() { t.text } else { t.text_faint })
        .when(right, |d| d.justify_end())
        .child(label)
        .when_some(rank, |d, (pos, desc)| {
            d.child(
                icon(if desc { "chevron-down" } else { "chevron-up" })
                    .size(px(11.))
                    .text_color(t.accent_text),
            )
            .when(multi, |d| {
                d.child(
                    div()
                        .text_size(px(9.))
                        .text_color(t.accent_text)
                        .child(pos.to_string()),
                )
            })
        })
        .on_click(cx.listener(move |tab, ev: &gpui::ClickEvent, _, cx| {
            tab.click_sort(key, ev.modifiers().shift, cx);
        }));
    cell = match width {
        Some(w) => cell.w(px(w)).flex_none(),
        None => cell.flex_1().min_w_0(),
    };
    cell.into_any_element()
}

pub fn render(tab: &mut Tab, _window: &mut Window, cx: &mut Context<Tab>) -> AnyElement {
    let t = theme(cx).clone();
    let header = div()
        .flex()
        .flex_none()
        .items_center()
        .h(px(28.))
        .px(px(10.))
        .border_b_1()
        .border_color(t.divider)
        .text_size(px(11.5))
        .font_weight(FontWeight::MEDIUM)
        .child(header_cell(tab, SortKey::Name, "Name", None, false, cx))
        .child(header_cell(
            tab,
            SortKey::Modified,
            "Date Modified",
            Some(DATE_W),
            false,
            cx,
        ))
        .child(header_cell(
            tab,
            SortKey::Size,
            "Size",
            Some(SIZE_W),
            true,
            cx,
        ))
        .child(header_cell(
            tab,
            SortKey::Kind,
            "Kind",
            Some(KIND_W),
            false,
            cx,
        ));

    let body: AnyElement = if let Some(err) = &tab.error {
        empty_state("triangle-alert", err.clone(), cx)
    } else if tab.entries.is_empty() && !tab.loading {
        let msg = if !tab.filter.is_empty() {
            format!("No items match “{}”", tab.filter)
        } else if matches!(tab.location, Location::Tag(_)) {
            "No items with this tag".to_string()
        } else {
            "This folder is empty".to_string()
        };
        empty_state("folder-open", msg, cx)
    } else {
        uniform_list(
            "list-rows",
            tab.entries.len(),
            cx.processor(|tab, range: std::ops::Range<usize>, _window, cx| {
                range.map(|ix| row(tab, ix, cx)).collect::<Vec<_>>()
            }),
        )
        .track_scroll(tab.scroll.clone())
        .size_full()
        .px(px(6.))
        .pt(px(4.))
        .pb(px(12.))
        .into_any_element()
    };

    let dest = tab.dir().map(|d| d.to_path_buf());
    div()
        .id("list-view")
        .size_full()
        .flex()
        .flex_col()
        .child(header)
        .child(
            div()
                .id("list-body")
                .flex_1()
                .min_h_0()
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
                    let dest2 = dest.clone();
                    d.on_drop(cx.listener(move |tab, files: &DraggedFiles, window, cx| {
                        tab.drop_on(dest.clone(), files.paths.clone(), window, cx);
                    }))
                    .on_drop(cx.listener(
                        move |tab, ext: &ExternalPaths, window, cx| {
                            tab.drop_on(dest2.clone(), ext.paths().to_vec(), window, cx);
                        },
                    ))
                })
                .child(body),
        )
        .into_any_element()
}

fn row(tab: &mut Tab, ix: usize, cx: &mut Context<Tab>) -> AnyElement {
    let t = theme(cx).clone();
    let entry = tab.entries[ix].clone();
    let selected = tab.selection.contains(&ix);
    let focused = tab.cursor == Some(ix);
    let tags = entry.tags.clone();
    let store_tags = Store::get(cx).tags.clone();
    let size = match entry.effective_size() {
        Some(s) => fmt::size(s),
        None if tab.sizing && entry.is_dir => "…".into(),
        None => "--".into(),
    };
    let icon_el = entry_icon_el(&entry, 18., cx);
    let navigable = entry.is_navigable();
    let path = entry.path.clone();
    let path2 = entry.path.clone();
    let payload = drag_payload(tab, ix);
    let muted = if entry.hidden { 0.55 } else { 1.0 };

    div()
        .id(ix)
        .w_full()
        .h(px(ROW_H))
        .px(px(4.))
        .flex()
        .items_center()
        .rounded(px(6.))
        .text_size(px(13.))
        .when(selected, |d| d.bg(t.selection))
        .when(!selected, |d| d.hover(|s| s.bg(t.hover)))
        .when(focused && !selected, |d| {
            d.border_1().border_color(t.selection)
        })
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
            d.drag_over::<DraggedFiles>(move |s, _, _, _| s.bg(t.selection))
                .on_drop(cx.listener(move |tab, files: &DraggedFiles, window, cx| {
                    cx.stop_propagation();
                    if !files.paths.contains(&path) {
                        tab.drop_on(path.clone(), files.paths.clone(), window, cx);
                    }
                }))
                .on_drop(cx.listener(move |tab, ext: &ExternalPaths, window, cx| {
                    cx.stop_propagation();
                    tab.drop_on(path2.clone(), ext.paths().to_vec(), window, cx);
                }))
        })
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(4.))
                .opacity(muted)
                .child(icon_el)
                .child(
                    div()
                        .min_w_0()
                        .ellipsis()
                        .text_color(t.text)
                        .child(entry.name.clone()),
                )
                .when(entry.is_symlink, |d| {
                    d.child(icon("link").size(px(11.)).text_color(t.text_faint))
                })
                .children(tags.iter().map(|tag| {
                    let color = store_tags
                        .iter()
                        .find(|s| s.name == tag.name)
                        .map(|s| s.color)
                        .unwrap_or(tag.color);
                    tag_dot(t.tag(color), px(8.))
                })),
        )
        .child(
            div()
                .w(px(DATE_W))
                .flex_none()
                .px(px(6.))
                .ellipsis()
                .text_color(t.text_muted)
                .child(fmt::date(entry.modified)),
        )
        .child(
            div()
                .w(px(SIZE_W))
                .flex_none()
                .px(px(6.))
                .flex()
                .justify_end()
                .text_color(t.text_muted)
                .child(size),
        )
        .child(
            div()
                .w(px(KIND_W))
                .flex_none()
                .px(px(6.))
                .ellipsis()
                .text_color(t.text_muted)
                .child(kind_label(&entry)),
        )
        .into_any_element()
}
