//! Workspace rendering: frosted sidebar, main card, toolbar, status bar, HUD.

use std::path::{Path, PathBuf};

use gpui::{
    AnyElement, AppContext, Context, DragMoveEvent, Empty, ExternalPaths, FontWeight,
    InteractiveElement, IntoElement, MouseButton, MouseDownEvent, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px,
    relative,
};
use haven_core::state::ViewMode;
use haven_core::{TagColor, fmt, home_dir};

use crate::actions::*;
use crate::icons::{IconSize, path_icon};
use crate::overlays::{self, Overlay, render_context_menu, render_overlay, sort_menu};
use crate::store::Store;
use crate::tab::{DraggedFiles, Location};
use crate::theme::{UI_FONT, theme};
use crate::ui::{icon, icon_button, section_label, tag_dot};
use crate::views::start::start_data;
use crate::workspace::Workspace;

#[derive(Clone)]
pub struct SidebarSplitter;

/// Drag payload for reordering sidebar pins.
#[derive(Clone)]
pub struct DraggedPin(pub PathBuf);

impl Render for DraggedPin {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        div()
            .px(px(10.))
            .py(px(4.))
            .rounded(px(6.))
            .bg(t.popover_bg)
            .text_size(px(12.))
            .text_color(t.text)
            .child(
                self.0
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            )
    }
}

fn places() -> Vec<(String, PathBuf, &'static str)> {
    let home = home_dir();
    let mut v = vec![
        ("Desktop".to_string(), home.join("Desktop"), "monitor"),
        ("Documents".to_string(), home.join("Documents"), "file-text"),
        ("Downloads".to_string(), home.join("Downloads"), "download"),
        ("Pictures".to_string(), home.join("Pictures"), "image"),
        ("Movies".to_string(), home.join("Movies"), "video"),
        ("Music".to_string(), home.join("Music"), "music"),
        (
            "Applications".to_string(),
            PathBuf::from("/Applications"),
            "app-window",
        ),
        (
            home.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Home".into()),
            home.clone(),
            "house",
        ),
    ];
    let icloud = home.join("Library/Mobile Documents/com~apple~CloudDocs");
    if icloud.exists() {
        v.push(("iCloud Drive".into(), icloud, "cloud"));
    }
    v.retain(|(_, p, _)| p.exists());
    v
}

