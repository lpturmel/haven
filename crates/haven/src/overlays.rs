//! Modal overlays, context menus and their keyboard handling.

use std::path::PathBuf;
use std::time::Duration;

use gpui::{
    Action, AnyElement, App, AppContext, Context, Entity, Focusable, FontWeight, Hsla,
    InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels, Point, ScrollHandle,
    ScrollStrategy, SharedString, StatefulInteractiveElement, Styled, Task,
    UniformListScrollHandle, Window, anchored, deferred, div, prelude::FluentBuilder, px,
    uniform_list,
};
use haven_core::fuzzy::{FuzzyFinder, FuzzyMatch};
use haven_core::ops::{BulkRename, CaseChange, Conflict, OpKind};
use haven_core::{Entry, Tag, TagColor, tildify};
use nucleo::pattern::{CaseMatching, Normalization, Pattern};

use crate::actions::*;
use crate::store::Store;
use crate::text_field::{TextField, TextFieldEvent};
use crate::theme::{Theme, theme};
use crate::ui::Ellipsis as _;
use crate::ui::{icon, kbd, tag_dot, text_button};
use crate::views::start::start_data;
use crate::workspace::Workspace;

// ------------------------------------------------------------------ context menu

pub enum MenuEntry {
    Item {
        label: String,
        icon: &'static str,
        shortcut: Option<String>,
        action: Box<dyn Action>,
        danger: bool,
        checked: bool,
    },
    Tags(Vec<(String, TagColor, bool)>),
    Header(String),
    Separator,
}

impl MenuEntry {
    pub fn item(
        label: impl Into<String>,
        icon: &'static str,
        action: impl Action,
        cx: &App,
    ) -> MenuEntry {
        let shortcut = shortcut_label(&action, cx);
        MenuEntry::Item {
            label: label.into(),
            icon,
            shortcut,
            action: Box::new(action),
            danger: false,
            checked: false,
        }
    }
    pub fn danger(mut self) -> MenuEntry {
        if let MenuEntry::Item { danger, .. } = &mut self {
            *danger = true;
        }
        self
    }
    pub fn checked(mut self, on: bool) -> MenuEntry {
        if let MenuEntry::Item { checked, .. } = &mut self {
            *checked = on;
        }
        self
    }
}

pub struct ContextMenu {
    pub position: Point<Pixels>,
    pub items: Vec<MenuEntry>,
}

/// Palette rows rendered (and reachable with ↓).
const PALETTE_ROWS: usize = 60;

pub fn build_context_menu(
    selected: &[Entry],
    on_item: bool,
    in_dir: bool,
    has_clip: bool,
    cx: &App,
) -> Vec<MenuEntry> {
    use MenuEntry as M;
    let state = Store::get(cx);
    let mut items = Vec::new();
    if on_item && !selected.is_empty() {
        let single = selected.len() == 1;
        let any_dir = selected.iter().any(|e| e.is_navigable());
        items.push(M::item("Open", "external-link", OpenSelected, cx));
        if any_dir {
            items.push(M::item("Open in New Tab", "plus", OpenInNewTab, cx));
        }
        if single && !selected[0].is_navigable() {
            for app in haven_macos::apps_for(&selected[0].path).into_iter().take(4) {
                let name = app
                    .file_stem()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                items.push(M::item(
                    format!("Open with {name}"),
                    "app-window",
                    OpenWith(app),
                    cx,
                ));
            }
        }
        items.push(M::item(
            "Reveal Containing Folder",
            "folder-open",
            RevealSelected,
            cx,
        ));
        items.push(M::Separator);
        if single {
            items.push(M::item("Rename…", "pencil", RenameSelected, cx));
        } else {
            items.push(M::item(
                format!("Rename {} Items…", selected.len()),
                "pencil",
                BulkRename,
                cx,
            ));
        }
        items.push(M::item("Duplicate", "copy", DuplicateSelected, cx));
        items.push(M::item("Copy", "copy", CopyFiles, cx));
        items.push(M::item("Cut", "scissors", CutFiles, cx));
        items.push(M::item("Copy Path", "link", CopyPath, cx));
        items.push(M::Separator);
        let tags: Vec<(String, TagColor, bool)> = state
            .tags
            .iter()
            .take(9)
            .map(|t| {
                let on = selected
                    .iter()
                    .all(|e| e.tags.iter().any(|x| x.name.eq_ignore_ascii_case(&t.name)));
                (t.name.clone(), t.color, on)
            })
            .collect();
        items.push(M::Tags(tags));
        items.push(M::item("Tags…", "tag", EditTags, cx));
        items.push(M::Separator);
        if any_dir {
            let pinned = selected
                .iter()
                .all(|e| state.sidebar_pins.contains(&e.path));
            items.push(M::item(
                if pinned {
                    "Unpin from Sidebar"
                } else {
                    "Pin to Sidebar"
                },
                "pin",
                PinToSidebar,
                cx,
            ));
            items.push(M::item("Calculate Size", "hash", CalculateSizes, cx));
        }
        let start_pinned = selected.iter().all(|e| state.start_pins.contains(&e.path));
        items.push(M::item(
            if start_pinned {
                "Unpin from Start"
            } else {
                "Pin to Start"
            },
            "star",
            PinToStart,
            cx,
        ));
        items.push(M::item(
            "Open in Terminal",
            "square-terminal",
            OpenInTerminal,
            cx,
        ));
        items.push(M::item("Open in Editor", "code", OpenInEditor, cx));
        items.push(M::Separator);
        items.push(M::item("Move to Trash", "trash-2", TrashSelected, cx).danger());
    } else if in_dir {
        items.push(M::item("New Folder", "folder-plus", NewFolder, cx));
        if has_clip {
            items.push(M::item("Paste", "clipboard-paste", PasteFiles, cx));
        }
        items.push(M::Separator);
        items.push(M::item(
            "Open in Terminal",
            "square-terminal",
            OpenInTerminal,
            cx,
        ));
        items.push(M::item("Open in Editor", "code", OpenInEditor, cx));
        items.push(M::item("Copy Path", "link", CopyPath, cx));
        items.push(M::Separator);
        items.push(M::item("Pin Folder to Sidebar", "pin", PinToSidebar, cx));
        items.push(M::item("Pin Folder to Start", "star", PinToStart, cx));
        items.push(M::item(
            "Calculate Folder Sizes",
            "hash",
            CalculateSizes,
            cx,
        ));
        items.push(M::item(
            if state.show_hidden {
                "Hide Hidden Files"
            } else {
                "Show Hidden Files"
            },
            "eye",
            ToggleHidden,
            cx,
        ));
        items.push(M::Separator);
        items.push(M::item("View as List", "list", ViewList, cx));
        items.push(M::item("View as Grid", "layout-grid", ViewGrid, cx));
        items.push(M::item("View as Columns", "columns-3", ViewColumns, cx));
    }
    items
}

