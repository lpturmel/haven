//! A browser tab: location, history, listing, selection and preview state.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use gpui::{
    Context, EventEmitter, FocusHandle, Font, Hsla, Modifiers, Pixels, Point, ScrollStrategy,
    SharedString, Task, TextRun, UniformListScrollHandle, Window, font,
};
use haven_core::dirsize::{self, Control};
use haven_core::kind::{is_image_ext, is_native_image_ext};
use haven_core::sort::{self, SortKey, SortSpec};
use haven_core::state::{FolderPrefs, ViewMode};
use haven_core::{Entry, list_dir, tags};

use crate::store::Store;
use crate::theme::{MONO_FONT, theme};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Location {
    Start,
    Dir(PathBuf),
    Tag(String),
}

impl Location {
    pub fn dir(&self) -> Option<&Path> {
        match self {
            Location::Dir(p) => Some(p),
            _ => None,
        }
    }

    pub fn title(&self) -> String {
        match self {
            Location::Start => "Home".into(),
            Location::Tag(t) => t.clone(),
            Location::Dir(p) => {
                if p == Path::new("/") {
                    "Macintosh HD".into()
                } else {
                    p.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| p.display().to_string())
                }
            }
        }
    }
}

/// Files being dragged inside Haven.
#[derive(Clone, Debug)]
pub struct DraggedFiles {
    pub paths: Vec<PathBuf>,
}

pub enum TabEvent {
    Navigated {
        old: Option<PathBuf>,
        new: Option<PathBuf>,
    },
    OpenInTabs(Vec<PathBuf>),
    ContextMenu {
        position: Point<Pixels>,
        on_item: bool,
    },
    Drop {
        paths: Vec<PathBuf>,
        dest: PathBuf,
        copy: bool,
    },
}

pub struct TextPreview {
    pub path: PathBuf,
    pub lines: Arc<Vec<SharedString>>,
    pub runs: Option<Arc<Vec<Vec<TextRun>>>>,
    pub lang: Option<&'static str>,
    pub truncated: bool,
    pub bytes: u64,
    /// The file's size and mtime when it was read, to notice in-place edits.
    pub file_size: u64,
    pub file_modified: Option<SystemTime>,
}

pub enum Preview {
    None,
    Loading,
    Text(TextPreview),
    Image(Entry),
    Folder { entry: Entry, count: Option<usize> },
    Info(Entry),
    Multi { count: usize, size: u64 },
}

pub struct Tab {
    pub location: Location,
    back: Vec<Location>,
    forward: Vec<Location>,
    pub all_entries: Vec<Entry>,
    pub entries: Vec<Entry>,
    pub filter: String,
    pub selection: BTreeSet<usize>,
    pub cursor: Option<usize>,
    anchor: Option<usize>,
    pending_collapse: Option<usize>,
    /// Path under the first click of a potential double-click. A second
    /// click only opens if the same item is still under it.
    click_origin: Option<PathBuf>,
    /// Set once a double-click opened something; the rest of that click
    /// gesture (count >= 2) is ignored until a fresh single click.
    click_consumed: bool,
    pub view: ViewMode,
    pub sort: SortSpec,
    pub loading: bool,
    pub error: Option<String>,
    generation: u64,
    load_task: Option<Task<()>>,
    pending_select: Vec<PathBuf>,
    pub scroll: UniformListScrollHandle,
    pub preview_scroll: UniformListScrollHandle,
    pub focus: FocusHandle,
    type_buffer: String,
    type_at: Instant,
    pub preview: Preview,
    preview_task: Option<Task<()>>,
    pub column_root: Option<PathBuf>,
    pub column_cache: HashMap<PathBuf, Arc<Vec<Entry>>>,
    column_loading: HashSet<PathBuf>,
    pub free_space: Option<u64>,
    size_control: Option<Arc<Control>>,
    pub sizing: bool,
    pub grid_cols: usize,
}

impl EventEmitter<TabEvent> for Tab {}

impl Tab {
    pub fn new(location: Location, cx: &mut Context<Self>) -> Tab {
        let mut tab = Tab {
            location: Location::Start,
            back: Vec::new(),
            forward: Vec::new(),
            all_entries: Vec::new(),
            entries: Vec::new(),
            filter: String::new(),
            selection: BTreeSet::new(),
            cursor: None,
            anchor: None,
            pending_collapse: None,
            click_origin: None,
            click_consumed: false,
            view: Store::get(cx).default_view,
            sort: Store::get(cx).default_sort.clone(),
            loading: false,
            error: None,
            generation: 0,
            load_task: None,
            pending_select: Vec::new(),
            scroll: UniformListScrollHandle::new(),
            preview_scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
            type_buffer: String::new(),
            type_at: Instant::now(),
            preview: Preview::None,
            preview_task: None,
            column_root: None,
            column_cache: HashMap::new(),
            column_loading: HashSet::new(),
            free_space: None,
            size_control: None,
            sizing: false,
            grid_cols: 1,
        };
        tab.navigate(location, false, cx);
        tab
    }

