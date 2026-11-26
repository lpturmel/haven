//! Rendering and keyboard handling for a tab's content area.

use gpui::{
    AppContext, Context, DragMoveEvent, Empty, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, Render, StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder,
    px,
};
use haven_core::state::ViewMode;

use crate::actions::*;
use crate::store::Store;
use crate::tab::{Location, Tab};
use crate::theme::theme;
use crate::views::{columns, grid, list, preview, start};

/// Drag marker for the preview splitter.
#[derive(Clone)]
pub struct PreviewSplitter;

impl Render for Tab {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let is_start = self.location == Location::Start;
        let content = if is_start {
            start::render(self, window, cx)
        } else {
            match self.view {
                ViewMode::List => list::render(self, window, cx),
                ViewMode::Grid => grid::render(self, window, cx),
                ViewMode::Columns => columns::render(self, window, cx),
            }
        };
        let state = Store::get(cx);
        let show_preview = state.preview_visible && !is_start && self.view != ViewMode::Columns;
        let preview_w = state.preview_width;

        div()
            .id("file-view")
            .key_context(FILE_VIEW)
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .on_action(cx.listener(|tab, _: &SelectNext, _, cx| {
                let step = if tab.view == ViewMode::Grid {
                    tab.grid_cols as isize
                } else {
                    1
                };
                tab.move_cursor(step, false, cx)
            }))
            .on_action(cx.listener(|tab, _: &SelectPrev, _, cx| {
                let step = if tab.view == ViewMode::Grid {
                    tab.grid_cols as isize
                } else {
                    1
                };
                tab.move_cursor(-step, false, cx)
            }))
            .on_action(cx.listener(|tab, _: &SelectNextExtend, _, cx| tab.move_cursor(1, true, cx)))
            .on_action(
                cx.listener(|tab, _: &SelectPrevExtend, _, cx| tab.move_cursor(-1, true, cx)),
            )
            .on_action(cx.listener(|tab, _: &SelectLeft, _, cx| match tab.view {
                ViewMode::Grid => tab.move_cursor(-1, false, cx),
                ViewMode::Columns => tab.column_left(cx),
                ViewMode::List => {}
            }))
            .on_action(cx.listener(|tab, _: &SelectRight, _, cx| match tab.view {
                ViewMode::Grid => tab.move_cursor(1, false, cx),
                ViewMode::Columns => tab.column_right(cx),
                ViewMode::List => {}
            }))
            .on_action(cx.listener(|tab, _: &SelectFirst, _, cx| tab.move_to_edge(false, cx)))
            .on_action(cx.listener(|tab, _: &SelectLast, _, cx| tab.move_to_edge(true, cx)))
            .on_action(cx.listener(|tab, _: &PageDown, _, cx| tab.move_cursor(20, false, cx)))
            .on_action(cx.listener(|tab, _: &PageUp, _, cx| tab.move_cursor(-20, false, cx)))
            .on_action(cx.listener(|tab, _: &SelectAllItems, _, cx| tab.select_all(cx)))
            .on_action(cx.listener(|tab, _: &ClearSelection, _, cx| tab.clear_selection(cx)))
            .on_action(
                cx.listener(|tab, _: &OpenSelected, window, cx| tab.open_selected(window, cx)),
            )
            .on_action(cx.listener(|tab, _: &OpenInNewTab, _, cx| tab.open_in_new_tab(cx)))
            .on_action(cx.listener(|tab, _: &GoBack, _, cx| tab.go_back(cx)))
            // Space is bound to TogglePreview, and bindings run before
            // on_key_down: let a type-to-select in progress take the space.
            .on_action(cx.listener(|tab, _: &TogglePreview, _, cx| {
                if !tab.type_select(" ", cx) {
                    cx.propagate();
                }
            }))
            .on_key_down(cx.listener(|tab, ev: &KeyDownEvent, _, cx| {
                let m = ev.keystroke.modifiers;
                if m.platform || m.control || m.alt || m.function {
                    return;
                }
                if let Some(ch) = ev.keystroke.key_char.as_deref()
                    && ch.chars().all(|c| !c.is_control())
                    && tab.type_select(ch, cx)
                {
                    cx.stop_propagation();
                }
            }))
            .on_drag_move(
                cx.listener(|_, ev: &DragMoveEvent<PreviewSplitter>, _, cx| {
                    let w = f32::from(ev.bounds.right() - ev.event.position.x).clamp(220., 900.);
                    Store::update(cx, |s| s.preview_width = w);
                    cx.notify();
                }),
            )
            .child(div().flex_1().min_w_0().h_full().child(content))
            .when(show_preview, |d| {
                d.child(
                    div()
                        .id("preview-splitter")
                        .w(px(5.))
                        .h_full()
                        .flex_none()
                        .cursor_col_resize()
                        .flex()
                        .justify_center()
                        .child(div().w(px(1.)).h_full().bg(t.divider))
                        .on_drag(PreviewSplitter, |_, _, _, cx| cx.new(|_| Empty)),
                )
                .child(
                    div()
                        .w(px(preview_w))
                        .flex_none()
                        .h_full()
                        .child(preview::render(self, window, cx)),
                )
            })
    }
}
