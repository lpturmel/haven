//! Haven — a fast, keyboard-first file manager for macOS.

mod actions;
mod assets;
mod icons;
mod native_drag;
mod overlays;
mod settings;
mod store;
mod tab;
mod tab_view;
mod text_field;
mod theme;
mod ui;
mod views;
mod workspace;
mod workspace_view;

use std::path::PathBuf;

use gpui::{
    App, AppContext, Application, Bounds, Menu, MenuItem, TitlebarOptions,
    WindowBackgroundAppearance, WindowBounds, WindowHandle, WindowOptions, point, px, size,
};

use crate::actions::*;
use crate::store::Store;
use crate::tab::Location;
use crate::views::start::StartData;
use crate::workspace::Workspace;

fn initial_tabs(cx: &App, args: &[PathBuf]) -> (Vec<Location>, usize) {
    if !args.is_empty() {
        return (args.iter().map(|p| Location::Dir(p.clone())).collect(), 0);
    }
    let session = &Store::get(cx).session;
    let tabs: Vec<Location> = session
        .tabs
        .iter()
        .map(|t| match t {
            Some(p) if p.is_dir() => Location::Dir(p.clone()),
            _ => Location::Start,
        })
        .collect();
    (tabs, session.active)
}

/// `HAVEN_DISPLAY=<n>` opens the window on the n-th display (0-based, in
/// `cx.displays()` order). Used by validation runs to keep Haven visible on a
/// secondary screen while the main one is busy.
fn target_display(cx: &App) -> Option<gpui::DisplayId> {
    let n: usize = std::env::var("HAVEN_DISPLAY").ok()?.parse().ok()?;
    cx.displays().get(n).map(|d| d.id())
}

/// `HAVEN_WINDOW_SIZE=<w>x<h>` (points) sets the initial window size, e.g.
/// `900x560` to check how the List view fits a narrow window.
fn window_size() -> Option<(f32, f32)> {
    parse_size(&std::env::var("HAVEN_WINDOW_SIZE").ok()?)
}

fn parse_size(s: &str) -> Option<(f32, f32)> {
    let (w, h) = s.trim().split_once(['x', 'X'])?;
    let (w, h): (f32, f32) = (w.trim().parse().ok()?, h.trim().parse().ok()?);
    // At least the window's minimum size.
    (w > 0. && h > 0.).then_some((w.max(900.), h.max(560.)))
}

/// Font smoothing from Settings, unless `HAVEN_FONT_SMOOTHING=0|1` overrides
/// it (for before/after captures).
pub fn apply_font_smoothing(cx: &App) {
    let on = match std::env::var("HAVEN_FONT_SMOOTHING").as_deref() {
        Ok("0") => false,
        Ok("1") => true,
        _ => Store::get(cx).font_smoothing,
    };
    gpui::set_font_smoothing(on);
}

fn open_window(cx: &mut App, args: Vec<PathBuf>) -> WindowHandle<Workspace> {
    let (w, h) = window_size().unwrap_or((1240., 780.));
    let bounds = Bounds::centered(target_display(cx), size(px(w), px(h)), cx);
    let (tabs, active) = initial_tabs(cx, &args);
    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            // gpui reads `bounds` relative to this display (primary if None).
            display_id: target_display(cx),
            titlebar: Some(TitlebarOptions {
                title: Some("Haven".into()),
                appears_transparent: true,
                traffic_light_position: Some(point(px(16.), px(17.))),
            }),
            // Transparent, not `Blurred`: gpui's own blur view does not blur on
            // current macOS (the backdrop shows through sharp). We install the
            // native sidebar material ourselves below.
            window_background: WindowBackgroundAppearance::Transparent,
            window_min_size: Some(size(px(900.), px(560.))),
            app_id: Some("dev.haven.app".into()),
            ..Default::default()
        },
        |window, cx| {
            install_backdrop(window);
            let ws = cx.new(|cx| Workspace::new(tabs, active, window, cx));
            let focus = ws.read(cx).active_tab().read(cx).focus.clone();
            window.focus(&focus);
            ws
        },
    )
    .expect("failed to open window")
}