    pub fn dir(&self) -> Option<&Path> {
        self.location.dir()
    }

    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    pub fn selected_entries(&self) -> Vec<&Entry> {
        self.selection
            .iter()
            .filter_map(|&i| self.entries.get(i))
            .collect()
    }

    pub fn selected_paths(&self) -> Vec<PathBuf> {
        self.selected_entries()
            .into_iter()
            .map(|e| e.path.clone())
            .collect()
    }

    pub fn cursor_entry(&self) -> Option<&Entry> {
        self.cursor.and_then(|i| self.entries.get(i))
    }

    // ---------------------------------------------------------------- navigation

    pub fn navigate(&mut self, location: Location, push: bool, cx: &mut Context<Self>) {
        if push && location != self.location {
            self.back.push(self.location.clone());
            self.forward.clear();
        }
        self.set_location(location, cx);
    }

    /// Navigates to `dir` and selects `select` once listed.
    pub fn reveal(&mut self, dir: PathBuf, select: Vec<PathBuf>, cx: &mut Context<Self>) {
        if self.location == Location::Dir(dir.clone()) {
            self.pending_select = select;
            self.apply_pending_select(cx);
            return;
        }
        self.navigate(Location::Dir(dir), true, cx);
        self.pending_select = select;
    }

    fn set_location(&mut self, location: Location, cx: &mut Context<Self>) {
        let old = self.dir().map(Path::to_path_buf);
        if let Some(c) = self.size_control.take() {
            c.cancel();
        }
        self.sizing = false;
        self.location = location;
        self.filter.clear();
        self.selection.clear();
        self.cursor = None;
        self.anchor = None;
        self.preview = Preview::None;
        self.error = None;
        self.pending_select.clear();
        self.click_origin = None;
        // A preview still loading for the old folder must not land here.
        self.preview_task = None;
        if let Some(dir) = self.dir().map(Path::to_path_buf) {
            let prefs = Store::get(cx).prefs_for(&dir);
            self.view = prefs.view;
            self.sort = prefs.sort;
            let keep_root = self
                .column_root
                .as_ref()
                .is_some_and(|r| dir.starts_with(r));
            if !keep_root {
                self.column_root = Some(dir.clone());
            }
        }
        self.all_entries.clear();
        self.entries.clear();
        self.reload(cx);
        let new = self.dir().map(Path::to_path_buf);
        cx.emit(TabEvent::Navigated { old, new });
        cx.notify();
    }

    pub fn go_back(&mut self, cx: &mut Context<Self>) {
        if let Some(prev) = self.back.pop() {
            let current = std::mem::replace(&mut self.location, Location::Start);
            self.location = current.clone();
            self.forward.push(current.clone());
            let select = current
                .dir()
                .map(|d| vec![d.to_path_buf()])
                .unwrap_or_default();
            self.set_location(prev, cx);
            self.pending_select = select;
        }
    }

    pub fn go_forward(&mut self, cx: &mut Context<Self>) {
        if let Some(next) = self.forward.pop() {
            self.back.push(self.location.clone());
            self.set_location(next, cx);
        }
    }

    pub fn go_parent(&mut self, cx: &mut Context<Self>) {
        let Some(dir) = self.dir().map(Path::to_path_buf) else {
            return;
        };
        if let Some(parent) = dir.parent() {
            self.navigate(Location::Dir(parent.to_path_buf()), true, cx);
            self.pending_select = vec![dir];
        }
    }

    // ---------------------------------------------------------------- loading

    /// Re-lists and then selects `paths` (e.g. items just created or renamed).
    pub fn reload_and_select(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        self.pending_select = paths;
        self.reload(cx);
    }

