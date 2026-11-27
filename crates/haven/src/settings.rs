//! The Settings window (Haven ▸ Settings…, ⌘,).
//!
//! Every control writes straight to the persisted [`Store`]; open workspaces
//! observe the store and pick changes up immediately (see `Workspace::new`).

use gpui::{
    App, AppContext, Bounds, Context, Entity, FocusHandle, Focusable, FontWeight, Global,
    InteractiveElement, IntoElement, KeyBinding, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, TitlebarOptions, Window, WindowBounds,
    WindowHandle, WindowKind, WindowOptions, actions, div, point, prelude::FluentBuilder, px, size,
};
use haven_core::state::{AppState, ViewMode};

use crate::actions::{MakeDefaultViewer, RestoreFinder};
use crate::store::Store;
use crate::text_field::{TextField, TextFieldEvent};
use crate::theme::{Theme, UI_FONT, theme};
use crate::ui::{icon, text_button};
use crate::workspace::Workspace;

actions!(settings, [CloseSettings]);

const CONTEXT: &str = "Settings";

pub const GRID_ICON_MIN: f32 = 48.;
pub const GRID_ICON_MAX: f32 = 128.;
pub const GRID_ICON_STEP: f32 = 8.;

/// Steps the grid icon size by `delta` steps, clamped to the allowed range.
pub fn step_grid_icon(current: f32, delta: i32) -> f32 {
    let snapped = (current / GRID_ICON_STEP).round() * GRID_ICON_STEP;
    (snapped + delta as f32 * GRID_ICON_STEP).clamp(GRID_ICON_MIN, GRID_ICON_MAX)
}

/// The open Settings window, so ⌘, focuses it instead of opening another.
#[derive(Default)]
struct SettingsWindow(Option<WindowHandle<Settings>>);
impl Global for SettingsWindow {}

/// The main workspace window, for settings that act through it (default
/// viewer). Updated whenever main.rs (re)opens the window.
pub struct MainWindow(pub WindowHandle<Workspace>);
impl Global for MainWindow {}

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-w", CloseSettings, Some(CONTEXT)),
        KeyBinding::new("escape", CloseSettings, Some(CONTEXT)),
    ]);
}

/// Opens the Settings window, or brings the open one to the front.
pub fn open(cx: &mut App) {
    // Check that the window still exists without updating it. When ⌘, is
    // pressed inside Settings, the action runs during that window's own
    // dispatch, and gpui has the window checked out, so `handle.update` fails
    // even though the window is open. `cx.windows()` still lists it. Activate
    // it once the dispatch has finished.
    if let Some(handle) = cx.try_global::<SettingsWindow>().and_then(|w| w.0)
        && cx
            .windows()
            .iter()
            .any(|w| w.window_id() == handle.window_id())
    {
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, _| window.activate_window());
        });
        return;
    }
    // Same display as the main window when `HAVEN_DISPLAY` picks one.
    let display = crate::target_display(cx);
    let bounds = Bounds::centered(display, size(px(560.), px(560.)), cx);
    let handle = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            display_id: display,
            titlebar: Some(TitlebarOptions {
                title: Some("Settings".into()),
                appears_transparent: true,
                traffic_light_position: Some(point(px(14.), px(14.))),
            }),
            is_resizable: false,
            is_minimizable: false,
            kind: WindowKind::Normal,
            app_id: Some("dev.haven.app".into()),
            ..Default::default()
        },
        |window, cx| {
            let view = cx.new(|cx| Settings::new(window, cx));
            window.focus(&view.read(cx).focus.clone());
            view
        },
    );
    if let Ok(handle) = handle {
        cx.set_global(SettingsWindow(Some(handle)));
    }
}

pub struct Settings {
    /// Last applied font smoothing, to repaint windows when it flips.
    smoothing: bool,
    focus: FocusHandle,
    terminal: Entity<TextField>,
    editor: Entity<TextField>,
    _subs: Vec<Subscription>,
}