pub fn sort_menu(ws: &Workspace, cx: &App) -> Vec<MenuEntry> {
    use haven_core::SortKey;
    let tab = ws.active_tab().read(cx);
    let mut items = vec![MenuEntry::Header("Sort by".into())];
    for key in SortKey::ALL {
        let rank = tab.sort.rank(key);
        let label = match rank {
            Some((_, desc)) => format!("{}  {}", key.label(), if desc { "↓" } else { "↑" }),
            None => key.label().to_string(),
        };
        items.push(
            MenuEntry::item(label, "arrow-up-down", SortBy(key), cx)
                .checked(rank.is_some_and(|(p, _)| p == 1)),
        );
    }
    items.push(MenuEntry::Separator);
    items.push(MenuEntry::Header("Then by (⇧-click a column)".into()));
    for key in SortKey::ALL {
        if tab.sort.primary().0 == key {
            continue;
        }
        let rank = tab.sort.rank(key).filter(|(p, _)| *p > 1);
        let label = match rank {
            Some((p, desc)) => format!("{} · {}  {}", p, key.label(), if desc { "↓" } else { "↑" }),
            None => key.label().to_string(),
        };
        items.push(
            MenuEntry::item(label, "arrow-down-wide-narrow", ThenSortBy(key), cx)
                .checked(rank.is_some()),
        );
    }
    items.push(MenuEntry::Separator);
    items.push(
        MenuEntry::item("Folders First", "folder", ToggleFoldersFirst, cx)
            .checked(tab.sort.folders_first),
    );
    items.push(MenuEntry::item(
        "Calculate Folder Sizes",
        "hash",
        CalculateSizes,
        cx,
    ));
    items
}

/// The ring around a context-menu tag circle, and its colour under the
/// pointer. A tag that's on keeps its ring: hover never replaces an "on"
/// state.
fn tag_ring(on: bool, t: &Theme) -> (Hsla, Option<Hsla>) {
    if on {
        (t.text, None)
    } else {
        (gpui::transparent_black(), Some(t.text_muted))
    }
}

pub fn render_context_menu(ws: &Workspace, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let menu = ws.context_menu.as_ref()?;
    let t = theme(cx).clone();
    let mut rows: Vec<AnyElement> = Vec::new();
    for (i, item) in menu.items.iter().enumerate() {
        match item {
            MenuEntry::Separator => rows.push(
                div()
                    .h(px(1.))
                    .my(px(4.))
                    .mx(px(8.))
                    .bg(t.divider)
                    .into_any_element(),
            ),
            MenuEntry::Header(label) => rows.push(
                div()
                    .px(px(12.))
                    .pt(px(4.))
                    .pb(px(2.))
                    .text_size(px(11.))
                    .text_color(t.text_faint)
                    .child(label.clone())
                    .into_any_element(),
            ),
            MenuEntry::Tags(tags) => rows.push(
                div()
                    .flex()
                    .gap(px(6.))
                    .px(px(12.))
                    .py(px(6.))
                    .children(tags.iter().enumerate().map(|(j, (name, color, on))| {
                        let name = name.clone();
                        let c = t.tag(*color);
                        let (ring, hover_ring) = tag_ring(*on, &t);
                        div()
                            .id(("menu-tag", j))
                            .size(px(18.))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(if *color == TagColor::None { t.hover } else { c })
                            .border_1()
                            .border_color(ring)
                            .when(*on, |d| {
                                d.child(icon("check").size(px(11.)).text_color(gpui::white()))
                            })
                            .when_some(hover_ring, |d, h| d.hover(move |s| s.border_color(h)))
                            .on_click(cx.listener(move |ws, _, window, cx| {
                                ws.dispatch_menu_action(
                                    Box::new(ToggleTag(name.clone())),
                                    window,
                                    cx,
                                )
                            }))
                    }))
                    .into_any_element(),
            ),
            MenuEntry::Item {
                label,
                icon: icon_name,
                shortcut,
                action,
                danger,
                checked,
            } => {
                let action = action.boxed_clone();
                let color = if *danger { t.danger } else { t.text };
                // Deliberately not the overlay-row recipe (10px inset,
                // `hover` fill): this matches a native macOS menu, with a
                // 4 + 8 inset that the header, tag row and separator share,
                // and the accent highlight with white text.
                rows.push(
                    div()
                        .id(("menu-item", i))
                        .h(px(26.))
                        .mx(px(4.))
                        .px(px(8.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .rounded(px(6.))
                        .text_size(px(13.))
                        .text_color(color)
                        .hover(|s| s.bg(t.accent).text_color(gpui::white()))
                        .on_click(cx.listener(move |ws, _, window, cx| {
                            ws.dispatch_menu_action(action.boxed_clone(), window, cx)
                        }))
                        .child(
                            div().w(px(14.)).flex_none().child(
                                if *checked {
                                    icon("check")
                                } else {
                                    icon(icon_name)
                                }
                                .size(px(14.))
                                .text_color(if *checked {
                                    t.accent_text
                                } else {
                                    t.text_muted
                                }),
                            ),
                        )
                        .child(div().flex_1().ellipsis().child(label.clone()))
                        .children(
                            shortcut.clone().map(|s| {
                                div().text_size(px(11.)).text_color(t.text_faint).child(s)
                            }),
                        )
                        .into_any_element(),
                );
            }
        }
    }
    let position = menu.position;
    Some(
        deferred(
            div()
                .id("menu-backdrop")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .occlude()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|ws, _, window, cx| ws.close_overlay(window, cx)),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|ws, _, window, cx| ws.close_overlay(window, cx)),
                )
                .child(
                    anchored()
                        .position(position)
                        .snap_to_window_with_margin(px(8.))
                        .child(
                            div()
                                .id("menu")
                                .occlude()
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .w(px(250.))
                                .py(px(4.))
                                .rounded(px(8.))
                                .bg(t.popover_bg)
                                .border_1()
                                .border_color(t.card_border)
                                .shadow_lg()
                                .children(rows),
                        ),
                ),
        )
        .with_priority(2)
        .into_any_element(),
    )
}

