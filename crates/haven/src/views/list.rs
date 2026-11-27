//! Detail list view with sortable, multi-key column headers.

use gpui::{
    AnyElement, Context, ExternalPaths, FontWeight, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, ParentElement, SharedString, StatefulInteractiveElement, Styled, Window,
    canvas, div, prelude::FluentBuilder, px, uniform_list,
};
use haven_core::kind::kind_label;
use haven_core::{SortKey, fmt};

use super::{drag_payload, drag_preview, entry_icon_el, file_drop_target};
use crate::store::Store;
use crate::tab::{DraggedFiles, Location, Tab};
use crate::theme::theme;
use crate::ui::Ellipsis as _;
use crate::ui::{empty_state, icon, tag_dot};

pub const ROW_H: f32 = 26.;
const DATE_W: f32 = 150.;
const SIZE_W: f32 = 76.;
const KIND_W: f32 = 120.;
/// The Name column never gets narrower than this; optional columns are
/// dropped instead.
const NAME_MIN_W: f32 = 180.;
/// Horizontal padding around the columns (list padding plus row padding).
const H_PAD: f32 = 20.;

/// Which optional columns fit beside a readable Name column. When the list
/// is narrow, Kind goes first, then Date Modified, then Size, so names never
/// collapse under the other columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Columns {
    pub date: bool,
    pub size: bool,
    pub kind: bool,
}

impl Columns {
    pub fn for_width(width: f32) -> Columns {
        // Not measured yet (first frame): show everything.
        if width <= 0. {
            return Columns {
                date: true,
                size: true,
                kind: true,
            };
        }
        let room = width - H_PAD - NAME_MIN_W;
        let size = room >= SIZE_W;
        let date = room >= SIZE_W + DATE_W;
        let kind = room >= SIZE_W + DATE_W + KIND_W;
        Columns { date, size, kind }
    }
}

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
    let cols = Columns::for_width(tab.list_width);
    let entity = cx.entity();
    let header = div()
        .flex()
        .flex_none()
        .items_center()
        .h(px(28.))
        .px(px(10.))
        .border_b_1()
        .border_color(t.divider)
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .child(header_cell(tab, SortKey::Name, "Name", None, false, cx))
        .when(cols.date, |d| {
            d.child(header_cell(
                tab,
                SortKey::Modified,
                "Date Modified",
                Some(DATE_W),
                false,
                cx,
            ))
        })
        .when(cols.size, |d| {
            d.child(header_cell(
                tab,
                SortKey::Size,
                "Size",
                Some(SIZE_W),
                true,
                cx,
            ))
        })
        .when(cols.kind, |d| {
            d.child(header_cell(
                tab,
                SortKey::Kind,
                "Kind",
                Some(KIND_W),
                false,
                cx,
            ))
        });

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
            cx.processor(move |tab, range: std::ops::Range<usize>, _window, cx| {
                range.map(|ix| row(tab, ix, cols, cx)).collect::<Vec<_>>()
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
        .relative()
        .flex()
        .flex_col()
        .child(
            // Measures the available width to decide which columns fit.
            canvas(
                move |bounds, _, cx| {
                    let w = f32::from(bounds.size.width);
                    entity.update(cx, |tab, cx| {
                        if Columns::for_width(tab.list_width) != Columns::for_width(w) {
                            cx.notify();
                        }
                        tab.list_width = w;
                    });
                },
                |_, _, _, _| {},
            )
            .absolute()
            .size_full(),
        )
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

fn row(tab: &mut Tab, ix: usize, cols: Columns, cx: &mut Context<Tab>) -> AnyElement {
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
            file_drop_target(d, t.selection, cx, move |tab, paths, window, cx| {
                if !paths.contains(&path) {
                    tab.drop_on(path.clone(), paths, window, cx);
                }
            })
        })
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(6.))
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
        .when(cols.date, |d| {
            d.child(
                div()
                    .w(px(DATE_W))
                    .flex_none()
                    .px(px(6.))
                    .ellipsis()
                    .text_color(t.text_muted)
                    .child(fmt::date(entry.modified)),
            )
        })
        .when(cols.size, |d| {
            d.child(
                div()
                    .w(px(SIZE_W))
                    .flex_none()
                    .px(px(6.))
                    .flex()
                    .justify_end()
                    .text_color(t.text_muted)
                    .child(size),
            )
        })
        .when(cols.kind, |d| {
            d.child(
                div()
                    .w(px(KIND_W))
                    .flex_none()
                    .px(px(6.))
                    .ellipsis()
                    .text_color(t.text_muted)
                    .child(kind_label(&entry)),
            )
        })
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_list_shows_every_column() {
        let all = Columns {
            date: true,
            size: true,
            kind: true,
        };
        assert_eq!(Columns::for_width(1200.), all);
        // Unmeasured (first frame) also shows everything.
        assert_eq!(Columns::for_width(0.), all);
    }

    #[test]
    fn narrow_list_drops_kind_then_date_then_size() {
        let full = H_PAD + NAME_MIN_W + SIZE_W + DATE_W + KIND_W;
        assert!(Columns::for_width(full).kind);
        let c = Columns::for_width(full - 1.);
        assert!(!c.kind && c.date && c.size);
        let c = Columns::for_width(H_PAD + NAME_MIN_W + SIZE_W + DATE_W - 1.);
        assert!(!c.kind && !c.date && c.size);
        let c = Columns::for_width(H_PAD + NAME_MIN_W + SIZE_W - 1.);
        assert_eq!(
            c,
            Columns {
                date: false,
                size: false,
                kind: false
            }
        );
    }

    #[test]
    fn name_column_keeps_its_minimum_width() {
        // The screenshot bug: a ~365px list squeezed Name to nothing.
        for w in [300., 365., 420., 500., 640., 800.] {
            let c = Columns::for_width(w);
            let fixed = [(c.date, DATE_W), (c.size, SIZE_W), (c.kind, KIND_W)]
                .iter()
                .filter(|(on, _)| *on)
                .map(|(_, w)| w)
                .sum::<f32>();
            assert!(
                w - H_PAD - fixed >= NAME_MIN_W || fixed == 0.,
                "width {w}: name column too narrow"
            );
        }
    }
}