impl Focusable for Settings {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Settings {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Settings {
        let state = Store::get(cx).clone();
        let terminal = cx.new(|cx| {
            let mut f = TextField::new("Terminal", cx);
            f.set_text(state.terminal_app.clone(), false, cx);
            f
        });
        let editor = cx.new(|cx| {
            let mut f = TextField::new("code", cx);
            f.set_text(state.editor_command.clone(), false, cx);
            f
        });
        let subs = vec![
            cx.subscribe_in(&terminal, window, |_, field, ev, window, cx| match ev {
                TextFieldEvent::Changed => {
                    let v = field.read(cx).text().trim().to_string();
                    Store::update(cx, |s| s.terminal_app = v);
                }
                // The field takes Esc for itself: close the window as
                // Esc does everywhere else in Settings.
                TextFieldEvent::Cancel => window.remove_window(),
                _ => {}
            }),
            cx.subscribe_in(&editor, window, |_, field, ev, window, cx| match ev {
                TextFieldEvent::Changed => {
                    let v = field.read(cx).text().trim().to_string();
                    Store::update(cx, |s| s.editor_command = v);
                }
                TextFieldEvent::Cancel => window.remove_window(),
                _ => {}
            }),
            cx.observe_global::<Store>(|this, cx| {
                let on = Store::get(cx).font_smoothing;
                if on != this.smoothing {
                    this.smoothing = on;
                    crate::apply_font_smoothing(cx);
                    cx.refresh_windows();
                }
                cx.notify()
            }),
            cx.on_release(|_, cx| cx.set_global(SettingsWindow(None))),
        ];
        Settings {
            smoothing: state.font_smoothing,
            focus: cx.focus_handle(),
            terminal,
            editor,
            _subs: subs,
        }
    }
}

fn row(
    t: &Theme,
    label: &'static str,
    hint: Option<&'static str>,
    control: impl IntoElement,
) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(16.))
        .min_h(px(36.))
        // Rows with a hint line are taller than `min_h`: keep them off their
        // neighbours.
        .py(px(6.))
        .child(
            // The label column gives way (its hint wraps) so the control
            // always fits inside the card.
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(div().text_size(px(13.)).text_color(t.text).child(label))
                .when_some(hint, |d, h| {
                    d.child(div().text_size(px(12.)).text_color(t.text_faint).child(h))
                }),
        )
        .child(div().flex_none().child(control))
}

fn group(t: &Theme, title: &'static str, rows: Vec<gpui::Div>) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(
            div()
                .px(px(2.))
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.text_muted)
                .child(title),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .px(px(14.))
                .py(px(6.))
                .rounded(px(8.))
                .bg(t.elevated)
                .border_1()
                .border_color(t.card_border)
                .children(rows),
        )
}

/// An iOS-style switch bound to one boolean of the persisted state.
fn switch(
    id: &'static str,
    on: bool,
    set: fn(&mut AppState, bool),
    cx: &mut Context<Settings>,
) -> impl IntoElement {
    let t = theme(cx).clone();
    div()
        .id(id)
        .w(px(34.))
        .h(px(20.))
        .flex_none()
        .rounded_full()
        .p(px(2.))
        .flex()
        .when(on, |d| d.justify_end())
        .bg(if on { t.accent } else { t.active })
        .cursor_pointer()
        .on_click(cx.listener(move |_, _, _, cx| {
            Store::update(cx, |s| set(s, !on));
            cx.notify();
        }))
        .child(div().size(px(16.)).rounded_full().bg(gpui::white()))
}

fn view_picker(current: ViewMode, cx: &mut Context<Settings>) -> impl IntoElement {
    let t = theme(cx).clone();
    let seg =
        |mode: ViewMode, label: &'static str, ic: &'static str, cx: &mut Context<Settings>| {
            let active = current == mode;
            div()
                .id(SharedString::from(format!("view-{label}")))
                .flex()
                .items_center()
                .gap(px(5.))
                .h(px(24.))
                .px(px(10.))
                .rounded(px(6.))
                .text_size(px(12.))
                .cursor_pointer()
                .when(active, |d| d.bg(t.active).text_color(t.text))
                .when(!active, |d| {
                    d.text_color(t.text_muted).hover(|s| s.bg(t.hover))
                })
                .on_click(cx.listener(move |_, _, _, cx| {
                    Store::update(cx, |s| s.default_view = mode);
                    cx.notify();
                }))
                .child(icon(ic).size(px(13.)).text_color(if active {
                    t.text
                } else {
                    t.text_muted
                }))
                .child(label)
        };
    div()
        .flex()
        .p(px(2.))
        .gap(px(2.))
        .rounded(px(7.))
        .bg(t.field_bg)
        .border_1()
        .border_color(t.field_border)
        .child(seg(ViewMode::List, "List", "list", cx))
        .child(seg(ViewMode::Grid, "Grid", "layout-grid", cx))
        .child(seg(ViewMode::Columns, "Columns", "columns-3", cx))
}