// ------------------------------------------------------------------ overlays

pub struct PaletteItem {
    pub label: String,
    pub detail: Option<String>,
    pub icon: &'static str,
    pub shortcut: Option<String>,
    pub action: Box<dyn Action>,
}

pub struct PaletteState {
    pub field: Entity<TextField>,
    pub items: Vec<PaletteItem>,
    pub matches: Vec<usize>,
    pub selected: usize,
    /// Keeps the selected row visible while moving with ↑↓.
    pub scroll: ScrollHandle,
}

pub struct FuzzyState {
    pub field: Entity<TextField>,
    pub finder: FuzzyFinder,
    pub results: Vec<FuzzyMatch>,
    pub selected: usize,
    /// The results are virtualized; this keeps the selection in view.
    pub scroll: UniformListScrollHandle,
    /// Results need refreshing (new query) even if nucleo reports no change.
    pub stale: bool,
    pub _ticker: Task<()>,
}

pub struct TagsState {
    pub field: Entity<TextField>,
    pub color: TagColor,
}

pub struct BulkRenameState {
    pub fields: [Entity<TextField>; 4],
    pub regex: bool,
    pub numbering: bool,
    pub case: CaseChange,
    pub paths: Vec<PathBuf>,
    pub preview: Vec<String>,
    pub error: Option<String>,
}