/// Puts Finder's frosted sidebar material behind the window content.
fn install_backdrop(window: &gpui::Window) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    if let RawWindowHandle::AppKit(h) = handle.as_raw() {
        // SAFETY: gpui hands out the window's live content NSView.
        let ok = unsafe { haven_macos::install_sidebar_material(h.ns_view.as_ptr()) };
        if !ok {
            eprintln!("haven: could not install the sidebar material");
        }
    }
}

/// Converts `file://` URLs (from Finder / `open`) to paths.
fn url_to_path(url: &str) -> Option<PathBuf> {
    let p = haven_core::pathres::expand(url, std::path::Path::new("/")).ok()?;
    p.exists().then_some(p)
}

fn menus() -> Vec<Menu> {
    vec![
        Menu {
            name: "Haven".into(),
            items: vec![
                MenuItem::action("Settings…", OpenSettings),
                MenuItem::separator(),
                MenuItem::action("Make Haven the Default File Viewer…", MakeDefaultViewer),
                MenuItem::action("Restore Finder as Default", RestoreFinder),
                MenuItem::separator(),
                MenuItem::action("Quit Haven", Quit),
            ],
        },
        Menu {
            name: "File".into(),
            items: vec![
                MenuItem::action("New Tab", NewTab),
                MenuItem::action("New Folder", NewFolder),
                MenuItem::action("Close Tab", CloseTab),
                MenuItem::action("Reopen Closed Tab", ReopenClosedTab),
                MenuItem::separator(),
                MenuItem::action("Open", OpenSelected),
                MenuItem::action("Open in New Tab", OpenInNewTab),
                MenuItem::action("Rename…", RenameSelected),
                MenuItem::action("Bulk Rename…", BulkRename),
                MenuItem::action("Duplicate", DuplicateSelected),
                MenuItem::action("Edit Tags…", EditTags),
                MenuItem::action("Move to Trash", TrashSelected),
                MenuItem::separator(),
                MenuItem::action("Eject", EjectSelected),
            ],
        },
        Menu {
            name: "Edit".into(),
            items: vec![
                MenuItem::action("Undo", UndoOp),
                MenuItem::separator(),
                MenuItem::action("Copy", CopyFiles),
                MenuItem::action("Cut", CutFiles),
                MenuItem::action("Paste", PasteFiles),
                MenuItem::action("Copy Path", CopyPath),
                MenuItem::action("Select All", SelectAllItems),
            ],
        },
        Menu {
            name: "View".into(),
            items: vec![
                MenuItem::action("as List", ViewList),
                MenuItem::action("as Grid", ViewGrid),
                MenuItem::action("as Columns", ViewColumns),
                MenuItem::separator(),
                MenuItem::action("Toggle Preview", TogglePreview),
                MenuItem::action("Toggle Sidebar", ToggleSidebar),
                MenuItem::action("Toggle Hidden Files", ToggleHidden),
                MenuItem::action("Calculate Folder Sizes", CalculateSizes),
                MenuItem::separator(),
                MenuItem::action("Command Palette", CommandPalette),
                MenuItem::action("Fuzzy Find", FuzzyFind),
            ],
        },
        Menu {
            name: "Go".into(),
            items: vec![
                MenuItem::action("Back", GoBack),
                MenuItem::action("Forward", GoForward),
                MenuItem::action("Enclosing Folder", GoParent),
                MenuItem::action("Start Page", GoStart),
                MenuItem::action("Home", GoHomeFolder),
                MenuItem::action("Go to Path…", FocusPathBar),
            ],
        },
    ]
}