fn stepper(value: f32, cx: &mut Context<Settings>) -> impl IntoElement {
    let t = theme(cx).clone();
    let btn = |id: &'static str,
               ic: &'static str,
               delta: i32,
               enabled: bool,
               cx: &mut Context<Settings>| {
        div()
            .id(id)
            .size(px(24.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.))
            .when(enabled, |d| {
                d.cursor_pointer()
                    .hover(|s| s.bg(t.hover))
                    .on_click(cx.listener(move |_, _, _, cx| {
                        Store::update(cx, |s| {
                            s.grid_icon_size = step_grid_icon(s.grid_icon_size, delta)
                        });
                        cx.notify();
                    }))
            })
            .child(
                icon(ic)
                    .size(px(13.))
                    .text_color(if enabled { t.text } else { t.text_faint }),
            )
    };
    div()
        .flex()
        .items_center()
        .gap(px(4.))
        .p(px(2.))
        .rounded(px(7.))
        .bg(t.field_bg)
        .border_1()
        .border_color(t.field_border)
        .child(btn("icon-smaller", "minus", -1, value > GRID_ICON_MIN, cx))
        .child(
            div()
                .w(px(44.))
                .flex()
                .justify_center()
                .text_size(px(12.))
                .text_color(t.text)
                .child(format!("{} pt", value.round() as i32)),
        )
        .child(btn("icon-larger", "plus", 1, value < GRID_ICON_MAX, cx))
}

fn field(f: &Entity<TextField>, cx: &App) -> impl IntoElement {
    let t = theme(cx);
    div()
        .w(px(200.))
        .h(px(26.))
        .px(px(8.))
        .flex()
        .items_center()
        .rounded(px(6.))
        .bg(t.field_bg)
        .border_1()
        .border_color(t.field_border)
        .text_size(px(13.))
        .child(f.clone())
}

fn on_main(cx: &mut App, action: Box<dyn gpui::Action>) {
    if let Some(main) = cx.try_global::<MainWindow>().map(|m| m.0) {
        let _ = main.update(cx, |_, window, cx| {
            window.activate_window();
            window.dispatch_action(action, cx);
        });
    }
}

impl Render for Settings {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let s = Store::get(cx).clone();

        let general = group(
            &t,
            "GENERAL",
            vec![
                row(
                    &t,
                    "Default view",
                    Some("Used for folders you haven't set a view for"),
                    view_picker(s.default_view, cx),
                ),
                row(
                    &t,
                    "Show hidden files",
                    Some("Dotfiles and hidden items (⇧⌘.)"),
                    switch("sw-hidden", s.show_hidden, |s, v| s.show_hidden = v, cx),
                ),
                row(
                    &t,
                    "Show preview pane",
                    Some("Quick look at the selection (⌥⌘P)"),
                    switch(
                        "sw-preview",
                        s.preview_visible,
                        |s, v| s.preview_visible = v,
                        cx,
                    ),
                ),
                row(
                    &t,
                    "Fuzzy find respects .gitignore",
                    None,
                    switch(
                        "sw-gitignore",
                        s.respect_gitignore,
                        |s, v| s.respect_gitignore = v,
                        cx,
                    ),
                ),
            ],
        );
        let appearance = group(
            &t,
            "APPEARANCE",
            vec![
                row(
                    &t,
                    "Font smoothing",
                    Some("Heavier, crisper text like native apps (best on non-Retina)"),
                    switch(
                        "sw-smoothing",
                        s.font_smoothing,
                        |s, v| s.font_smoothing = v,
                        cx,
                    ),
                ),
                row(&t, "Grid icon size", None, stepper(s.grid_icon_size, cx)),
            ],
        );
        let tools = group(
            &t,
            "TOOLS",
            vec![
                row(
                    &t,
                    "Terminal app",
                    Some("Open in Terminal (⌥⌘T)"),
                    field(&self.terminal, cx),
                ),
                row(
                    &t,
                    "Editor command",
                    Some("Open in Editor (⌥⌘E)"),
                    field(&self.editor, cx),
                ),
            ],
        );
        let viewer = group(
            &t,
            "DEFAULT FILE VIEWER",
            vec![row(
                &t,
                "Open folders in Haven",
                Some("“Reveal in Finder” and opened folders go to Haven"),
                div()
                    .flex()
                    .gap(px(6.))
                    .child(
                        text_button("make-default", "Make Default…", false, cx)
                            .on_click(|_, _, cx| on_main(cx, Box::new(MakeDefaultViewer))),
                    )
                    .child(
                        text_button("restore-finder", "Use Finder", false, cx)
                            .on_click(|_, _, cx| on_main(cx, Box::new(RestoreFinder))),
                    ),
            )],
        );