impl Workspace {
    fn sidebar_row(
        &self,
        id: SharedString,
        label: String,
        icon_name: &'static str,
        active: bool,
        path: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let t = theme(cx).clone();
        let row = div()
            .id(id)
            .h(px(28.))
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(8.))
            .rounded(px(6.))
            .text_size(px(13.))
            .text_color(t.text)
            .cursor_pointer()
            .when(active, |d| d.bg(t.active))
            .when(!active, |d| d.hover(|s| s.bg(t.hover)))
            .child(icon(icon_name).size(px(15.)).text_color(if active {
                t.accent_text
            } else {
                t.text_muted
            }))
            .child(div().flex_1().min_w_0().truncate().child(label));
        match path {
            Some(path) => {
                let (p1, p2, p3) = (path.clone(), path.clone(), path.clone());
                row.on_click(cx.listener(move |ws, ev: &gpui::ClickEvent, window, cx| {
                    if ev.modifiers().platform {
                        ws.add_tab(Location::Dir(p1.clone()), true, window, cx);
                    } else {
                        ws.navigate_to(&p1, window, cx);
                    }
                }))
                .drag_over::<DraggedFiles>(move |s, _, _, _| s.bg(t.selection))
                .on_drop(cx.listener(move |ws, files: &DraggedFiles, window, cx| {
                    let copy = window.modifiers().alt;
                    let kind = if copy {
                        haven_core::ops::OpKind::Copy
                    } else {
                        haven_core::ops::OpKind::Move
                    };
                    ws.start_transfer(kind, files.paths.clone(), p2.clone(), window, cx);
                }))
                .on_drop(cx.listener(
                    move |ws, ext: &ExternalPaths, window, cx| {
                        ws.start_transfer(
                            haven_core::ops::OpKind::Copy,
                            ext.paths().to_vec(),
                            p3.clone(),
                            window,
                            cx,
                        );
                    },
                ))
            }
            None => row,
        }
    }

    fn section(
        &self,
        key: &'static str,
        title: &'static str,
        rows: Vec<AnyElement>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = theme(cx).clone();
        let collapsed = Store::get(cx).is_collapsed(key);
        div()
            .flex()
            .flex_col()
            .gap(px(1.))
            .child(
                div()
                    .id(SharedString::from(format!("sec-{key}")))
                    .group("sec")
                    .flex()
                    .items_center()
                    .justify_between()
                    .px(px(8.))
                    .pt(px(12.))
                    .pb(px(4.))
                    .cursor_pointer()
                    .on_click(cx.listener(move |_, _, _, cx| {
                        Store::update(cx, |s| s.toggle_section(key));
                        cx.notify();
                    }))
                    .child(section_label(title, cx))
                    .child(
                        icon(if collapsed {
                            "chevron-right"
                        } else {
                            "chevron-down"
                        })
                        .size(px(12.))
                        .text_color(t.text_faint)
                        .invisible()
                        .group_hover("sec", |s| s.visible()),
                    ),
            )
            .when(!collapsed, |d| d.children(rows))
            .into_any_element()
    }

    fn render_sidebar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = theme(cx).clone();
        let state = Store::get(cx).clone();
        let active_loc = self.active_tab().read(cx).location.clone();
        let active_dir = active_loc.dir().map(Path::to_path_buf);

        let home_row = self
            .sidebar_row(
                "sb-home".into(),
                "Home".into(),
                "house",
                active_loc == Location::Start,
                None,
                cx,
            )
            .on_click(cx.listener(|ws, _, window, cx| {
                let tab = ws.active_tab().clone();
                tab.update(cx, |t, cx| t.navigate(Location::Start, true, cx));
                window.focus(&tab.read(cx).focus.clone());
            }))
            .into_any_element();

        let pins: Vec<AnyElement> = state
            .sidebar_pins
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let label = p
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| p.display().to_string());
                let active = active_dir.as_ref() == Some(p);
                let path = p.clone();
                let unpin = p.clone();
                self.sidebar_row(
                    SharedString::from(format!("sb-pin-{i}")),
                    label,
                    "folder",
                    active,
                    Some(p.clone()),
                    cx,
                )
                .group("pinrow")
                .on_drag(DraggedPin(path.clone()), |pin, _, _, cx| {
                    cx.new(|_| pin.clone())
                })
                .drag_over::<DraggedPin>(move |s, _, _, _| s.border_t_2().border_color(t.accent))
                .on_drop(cx.listener(move |_, pin: &DraggedPin, _, cx| {
                    Store::update(cx, |s| s.reorder_sidebar_pin(&pin.0, i));
                    cx.notify();
                }))
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |ws, ev: &MouseDownEvent, _, cx| {
                        let items = vec![
                            overlays::MenuEntry::item(
                                "Open in New Tab",
                                "plus",
                                NavigateTo(path.clone()),
                                cx,
                            ),
                            overlays::MenuEntry::item(
                                "Remove from Sidebar",
                                "pin-off",
                                UnpinSidebar(path.clone()),
                                cx,
                            ),
                        ];
                        ws.open_menu_at(ev.position, items, cx);
                    }),
                )
                .child(
                    div()
                        .id(SharedString::from(format!("unpin-{i}")))
                        .invisible()
                        .group_hover("pinrow", |s| s.visible())
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.stop_propagation();
                            Store::update(cx, |s| s.toggle_sidebar_pin(&unpin));
                            cx.notify();
                        }))
                        .child(icon("x").size(px(12.)).text_color(t.text_faint)),
                )
                .into_any_element()
            })
            .collect();
        let pins = if pins.is_empty() {
            vec![
                div()
                    .px(px(8.))
                    .py(px(4.))
                    .text_size(px(11.))
                    .text_color(t.text_faint)
                    .child("Drop folders here to pin")
                    .into_any_element(),
            ]
        } else {
            pins
        };

        let place_rows: Vec<AnyElement> = places()
            .into_iter()
            .enumerate()
            .map(|(i, (label, path, ic))| {
                let active = active_dir.as_ref() == Some(&path);
                self.sidebar_row(
                    SharedString::from(format!("sb-place-{i}")),
                    label,
                    ic,
                    active,
                    Some(path),
                    cx,
                )
                .into_any_element()
            })
            .collect();

        let drives: Vec<AnyElement> = start_data(cx)
            .volumes
            .clone()
            .into_iter()
            .enumerate()
            .map(|(i, v)| {
                let active = active_dir.as_ref() == Some(&v.mount);
                let mount = v.mount.clone();
                let row = self.sidebar_row(
                    SharedString::from(format!("sb-drive-{i}")),
                    v.name.clone(),
                    if v.ejectable { "usb" } else { "hard-drive" },
                    active,
                    Some(v.mount.clone()),
                    cx,
                );
                row.when(v.ejectable, |d| {
                    d.child(
                        div()
                            .id(SharedString::from(format!("eject-{i}")))
                            .px(px(2.))
                            .on_click(cx.listener(move |ws, _, _, cx| {
                                cx.stop_propagation();
                                let m = mount.clone();
                                let task = cx
                                    .background_executor()
                                    .spawn(async move { haven_core::volumes::eject(&m) });
                                cx.spawn(async move |this, cx| {
                                    let r = task.await;
                                    let _ = this.update(cx, |ws, cx| {
                                        match r {
                                            Ok(()) => ws.show_toast("Ejected", false, cx),
                                            Err(e) => ws.show_toast(e, true, cx),
                                        }
                                        crate::workspace::refresh_start_data(cx);
                                    });
                                })
                                .detach();
                                let _ = ws;
                            }))
                            .child(icon("arrow-up").size(px(12.)).text_color(t.text_faint)),
                    )
                })
                .into_any_element()
            })
            .collect();

        let tags: Vec<AnyElement> = state
            .tags
            .iter()
            .enumerate()
            .map(|(i, tag)| {
                let active = active_loc == Location::Tag(tag.name.clone());
                let name = tag.name.clone();
                let color = if tag.color == TagColor::None {
                    t.text_faint
                } else {
                    t.tag(tag.color)
                };
                div()
                    .id(SharedString::from(format!("sb-tag-{i}")))
                    .h(px(28.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .rounded(px(6.))
                    .text_size(px(13.))
                    .text_color(t.text)
                    .cursor_pointer()
                    .when(active, |d| d.bg(t.active))
                    .when(!active, |d| d.hover(|s| s.bg(t.hover)))
                    .on_click(cx.listener(move |ws, _, window, cx| {
                        let tab = ws.active_tab().clone();
                        tab.update(cx, |t, cx| {
                            t.navigate(Location::Tag(name.clone()), true, cx)
                        });
                        window.focus(&tab.read(cx).focus.clone());
                    }))
                    .child(
                        div()
                            .w(px(15.))
                            .flex()
                            .justify_center()
                            .child(tag_dot(color, px(9.))),
                    )
                    .child(tag.name.clone())
                    .into_any_element()
            })
            .collect();

        let width = state.sidebar_width;
        let pin_drop = div()
            .id("sb-pins-drop")
            .rounded(px(6.))
            .drag_over::<DraggedFiles>(move |s, _, _, _| s.bg(t.hover))
            .on_drop(cx.listener(|_, files: &DraggedFiles, _, cx| {
                let dirs: Vec<PathBuf> =
                    files.paths.iter().filter(|p| p.is_dir()).cloned().collect();
                Store::update(cx, |s| {
                    for d in &dirs {
                        if !s.sidebar_pins.contains(d) {
                            s.sidebar_pins.push(d.clone());
                        }
                    }
                });
                cx.notify();
            }))
            .child(self.section("pinned", "Pinned", pins, cx));

        div()
            .id("sidebar")
            .w(px(width))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .bg(t.sidebar_bg)
            .child(
                // Traffic-light area doubles as a window drag handle.
                div()
                    .h(px(46.))
                    .flex_none()
                    .on_mouse_down(MouseButton::Left, |ev, window, _| {
                        if ev.click_count == 2 {
                            window.titlebar_double_click();
                        } else {
                            window.start_window_move();
                        }
                    }),
            )
            .child(
                div()
                    .id("sidebar-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(10.))
                    .pb(px(12.))
                    .child(home_row)
                    .child(pin_drop)
                    .child(self.section("places", "Your Files", place_rows, cx))
                    .child(self.section("drives", "Drives", drives, cx))
                    .child(self.section("tags", "Tags", tags, cx)),
            )
            .into_any_element()
    }

    fn render_tab_strip(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = theme(cx).clone();
        let tabs: Vec<AnyElement> = self
            .tabs
            .iter()
            .enumerate()
            .map(|(i, tab)| {
                let tab = tab.read(cx);
                let active = i == self.active;
                let title = tab.location.title();
                let ic = match &tab.location {
                    Location::Start => "house",
                    Location::Tag(_) => "tag",
                    Location::Dir(_) => "folder",
                };
                let dest = tab.dir().map(Path::to_path_buf);
                div()
                    .id(("tab", i))
                    .group("tab")
                    .h(px(28.))
                    .min_w(px(100.))
                    .max_w(px(200.))
                    .flex_1()
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .rounded(px(7.))
                    .text_size(px(12.))
                    .cursor_pointer()
                    .when(active, |d| {
                        d.bg(t.elevated)
                            .text_color(t.text)
                            .border_1()
                            .border_color(t.card_border)
                    })
                    .when(!active, |d| {
                        d.text_color(t.text_muted).hover(|s| s.bg(t.hover))
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |ws, _: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            ws.activate(i, window, cx)
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Middle,
                        cx.listener(move |ws, _: &MouseDownEvent, window, cx| {
                            ws.close_tab(i, window, cx)
                        }),
                    )
                    .when_some(dest, |d, dest| {
                        d.drag_over::<DraggedFiles>(move |s, _, _, _| s.bg(t.selection))
                            .on_drop(cx.listener(move |ws, files: &DraggedFiles, window, cx| {
                                let kind = if window.modifiers().alt {
                                    haven_core::ops::OpKind::Copy
                                } else {
                                    haven_core::ops::OpKind::Move
                                };
                                ws.start_transfer(
                                    kind,
                                    files.paths.clone(),
                                    dest.clone(),
                                    window,
                                    cx,
                                );
                            }))
                    })
                    .child(icon(ic).size(px(13.)).text_color(if active {
                        t.accent_text
                    } else {
                        t.text_faint
                    }))
                    .child(div().flex_1().min_w_0().truncate().child(title))
                    .child(
                        div()
                            .id(("tab-close", i))
                            .size(px(16.))
                            .rounded(px(4.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .invisible()
                            .group_hover("tab", |s| s.visible())
                            .hover(|s| s.bg(t.active))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(
                                cx.listener(move |ws, _, window, cx| ws.close_tab(i, window, cx)),
                            )
                            .child(icon("x").size(px(11.)).text_color(t.text_muted)),
                    )
                    .into_any_element()
            })
            .collect();
        div()
            .id("tab-strip")
            .h(px(40.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(4.))
            .px(px(8.))
            .when(!self.sidebar_visible, |d| d.pl(px(80.)))
            .on_mouse_down(MouseButton::Left, |ev, window, _| {
                if ev.click_count == 2 {
                    window.titlebar_double_click();
                } else {
                    window.start_window_move();
                }
            })
            .children(tabs)
            .child(icon_button("new-tab", "plus", cx).on_click(
                cx.listener(|ws, _, window, cx| ws.add_tab(Location::Start, true, window, cx)),
            ))
            .child(div().flex_1())
            .into_any_element()
    }

    fn render_path_bar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = theme(cx).clone();
        let tab = self.active_tab().read(cx);
        let location = tab.location.clone();
        let border = if self.path_error.is_some() {
            t.danger
        } else if self.path_editing {
            t.accent
        } else {
            t.field_border
        };
        let inner: AnyElement = if self.path_editing {
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(
                    icon("text-cursor-input")
                        .size(px(13.))
                        .text_color(t.accent_text),
                )
                .child(self.path_field.clone())
                .into_any_element()
        } else {
            let crumbs: Vec<AnyElement> = match &location {
                Location::Start => vec![div().text_color(t.text).child("Home").into_any_element()],
                Location::Tag(name) => vec![
                    div()
                        .text_color(t.text_muted)
                        .child("Tags")
                        .into_any_element(),
                    icon("chevron-right")
                        .size(px(12.))
                        .text_color(t.text_faint)
                        .into_any_element(),
                    div()
                        .text_color(t.text)
                        .child(name.clone())
                        .into_any_element(),
                ],
                Location::Dir(dir) => {
                    let ancestors: Vec<PathBuf> = {
                        let mut v: Vec<PathBuf> = dir.ancestors().map(Path::to_path_buf).collect();
                        v.reverse();
                        v
                    };
                    let n = ancestors.len();
                    let mut out = Vec::new();
                    for (i, a) in ancestors.into_iter().enumerate() {
                        let last = i + 1 == n;
                        let label = if a == Path::new("/") {
                            start_data(cx)
                                .volumes
                                .first()
                                .map(|v| v.name.clone())
                                .unwrap_or_else(|| "/".into())
                        } else {
                            a.file_name()
                                .map(|s| s.to_string_lossy().into_owned())
                                .unwrap_or_default()
                        };
                        if i > 0 {
                            out.push(
                                icon("chevron-right")
                                    .size(px(11.))
                                    .text_color(t.text_faint)
                                    .into_any_element(),
                            );
                        }
                        let icon_el = (i == 0)
                            .then(|| path_icon(&a, true, false, "", IconSize::Small, cx))
                            .flatten();
                        out.push(
                            div()
                                .id(("crumb", i))
                                .flex_none()
                                .flex()
                                .items_center()
                                .gap(px(5.))
                                .px(px(5.))
                                .py(px(2.))
                                .rounded(px(4.))
                                .text_color(if last { t.text } else { t.text_muted })
                                .when(last, |d| d.font_weight(FontWeight::MEDIUM))
                                .hover(|s| s.bg(t.hover).text_color(t.text))
                                .cursor_pointer()
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .on_click(cx.listener({
                                    let a = a.clone();
                                    move |ws, _, window, cx| ws.navigate_to(&a, window, cx)
                                }))
                                .drag_over::<DraggedFiles>(move |s, _, _, _| s.bg(t.selection))
                                .on_drop(cx.listener({
                                    let a = a.clone();
                                    move |ws, files: &DraggedFiles, window, cx| {
                                        ws.start_transfer(
                                            haven_core::ops::OpKind::Move,
                                            files.paths.clone(),
                                            a.clone(),
                                            window,
                                            cx,
                                        )
                                    }
                                }))
                                .children(icon_el.map(|img| gpui::img(img).size(px(14.))))
                                .child(label)
                                .into_any_element(),
                        );
                    }
                    out
                }
            };
            div()
                .id("crumbs")
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(1.))
                .overflow_x_scroll()
                .children(crumbs)
                .into_any_element()
        };

        let completions = if self.path_editing && !self.completions.is_empty() {
            let rows: Vec<AnyElement> = self
                .completions
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    let selected = self.completion_ix == Some(i);
                    let replacement = c.replacement.clone();
                    div()
                        .id(("cmp", i))
                        .h(px(26.))
                        .px(px(10.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .rounded(px(5.))
                        .text_size(px(12.5))
                        .text_color(t.text)
                        .when(selected, |d| d.bg(t.selection))
                        .hover(|s| s.bg(t.hover))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |ws, _: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                ws.path_field.update(cx, |f, cx| {
                                    f.set_text(replacement.clone(), false, cx);
                                    cx.emit(crate::text_field::TextFieldEvent::Changed);
                                });
                                window.focus(&ws.path_field.read(cx).focus_handle_ref().clone());
                            }),
                        )
                        .child(
                            icon(if c.is_dir { "folder" } else { "file" })
                                .size(px(13.))
                                .text_color(if c.is_dir { t.folder } else { t.text_muted }),
                        )
                        .child(c.name.clone())
                        .into_any_element()
                })
                .collect();
            Some(
                div()
                    .absolute()
                    .top(px(34.))
                    .left_0()
                    .w_full()
                    .p(px(4.))
                    .rounded(px(8.))
                    .bg(t.popover_bg)
                    .border_1()
                    .border_color(t.card_border)
                    .shadow_lg()
                    .occlude()
                    .children(rows)
                    .child(
                        div()
                            .px(px(10.))
                            .pt(px(4.))
                            .text_size(px(10.5))
                            .text_color(t.text_faint)
                            .child("Tab completes · ↑↓ choose · ↩ go"),
                    ),
            )
        } else {
            None
        };

        div()
            .id("path-bar")
            .relative()
            .flex_1()
            .min_w_0()
            .h(px(30.))
            .px(px(8.))
            .flex()
            .items_center()
            .rounded(px(7.))
            .bg(t.field_bg)
            .border_1()
            .border_color(border)
            .text_size(px(12.5))
            .cursor_text()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|ws, _: &MouseDownEvent, window, cx| {
                    if !ws.path_editing {
                        ws.begin_path_edit(window, cx);
                    }
                }),
            )
            .child(inner)
            .when_some(self.path_error.clone(), |d, e| {
                d.child(
                    div()
                        .flex_none()
                        .pl(px(8.))
                        .text_size(px(11.))
                        .text_color(t.danger)
                        .child(e),
                )
            })
            .children(completions)
            .into_any_element()
    }

    fn render_toolbar(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let t = theme(cx).clone();
        let tab = self.active_tab().read(cx);
        let (can_back, can_fwd, has_dir) =
            (tab.can_go_back(), tab.can_go_forward(), tab.dir().is_some());
        let view = tab.view;
        let preview_on = Store::get(cx).preview_visible;
        let nav = |id: &'static str, ic: &'static str, enabled: bool, cx: &mut Context<Self>| {
            icon_button(id, ic, cx).when(!enabled, |d| d.opacity(0.35))
        };
        let seg = |id: &'static str, ic: &'static str, mode: ViewMode, cx: &mut Context<Self>| {
            let on = view == mode && has_dir;
            div()
                .id(id)
                .size(px(26.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(5.))
                .cursor_pointer()
                .when(on, |d| d.bg(t.active))
                .hover(|s| s.bg(t.hover))
                .on_click(cx.listener(move |ws, _, _, cx| {
                    ws.active_tab()
                        .clone()
                        .update(cx, |tab, cx| tab.set_view(mode, cx));
                }))
                .child(
                    icon(ic)
                        .size(px(14.))
                        .text_color(if on { t.text } else { t.text_muted }),
                )
        };
        let filter_focused = self
            .filter_field
            .read(cx)
            .focus_handle_ref()
            .is_focused(window);
        div()
            .h(px(44.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(6.))
            .px(px(10.))
            .border_b_1()
            .border_color(t.divider)
            .child(
                nav("back", "arrow-left", can_back, cx).on_click(cx.listener(|ws, _, _, cx| {
                    ws.active_tab().clone().update(cx, |t, cx| t.go_back(cx))
                })),
            )
            .child(
                nav("fwd", "arrow-right", can_fwd, cx).on_click(cx.listener(|ws, _, _, cx| {
                    ws.active_tab().clone().update(cx, |t, cx| t.go_forward(cx))
                })),
            )
            .child(
                nav("up", "arrow-up", has_dir, cx).on_click(cx.listener(|ws, _, _, cx| {
                    ws.active_tab().clone().update(cx, |t, cx| t.go_parent(cx))
                })),
            )
            .child(self.render_path_bar(cx))
            .child(
                div()
                    .id("filter")
                    .w(px(190.))
                    .flex_none()
                    .h(px(30.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .rounded(px(7.))
                    .bg(t.field_bg)
                    .border_1()
                    .border_color(if filter_focused {
                        t.accent
                    } else {
                        t.field_border
                    })
                    .text_size(px(12.5))
                    .text_color(t.text)
                    .child(icon("search").size(px(13.)).text_color(t.text_faint))
                    .child(self.filter_field.clone())
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(t.text_faint)
                            .child("⌘P"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .p(px(2.))
                    .gap(px(1.))
                    .rounded(px(7.))
                    .bg(t.field_bg)
                    .border_1()
                    .border_color(t.field_border)
                    .child(seg("v-list", "list", ViewMode::List, cx))
                    .child(seg("v-grid", "layout-grid", ViewMode::Grid, cx))
                    .child(seg("v-cols", "columns-3", ViewMode::Columns, cx)),
            )
            .child(icon_button("sort", "arrow-up-down", cx).on_mouse_down(
                MouseButton::Left,
                cx.listener(|ws, ev: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    let items = sort_menu(ws, cx);
                    ws.open_menu_at(ev.position, items, cx);
                }),
            ))
            .child(
                icon_button("preview-toggle", "panel-right", cx)
                    .when(preview_on, |d| d.bg(t.active))
                    .on_click(cx.listener(|_, _, window, cx| {
                        window.dispatch_action(Box::new(TogglePreview), cx)
                    })),
            )
            .child(
                icon_button("palette", "command", cx).on_click(
                    cx.listener(|ws, _, window, cx| overlays::open_palette(ws, window, cx)),
                ),
            )
            .into_any_element()
    }

    fn render_status(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = theme(cx).clone();
        let tab = self.active_tab().read(cx);
        let total = tab.entries.len();
        let selected = tab.selected_entries();
        let sel_size: u64 = selected.iter().filter_map(|e| e.effective_size()).sum();
        let left = match &tab.location {
            Location::Start => "Home".to_string(),
            _ if !selected.is_empty() => format!(
                "{} of {} selected · {}",
                selected.len(),
                total,
                fmt::size(sel_size)
            ),
            _ => format!("{} item{}", total, if total == 1 { "" } else { "s" }),
        };
        let filtered = !tab.filter.is_empty();
        let hidden = Store::get(cx).show_hidden;
        div()
            .h(px(26.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(14.))
            .border_t_1()
            .border_color(t.divider)
            .text_size(px(11.))
            .text_color(t.text_faint)
            .child(left)
            .when(filtered, |d| {
                d.child(
                    div()
                        .text_color(t.accent_text)
                        .child(format!("filtered by “{}”", tab.filter)),
                )
            })
            .when(tab.loading, |d| d.child("Loading…"))
            .when(tab.sizing, |d| d.child("Calculating sizes…"))
            .child(div().flex_1())
            .when(hidden, |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .child(icon("eye").size(px(11.)).text_color(t.text_faint))
                        .child("hidden files"),
                )
            })
            .children(
                tab.free_space
                    .map(|f| format!("{} available", fmt::size(f))),
            )
            .into_any_element()
    }

    fn render_hud(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let t = theme(cx).clone();
        if self.jobs.is_empty() && self.toast.is_none() {
            return None;
        }
        let jobs: Vec<AnyElement> = self
            .jobs
            .iter()
            .map(|job| {
                let p = &job.progress;
                let frac = p.fraction().clamp(0., 1.);
                let done = job.done_at.is_some();
                let current = p.current.lock().map(|c| c.clone()).unwrap_or_default();
                let id = job.id;
                div()
                    .w(px(300.))
                    .p(px(10.))
                    .rounded(px(8.))
                    .bg(t.popover_bg)
                    .border_1()
                    .border_color(t.card_border)
                    .shadow_lg()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(12.))
                                    .text_color(t.text)
                                    .child(job.label.clone()),
                            )
                            .child(if done {
                                icon("check")
                                    .size(px(13.))
                                    .text_color(t.success)
                                    .into_any_element()
                            } else {
                                icon_button(("cancel-job", id), "x", cx)
                                    .size(px(18.))
                                    .on_click(
                                        cx.listener(move |ws, _, _, cx| ws.cancel_job(id, cx)),
                                    )
                                    .into_any_element()
                            }),
                    )
                    .child(
                        div().h(px(4.)).w_full().rounded_full().bg(t.hover).child(
                            div()
                                .h_full()
                                .rounded_full()
                                .bg(if done { t.success } else { t.accent })
                                .w(relative(if done { 1. } else { frac })),
                        ),
                    )
                    .when(!done && !current.is_empty(), |d| {
                        d.child(
                            div()
                                .truncate()
                                .text_size(px(10.5))
                                .text_color(t.text_faint)
                                .child(current),
                        )
                    })
                    .into_any_element()
            })
            .collect();
        Some(
            div()
                .absolute()
                .bottom(px(36.))
                .right(px(16.))
                .flex()
                .flex_col()
                .items_end()
                .gap(px(8.))
                .children(jobs)
                .when_some(self.toast.as_ref(), |d, toast| {
                    d.child(
                        div()
                            .px(px(12.))
                            .py(px(7.))
                            .rounded(px(8.))
                            .bg(if toast.error { t.danger } else { t.popover_bg })
                            .border_1()
                            .border_color(t.card_border)
                            .shadow_lg()
                            .text_size(px(12.))
                            .text_color(if toast.error { gpui::white() } else { t.text })
                            .child(toast.message.clone()),
                    )
                })
                .into_any_element(),
        )
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let sidebar = self.sidebar_visible.then(|| self.render_sidebar(cx));
        let tab_strip = self.render_tab_strip(cx);
        let toolbar = self.render_toolbar(window, cx);
        let status = self.render_status(cx);
        let hud = self.render_hud(cx);
        let overlay = render_overlay(self, window, cx);
        let menu = render_context_menu(self, cx);
        let content = self.active_tab().clone();

        div()
            .id("workspace")
            .key_context(WORKSPACE)
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .flex()
            .font_family(UI_FONT)
            .text_color(t.text)
            .on_action(cx.listener(|_, _: &Quit, _, cx| {
                Store::flush(cx);
                cx.quit()
            }))
            .on_action(cx.listener(|ws, _: &NewTab, window, cx| ws.add_tab(Location::Start, true, window, cx)))
            .on_action(cx.listener(|ws, _: &CloseTab, window, cx| ws.close_tab(ws.active, window, cx)))
            .on_action(cx.listener(|ws, _: &ReopenClosedTab, window, cx| ws.reopen_closed(window, cx)))
            .on_action(cx.listener(|ws, _: &NextTab, window, cx| ws.activate((ws.active + 1) % ws.tabs.len(), window, cx)))
            .on_action(cx.listener(|ws, _: &PrevTab, window, cx| ws.activate((ws.active + ws.tabs.len() - 1) % ws.tabs.len(), window, cx)))
            .on_action(cx.listener(|ws, a: &ActivateTab, window, cx| {
                let ix = if a.0 == 8 { ws.tabs.len() - 1 } else { a.0 };
                ws.activate(ix, window, cx)
            }))
            .on_action(cx.listener(|ws, _: &FocusPathBar, window, cx| ws.begin_path_edit(window, cx)))
            .on_action(cx.listener(|ws, _: &FocusFilter, window, cx| {
                ws.filter_field.update(cx, |f, cx| {
                    let text = f.text().to_string();
                    f.set_text(text, true, cx)
                });
                window.focus(&ws.filter_field.read(cx).focus_handle_ref().clone());
            }))
            .on_action(cx.listener(|ws, _: &CommandPalette, window, cx| overlays::open_palette(ws, window, cx)))
            .on_action(cx.listener(|ws, _: &FuzzyFind, window, cx| overlays::open_fuzzy(ws, window, cx)))
            .on_action(cx.listener(|ws, _: &ToggleHidden, _, cx| {
                let on = Store::update(cx, |s| {
                    s.show_hidden = !s.show_hidden;
                    s.show_hidden
                });
                ws.show_toast(if on { "Showing hidden files" } else { "Hiding hidden files" }, false, cx);
                for tab in ws.tabs.clone() {
                    tab.update(cx, |t, cx| {
                        t.column_cache.clear();
                        t.reload(cx)
                    });
                }
            }))
            .on_action(cx.listener(|ws, _: &TogglePreview, _, cx| {
                Store::update(cx, |s| s.preview_visible = !s.preview_visible);
                ws.active_tab().clone().update(cx, |t, cx| t.invalidate_preview(cx));
                cx.notify();
            }))
            .on_action(cx.listener(|ws, _: &GetInfo, _, cx| {
                Store::update(cx, |s| s.preview_visible = true);
                ws.active_tab().clone().update(cx, |t, cx| t.invalidate_preview(cx));
            }))
            .on_action(cx.listener(|ws, _: &ToggleSidebar, _, cx| {
                ws.sidebar_visible = !ws.sidebar_visible;
                cx.notify();
            }))
            .on_action(cx.listener(|ws, _: &ViewList, _, cx| ws.active_tab().clone().update(cx, |t, cx| t.set_view(ViewMode::List, cx))))
            .on_action(cx.listener(|ws, _: &ViewGrid, _, cx| ws.active_tab().clone().update(cx, |t, cx| t.set_view(ViewMode::Grid, cx))))
            .on_action(cx.listener(|ws, _: &ViewColumns, _, cx| ws.active_tab().clone().update(cx, |t, cx| t.set_view(ViewMode::Columns, cx))))
            .on_action(cx.listener(|ws, _: &GoBack, _, cx| ws.active_tab().clone().update(cx, |t, cx| t.go_back(cx))))
            .on_action(cx.listener(|ws, _: &GoForward, _, cx| ws.active_tab().clone().update(cx, |t, cx| t.go_forward(cx))))
            .on_action(cx.listener(|ws, _: &GoParent, _, cx| ws.active_tab().clone().update(cx, |t, cx| t.go_parent(cx))))
            .on_action(cx.listener(|ws, _: &GoStart, _, cx| ws.active_tab().clone().update(cx, |t, cx| t.navigate(Location::Start, true, cx))))
            .on_action(cx.listener(|ws, _: &GoHomeFolder, window, cx| ws.navigate_to(&home_dir(), window, cx)))
            .on_action(cx.listener(|ws, a: &NavigateTo, window, cx| ws.navigate_to(&a.0, window, cx)))
            .on_action(cx.listener(|ws, a: &ShowTag, window, cx| {
                let tab = ws.active_tab().clone();
                tab.update(cx, |t, cx| t.navigate(Location::Tag(a.0.clone()), true, cx));
                ws.close_overlay(window, cx);
            }))
            .on_action(cx.listener(|ws, _: &Refresh, _, cx| {
                crate::workspace::refresh_start_data(cx);
                ws.active_tab().clone().update(cx, |t, cx| {
                    t.column_cache.clear();
                    t.reload(cx)
                })
            }))
            .on_action(cx.listener(|ws, _: &UndoOp, _, cx| ws.undo(cx)))
            .on_action(cx.listener(|ws, _: &CopyPath, _, cx| ws.copy_path(cx)))
            .on_action(cx.listener(|ws, _: &NewFolder, window, cx| ws.new_folder(window, cx)))
            .on_action(cx.listener(|ws, _: &PasteFiles, window, cx| ws.paste(window, cx)))
            .on_action(cx.listener(|ws, _: &OpenInTerminal, _, cx| ws.open_in_terminal(cx)))
            .on_action(cx.listener(|ws, _: &OpenInEditor, _, cx| ws.open_in_editor(cx)))
            .on_action(cx.listener(|ws, _: &RenameSelected, window, cx| ws.open_rename(None, window, cx)))
            .on_action(cx.listener(|ws, _: &BulkRename, window, cx| ws.open_bulk_rename(window, cx)))
            .on_action(cx.listener(|ws, _: &CopyFiles, _, cx| ws.copy_files(false, cx)))
            .on_action(cx.listener(|ws, _: &CutFiles, _, cx| ws.copy_files(true, cx)))
            .on_action(cx.listener(|ws, _: &TrashSelected, _, cx| ws.trash_selected(cx)))
            .on_action(cx.listener(|ws, _: &DuplicateSelected, _, cx| ws.duplicate(cx)))
            .on_action(cx.listener(|ws, _: &EditTags, window, cx| overlays::open_tags(ws, window, cx)))
            .on_action(cx.listener(|ws, a: &ToggleTag, _, cx| ws.toggle_tag_on_selection(&a.0, cx)))
            .on_action(cx.listener(|ws, _: &PinToSidebar, _, cx| ws.pin_selection(true, cx)))
            .on_action(cx.listener(|ws, _: &PinToStart, _, cx| ws.pin_selection(false, cx)))
            .on_action(cx.listener(|_, a: &UnpinSidebar, _, cx| {
                Store::update(cx, |s| s.sidebar_pins.retain(|p| p != &a.0));
                cx.notify();
            }))
            .on_action(cx.listener(|ws, _: &CalculateSizes, _, cx| ws.active_tab().clone().update(cx, |t, cx| t.calculate_sizes(cx))))
            .on_action(cx.listener(|ws, a: &SortBy, _, cx| ws.active_tab().clone().update(cx, |t, cx| t.click_sort(a.0, false, cx))))
            .on_action(cx.listener(|ws, a: &ThenSortBy, _, cx| ws.active_tab().clone().update(cx, |t, cx| t.click_sort(a.0, true, cx))))
            .on_action(cx.listener(|ws, _: &ToggleFoldersFirst, _, cx| {
                ws.active_tab().clone().update(cx, |t, cx| {
                    let mut s = t.sort.clone();
                    s.folders_first = !s.folders_first;
                    t.set_sort(s, cx)
                })
            }))
            .on_action(cx.listener(|ws, a: &OpenWith, _, cx| {
                let paths = ws.targets(cx);
                let _ = std::process::Command::new("/usr/bin/open").arg("-a").arg(&a.0).args(&paths).spawn();
                Store::update(cx, |s| paths.iter().for_each(|p| s.record_opened(p)));
            }))
            .on_action(cx.listener(|ws, _: &RevealSelected, _, cx| {
                let tab = ws.active_tab().clone();
                let target = tab.read(cx).selected_paths().into_iter().next();
                if let Some(p) = target
                    && let Some(parent) = p.parent().map(Path::to_path_buf) {
                        tab.update(cx, |t, cx| t.reveal(parent, vec![p], cx));
                    }
            }))
            .on_action(cx.listener(|ws, _: &MakeDefaultViewer, window, cx| {
                ws.overlay = Some(Overlay::Confirm {
                    title: "Make Haven the default file viewer?".into(),
                    body: "Sets NSFileViewer and the folder handler so “Reveal in Finder” and opened folders go to Haven. You can undo this with “Restore Finder as Default File Viewer”.".into(),
                    confirm: "Make Default".into(),
                    action: Box::new(ConfirmDefaultViewer(true)),
                });
                window.focus(&ws.focus);
                cx.notify();
            }))
            .on_action(cx.listener(|ws, _: &RestoreFinder, _, cx| ws.set_default_viewer(false, cx)))
            .on_action(cx.listener(|ws, a: &ConfirmDefaultViewer, _, cx| ws.set_default_viewer(a.0, cx)))
            .on_drag_move(cx.listener(|_, ev: &DragMoveEvent<SidebarSplitter>, _, cx| {
                let w = f32::from(ev.event.position.x - ev.bounds.left()).clamp(160., 360.);
                Store::update(cx, |s| s.sidebar_width = w);
                cx.notify();
            }))
            .children(sidebar)
            .when(self.sidebar_visible, |d| {
                d.child(
                    div()
                        .id("sidebar-splitter")
                        .w(px(4.))
                        .h_full()
                        .flex_none()
                        .cursor_col_resize()
                        .on_drag(SidebarSplitter, |_, _, _, cx| cx.new(|_| Empty)),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .pt(px(6.))
                    .pr(px(6.))
                    .pb(px(6.))
                    .when(!self.sidebar_visible, |d| d.pl(px(6.)))
                    .child(
                        div()
                            .size_full()
                            .flex()
                            .flex_col()
                            .rounded(px(10.))
                            .bg(t.card_bg)
                            .border_1()
                            .border_color(t.card_border)
                            .overflow_hidden()
                            .child(tab_strip)
                            .child(toolbar)
                            .child(div().flex_1().min_h_0().child(content))
                            .child(status),
                    ),
            )
            .children(hud)
            .children(overlay)
            .children(menu)
    }
}