impl BulkRenameState {
    pub fn new(
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> BulkRenameState {
        let fields = ["Find", "Replace with", "Prefix", "Suffix"]
            .map(|p| cx.new(|cx| TextField::new(p, cx)));
        window.focus(&fields[0].focus_handle(cx));
        let mut s = BulkRenameState {
            fields,
            regex: false,
            numbering: false,
            case: CaseChange::Keep,
            paths,
            preview: Vec::new(),
            error: None,
        };
        s.recompute(cx);
        s
    }

    pub fn spec(&self, cx: &App) -> BulkRename {
        BulkRename {
            find: self.fields[0].read(cx).text().to_string(),
            replace: self.fields[1].read(cx).text().to_string(),
            regex: self.regex,
            prefix: self.fields[2].read(cx).text().to_string(),
            suffix: self.fields[3].read(cx).text().to_string(),
            number_from: self.numbering.then_some(1),
            case: self.case,
        }
    }

    pub fn names(&self) -> Vec<String> {
        self.paths
            .iter()
            .map(|p| {
                p.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            })
            .collect()
    }

    pub fn recompute(&mut self, cx: &App) {
        match self.spec(cx).preview(&self.names()) {
            Ok(p) => {
                self.preview = p;
                self.error = None;
            }
            Err(e) => self.error = Some(e),
        }
    }
}

pub enum Overlay {
    Palette(PaletteState),
    Fuzzy(FuzzyState),
    Rename {
        field: Entity<TextField>,
        path: PathBuf,
        error: Option<String>,
    },
    BulkRename(BulkRenameState),
    Tags(TagsState),
    Conflict {
        kind: OpKind,
        sources: Vec<PathBuf>,
        dest: PathBuf,
        count: usize,
    },
    Confirm {
        title: String,
        body: String,
        confirm: String,
        action: Box<dyn Action>,
    },
}

fn commands(cx: &App) -> Vec<PaletteItem> {
    let mut v: Vec<(&str, &'static str, Box<dyn Action>)> = vec![
        ("Fuzzy Find Files", "search", Box::new(FuzzyFind)),
        ("Go to Path…", "text-cursor-input", Box::new(FocusPathBar)),
        ("Filter Current Folder", "search", Box::new(FocusFilter)),
        ("New Tab", "plus", Box::new(NewTab)),
        ("Close Tab", "x", Box::new(CloseTab)),
        ("Reopen Closed Tab", "history", Box::new(ReopenClosedTab)),
        ("Go Back", "arrow-left", Box::new(GoBack)),
        ("Go Forward", "arrow-right", Box::new(GoForward)),
        ("Enclosing Folder", "arrow-up", Box::new(GoParent)),
        ("Start Page", "house", Box::new(GoStart)),
        ("Home Folder", "user", Box::new(GoHomeFolder)),
        ("View as List", "list", Box::new(ViewList)),
        ("View as Grid", "layout-grid", Box::new(ViewGrid)),
        ("View as Columns", "columns-3", Box::new(ViewColumns)),
        (
            "Toggle Preview Pane",
            "panel-right",
            Box::new(TogglePreview),
        ),
        ("Toggle Sidebar", "panel-right", Box::new(ToggleSidebar)),
        ("Toggle Hidden Files", "eye", Box::new(ToggleHidden)),
        (
            "Toggle Folders First",
            "folder",
            Box::new(ToggleFoldersFirst),
        ),
        ("Calculate Folder Sizes", "hash", Box::new(CalculateSizes)),
        ("New Folder", "folder-plus", Box::new(NewFolder)),
        ("Rename…", "pencil", Box::new(RenameSelected)),
        ("Bulk Rename…", "pencil", Box::new(BulkRename)),
        ("Duplicate", "copy", Box::new(DuplicateSelected)),
        ("Copy Files", "copy", Box::new(CopyFiles)),
        ("Cut Files", "scissors", Box::new(CutFiles)),
        ("Paste Files", "clipboard-paste", Box::new(PasteFiles)),
        ("Copy Path", "link", Box::new(CopyPath)),
        ("Move to Trash", "trash-2", Box::new(TrashSelected)),
        ("Undo", "undo-2", Box::new(UndoOp)),
        ("Edit Tags…", "tag", Box::new(EditTags)),
        ("Pin to Sidebar", "pin", Box::new(PinToSidebar)),
        ("Pin to Start", "star", Box::new(PinToStart)),
        (
            "Open in Terminal",
            "square-terminal",
            Box::new(OpenInTerminal),
        ),
        ("Open in Editor", "code", Box::new(OpenInEditor)),
        ("Refresh", "refresh-cw", Box::new(Refresh)),
        (
            "Make Haven the Default File Viewer",
            "settings",
            Box::new(MakeDefaultViewer),
        ),
        (
            "Restore Finder as Default File Viewer",
            "settings",
            Box::new(RestoreFinder),
        ),
    ];
    for key in haven_core::SortKey::ALL {
        let label: &'static str = Box::leak(format!("Sort by {}", key.label()).into_boxed_str());
        v.push((label, "arrow-up-down", Box::new(SortBy(key))));
    }
    let mut items: Vec<PaletteItem> = v
        .into_iter()
        .map(|(label, icon, action)| PaletteItem {
            shortcut: shortcut_label(action.as_ref(), cx),
            label: label.to_string(),
            detail: None,
            icon,
            action,
        })
        .collect();
    let state = Store::get(cx);
    let mut places: Vec<(PathBuf, &'static str)> = Vec::new();
    places.extend(state.sidebar_pins.iter().map(|p| (p.clone(), "pin")));
    places.extend(state.start_pins.iter().map(|p| (p.clone(), "star")));
    places.extend(
        start_data(cx)
            .volumes
            .iter()
            .map(|v| (v.mount.clone(), "hard-drive")),
    );
    places.extend(
        start_data(cx)
            .recents
            .iter()
            .take(20)
            .map(|r| (r.path.clone(), "clock")),
    );
    for (path, icon) in places {
        items.push(PaletteItem {
            label: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string()),
            detail: Some(tildify(&path)),
            icon,
            shortcut: None,
            action: Box::new(NavigateTo(path)),
        });
    }
    for tag in &state.tags {
        items.push(PaletteItem {
            label: format!("Tag: {}", tag.name),
            detail: None,
            icon: "tag",
            shortcut: None,
            action: Box::new(ShowTag(tag.name.clone())),
        });
    }
    items
}

/// The conflict and confirm dialogs are modal: Enter takes their primary
/// button, and nothing else (palette, fuzzy find) may replace them, or a
/// pending transfer would silently never run.
pub fn modal_open(ws: &Workspace) -> bool {
    matches!(
        ws.overlay,
        Some(Overlay::Conflict { .. }) | Some(Overlay::Confirm { .. })
    )
}

/// Runs the primary button of an open modal. Returns false if none is open.
pub fn confirm_modal(ws: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) -> bool {
    match &ws.overlay {
        Some(Overlay::Conflict {
            kind,
            sources,
            dest,
            ..
        }) => {
            let (kind, sources, dest) = (*kind, sources.clone(), dest.clone());
            ws.close_overlay(window, cx);
            ws.run_transfer(kind, sources, dest, Conflict::KeepBoth, cx);
            true
        }
        Some(Overlay::Confirm { action, .. }) => {
            let action = action.boxed_clone();
            ws.overlay = None;
            ws.dispatch_menu_action(action, window, cx);
            true
        }
        _ => false,
    }
}

pub fn open_palette(ws: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    if modal_open(ws) {
        return;
    }
    let field = cx.new(|cx| TextField::new("Type a command, place or tag…", cx));
    ws.subscribe_overlay_field(&field, window, cx);
    window.focus(&field.focus_handle(cx));
    let items = commands(cx);
    let matches = (0..items.len()).collect();
    ws.overlay = Some(Overlay::Palette(PaletteState {
        field,
        items,
        matches,
        selected: 0,
        scroll: ScrollHandle::new(),
    }));
    cx.notify();
}

pub fn open_fuzzy(ws: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    if modal_open(ws) {
        return;
    }
    let root = ws
        .active_tab()
        .read(cx)
        .dir()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(haven_core::home_dir);
    let field = cx.new(|cx| TextField::new(format!("Find in {}", tildify(&root)), cx));
    ws.subscribe_overlay_field(&field, window, cx);
    window.focus(&field.focus_handle(cx));
    let state = Store::get(cx);
    let finder = FuzzyFinder::new(
        &root,
        state.respect_gitignore,
        state.show_hidden,
        std::sync::Arc::new(|| {}),
    );
    let ticker = cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor()
                .timer(Duration::from_millis(33))
                .await;
            let alive = this
                .update(cx, |ws, cx| {
                    let Some(Overlay::Fuzzy(f)) = &mut ws.overlay else {
                        return false;
                    };
                    // (Not `|| results.is_empty()`: a query with no match
                    // would then re-render the window 30 times a second.)
                    if f.finder.tick() || f.stale {
                        f.stale = false;
                        f.results = f.finder.results(200);
                        if f.selected >= f.results.len() {
                            f.selected = 0;
                        }
                        cx.notify();
                    }
                    true
                })
                .unwrap_or(false);
            if !alive {
                break;
            }
        }
    });
    ws.overlay = Some(Overlay::Fuzzy(FuzzyState {
        field,
        finder,
        results: Vec::new(),
        selected: 0,
        scroll: UniformListScrollHandle::new(),
        stale: true,
        _ticker: ticker,
    }));
    cx.notify();
}

pub fn open_tags(ws: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    if ws.targets(cx).is_empty() {
        return;
    }
    let field = cx.new(|cx| TextField::new("New tag name…", cx));
    ws.subscribe_overlay_field(&field, window, cx);
    window.focus(&field.focus_handle(cx));
    ws.overlay = Some(Overlay::Tags(TagsState {
        field,
        color: TagColor::Blue,
    }));
    cx.notify();
}

struct Hay(usize, String);

impl AsRef<str> for Hay {
    fn as_ref(&self) -> &str {
        &self.1
    }
}