    /// Re-lists the current location, preserving selection by path.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        if self.pending_select.is_empty() {
            self.pending_select = self.selected_paths();
        }
        self.generation += 1;
        let generation = self.generation;
        let show_hidden = Store::get(cx).show_hidden;
        self.loading = true;
        let location = self.location.clone();
        let sort = self.sort.clone();
        let task = cx.background_executor().spawn(async move {
            let entries = match &location {
                Location::Start => Ok(Vec::new()),
                Location::Dir(dir) => list_dir(dir, show_hidden).map_err(|e| e.to_string()),
                Location::Tag(name) => Ok(tags::search_tag(name)
                    .iter()
                    .filter_map(|p| Entry::from_path(p).ok())
                    .filter(|e| show_hidden || !e.hidden)
                    .collect()),
            };
            let free = location
                .dir()
                .and_then(haven_core::volumes::available_space);
            entries.map(|mut e| {
                sort::sort_entries(&mut e, &sort);
                (e, free)
            })
        });
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |tab, cx| {
                if tab.generation != generation {
                    return;
                }
                tab.loading = false;
                match result {
                    Ok((entries, free)) => {
                        // Teach the store about tags seen on disk.
                        let seen: Vec<_> = entries
                            .iter()
                            .flat_map(|e| e.tags.iter().cloned())
                            .collect();
                        if !seen.is_empty() {
                            Store::update(cx, |s| {
                                for t in &seen {
                                    s.learn_tag(t);
                                }
                            });
                        }
                        // Carry over computed folder sizes.
                        let sizes: HashMap<PathBuf, u64> = tab
                            .all_entries
                            .iter()
                            .filter_map(|e| e.dir_size.map(|s| (e.path.clone(), s)))
                            .collect();
                        tab.all_entries = entries;
                        for e in &mut tab.all_entries {
                            if let Some(s) = sizes.get(&e.path) {
                                e.dir_size = Some(*s);
                            }
                        }
                        tab.free_space = free;
                        tab.error = None;
                        if let Some(dir) = tab.dir() {
                            tab.column_cache
                                .insert(dir.to_path_buf(), Arc::new(tab.all_entries.clone()));
                        }
                    }
                    Err(e) => {
                        tab.all_entries.clear();
                        tab.error = Some(e);
                    }
                }
                tab.apply_filter_keep_selection(cx);
                tab.apply_pending_select(cx);
                tab.load_tags_pass(generation, cx);
                cx.notify();
            });
        }));
    }

    /// Second pass for large folders: tags are read after the first paint.
    fn load_tags_pass(&mut self, generation: u64, cx: &mut Context<Self>) {
        if !haven_core::listing::needs_tag_pass(self.all_entries.len())
            || !matches!(self.location, Location::Dir(_))
        {
            return;
        }
        let paths: Vec<PathBuf> = self.all_entries.iter().map(|e| e.path.clone()).collect();
        let task = cx
            .background_executor()
            .spawn(async move { haven_core::listing::read_tags_bulk(&paths) });
        cx.spawn(async move |this, cx| {
            let found = task.await;
            if found.is_empty() {
                return;
            }
            let _ = this.update(cx, |tab, cx| {
                if tab.generation != generation {
                    return;
                }
                let map: HashMap<PathBuf, Vec<haven_core::Tag>> = found.into_iter().collect();
                Store::update(cx, |s| {
                    for t in map.values().flatten() {
                        s.learn_tag(t);
                    }
                });
                for e in tab.all_entries.iter_mut().chain(tab.entries.iter_mut()) {
                    if let Some(t) = map.get(&e.path) {
                        e.tags = t.clone();
                    }
                }
                if tab.sort.keys.iter().any(|(k, _)| *k == SortKey::Tags) {
                    tab.resort(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn apply_pending_select(&mut self, cx: &mut Context<Self>) {
        if self.pending_select.is_empty() {
            return;
        }
        let wanted: HashSet<&PathBuf> = self.pending_select.iter().collect();
        let ixs: Vec<usize> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| wanted.contains(&e.path))
            .map(|(i, _)| i)
            .collect();
        let first_wanted = self.pending_select.first().cloned();
        self.pending_select.clear();
        if ixs.is_empty() {
            // The selected items are gone (deleted or renamed elsewhere):
            // drop their preview too.
            self.update_preview(cx);
            return;
        }
        self.selection = ixs.iter().copied().collect();
        let cursor = first_wanted
            .and_then(|p| self.entries.iter().position(|e| e.path == p))
            .unwrap_or(ixs[0]);
        self.cursor = Some(cursor);
        self.anchor = Some(cursor);
        self.scroll_to_cursor();
        self.update_preview(cx);
    }

    fn apply_filter_keep_selection(&mut self, cx: &mut Context<Self>) {
        let selected: Vec<PathBuf> = if self.pending_select.is_empty() {
            self.selected_paths()
        } else {
            std::mem::take(&mut self.pending_select)
        };
        let needle = self.filter.to_lowercase();
        self.entries = if needle.is_empty() {
            self.all_entries.clone()
        } else {
            self.all_entries
                .iter()
                .filter(|e| e.name.to_lowercase().contains(&needle))
                .cloned()
                .collect()
        };
        self.selection.clear();
        self.cursor = None;
        self.anchor = None;
        self.pending_select = selected;
        if self.pending_select.is_empty() && !self.filter.is_empty() && !self.entries.is_empty() {
            self.select_only(0, cx);
        }
    }

    pub fn set_filter(&mut self, filter: &str, cx: &mut Context<Self>) {
        if self.filter == filter {
            return;
        }
        self.filter = filter.to_string();
        self.apply_filter_keep_selection(cx);
        self.apply_pending_select(cx);
        // Keep something selected while filtering so Enter opens the best match.
        if self.selection.is_empty() && !self.filter.is_empty() && !self.entries.is_empty() {
            self.select_only(0, cx);
        }
        cx.notify();
    }

    pub fn set_sort(&mut self, sort: SortSpec, cx: &mut Context<Self>) {
        self.sort = sort;
        self.resort(cx);
        self.save_prefs(cx);
    }

    pub fn click_sort(&mut self, key: SortKey, shift: bool, cx: &mut Context<Self>) {
        let mut spec = self.sort.clone();
        if shift {
            spec.shift_click(key)
        } else {
            spec.click(key)
        }
        self.set_sort(spec, cx);
    }

    fn resort(&mut self, cx: &mut Context<Self>) {
        sort::sort_entries(&mut self.all_entries, &self.sort);
        self.apply_filter_keep_selection(cx);
        self.apply_pending_select(cx);
        cx.notify();
    }

    pub fn set_view(&mut self, view: ViewMode, cx: &mut Context<Self>) {
        self.view = view;
        if view == ViewMode::Columns {
            self.column_root = self.dir().map(Path::to_path_buf);
        }
        self.save_prefs(cx);
        self.update_preview(cx);
        self.scroll_to_cursor();
        cx.notify();
    }

    fn save_prefs(&self, cx: &mut Context<Self>) {
        if let Some(dir) = self.dir().map(Path::to_path_buf) {
            let prefs = FolderPrefs {
                view: self.view,
                sort: self.sort.clone(),
            };
            Store::update(cx, |s| {
                s.default_sort = prefs.sort.clone();
                s.set_prefs(&dir, prefs)
            });
        }
    }

    // ---------------------------------------------------------------- folder sizes

    pub fn calculate_sizes(&mut self, cx: &mut Context<Self>) {
        let dirs: Vec<PathBuf> = self
            .all_entries
            .iter()
            .filter(|e| e.is_dir && e.dir_size.is_none())
            .map(|e| e.path.clone())
            .collect();
        if dirs.is_empty() {
            return;
        }
        if let Some(c) = self.size_control.take() {
            c.cancel();
        }
        let control = Control::new();
        self.size_control = Some(control.clone());
        self.sizing = true;
        let (tx, rx) = smol::channel::unbounded::<(PathBuf, u64)>();
        cx.background_executor()
            .spawn(async move {
                for d in dirs {
                    if control.is_cancelled() {
                        break;
                    }
                    if let Some(size) = dirsize::dir_size(&d, &control)
                        && tx.send((d, size)).await.is_err()
                    {
                        break;
                    }
                }
            })
            .detach();
        cx.spawn(async move |this, cx| {
            loop {
                let Ok(first) = rx.recv().await else { break };
                let mut batch = vec![first];
                cx.background_executor()
                    .timer(Duration::from_millis(120))
                    .await;
                while let Ok(more) = rx.try_recv() {
                    batch.push(more);
                }
                let alive = this
                    .update(cx, |tab, cx| {
                        let map: HashMap<_, _> = batch.into_iter().collect();
                        for e in tab.all_entries.iter_mut().chain(tab.entries.iter_mut()) {
                            if let Some(s) = map.get(&e.path) {
                                e.dir_size = Some(*s);
                            }
                        }
                        if tab.sort.keys.iter().any(|(k, _)| *k == SortKey::Size) {
                            tab.resort(cx);
                        }
                        cx.notify();
                    })
                    .is_ok();
                if !alive {
                    break;
                }
            }
            let _ = this.update(cx, |tab, cx| {
                tab.sizing = false;
                cx.notify();
            });
        })
        .detach();
    }

    // ---------------------------------------------------------------- selection

    pub fn select_only(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix >= self.entries.len() {
            return;
        }
        self.selection.clear();
        self.selection.insert(ix);
        self.cursor = Some(ix);
        self.anchor = Some(ix);
        self.scroll_to_cursor();
        self.update_preview(cx);
        cx.notify();
    }

    fn extend_to(&mut self, ix: usize, cx: &mut Context<Self>) {
        let anchor = self.anchor.unwrap_or(ix);
        let (a, b) = if anchor <= ix {
            (anchor, ix)
        } else {
            (ix, anchor)
        };
        self.selection = (a..=b).collect();
        self.cursor = Some(ix);
        self.scroll_to_cursor();
        self.update_preview(cx);
        cx.notify();
    }

    pub fn toggle_select(&mut self, ix: usize, cx: &mut Context<Self>) {
        if !self.selection.remove(&ix) {
            self.selection.insert(ix);
        }
        self.cursor = Some(ix);
        self.anchor = Some(ix);
        self.update_preview(cx);
        cx.notify();
    }

    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        self.selection = (0..self.entries.len()).collect();
        self.update_preview(cx);
        cx.notify();
    }

    pub fn clear_selection(&mut self, cx: &mut Context<Self>) {
        self.selection.clear();
        self.cursor = None;
        self.anchor = None;
        self.preview = Preview::None;
        self.preview_task = None;
        cx.notify();
    }

    pub fn move_cursor(&mut self, delta: isize, extend: bool, cx: &mut Context<Self>) {
        let len = self.entries.len();
        if len == 0 {
            return;
        }
        let next = match self.cursor {
            None => {
                if delta >= 0 {
                    0
                } else {
                    len - 1
                }
            }
            Some(c) => (c as isize + delta).clamp(0, len as isize - 1) as usize,
        };
        if extend {
            self.extend_to(next, cx)
        } else {
            self.select_only(next, cx)
        }
    }

    pub fn move_to_edge(&mut self, end: bool, cx: &mut Context<Self>) {
        if self.entries.is_empty() {
            return;
        }
        self.select_only(if end { self.entries.len() - 1 } else { 0 }, cx);
    }

    pub fn scroll_to_cursor(&self) {
        if let Some(c) = self.cursor {
            let row = if self.view == ViewMode::Grid {
                c / self.grid_cols.max(1)
            } else {
                c
            };
            self.scroll.scroll_to_item(row, ScrollStrategy::Center);
        }
    }

    pub fn mouse_down_item(
        &mut self,
        ix: usize,
        modifiers: Modifiers,
        click_count: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus);
        self.pending_collapse = None;
        if modifiers.control {
            let position = window.mouse_position();
            self.context_item(ix, position, window, cx);
            return;
        }
        let path = self.entries.get(ix).map(|e| e.path.clone());
        if click_count >= 2 {
            // After a double-click opened something (e.g. navigated into a
            // folder), the gesture's remaining clicks must not act on whatever
            // now sits under the pointer.
            if self.click_consumed {
                return;
            }
            // Only open what the first click selected.
            if path.is_some() && self.click_origin == path {
                self.click_consumed = true;
                self.click_origin = None;
                self.select_only(ix, cx);
                self.open_selected(window, cx);
                return;
            }
            // A quick click on a different item (e.g. a shift-click right after
            // a click elsewhere): handle it as a normal click below.
        } else {
            self.click_consumed = false;
        }
        self.click_origin = path;
        if modifiers.platform {
            self.toggle_select(ix, cx);
        } else if modifiers.shift {
            self.extend_to(ix, cx);
        } else if self.selection.contains(&ix) && self.selection.len() > 1 {
            self.pending_collapse = Some(ix);
        } else {
            self.select_only(ix, cx);
        }
    }

    pub fn click_item(&mut self, ix: usize, cx: &mut Context<Self>) {
        if self.pending_collapse.take() == Some(ix) {
            self.select_only(ix, cx);
        }
    }

    /// Right-click: select the item unless it is already part of the selection.
    pub fn context_item(
        &mut self,
        ix: usize,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus);
        if !self.selection.contains(&ix) {
            self.select_only(ix, cx);
        }
        cx.emit(TabEvent::ContextMenu {
            position,
            on_item: true,
        });
    }

    pub fn context_background(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus);
        self.clear_selection(cx);
        cx.emit(TabEvent::ContextMenu {
            position,
            on_item: false,
        });
    }

    /// Type-to-select: jumps to the first entry starting with the typed prefix.
    pub fn type_select(&mut self, ch: &str, cx: &mut Context<Self>) -> bool {
        if self.type_at.elapsed() > Duration::from_millis(800) {
            self.type_buffer.clear();
        }
        if self.type_buffer.is_empty() && ch == " " {
            return false;
        }
        self.type_at = Instant::now();
        self.type_buffer.push_str(&ch.to_lowercase());
        let needle = self.type_buffer.clone();
        if let Some(ix) = self
            .entries
            .iter()
            .position(|e| e.name.to_lowercase().starts_with(&needle))
        {
            self.select_only(ix, cx);
        }
        true
    }

    // ---------------------------------------------------------------- opening

    pub fn open_selected(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let selected: Vec<Entry> = self.selected_entries().into_iter().cloned().collect();
        if selected.is_empty() {
            return;
        }
        if selected.len() == 1 && selected[0].is_navigable() {
            self.navigate(Location::Dir(selected[0].path.clone()), true, cx);
            return;
        }
        let mut dirs = Vec::new();
        for e in selected {
            if e.is_navigable() {
                dirs.push(e.path.clone());
            } else {
                cx.open_with_system(&e.path);
                Store::update(cx, |s| s.record_opened(&e.path));
            }
        }
        if !dirs.is_empty() {
            cx.emit(TabEvent::OpenInTabs(dirs));
        }
    }

    pub fn open_in_new_tab(&mut self, cx: &mut Context<Self>) {
        let dirs: Vec<PathBuf> = self
            .selected_entries()
            .into_iter()
            .filter(|e| e.is_navigable())
            .map(|e| e.path.clone())
            .collect();
        if !dirs.is_empty() {
            cx.emit(TabEvent::OpenInTabs(dirs));
        }
    }

    pub fn drop_on(
        &mut self,
        dest: PathBuf,
        paths: Vec<PathBuf>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let copy = window.modifiers().alt;
        cx.emit(TabEvent::Drop { paths, dest, copy });
    }

    // ---------------------------------------------------------------- columns

    /// Ancestor directories shown as columns before the current one.
    pub fn column_chain(&self) -> Vec<PathBuf> {
        let (Some(root), Some(dir)) = (self.column_root.as_ref(), self.dir()) else {
            return Vec::new();
        };
        let mut chain: Vec<PathBuf> = dir
            .ancestors()
            .take_while(|a| a.starts_with(root))
            .map(Path::to_path_buf)
            .collect();
        chain.reverse();
        chain.pop(); // current dir is rendered from `entries`
        chain
    }

    pub fn column_listing(
        &mut self,
        dir: &Path,
        cx: &mut Context<Self>,
    ) -> Option<Arc<Vec<Entry>>> {
        if let Some(hit) = self.column_cache.get(dir) {
            return Some(hit.clone());
        }
        if self.column_loading.insert(dir.to_path_buf()) {
            let dir = dir.to_path_buf();
            let show_hidden = Store::get(cx).show_hidden;
            let sort = self.sort.clone();
            let task = cx.background_executor().spawn({
                let dir = dir.clone();
                async move {
                    let mut e = list_dir(&dir, show_hidden).unwrap_or_default();
                    sort::sort_entries(&mut e, &sort);
                    e
                }
            });
            cx.spawn(async move |this, cx| {
                let entries = task.await;
                let _ = this.update(cx, |tab, cx| {
                    tab.column_loading.remove(&dir);
                    tab.column_cache.insert(dir, Arc::new(entries));
                    cx.notify();
                });
            })
            .detach();
        }
        None
    }

    pub fn column_left(&mut self, cx: &mut Context<Self>) {
        self.go_parent(cx);
    }

    pub fn column_right(&mut self, cx: &mut Context<Self>) {
        let Some(entry) = self.cursor_entry().cloned() else {
            self.move_cursor(1, false, cx);
            return;
        };
        if entry.is_navigable() {
            self.navigate(Location::Dir(entry.path.clone()), true, cx);
            if let Some(first) = self
                .column_cache
                .get(&entry.path)
                .and_then(|v| v.first())
                .map(|e| e.path.clone())
            {
                self.pending_select = vec![first];
            }
        }
    }

    // ---------------------------------------------------------------- preview

    pub fn invalidate_preview(&mut self, cx: &mut Context<Self>) {
        self.preview = Preview::None;
        self.update_preview(cx);
    }

    pub fn update_preview(&mut self, cx: &mut Context<Self>) {
        let visible = Store::get(cx).preview_visible || self.view == ViewMode::Columns;
        if !visible {
            return;
        }
        let selected: Vec<Entry> = self.selected_entries().into_iter().cloned().collect();
        if selected.len() > 1 {
            let size = selected.iter().filter_map(|e| e.effective_size()).sum();
            self.preview = Preview::Multi {
                count: selected.len(),
                size,
            };
            self.preview_task = None;
            return;
        }
        let Some(entry) = selected.into_iter().next() else {
            self.preview = Preview::None;
            self.preview_task = None;
            return;
        };
        if let Preview::Text(t) = &self.preview
            && t.path == entry.path
            && t.file_size == entry.size
            && t.file_modified == entry.modified
        {
            return;
        }
        if entry.is_dir {
            let path = entry.path.clone();
            self.preview = Preview::Folder { entry, count: None };
            self.preview_task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(40))
                    .await;
                let count = cx
                    .background_executor()
                    .spawn(async move { std::fs::read_dir(&path).map(|r| r.count()).ok() })
                    .await;
                let _ = this.update(cx, |tab, cx| {
                    if let Preview::Folder { count: c, .. } = &mut tab.preview {
                        *c = count;
                        cx.notify();
                    }
                });
            }));
            return;
        }
        if is_image_ext(&entry.ext) || is_native_image_ext(&entry.ext) {
            self.preview = Preview::Image(entry);
            self.preview_task = None;
            return;
        }
        let path = entry.path.clone();
        let (file_size, file_modified) = (entry.size, entry.modified);
        self.preview = Preview::Loading;
        let mono = font(MONO_FONT);
        self.preview_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(35))
                .await;
            let loaded = cx
                .background_executor()
                .spawn(load_text(path.clone()))
                .await;
            let Some((text, truncated, bytes)) = loaded else {
                let _ = this.update(cx, |tab, cx| {
                    tab.preview = Preview::Info(entry);
                    cx.notify();
                });
                return;
            };
            let first_line = text.lines().next().map(str::to_string);
            let lang = haven_highlight::detect(&path, first_line.as_deref());
            let text = Arc::new(text);
            let lines: Arc<Vec<SharedString>> = Arc::new(
                text.split('\n')
                    .take(50_000)
                    .map(|l| SharedString::from(l.to_string()))
                    .collect(),
            );
            let _ = this.update(cx, |tab, cx| {
                tab.preview_scroll = UniformListScrollHandle::new();
                tab.preview = Preview::Text(TextPreview {
                    path: path.clone(),
                    lines: lines.clone(),
                    runs: None,
                    lang: lang.map(|l| l.name()),
                    truncated,
                    bytes,
                    file_size,
                    file_modified,
                });
                cx.notify();
            });
            let Some(lang) = lang else { return };
            if text.len() > 1_500_000 {
                return;
            }
            let spans = cx
                .background_executor()
                .spawn({
                    let text = text.clone();
                    async move { haven_highlight::highlight(lang, &text, None) }
                })
                .await;
            let Some(spans) = spans else { return };
            let _ = this.update(cx, |tab, cx| {
                let t = theme(cx).clone();
                let runs = build_runs(&lines, &spans, &mono, t.text, |s| t.syntax(s));
                if let Preview::Text(tp) = &mut tab.preview
                    && tp.path == path
                {
                    tp.runs = Some(Arc::new(runs));
                    cx.notify();
                }
            });
        }));
    }
}