fn main() {
    let args: Vec<PathBuf> = std::env::args()
        .skip(1)
        .filter(|a| !a.starts_with("-psn"))
        .filter_map(|a| {
            haven_core::pathres::expand(&a, &std::env::current_dir().unwrap_or_default()).ok()
        })
        .filter(|p| p.is_dir())
        .collect();

    let (url_tx, url_rx) = smol::channel::unbounded::<Vec<String>>();
    let (reopen_tx, reopen_rx) = smol::channel::unbounded::<()>();
    let app = Application::new().with_assets(assets::Assets);
    app.on_open_urls(move |urls| {
        let _ = url_tx.try_send(urls);
    });
    app.on_reopen(move |_| {
        let _ = reopen_tx.try_send(());
    });
    app.run(move |cx: &mut App| {
        haven_macos::force_dark_appearance();
        cx.set_global(theme::Theme::dark());
        Store::init(cx);
        apply_font_smoothing(cx);
        icons::IconCache::init(cx);
        cx.set_global(StartData::default());
        actions::bind_keys(cx);
        text_field::bind_keys(cx);
        settings::bind_keys(cx);
        cx.set_menus(menus());
        cx.on_action(|_: &OpenSettings, cx| settings::open(cx));
        cx.on_action(|_: &Quit, cx| {
            workspace::wait_for_tag_writes();
            Store::flush(cx);
            cx.quit()
        });

        // The window may be closed while the app keeps running (red button):
        // reopen events and opened URLs then create a fresh window.
        let handle = std::rc::Rc::new(std::cell::Cell::new(open_window(cx, args.clone())));
        cx.set_global(settings::MainWindow(handle.get()));
        // `HAVEN_OPEN_SETTINGS=1` opens Settings at launch, so validation runs
        // can capture it without sending ⌘, to a background window.
        if std::env::var_os("HAVEN_OPEN_SETTINGS").is_some_and(|v| v == "1") {
            settings::open(cx);
        }

        // Folders/files opened via Finder, `open -a Haven`, or NSFileViewer.
        let urls_handle = handle.clone();
        cx.spawn(async move |cx| {
            while let Ok(urls) = url_rx.recv().await {
                let paths: Vec<PathBuf> = urls.iter().filter_map(|u| url_to_path(u)).collect();
                let shown = urls_handle.get().update(cx, |ws, window, cx| {
                    for p in &paths {
                        if p.is_dir() {
                            ws.add_tab(Location::Dir(p.clone()), true, window, cx);
                        } else if let Some(parent) = p.parent() {
                            ws.add_tab(Location::Dir(parent.to_path_buf()), true, window, cx);
                            let tab = ws.active_tab().clone();
                            tab.update(cx, |t, cx| {
                                t.reveal(parent.to_path_buf(), vec![p.clone()], cx)
                            });
                        }
                    }
                    window.activate_window();
                });
                if shown.is_err() {
                    let dirs: Vec<PathBuf> = paths
                        .iter()
                        .filter_map(|p| {
                            if p.is_dir() {
                                Some(p.clone())
                            } else {
                                p.parent().map(|d| d.to_path_buf())
                            }
                        })
                        .collect();
                    if let Ok(w) = cx.update(|cx| {
                        let w = open_window(cx, dirs);
                        cx.set_global(settings::MainWindow(w));
                        w
                    }) {
                        urls_handle.set(w);
                    }
                }
            }
        })
        .detach();
        let reopen_handle = handle.clone();
        cx.spawn(async move |cx| {
            while reopen_rx.recv().await.is_ok() {
                let shown = reopen_handle
                    .get()
                    .update(cx, |_, window, _| window.activate_window());
                if shown.is_err()
                    && let Ok(w) = cx.update(|cx| {
                        let w = open_window(cx, Vec::new());
                        cx.set_global(settings::MainWindow(w));
                        w
                    })
                {
                    reopen_handle.set(w);
                }
            }
        })
        .detach();
        cx.activate(true);
    });
}

#[cfg(test)]
mod tests {
    use super::parse_size;

    #[test]
    fn parses_window_size_env() {
        assert_eq!(parse_size("1000x700"), Some((1000., 700.)));
        assert_eq!(parse_size(" 1200 X 800 "), Some((1200., 800.)));
        // Clamped to the window's minimum size.
        assert_eq!(parse_size("300x200"), Some((900., 560.)));
        assert_eq!(parse_size("wide"), None);
        assert_eq!(parse_size("0x0"), None);
    }
}
