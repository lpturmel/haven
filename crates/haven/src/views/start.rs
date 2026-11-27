//! Start page: pinned items, drives and recently used files.

use std::path::PathBuf;
use std::time::Instant;

use gpui::{
    AnyElement, App, Context, FontWeight, Global, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, ParentElement, StatefulInteractiveElement, Styled, Window, div,
    prelude::FluentBuilder, px, relative,
};
use haven_core::recents::Recent;
use haven_core::volumes::Volume;
use haven_core::{Entry, fmt, tildify};

use super::entry_icon_el;
use crate::actions::EjectVolume;
use crate::actions::NavigateTo;
use crate::store::Store;
use crate::tab::{Location, Tab};
use crate::theme::theme;
use crate::ui::Ellipsis as _;
use crate::ui::{icon, icon_button, tag_dot};

/// Drives and recents, refreshed by the workspace.
#[derive(Default)]
pub struct StartData {
    pub volumes: Vec<Volume>,
    pub recents: Vec<Recent>,
    /// Metadata for the recents shown (stat'ed in the background, not per render).
    pub recent_entries: Vec<Option<Entry>>,
    pub recents_loaded: Option<Instant>,
    /// Sidebar places that exist (checked in the background).
    pub places: Option<Vec<(String, PathBuf, &'static str)>>,
    pub pinned: Vec<Entry>,
}

impl Global for StartData {}

pub fn start_data(cx: &App) -> &StartData {
    cx.global::<StartData>()
}

fn section(icon_name: &str, title: &str, cx: &App) -> gpui::Div {
    let t = theme(cx);
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .mb(px(12.))
        .child(icon(icon_name).size(px(16.)).text_color(t.text_muted))
        .child(
            div()
                .text_size(px(16.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.text)
                .child(title.to_string()),
        )
}

pub fn render(_tab: &mut Tab, _window: &mut Window, cx: &mut Context<Tab>) -> AnyElement {
    let t = theme(cx).clone();
    let pinned = start_data(cx).pinned.clone();
    let volumes = start_data(cx).volumes.clone();
    let recents = start_data(cx).recents.clone();
    let store_tags = Store::get(cx).tags.clone();

    let pinned_tiles: Vec<AnyElement> = pinned
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let path = e.path.clone();
            let unpin = e.path.clone();
            let is_dir = e.is_navigable();
            div()
                .id(("pin", i))
                .group("pin")
                .relative()
                .w(px(104.))
                .flex()
                .flex_col()
                .items_center()
                .gap(px(6.))
                .p(px(8.))
                .rounded(px(8.))
                .hover(|s| s.bg(t.hover))
                .cursor_pointer()
                .on_click(cx.listener(move |tab, ev: &gpui::ClickEvent, _w, cx| {
                    if is_dir {
                        tab.navigate(Location::Dir(path.clone()), true, cx);
                    } else if ev.click_count() >= 1 {
                        cx.open_with_system(&path);
                        Store::update(cx, |s| s.record_opened(&path));
                    }
                }))
                .child(entry_icon_el(e, 56., cx))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .max_w_full()
                        .children(e.tags.first().map(|tag| {
                            let color = store_tags
                                .iter()
                                .find(|s| s.name == tag.name)
                                .map(|s| s.color)
                                .unwrap_or(tag.color);
                            tag_dot(t.tag(color), px(8.))
                        }))
                        .child(
                            div()
                                .ellipsis()
                                .text_size(px(12.))
                                .text_color(t.text)
                                .child(e.name.clone()),
                        ),
                )
                .child(
                    div()
                        .absolute()
                        .top(px(2.))
                        .right(px(2.))
                        .invisible()
                        .group_hover("pin", |s| s.visible())
                        .child(icon_button(("unpin", i), "x", cx).size(px(20.)).on_click(
                            cx.listener(move |_, _, _, cx| {
                                cx.stop_propagation();
                                Store::update(cx, |s| s.toggle_start_pin(&unpin));
                                // The tiles come from StartData, not the store.
                                crate::workspace::refresh_pins(cx);
                                cx.notify();
                            }),
                        )),
                )
                .into_any_element()
        })
        .collect();

    let drives: Vec<AnyElement> =
        volumes
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let mount = v.mount.clone();
                let entry = Entry::from_path(&v.mount).ok();
                div()
                    .id(("drive", i))
                    .w(px(250.))
                    .flex()
                    .gap(px(12.))
                    .items_center()
                    .p(px(12.))
                    .rounded(px(8.))
                    .bg(t.elevated)
                    .border_1()
                    .border_color(t.card_border)
                    .hover(|s| s.bg(t.active))
                    .cursor_pointer()
                    .on_click(cx.listener(move |tab, _, _, cx| {
                        tab.navigate(Location::Dir(mount.clone()), true, cx)
                    }))
                    .child(match &entry {
                        Some(e) => entry_icon_el(e, 40., cx),
                        None => icon("hard-drive").size(px(36.)).into_any_element(),
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(
                                div()
                                    .ellipsis()
                                    .text_size(px(13.))
                                    .text_color(t.text)
                                    .child(v.name.clone()),
                            )
                            .child(div().text_size(px(11.)).text_color(t.text_muted).child(
                                format!(
                                    "{} free of {}",
                                    fmt::size(v.available),
                                    fmt::size(v.total)
                                ),
                            ))
                            .child(
                                div().h(px(4.)).w_full().rounded_full().bg(t.hover).child(
                                    div()
                                        .h_full()
                                        .rounded_full()
                                        .bg(if v.used_fraction() > 0.9 {
                                            t.danger
                                        } else {
                                            t.accent
                                        })
                                        .w(relative(v.used_fraction())),
                                ),
                            ),
                    )
                    .when(v.ejectable, |d| {
                        let m = v.mount.clone();
                        d.child(
                            div()
                                .id(("eject-drive", i))
                                .size(px(20.))
                                .flex_none()
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(4.))
                                .cursor_pointer()
                                .hover(|s| s.bg(t.control_hover))
                                .tooltip(|_, cx| crate::ui::tooltip("Eject", cx))
                                .on_click(move |_, window, cx| {
                                    cx.stop_propagation();
                                    window.dispatch_action(Box::new(EjectVolume(m.clone())), cx);
                                })
                                .child(icon("eject").size(px(13.)).text_color(t.text_muted)),
                        )
                    })
                    .into_any_element()
            })
            .collect();

    let recent_rows: Vec<AnyElement> = recents
        .iter()
        .take(24)
        .enumerate()
        .map(|(i, r)| {
            let path = r.path.clone();
            let open = r.path.clone();
            let entry = start_data(cx).recent_entries.get(i).cloned().flatten();
            let parent = r.path.parent().map(tildify).unwrap_or_default();
            div()
                .id(("recent", i))
                .h(px(30.))
                .px(px(8.))
                .flex()
                .items_center()
                .gap(px(10.))
                .rounded(px(6.))
                .hover(|s| s.bg(t.hover))
                .cursor_pointer()
                .on_mouse_down(
                    MouseButton::Left,
                    // A click opens the file (the first click used to navigate
                    // away, so the double-click branch could never run);
                    // ⌘-click or a click on the folder path reveals it.
                    cx.listener(move |_, ev: &MouseDownEvent, window, cx| {
                        if ev.modifiers.platform {
                            window.dispatch_action(Box::new(NavigateTo(path.clone())), cx);
                        } else if ev.click_count == 1 {
                            cx.open_with_system(&open);
                            Store::update(cx, |s| s.record_opened(&open));
                        }
                    }),
                )
                // Reserve the icon slot while it loads, so names stay aligned.
                .child(match entry.as_ref() {
                    Some(e) => entry_icon_el(e, 18., cx),
                    None => div().size(px(18.)).flex_none().into_any_element(),
                })
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .ellipsis()
                        .text_size(px(13.))
                        .text_color(t.text)
                        .child(
                            r.path
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default(),
                        ),
                )
                .child(
                    div()
                        .id(("recent-reveal", i))
                        .w(px(300.))
                        .flex_none()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(12.))
                        .text_color(t.text_muted)
                        .hover(|s| s.text_color(t.text))
                        .on_mouse_down(MouseButton::Left, {
                            let reveal = r.path.clone();
                            move |_, window, cx| {
                                cx.stop_propagation();
                                window.dispatch_action(Box::new(NavigateTo(reveal.clone())), cx);
                            }
                        })
                        .child(parent),
                )
                .child(
                    div()
                        .w(px(90.))
                        .flex_none()
                        .flex()
                        .justify_end()
                        .text_size(px(12.))
                        .text_color(t.text_faint)
                        .child(fmt::relative(r.used)),
                )
                .into_any_element()
        })
        .collect();

    let recents_loading = start_data(cx).recents_loaded.is_none();

    div()
        .id("start")
        .size_full()
        .overflow_y_scroll()
        .px(px(28.))
        .py(px(22.))
        .flex()
        .flex_col()
        .gap(px(28.))
        .child(
            div()
                .id("start-pinned")
                .rounded(px(8.))
                .map(|d| {
                    super::file_drop_target(d, t.hover, cx, |_, dropped, _, cx| {
                        pin_all(&dropped.paths, cx)
                    })
                })
                .child(section("pin", "Pinned", cx))
                .child(if pinned_tiles.is_empty() {
                    div()
                        .text_size(px(12.))
                        .text_color(t.text_faint)
                        .child("Drag files or folders here, or use “Pin to Start” from the context menu.")
                        .into_any_element()
                } else {
                    div().flex().flex_wrap().gap(px(6.)).children(pinned_tiles).into_any_element()
                }),
        )
        .child(
            div()
                .child(section("hard-drive", "Drives", cx))
                .child(div().flex().flex_wrap().gap(px(12.)).children(drives)),
        )
        .child(
            div()
                .child(section("history", "Recently used files", cx))
                .when(recents_loading, |d| d.child(div().text_size(px(12.)).text_color(t.text_faint).child("Loading…")))
                .when(!recents_loading && recent_rows.is_empty(), |d| {
                    d.child(div().text_size(px(12.)).text_color(t.text_faint).child("Nothing yet"))
                })
                .child(div().flex().flex_col().children(recent_rows)),
        )
        .into_any_element()
}

fn pin_all(paths: &[PathBuf], cx: &mut Context<Tab>) {
    let paths = paths.to_vec();
    Store::update(cx, |s| {
        for p in &paths {
            if !s.start_pins.contains(p) {
                s.start_pins.push(p.clone());
            }
        }
    });
    crate::workspace::refresh_pins(cx);
    cx.notify();
}