fn refilter_palette(p: &mut PaletteState, cx: &App) {
    let query = p.field.read(cx).text().to_string();
    if query.trim().is_empty() {
        p.matches = (0..p.items.len()).collect();
    } else {
        let pattern = Pattern::parse(&query, CaseMatching::Smart, Normalization::Smart);
        let mut matcher = nucleo::Matcher::new(nucleo::Config::DEFAULT);
        let haystacks: Vec<Hay> = p
            .items
            .iter()
            .enumerate()
            .map(|(i, it)| {
                Hay(
                    i,
                    format!("{} {}", it.label, it.detail.clone().unwrap_or_default()),
                )
            })
            .collect();
        let mut scored = pattern.match_list(haystacks, &mut matcher);
        scored.sort_by_key(|s| std::cmp::Reverse(s.1));
        p.matches = scored.into_iter().map(|(h, _)| h.0).collect();
    }
    p.selected = 0;
}

pub fn handle_overlay_event(
    ws: &mut Workspace,
    ev: &TextFieldEvent,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    if matches!(ev, TextFieldEvent::Cancel) {
        ws.close_overlay(window, cx);
        return;
    }
    let Some(overlay) = ws.overlay.as_mut() else {
        return;
    };
    match overlay {
        Overlay::Palette(p) => match ev {
            TextFieldEvent::Changed => refilter_palette(p, cx),
            TextFieldEvent::Down => {
                // Only the first PALETTE_ROWS matches are rendered.
                let shown = p.matches.len().min(PALETTE_ROWS);
                p.selected = (p.selected + 1).min(shown.saturating_sub(1));
                p.scroll.scroll_to_item(p.selected);
            }
            TextFieldEvent::Up => {
                p.selected = p.selected.saturating_sub(1);
                p.scroll.scroll_to_item(p.selected);
            }
            TextFieldEvent::Submit => {
                if let Some(&ix) = p.matches.get(p.selected) {
                    let action = p.items[ix].action.boxed_clone();
                    ws.overlay = None;
                    ws.dispatch_menu_action(action, window, cx);
                }
            }
            _ => {}
        },
        Overlay::Fuzzy(f) => match ev {
            TextFieldEvent::Changed => {
                let q = f.field.read(cx).text().to_string();
                f.finder.set_query(&q);
                f.selected = 0;
                f.stale = true;
                f.scroll.scroll_to_item(0, ScrollStrategy::Top);
            }
            TextFieldEvent::Down => {
                f.selected = (f.selected + 1).min(f.results.len().saturating_sub(1));
                // Non-strict: scrolls only when the row is out of view.
                f.scroll.scroll_to_item(f.selected, ScrollStrategy::Bottom);
            }
            TextFieldEvent::Up => {
                f.selected = f.selected.saturating_sub(1);
                f.scroll.scroll_to_item(f.selected, ScrollStrategy::Top);
            }
            TextFieldEvent::Submit => {
                if let Some(m) = f.results.get(f.selected) {
                    let (path, is_dir) = (m.path.clone(), m.is_dir);
                    reveal_found(ws, path, is_dir, window, cx);
                }
            }
            _ => {}
        },
        Overlay::Rename { error, .. } => match ev {
            TextFieldEvent::Submit => ws.submit_rename(window, cx),
            TextFieldEvent::Changed => *error = None,
            _ => {}
        },
        Overlay::BulkRename(b) => match ev {
            TextFieldEvent::Changed => b.recompute(cx),
            TextFieldEvent::Complete | TextFieldEvent::Down => {
                let focused = b
                    .fields
                    .iter()
                    .position(|f| f.focus_handle(cx).is_focused(window))
                    .unwrap_or(0);
                window.focus(&b.fields[(focused + 1) % 4].focus_handle(cx));
            }
            TextFieldEvent::Up => {
                let focused = b
                    .fields
                    .iter()
                    .position(|f| f.focus_handle(cx).is_focused(window))
                    .unwrap_or(0);
                window.focus(&b.fields[(focused + 3) % 4].focus_handle(cx));
            }
            TextFieldEvent::Submit => apply_bulk_rename(ws, window, cx),
            _ => {}
        },
        Overlay::Tags(t) => {
            if let TextFieldEvent::Submit = ev {
                let name = t.field.read(cx).text().trim().to_string();
                if !name.is_empty() {
                    // An existing tag keeps its colour; the picker only
                    // colours new tags (else Finder and Haven disagree).
                    // Same for its spelling ("red" applies the existing "Red").
                    let tag = Store::get(cx)
                        .tags
                        .iter()
                        .find(|k| k.name.eq_ignore_ascii_case(&name))
                        .cloned()
                        .unwrap_or_else(|| Tag::new(name, t.color));
                    ws.apply_tag(tag, true, cx);
                    t_clear(ws, cx);
                }
            }
        }
        _ => {}
    }
    cx.notify();
}

fn t_clear(ws: &mut Workspace, cx: &mut Context<Workspace>) {
    if let Some(Overlay::Tags(t)) = &ws.overlay {
        t.field.update(cx, |f, cx| f.set_text("", false, cx));
    }
}

pub fn apply_bulk_rename(ws: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let Some(Overlay::BulkRename(b)) = &ws.overlay else {
        return;
    };
    let spec = b.spec(cx);
    let paths = b.paths.clone();
    match spec.apply(&paths) {
        Ok(undo) => {
            let new: Vec<PathBuf> = match &undo {
                haven_core::ops::Undo::Moved(pairs) => {
                    pairs.iter().map(|(_, to)| to.clone()).collect()
                }
                _ => vec![],
            };
            ws.close_overlay(window, cx);
            let tab = ws.active_tab().clone();
            tab.update(cx, |t, cx| t.reload_and_select(new, cx));
            ws.push_undo(undo);
            ws.show_toast(format!("Renamed {} items", paths.len()), false, cx);
        }
        Err(e) => {
            if let Some(Overlay::BulkRename(b)) = &mut ws.overlay {
                b.error = Some(e.to_string());
            }
            cx.notify();
        }
    }
}

// ------------------------------------------------------------------ rendering

