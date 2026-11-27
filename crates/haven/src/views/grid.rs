//! Icon grid with native icons and image thumbnails.

use gpui::{
    AnyElement, Context, ExternalPaths, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, ObjectFit, ParentElement, StatefulInteractiveElement, Styled, StyledImage,
    Window, canvas, div, img, prelude::FluentBuilder, px, uniform_list,
};
use haven_core::kind::is_image_ext;

use super::{drag_payload, drag_preview, entry_icon_el, file_drop_target};
use crate::store::Store;
use crate::tab::{DraggedFiles, Tab};
use crate::theme::theme;
use crate::ui::{empty_state, tag_dot};

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
            let dest2 = dest.clone();
            d.on_drop(cx.listener(move |tab, files: &DraggedFiles, window, cx| {
                tab.drop_on(dest.clone(), files.paths.clone(), window, cx);
            }))
            .on_drop(cx.listener(move |tab, ext: &ExternalPaths, window, cx| {
                tab.drop_on(dest2.clone(), ext.paths().to_vec(), window, cx);
            }))
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
        .opacity(if entry.hidden { 0.55 } else { 1.0 })
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
            file_drop_target(d, t.selection, cx, move |tab, paths, window, cx| {
                if !paths.contains(&path) {
                    tab.drop_on(path.clone(), paths, window, cx);
                }
            })
        })
        .child(
            div()
                .size(px(icon_size))
                .flex()
                .items_center()
                .justify_center()
                .child(visual),
        )
        .child(
            div()
                .w_full()
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
                .child(
                    // flex_1 gives the label a definite width so long names
                    // wrap inside the cell (and ellipsize on line 2) instead
                    // of overflowing it on both sides.
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .text_size(px(12.))
                        .text_color(t.text)
                        .line_height(px(15.))
                        .text_center()
                        .line_clamp(2)
                        .text_ellipsis()
                        .child(entry.name.clone()),
                ),
        )
        .into_any_element()
}
