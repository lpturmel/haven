//! Miller columns: ancestors → current folder → peek/preview.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    ParentElement, SharedString, StatefulInteractiveElement, Styled, Window, div,
    prelude::FluentBuilder, px, uniform_list,
};
use haven_core::Entry;

use super::{drag_payload, drag_preview, entry_icon_el, preview};
use crate::tab::{Location, Tab};
use crate::theme::theme;
use crate::ui::Ellipsis as _;
use crate::ui::icon;
use crate::views::file_drop_target;

const COL_W: f32 = 230.;
const ROW_H: f32 = 24.;

pub fn render(tab: &mut Tab, window: &mut Window, cx: &mut Context<Tab>) -> AnyElement {
    let t = theme(cx).clone();
    let chain = tab.column_chain();
    let mut columns: Vec<AnyElement> = Vec::new();

    // Ancestor columns, each highlighting the next directory in the chain.
    let dir = tab.dir().map(Path::to_path_buf);
    for (i, ancestor) in chain.iter().enumerate() {
        let highlight = chain.get(i + 1).cloned().or_else(|| dir.clone());
        let listing = tab.column_listing(ancestor, cx);
        let error = tab.column_errors.get(ancestor).cloned();
        columns.push(static_column(
            ancestor.clone(),
            listing,
            error,
            highlight,
            false,
            cx,
        ));
    }

    // Current column: interactive, full selection support.
    let current = uniform_list(
        "col-current",
        tab.entries.len(),
        cx.processor(|tab, range: std::ops::Range<usize>, _w, cx| {
            range.map(|ix| current_row(tab, ix, cx)).collect::<Vec<_>>()
        }),
    )
    .track_scroll(tab.scroll.clone())
    .size_full()
    .px(px(6.))
    .py(px(4.));
    columns.push(
        column_frame("col-cur", cx)
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
            .when_some(tab.dir().map(Path::to_path_buf), |d, dest| {
                file_drop_target(
                    d,
                    gpui::transparent_black(),
                    cx,
                    move |tab, dropped, _, cx| tab.drop_on(dest.clone(), dropped, cx),
                )
            })
            .child(current)
            .into_any_element(),
    );

    // Peek column (selected folder) or preview (selected file).
    let trailing: AnyElement = match tab.cursor_entry().cloned() {
        Some(e) if e.is_navigable() && tab.selection.len() == 1 => {
            let listing = tab.column_listing(&e.path, cx);
            let error = tab.column_errors.get(&e.path).cloned();
            static_column(e.path.clone(), listing, error, None, true, cx)
        }
        Some(_) => div()
            .flex_1()
            .min_w(px(320.))
            .h_full()
            .child(preview::render(tab, window, cx))
            .into_any_element(),
        None => div().flex_1().into_any_element(),
    };
    columns.push(trailing);

    div()
        .id("columns")
        .size_full()
        .flex()
        .overflow_x_scroll()
        .bg(t.card_bg)
        .children(columns)
        .into_any_element()
}

fn column_frame(id: impl Into<SharedString>, cx: &Context<Tab>) -> gpui::Stateful<gpui::Div> {
    let t = theme(cx);
    div()
        .id(id.into())
        .w(px(COL_W))
        .flex_none()
        .h_full()
        .border_r_1()
        .border_color(t.divider)
}

fn static_column(
    dir: PathBuf,
    listing: Option<Arc<Vec<Entry>>>,
    error: Option<String>,
    highlight: Option<PathBuf>,
    peek: bool,
    cx: &mut Context<Tab>,
) -> AnyElement {
    let Some(entries) = listing else {
        return column_frame(SharedString::from(format!("col-{}", dir.display())), cx)
            .into_any_element();
    };
    if let Some(err) = error {
        return column_frame(SharedString::from(format!("col-{}", dir.display())), cx)
            .child(crate::ui::empty_state("triangle-alert", err, cx))
            .into_any_element();
    }
    let id = SharedString::from(format!("col-{}", dir.display()));
    let count = entries.len();
    column_frame(id.clone(), cx)
        .child(
            uniform_list(
                id,
                count,
                cx.processor(move |_tab, range: std::ops::Range<usize>, _w, cx| {
                    range
                        .map(|ix| {
                            let e = &entries[ix];
                            let is_hl = highlight.as_ref() == Some(&e.path);
                            static_row(&dir, e, is_hl, peek, ix, cx)
                        })
                        .collect::<Vec<_>>()
                }),
            )
            .size_full()
            .px(px(6.))
            .py(px(4.)),
        )
        .into_any_element()
}

fn static_row(
    dir: &Path,
    e: &Entry,
    highlighted: bool,
    peek: bool,
    ix: usize,
    cx: &mut Context<Tab>,
) -> AnyElement {
    let t = theme(cx).clone();
    let dir = dir.to_path_buf();
    let path = e.path.clone();
    let navigable = e.is_navigable();
    div()
        .id(ix)
        .w_full()
        .h(px(ROW_H))
        .px(px(8.))
        .flex()
        .items_center()
        .gap(px(8.))
        .rounded(px(6.))
        .text_size(px(13.))
        .when(highlighted, |d| d.bg(t.selection_inactive))
        .when(!highlighted, |d| d.hover(|s| s.bg(t.hover)))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |tab, _: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                window.focus(&tab.focus);
                if navigable && !peek {
                    tab.navigate(Location::Dir(path.clone()), true, cx);
                } else {
                    tab.reveal(dir.clone(), vec![path.clone()], cx);
                }
            }),
        )
        .child(entry_icon_el(e, 16., cx))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .ellipsis()
                .text_color(t.text)
                .child(e.name.clone()),
        )
        .when(navigable, |d| {
            d.child(icon("chevron-right").size(px(12.)).text_color(t.text_faint))
        })
        .into_any_element()
}

fn current_row(tab: &mut Tab, ix: usize, cx: &mut Context<Tab>) -> AnyElement {
    let t = theme(cx).clone();
    let e = tab.entries[ix].clone();
    let selected = tab.selection.contains(&ix);
    let payload = drag_payload(tab, ix);
    div()
        .id(ix)
        .w_full()
        .h(px(ROW_H))
        .px(px(8.))
        .flex()
        .items_center()
        .gap(px(8.))
        .rounded(px(6.))
        .text_size(px(13.))
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
        .when(e.is_navigable(), |d| {
            let dest = e.path.clone();
            file_drop_target(d, t.selection, cx, move |tab, dropped, _, cx| {
                if !dropped.paths.contains(&dest) {
                    tab.drop_on(dest.clone(), dropped, cx);
                }
            })
        })
        .on_drag(payload, |files, _, _, cx| drag_preview(files, cx))
        .child(entry_icon_el(&e, 16., cx))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .ellipsis()
                .text_color(t.text)
                .child(e.name.clone()),
        )
        .when(e.is_navigable(), |d| {
            d.child(icon("chevron-right").size(px(12.)).text_color(t.text_faint))
        })
        .into_any_element()
}