fn panel(width: f32, cx: &App) -> gpui::Stateful<gpui::Div> {
    let t = theme(cx);
    div()
        .id("overlay-panel")
        .occlude()
        .w(px(width))
        .max_w_full()
        .rounded(px(12.))
        .bg(t.popover_bg)
        .border_1()
        .border_color(t.card_border)
        .shadow_lg()
        .overflow_hidden()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

fn field_box(field: &Entity<TextField>, icon_name: &str, error: bool, cx: &App) -> gpui::Div {
    let t = theme(cx);
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .h(px(34.))
        .px(px(10.))
        .rounded(px(7.))
        .bg(t.field_bg)
        .border_1()
        .border_color(if error { t.danger } else { t.field_border })
        .text_size(px(14.))
        .text_color(t.text)
        .child(icon(icon_name).size(px(15.)).text_color(t.text_faint))
        .child(field.clone())
}

/// Opens a fuzzy-find hit exactly where it was found. (Not via the path
/// resolver: that would follow symlinks, jump `/tmp/x` to `/private/tmp/x`
/// and expand `$`/`~` in real file names.)
fn reveal_found(
    ws: &mut Workspace,
    path: PathBuf,
    is_dir: bool,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let tab = ws.active_tab().clone();
    tab.update(cx, |t, cx| {
        if is_dir {
            t.navigate(crate::tab::Location::Dir(path), true, cx);
        } else if let Some(parent) = path.parent() {
            t.reveal(parent.to_path_buf(), vec![path.clone()], cx);
        }
    });
    ws.close_overlay(window, cx);
    window.focus(&tab.read(cx).focus.clone());
}

fn fuzzy_row(ws: &Workspace, row: usize, cx: &mut Context<Workspace>) -> AnyElement {
    let t = theme(cx).clone();
    let Some(Overlay::Fuzzy(f)) = &ws.overlay else {
        return div().into_any_element();
    };
    let Some(m) = f.results.get(row) else {
        return div().into_any_element();
    };
    let (path, is_dir) = (m.path.clone(), m.is_dir);
    let item =
        div()
            .id(("fz", row))
            .w_full()
            .h(px(28.))
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(10.))
            .rounded(px(6.))
            .text_size(px(13.))
            .when(row == f.selected, |d| d.bg(t.selection))
            .when(row != f.selected, |d| d.hover(|s| s.bg(t.hover)))
            .on_click(cx.listener(move |ws, _, window, cx| {
                reveal_found(ws, path.clone(), is_dir, window, cx)
            }))
            .child(
                icon(if m.is_dir { "folder" } else { "file" })
                    .size(px(14.))
                    .text_color(if m.is_dir { t.folder } else { t.text_muted }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .child(highlighted(&m.rel, &m.indices, t.text_muted, t.text)),
            );
    // uniform_list rows need an explicit width to span the list.
    div().w_full().px(px(6.)).child(item).into_any_element()
}

fn highlighted(text: &str, indices: &[u32], base: Hsla, hl: Hsla) -> AnyElement {
    let mut runs: Vec<AnyElement> = Vec::new();
    let mut buf = String::new();
    let mut in_hl = false;
    for (i, ch) in text.chars().enumerate() {
        let is = indices.binary_search(&(i as u32)).is_ok();
        if is != in_hl && !buf.is_empty() {
            runs.push(
                div()
                    .text_color(if in_hl { hl } else { base })
                    .child(std::mem::take(&mut buf))
                    .into_any_element(),
            );
        }
        in_hl = is;
        buf.push(ch);
    }
    if !buf.is_empty() {
        runs.push(
            div()
                .text_color(if in_hl { hl } else { base })
                .child(buf)
                .into_any_element(),
        );
    }
    div()
        .flex()
        .whitespace_nowrap()
        .children(runs)
        .into_any_element()
}

pub fn render_overlay(
    ws: &mut Workspace,
    _window: &mut Window,
    cx: &mut Context<Workspace>,
) -> Option<AnyElement> {
    let t = theme(cx).clone();
    let overlay = ws.overlay.as_ref()?;
    let content: AnyElement = match overlay {
        Overlay::Palette(p) => {
            let rows: Vec<AnyElement> = p
                .matches
                .iter()
                .take(PALETTE_ROWS)
                .enumerate()
                .map(|(row, &ix)| {
                    let item = &p.items[ix];
                    let selected = row == p.selected;
                    let action = item.action.boxed_clone();
                    div()
                        .id(("pal", row))
                        .h(px(32.))
                        .mx(px(6.))
                        .px(px(10.))
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .rounded(px(6.))
                        .when(selected, |d| d.bg(t.selection))
                        .when(!selected, |d| d.hover(|s| s.bg(t.hover)))
                        .on_click(cx.listener(move |ws, _, window, cx| {
                            ws.overlay = None;
                            ws.dispatch_menu_action(action.boxed_clone(), window, cx)
                        }))
                        .child(icon(item.icon).size(px(15.)).text_color(t.text_muted))
                        .child(
                            div()
                                .text_size(px(13.))
                                .text_color(t.text)
                                .child(item.label.clone()),
                        )
                        .children(item.detail.clone().map(|d| {
                            div()
                                .flex_1()
                                .min_w_0()
                                .ellipsis()
                                .text_size(px(12.))
                                .text_color(t.text_faint)
                                .child(d)
                        }))
                        .when(item.detail.is_none(), |d| d.child(div().flex_1()))
                        .children(item.shortcut.clone().map(|s| kbd(s, cx)))
                        .into_any_element()
                })
                .collect();
            panel(620., cx)
                .child(
                    div()
                        .p(px(10.))
                        .child(field_box(&p.field, "command", false, cx)),
                )
                .child(div().h(px(1.)).bg(t.divider))
                .child(
                    div()
                        .id("pal-list")
                        .max_h(px(420.))
                        .overflow_y_scroll()
                        .track_scroll(&p.scroll)
                        .py(px(6.))
                        .children(rows),
                )
                .into_any_element()
        }
        Overlay::Fuzzy(f) => {
            let count = f.results.len();
            let status = format!(
                "{} of {} {}",
                f.finder.matched_count(),
                f.finder.total(),
                if f.finder.is_walking() {
                    "· scanning…"
                } else {
                    ""
                }
            );
            panel(680., cx)
                .child(
                    div()
                        .p(px(10.))
                        .child(field_box(&f.field, "search", false, cx)),
                )
                .child(div().h(px(1.)).bg(t.divider))
                .child(
                    uniform_list(
                        "fz-list",
                        count,
                        cx.processor(|ws, range: std::ops::Range<usize>, _window, cx| {
                            range.map(|row| fuzzy_row(ws, row, cx)).collect::<Vec<_>>()
                        }),
                    )
                    .track_scroll(f.scroll.clone())
                    .h(px(400.))
                    .py(px(6.)),
                )
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .px(px(14.))
                        .py(px(7.))
                        .border_t_1()
                        .border_color(t.divider)
                        .text_size(px(11.))
                        .text_color(t.text_faint)
                        .child(status)
                        .child("↑↓ select · ↩ reveal · esc close"),
                )
                .into_any_element()
        }
        Overlay::Rename { field, path, error } => {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            panel(440., cx)
                .p(px(16.))
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(
                    div()
                        .text_size(px(14.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(t.text)
                        .child(format!("Rename “{name}”")),
                )
                .child(field_box(field, "pencil", error.is_some(), cx))
                .children(
                    error
                        .clone()
                        .map(|e| div().text_size(px(12.)).text_color(t.danger).child(e)),
                )
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(8.))
                        .child(
                            text_button("rn-cancel", "Cancel", false, cx)
                                .on_click(cx.listener(|ws, _, w, cx| ws.close_overlay(w, cx))),
                        )
                        .child(
                            text_button("rn-ok", "Rename", true, cx)
                                .on_click(cx.listener(|ws, _, w, cx| ws.submit_rename(w, cx))),
                        ),
                )
                .into_any_element()
        }
        Overlay::BulkRename(b) => {
            let names = b.names();
            let toggle = |id: &'static str,
                          label: &'static str,
                          on: bool,
                          cx: &mut Context<Workspace>,
                          f: fn(&mut BulkRenameState)| {
                div()
                    .id(id)
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .px(px(8.))
                    .h(px(26.))
                    .rounded(px(6.))
                    .text_size(px(12.))
                    .cursor_pointer()
                    .bg(if on { t.selection } else { t.hover })
                    .text_color(t.text)
                    .on_click(cx.listener(move |ws, _, _, cx| {
                        if let Some(Overlay::BulkRename(b)) = &mut ws.overlay {
                            f(b);
                            b.recompute(cx);
                            cx.notify();
                        }
                    }))
                    .child(label)
            };
            let case_label = match b.case {
                CaseChange::Keep => "Case: keep",
                CaseChange::Lower => "Case: lower",
                CaseChange::Upper => "Case: UPPER",
            };
            panel(640., cx)
                .p(px(16.))
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(
                    div()
                        .text_size(px(14.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(t.text)
                        .child(format!("Rename {} items", b.paths.len())),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(div().flex_1().child(field_box(
                            &b.fields[0],
                            "search",
                            b.error.is_some(),
                            cx,
                        )))
                        .child(
                            div()
                                .flex_1()
                                .child(field_box(&b.fields[1], "pencil", false, cx)),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(div().flex_1().child(field_box(
                            &b.fields[2],
                            "arrow-left",
                            false,
                            cx,
                        )))
                        .child(div().flex_1().child(field_box(
                            &b.fields[3],
                            "arrow-right",
                            false,
                            cx,
                        ))),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(6.))
                        .child(toggle("br-regex", "Regex", b.regex, cx, |b| {
                            b.regex = !b.regex
                        }))
                        .child(toggle(
                            "br-num",
                            "Number 1, 2, 3…",
                            b.numbering,
                            cx,
                            |b| b.numbering = !b.numbering,
                        ))
                        .child(toggle(
                            "br-case",
                            case_label,
                            b.case != CaseChange::Keep,
                            cx,
                            |b| {
                                b.case = match b.case {
                                    CaseChange::Keep => CaseChange::Lower,
                                    CaseChange::Lower => CaseChange::Upper,
                                    CaseChange::Upper => CaseChange::Keep,
                                }
                            },
                        )),
                )
                .children(
                    b.error
                        .clone()
                        .map(|e| div().text_size(px(12.)).text_color(t.danger).child(e)),
                )
                .child(
                    div()
                        .id("br-preview")
                        .max_h(px(220.))
                        .overflow_y_scroll()
                        .rounded(px(6.))
                        .bg(t.field_bg)
                        .p(px(8.))
                        .text_size(px(12.))
                        .children(names.iter().zip(b.preview.iter()).take(200).map(
                            |(old, new)| {
                                let changed = old != new;
                                div()
                                    .flex()
                                    .gap(px(8.))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .ellipsis()
                                            .text_color(t.text_muted)
                                            .child(old.clone()),
                                    )
                                    .child(
                                        icon("arrow-right").size(px(12.)).text_color(t.text_faint),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .ellipsis()
                                            .text_color(if changed {
                                                t.accent_text
                                            } else {
                                                t.text_muted
                                            })
                                            .child(new.clone()),
                                    )
                            },
                        )),
                )
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .items_center()
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(t.text_faint)
                                .child("Tab moves between fields · ↩ applies"),
                        )
                        .child(
                            div()
                                .flex()
                                .gap(px(8.))
                                .child(
                                    text_button("br-cancel", "Cancel", false, cx).on_click(
                                        cx.listener(|ws, _, w, cx| ws.close_overlay(w, cx)),
                                    ),
                                )
                                .child(text_button("br-ok", "Rename", true, cx).on_click(
                                    cx.listener(|ws, _, w, cx| apply_bulk_rename(ws, w, cx)),
                                )),
                        ),
                )
                .into_any_element()
        }
        Overlay::Tags(ts) => {
            let selected: Vec<Entry> = ws
                .active_tab()
                .read(cx)
                .selected_entries()
                .into_iter()
                .cloned()
                .collect();
            let known = Store::get(cx).tags.clone();
            let color = ts.color;
            let rows: Vec<AnyElement> = known
                .iter()
                .enumerate()
                .map(|(i, tag)| {
                    let count = selected
                        .iter()
                        .filter(|e| {
                            e.tags
                                .iter()
                                .any(|x| x.name.eq_ignore_ascii_case(&tag.name))
                        })
                        .count();
                    let state = if count == 0 {
                        ""
                    } else if count == selected.len() {
                        "check"
                    } else {
                        "ellipsis"
                    };
                    let name = tag.name.clone();
                    div()
                        .id(("tagrow", i))
                        .h(px(28.))
                        .px(px(10.))
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .rounded(px(6.))
                        .hover(|s| s.bg(t.hover))
                        .cursor_pointer()
                        .on_click(
                            cx.listener(move |ws, _, _, cx| ws.toggle_tag_on_selection(&name, cx)),
                        )
                        .child(
                            div().w(px(14.)).children(
                                (!state.is_empty())
                                    .then(|| icon(state).size(px(13.)).text_color(t.accent_text)),
                            ),
                        )
                        .child(tag_dot(
                            if tag.color == TagColor::None {
                                t.text_faint
                            } else {
                                t.tag(tag.color)
                            },
                            px(10.),
                        ))
                        .child(
                            div()
                                .text_size(px(13.))
                                .text_color(t.text)
                                .child(tag.name.clone()),
                        )
                        .into_any_element()
                })
                .collect();
            panel(360., cx)
                .p(px(12.))
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(
                    div()
                        .text_size(px(14.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(t.text)
                        .child(format!(
                            "Tags for {}",
                            if selected.len() == 1 {
                                format!("“{}”", selected[0].name)
                            } else {
                                format!("{} items", selected.len())
                            }
                        )),
                )
                .child(
                    div()
                        .id("tag-list")
                        .max_h(px(260.))
                        .overflow_y_scroll()
                        .children(rows),
                )
                .child(div().h(px(1.)).bg(t.divider))
                .child(field_box(&ts.field, "tag", false, cx))
                .child(
                    div()
                        .flex()
                        .gap(px(6.))
                        .children(TagColor::ALL.iter().enumerate().map(|(i, c)| {
                            let c = *c;
                            div()
                                .id(("color", i))
                                .size(px(20.))
                                .rounded_full()
                                .bg(if c == TagColor::None {
                                    t.hover
                                } else {
                                    t.tag(c)
                                })
                                .border_2()
                                .border_color(if c == color {
                                    t.text
                                } else {
                                    gpui::transparent_black()
                                })
                                .cursor_pointer()
                                .on_click(cx.listener(move |ws, _, _, cx| {
                                    if let Some(Overlay::Tags(ts)) = &mut ws.overlay {
                                        ts.color = c;
                                        cx.notify();
                                    }
                                }))
                        })),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(t.text_faint)
                        .child("Tags are native Finder tags (visible in Finder and Spotlight)."),
                )
                .into_any_element()
        }
        Overlay::Conflict {
            kind,
            sources,
            dest,
            count,
        } => {
            let (kind, sources, dest, count) = (*kind, sources.clone(), dest.clone(), *count);
            let button = |id: &'static str,
                          label: &'static str,
                          policy: Conflict,
                          primary: bool,
                          cx: &mut Context<Workspace>| {
                let (s, d) = (sources.clone(), dest.clone());
                text_button(id, label, primary, cx).on_click(cx.listener(move |ws, _, w, cx| {
                    ws.close_overlay(w, cx);
                    ws.run_transfer(kind, s.clone(), d.clone(), policy, cx);
                }))
            };
            panel(460., cx)
                .p(px(16.))
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(
                    div()
                        .flex()
                        .gap(px(10.))
                        .items_center()
                        .child(icon("triangle-alert").size(px(20.)).text_color(t.warning))
                        .child(
                            div()
                                .text_size(px(14.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(t.text)
                                .child(format!(
                                    "{count} item{} already exist{} in “{}”",
                                    if count == 1 { "" } else { "s" },
                                    if count == 1 { "s" } else { "" },
                                    dest.file_name()
                                        .map(|n| n.to_string_lossy().into_owned())
                                        .unwrap_or_default()
                                )),
                        ),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(t.text_muted)
                        .child("Replaced items are moved to the Trash, so you can recover them."),
                )
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(8.))
                        .child(
                            text_button("cf-cancel", "Cancel", false, cx)
                                .on_click(cx.listener(|ws, _, w, cx| ws.close_overlay(w, cx))),
                        )
                        .child(button("cf-skip", "Skip", Conflict::Skip, false, cx))
                        .child(button(
                            "cf-replace",
                            "Replace",
                            Conflict::Replace,
                            false,
                            cx,
                        ))
                        .child(button("cf-keep", "Keep Both", Conflict::KeepBoth, true, cx)),
                )
                .into_any_element()
        }
        Overlay::Confirm {
            title,
            body,
            confirm,
            action,
        } => {
            let action = action.boxed_clone();
            panel(440., cx)
                .p(px(16.))
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(
                    div()
                        .text_size(px(14.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(t.text)
                        .child(title.clone()),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(t.text_muted)
                        .child(body.clone()),
                )
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(8.))
                        .child(
                            text_button("cm-cancel", "Cancel", false, cx)
                                .on_click(cx.listener(|ws, _, w, cx| ws.close_overlay(w, cx))),
                        )
                        .child(
                            text_button("cm-ok", SharedString::from(confirm.clone()), true, cx)
                                .on_click(cx.listener(move |ws, _, w, cx| {
                                    ws.overlay = None;
                                    ws.dispatch_menu_action(action.boxed_clone(), w, cx)
                                })),
                        ),
                )
                .into_any_element()
        }
    };
    Some(
        div()
            .id("overlay-scrim")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .occlude()
            .bg(t.scrim)
            .flex()
            .flex_col()
            .items_center()
            .pt(px(90.))
            .px(px(16.))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|ws, _, window, cx| ws.close_overlay(window, cx)),
            )
            .child(content)
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_on_tag_keeps_its_ring_under_the_pointer() {
        let t = Theme::dark();
        assert_eq!(tag_ring(true, &t), (t.text, None));
        // Off: no ring, and hovering shows a muted one.
        assert_eq!(
            tag_ring(false, &t),
            (gpui::transparent_black(), Some(t.text_muted))
        );
    }
}