/// Reads a text file for preview. Returns `None` for binary content.
async fn load_text(path: PathBuf) -> Option<(String, bool, u64)> {
    use std::io::Read;
    const LIMIT: u64 = 2 * 1024 * 1024;
    let mut file = std::fs::File::open(&path).ok()?;
    let len = file.metadata().ok()?.len();
    let mut buf = Vec::with_capacity(len.min(LIMIT) as usize);
    file.by_ref().take(LIMIT).read_to_end(&mut buf).ok()?;
    let sniff = &buf[..buf.len().min(8192)];
    if sniff.contains(&0) {
        return None;
    }
    let text = match String::from_utf8(buf) {
        Ok(s) => s,
        Err(e) => {
            let bytes = e.into_bytes();
            let valid = std::str::from_utf8(&bytes)
                .map(|_| bytes.len())
                .unwrap_or_else(|e| e.valid_up_to());
            // Allow a cut multi-byte char at the end; otherwise treat as binary.
            if bytes.len() - valid > 4 {
                return None;
            }
            String::from_utf8_lossy(&bytes[..valid]).into_owned()
        }
    };
    Some((haven_highlight::normalize(&text), len > LIMIT, len))
}

fn build_runs(
    lines: &[SharedString],
    spans: &haven_highlight::LineSpans,
    font: &Font,
    default: Hsla,
    color: impl Fn(haven_highlight::Style) -> Hsla,
) -> Vec<Vec<TextRun>> {
    let run = |len: usize, c: Hsla| TextRun {
        len,
        font: font.clone(),
        color: c,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let mut out = Vec::new();
            let mut pos = 0;
            if let Some(line_spans) = spans.get(i) {
                for (range, style) in line_spans {
                    let start = range.start.min(line.len());
                    let end = range.end.min(line.len());
                    if start < pos || end <= start {
                        continue;
                    }
                    if start > pos {
                        out.push(run(start - pos, default));
                    }
                    out.push(run(end - start, color(*style)));
                    pos = end;
                }
            }
            if pos < line.len() {
                out.push(run(line.len() - pos, default));
            }
            out
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{AppContext as _, Entity, TestAppContext};

    fn init(cx: &mut TestAppContext) {
        cx.update(|cx| {
            crate::store::init_test_env();
            crate::store::Store::init(cx);
            cx.set_global(crate::theme::Theme::dark());
            crate::icons::IconCache::init(cx);
            cx.set_global(crate::views::start::StartData::default());
        });
    }

    fn fixture() -> PathBuf {
        let dir = tempfile::tempdir().unwrap().keep();
        let root = std::fs::canonicalize(dir).unwrap();
        std::fs::create_dir(root.join("alpha")).unwrap();
        std::fs::create_dir(root.join("beta")).unwrap();
        std::fs::write(root.join("alpha/inner.txt"), b"x").unwrap();
        std::fs::write(root.join("notes.md"), b"# hi").unwrap();
        std::fs::write(root.join("big.bin"), vec![0u8; 5000]).unwrap();
        root
    }

    #[gpui::test]
    async fn navigation_history_and_parent_selection(cx: &mut TestAppContext) {
        init(cx);
        let root = fixture();
        let tab = cx.new(|cx| Tab::new(Location::Dir(root.clone()), cx));
        cx.run_until_parked();
        tab.read_with(cx, |t, _| {
            let names: Vec<_> = t.entries.iter().map(|e| e.name.as_str()).collect();
            assert_eq!(names, ["alpha", "beta", "big.bin", "notes.md"]);
        });

        tab.update(cx, |t, cx| {
            t.navigate(Location::Dir(root.join("alpha")), true, cx)
        });
        cx.run_until_parked();
        tab.read_with(cx, |t, _| {
            assert_eq!(t.entries.len(), 1);
            assert!(t.can_go_back());
        });

        tab.update(cx, |t, cx| t.go_parent(cx));
        cx.run_until_parked();
        tab.read_with(cx, |t, _| {
            assert_eq!(t.dir(), Some(root.as_path()));
            assert_eq!(t.cursor_entry().map(|e| e.name.as_str()), Some("alpha"));
        });

        tab.update(cx, |t, cx| t.go_back(cx));
        cx.run_until_parked();
        tab.update(cx, |t, cx| t.go_forward(cx));
        cx.run_until_parked();
        tab.read_with(cx, |t, _| assert_eq!(t.dir(), Some(root.as_path())));
    }

    #[gpui::test]
    async fn selection_filter_and_sort(cx: &mut TestAppContext) {
        init(cx);
        let root = fixture();
        let tab = cx.new(|cx| Tab::new(Location::Dir(root.clone()), cx));
        cx.run_until_parked();
        tab.update(cx, |t, cx| {
            t.move_cursor(1, false, cx);
            t.move_cursor(1, true, cx);
            assert_eq!(t.selection.len(), 2);
            t.set_filter("no", cx);
            assert_eq!(t.entries.len(), 1);
            assert_eq!(t.cursor_entry().unwrap().name, "notes.md");
            t.set_filter("", cx);
            assert_eq!(t.entries.len(), 4);
            t.click_sort(SortKey::Size, false, cx);
            let files: Vec<_> = t
                .entries
                .iter()
                .filter(|e| !e.is_dir)
                .map(|e| e.name.as_str())
                .collect();
            assert_eq!(files, ["big.bin", "notes.md"]);
            assert!(t.type_select("n", cx));
            assert_eq!(t.cursor_entry().unwrap().name, "notes.md");
        });
    }

    #[gpui::test]
    async fn reload_preserves_selection_and_previews_text(cx: &mut TestAppContext) {
        init(cx);
        let root = fixture();
        let tab = cx.new(|cx| Tab::new(Location::Dir(root.clone()), cx));
        cx.run_until_parked();
        tab.update(cx, |t, cx| {
            let ix = t.entries.iter().position(|e| e.name == "notes.md").unwrap();
            t.select_only(ix, cx);
        });
        std::fs::write(root.join("aaa.txt"), b"").unwrap();
        tab.update(cx, |t, cx| t.reload(cx));
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        tab.read_with(cx, |t, _| {
            assert_eq!(t.entries.len(), 5);
            assert_eq!(t.cursor_entry().unwrap().name, "notes.md");
            match &t.preview {
                Preview::Text(tp) => {
                    assert_eq!(tp.lang, Some("Markdown"));
                    assert_eq!(tp.lines[0].as_ref(), "# hi");
                }
                _ => panic!("expected text preview"),
            }
        });
    }

    fn click(tab: &Entity<Tab>, ix: usize, count: usize, cx: &mut gpui::VisualTestContext) {
        tab.update_in(cx, |t, window, cx| {
            t.mouse_down_item(ix, gpui::Modifiers::default(), count, window, cx)
        });
        cx.run_until_parked();
    }

    #[gpui::test]
    async fn double_click_on_folder_never_opens_what_lands_under_the_pointer(
        cx: &mut TestAppContext,
    ) {
        init(cx);
        let root = fixture();
        let (tab, cx) = cx.add_window_view(|_, cx| Tab::new(Location::Dir(root.clone()), cx));
        cx.run_until_parked();
        // Double-click `alpha` (row 0): opens the folder.
        click(&tab, 0, 1, cx);
        click(&tab, 0, 2, cx);
        tab.read_with(cx, |t, _| {
            assert_eq!(t.dir(), Some(root.join("alpha").as_path()));
            assert_eq!(t.entries[0].name, "inner.txt");
        });
        // A late click of the same gesture lands on `inner.txt`, now in row 0.
        // It must not open the file (the test platform panics on any open).
        click(&tab, 0, 2, cx);
        click(&tab, 0, 3, cx);
        tab.read_with(cx, |t, _| {
            assert_eq!(t.dir(), Some(root.join("alpha").as_path()))
        });
    }

    #[gpui::test]
    async fn double_click_must_hit_the_item_the_first_click_selected(cx: &mut TestAppContext) {
        init(cx);
        let root = fixture();
        let (tab, cx) = cx.add_window_view(|_, cx| Tab::new(Location::Dir(root.clone()), cx));
        cx.run_until_parked();
        // First click on `alpha`, second (count 2) on `beta`: no open.
        click(&tab, 0, 1, cx);
        click(&tab, 1, 2, cx);
        tab.read_with(cx, |t, _| assert_eq!(t.dir(), Some(root.as_path())));
        // A proper double-click on `beta` still opens it.
        click(&tab, 1, 1, cx);
        click(&tab, 1, 2, cx);
        tab.read_with(cx, |t, _| {
            assert_eq!(t.dir(), Some(root.join("beta").as_path()))
        });
    }

    #[gpui::test]
    async fn preview_refreshes_when_the_file_changes_in_place(cx: &mut TestAppContext) {
        init(cx);
        let root = fixture();
        let tab = cx.new(|cx| Tab::new(Location::Dir(root.clone()), cx));
        cx.run_until_parked();
        let preview_first_line = |tab: &Entity<Tab>, cx: &mut TestAppContext| {
            cx.executor().advance_clock(Duration::from_millis(100));
            cx.run_until_parked();
            tab.read_with(cx, |t, _| match &t.preview {
                Preview::Text(tp) => tp.lines[0].to_string(),
                _ => panic!("expected text preview"),
            })
        };
        tab.update(cx, |t, cx| {
            let ix = t.entries.iter().position(|e| e.name == "notes.md").unwrap();
            t.select_only(ix, cx);
        });
        assert_eq!(preview_first_line(&tab, cx), "# hi");
        // Same path, new content (e.g. an undo restored an older version).
        std::fs::write(root.join("notes.md"), b"# changed on disk").unwrap();
        tab.update(cx, |t, cx| t.reload(cx));
        cx.run_until_parked();
        assert_eq!(preview_first_line(&tab, cx), "# changed on disk");
    }

    #[gpui::test]
    async fn quick_shift_click_on_another_row_still_extends(cx: &mut TestAppContext) {
        init(cx);
        let root = fixture();
        let (tab, cx) = cx.add_window_view(|_, cx| Tab::new(Location::Dir(root.clone()), cx));
        cx.run_until_parked();
        click(&tab, 0, 1, cx);
        // macOS can report count 2 for a fast second click elsewhere.
        tab.update_in(cx, |t, window, cx| {
            let shift = gpui::Modifiers {
                shift: true,
                ..Default::default()
            };
            t.mouse_down_item(2, shift, 2, window, cx)
        });
        tab.read_with(cx, |t, _| {
            assert_eq!(t.dir(), Some(root.as_path()));
            assert_eq!(t.selection.len(), 3);
        });
    }
}
