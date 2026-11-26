//! Haven — a fast, keyboard-first file manager for macOS.

mod actions;
mod assets;
mod icons;
mod overlays;
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

fn open_window(cx: &mut App, args: Vec<PathBuf>) -> WindowHandle<Workspace> {
    let bounds = Bounds::centered(None, size(px(1240.), px(780.)), cx);
    let (tabs, active) = initial_tabs(cx, &args);
    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("Haven".into()),
                appears_transparent: true,
                traffic_light_position: Some(point(px(16.), px(17.))),
            }),
            window_background: WindowBackgroundAppearance::Blurred,
            window_min_size: Some(size(px(900.), px(560.))),
            app_id: Some("dev.haven.app".into()),
            ..Default::default()
        },
        |window, cx| {
            let ws = cx.new(|cx| Workspace::new(tabs, active, window, cx));
            let focus = ws.read(cx).active_tab().read(cx).focus.clone();
            window.focus(&focus);
            ws
        },
    )
    .expect("failed to open window")
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
        icons::IconCache::init(cx);
        cx.set_global(StartData::default());
        actions::bind_keys(cx);
        text_field::bind_keys(cx);
        cx.set_menus(menus());
        cx.on_action(|_: &Quit, cx| {
            Store::flush(cx);
            cx.quit()
        });

        let window = open_window(cx, args.clone());

        // Folders/files opened via Finder, `open -a Haven`, or NSFileViewer.
        let handle = window;
        cx.spawn(async move |cx| {
            while let Ok(urls) = url_rx.recv().await {
                let paths: Vec<PathBuf> = urls.iter().filter_map(|u| url_to_path(u)).collect();
                let _ = handle.update(cx, |ws, window, cx| {
                    for p in paths {
                        if p.is_dir() {
                            ws.add_tab(Location::Dir(p), true, window, cx);
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
            }
        })
        .detach();
        cx.spawn(async move |cx| {
            while reopen_rx.recv().await.is_ok() {
                let _ = handle.update(cx, |_, window, _| window.activate_window());
            }
        })
        .detach();
        cx.activate(true);
    });
}