        div()
            .id("settings")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|_, _: &CloseSettings, window, _| window.remove_window()))
            .size_full()
            .flex()
            .flex_col()
            .bg(t.card_bg)
            .font_family(UI_FONT)
            .text_color(t.text)
            .child(
                // Title bar (the traffic lights sit on its left).
                div()
                    .h(px(40.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .border_b_1()
                    .border_color(t.divider)
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Settings"),
            )
            .child(
                div()
                    .id("settings-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        // Padding on an inner box, not on the scroll
                        // container itself, so the bottom padding counts
                        // toward the scrollable height and the last card
                        // doesn't sit flush against the window edge.
                        div()
                            .px(px(20.))
                            .py(px(16.))
                            .flex()
                            .flex_col()
                            .gap(px(18.))
                            .child(general)
                            .child(appearance)
                            .child(tools)
                            .child(viewer),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_icon_steps_and_clamps() {
        assert_eq!(step_grid_icon(72., 1), 80.);
        assert_eq!(step_grid_icon(72., -1), 64.);
        assert_eq!(step_grid_icon(GRID_ICON_MAX, 1), GRID_ICON_MAX);
        assert_eq!(step_grid_icon(GRID_ICON_MIN, -1), GRID_ICON_MIN);
        // Odd persisted values snap onto the grid first.
        assert_eq!(step_grid_icon(70., 1), 80.);
        assert_eq!(step_grid_icon(500., 0), GRID_ICON_MAX);
    }

    fn init_settings_app(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            crate::store::init_test_env();
            cx.set_global(crate::theme::Theme::dark());
            Store::init(cx);
            cx.on_action(|_: &crate::actions::OpenSettings, cx| open(cx));
        });
    }

    fn settings_handle(cx: &mut gpui::TestAppContext) -> WindowHandle<Settings> {
        cx.update(|cx| cx.try_global::<SettingsWindow>().and_then(|w| w.0))
            .expect("Settings window is open")
    }

    /// ⌘, pressed while Settings itself is the key window must not open a
    /// second Settings window. The action runs inside that window's own
    /// dispatch, while gpui has the window checked out.
    #[gpui::test]
    async fn open_settings_from_inside_settings_keeps_one_window(cx: &mut gpui::TestAppContext) {
        init_settings_app(cx);
        cx.update(open);
        cx.run_until_parked();
        let first = settings_handle(cx);
        assert_eq!(cx.update(|cx| cx.windows().len()), 1);

        first
            .update(cx, |_, window, cx| {
                window.dispatch_action(Box::new(crate::actions::OpenSettings), cx)
            })
            .unwrap();
        cx.run_until_parked();

        assert_eq!(
            cx.update(|cx| cx.windows().len()),
            1,
            "a second Settings window opened"
        );
        assert_eq!(settings_handle(cx).window_id(), first.window_id());
    }

    /// Once Settings is closed, ⌘, opens a fresh one (and only one).
    #[gpui::test]
    async fn open_settings_after_close_opens_a_new_window(cx: &mut gpui::TestAppContext) {
        init_settings_app(cx);
        cx.update(open);
        cx.run_until_parked();
        let first = settings_handle(cx);
        first
            .update(cx, |_, window, _| window.remove_window())
            .unwrap();
        cx.run_until_parked();
        assert_eq!(cx.update(|cx| cx.windows().len()), 0);

        cx.update(open);
        cx.run_until_parked();
        assert_eq!(cx.update(|cx| cx.windows().len()), 1);
        assert_ne!(settings_handle(cx).window_id(), first.window_id());
    }

    #[gpui::test]
    async fn font_smoothing_switch_applies_live(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            crate::store::init_test_env();
            cx.set_global(crate::theme::Theme::dark());
            Store::init(cx);
            Store::update(cx, |s| s.font_smoothing = true);
            crate::apply_font_smoothing(cx);
        });
        assert!(gpui::font_smoothing());
        let (_view, cx) = cx.add_window_view(Settings::new);
        // What the switch does: write the store; the window applies it.
        cx.update(|_, cx| Store::update(cx, |s| s.font_smoothing = false));
        cx.run_until_parked();
        assert!(!gpui::font_smoothing());
        cx.update(|_, cx| Store::update(cx, |s| s.font_smoothing = true));
        cx.run_until_parked();
        assert!(gpui::font_smoothing());
    }
}
