//! Root view: sidebar + main card (tabs, toolbar, content, status bar) + overlays.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    Action, App, AppContext, ClipboardItem, Context, Entity, FocusHandle, Focusable, Pixels, Point,
    Subscription, Task, Window,
};

use haven_core::complete::{self, Candidate};
use haven_core::ops::{self, Conflict, OpKind, Progress, Undo};
use haven_core::pathres::{self, Resolved};
use haven_core::watch::DirWatcher;
use haven_core::{Entry, Tag, recents, tildify, volumes};

use crate::overlays::{ContextMenu, MenuEntry, Overlay};
use crate::store::Store;
use crate::tab::{Location, Tab, TabEvent};
use crate::text_field::{TextField, TextFieldEvent};
use crate::views::start::StartData;

pub struct Job {
    pub id: usize,
    pub label: String,
    pub progress: Arc<Progress>,
    pub done_at: Option<Instant>,
}

pub struct Toast {
    pub message: String,
    pub error: bool,
    pub at: Instant,
}

pub struct Workspace {
    pub tabs: Vec<Entity<Tab>>,
    pub active: usize,
    closed: Vec<Location>,
    pub focus: FocusHandle,
    pub path_field: Entity<TextField>,
    pub path_editing: bool,
    pub completions: Vec<Candidate>,
    pub completion_ix: Option<usize>,
    pub path_error: Option<String>,
    completion_task: Option<Task<()>>,
    pub filter_field: Entity<TextField>,
    pub overlay: Option<Overlay>,
    pub context_menu: Option<ContextMenu>,
    pub clipboard: Option<(Vec<PathBuf>, bool)>,
    pub jobs: Vec<Job>,
    next_job: usize,
    undo_stack: Vec<Undo>,
    /// An undo is running in the background; further ⌘Z wait for it.
    undo_running: bool,
    /// Tab folders with a background "does it still exist?" check running.
    exists_inflight: HashSet<PathBuf>,
    /// Tag writes are read-modify-write on each file's xattr: they go through
    /// one background worker, in order, so quick successive changes can't race.
    tag_tx: smol::channel::Sender<(Vec<PathBuf>, Tag, bool)>,
    pub toast: Option<Toast>,
    pub sidebar_visible: bool,
    /// Volumes being ejected right now (their eject buttons show a spinner).
    pub ejecting: HashSet<PathBuf>,
    /// Last seen `show_hidden` / `preview_visible`, to react when Settings
    /// flips them from another window.
    seen_show_hidden: bool,
    seen_preview_visible: bool,
    watcher: Option<DirWatcher>,
    overlay_subs: Vec<Subscription>,
    _subs: Vec<Subscription>,
}

impl Workspace {
    pub fn new(
        initial: Vec<Location>,
        active: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Workspace {
        let path_field = cx.new(|cx| TextField::new("Type a path…  (~, .., $VAR)", cx));
        let filter_field = cx.new(|cx| TextField::new("Filter", cx));
        let mut subs = vec![
            cx.subscribe_in(&path_field, window, Self::on_path_event),
            cx.subscribe_in(&filter_field, window, Self::on_filter_event),
            cx.observe_global::<Store>(|ws, cx| {
                // Settings can flip "show hidden files" from another window.
                let hidden = Store::get(cx).show_hidden;
                if hidden != ws.seen_show_hidden {
                    ws.seen_show_hidden = hidden;
                    for tab in ws.tabs.clone() {
                        tab.update(cx, |t, cx| {
                            t.column_cache.clear();
                            t.reload(cx)
                        });
                    }
                }
                // The preview isn't kept up to date while hidden: rebuild it
                // for the current selection when it comes back.
                let preview = Store::get(cx).preview_visible;
                if preview != ws.seen_preview_visible {
                    ws.seen_preview_visible = preview;
                    for tab in ws.tabs.clone() {
                        tab.update(cx, |t, cx| t.invalidate_preview(cx));
                    }
                }
                cx.notify()
            }),
            cx.observe_global::<StartData>(|this, cx| {
                for tab in &this.tabs {
                    tab.update(cx, |_, cx| cx.notify());
                }
            }),
        ];

        // Directory watcher → debounced reloads of affected tabs.
        let (tx, rx) = smol::channel::unbounded::<PathBuf>();
        let watcher = DirWatcher::new(move |dir| {
            let _ = tx.try_send(dir);
        })
        .ok();
        cx.spawn(async move |this, cx| {
            while let Ok(first) = rx.recv().await {
                cx.background_executor()
                    .timer(Duration::from_millis(80))
                    .await;
                let mut dirs = vec![first];
                while let Ok(d) = rx.try_recv() {
                    dirs.push(d);
                }
                if this.update(cx, |ws, cx| ws.reload_dirs(&dirs, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();

        let (tag_tx, tag_rx) = smol::channel::unbounded::<(Vec<PathBuf>, Tag, bool)>();
        let (tag_done_tx, tag_done_rx) = smol::channel::unbounded::<Result<(), String>>();
        cx.background_executor()
            .spawn(async move {
                while let Ok((paths, tag, on)) = tag_rx.recv().await {
                    let r = haven_core::tags::set_tag(&paths, &tag, on).map_err(|e| e.to_string());
                    PENDING_TAG_WRITES.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                    // Keep draining even if nobody listens any more (window
                    // closed): queued tag writes must still land on disk.
                    let _ = tag_done_tx.send(r).await;
                }
            })
            .detach();
        cx.spawn(async move |this, cx| {
            while let Ok(result) = tag_done_rx.recv().await {
                let alive = this.update(cx, |ws, cx| {
                    if let Err(e) = result {
                        ws.show_toast(format!("Couldn't tag: {e}"), true, cx);
                    }
                    ws.active_tab().clone().update(cx, |t, cx| t.reload(cx));
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();

        let mut ws = Workspace {
            tabs: Vec::new(),
            active: 0,
            closed: Vec::new(),
            focus: cx.focus_handle(),
            path_field,
            path_editing: false,
            completions: Vec::new(),
            completion_ix: None,
            path_error: None,
            completion_task: None,
            filter_field,
            overlay: None,
            context_menu: None,
            clipboard: None,
            jobs: Vec::new(),
            next_job: 0,
            undo_stack: Vec::new(),
            undo_running: false,
            exists_inflight: HashSet::new(),
            tag_tx,
            toast: None,
            sidebar_visible: true,
            ejecting: HashSet::new(),
            seen_show_hidden: Store::get(cx).show_hidden,
            seen_preview_visible: Store::get(cx).preview_visible,
            watcher,
            overlay_subs: Vec::new(),
            _subs: Vec::new(),
        };
        let initial = if initial.is_empty() {
            vec![Location::Start]
        } else {
            initial
        };
        // Insert in saved order (add_tab would insert each after the active one).
        for (i, loc) in initial.into_iter().enumerate() {
            ws.add_tab_at(loc, i, false, window, cx);
        }
        // Focus the restored active tab (not just the first one) so key
        // bindings reach the visible view.
        ws.activate(active.min(ws.tabs.len() - 1), window, cx);
        subs.push(cx.on_focus_out(
            &ws.path_field.focus_handle(cx),
            window,
            |ws, _, window, cx| {
                // gpui also reports focus-out when the window is deactivated;
                // keep the edit then (like Finder's Go to Folder) so it is still
                // there, focused, when the user comes back.
                if ws.path_editing && window.is_window_active() {
                    // Don't force focus back to the file view: focus may have
                    // moved on purpose (filter field, palette). `end_path_edit`
                    // still rescues focus left on nothing or on the hidden field.
                    ws.end_path_edit(false, window, cx);
                }
            },
        ));
        // When the window comes back to the front with nothing focused, put
        // focus on the active tab; otherwise no key binding can fire.
        subs.push(cx.observe_window_activation(window, |ws, window, cx| {
            if window.is_window_active() && window.focused(cx).is_none() {
                let focus = ws.active_tab().read(cx).focus.clone();
                window.focus(&focus);
            }
        }));
        // `add_tab` already pushed the initial tabs' subscriptions: keep them.
        ws._subs.extend(subs);
        refresh_start_data(cx);
        refresh_pins(cx);
        // Keep drives fresh while running.
        cx.spawn(async move |_, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(5)).await;
                let vols = cx
                    .background_executor()
                    .spawn(async { volumes::list_volumes() })
                    .await;
                let ok = cx.update(|cx| {
                    if cx.global::<StartData>().volumes != vols {
                        cx.global_mut::<StartData>().volumes = vols;
                    }
                });
                if ok.is_err() {
                    break;
                }
            }
        })
        .detach();
        ws
    }

    // ---------------------------------------------------------------- tabs

    pub fn active_tab(&self) -> &Entity<Tab> {
        &self.tabs[self.active]
    }

    pub fn add_tab(
        &mut self,
        location: Location,
        activate: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let at = if self.tabs.is_empty() {
            0
        } else {
            self.active + 1
        };
        self.add_tab_at(location, at, activate, window, cx);
    }

    /// Inserts a tab at `at` (clamped). Keeps the active tab active unless
    /// `activate` (or it is the first tab).
    pub fn add_tab_at(
        &mut self,
        location: Location,
        at: usize,
        activate: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tab = cx.new(|cx| Tab::new(location, cx));
        let sub = cx.subscribe_in(&tab, window, Self::on_tab_event);
        self._subs.push(sub);
        if let Some(dir) = tab.read(cx).dir().map(Path::to_path_buf)
            && let Some(w) = self.watcher.as_mut()
        {
            w.watch(&dir);
        }
        let insert_at = at.min(self.tabs.len());
        self.tabs.insert(insert_at, tab);
        if activate || self.tabs.len() == 1 {
            self.activate(insert_at, window, cx);
        } else if insert_at <= self.active && self.tabs.len() > 1 {
            // Inserted before the active tab: keep pointing at the same tab.
            self.active += 1;
        }
        self.save_session(cx);
        cx.notify();
    }

    pub fn activate(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        self.active = ix;
        self.end_path_edit(false, window, cx);
        let filter = self.tabs[ix].read(cx).filter.clone();
        self.filter_field
            .update(cx, |f, cx| f.set_text(filter, false, cx));
        let focus = self.tabs[ix].read(cx).focus.clone();
        window.focus(&focus);
        self.save_session(cx);
        cx.notify();
    }

    pub fn close_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.len() <= 1 {
            // Closing the last tab goes back to the start page rather than closing the window.
            self.active_tab()
                .clone()
                .update(cx, |t, cx| t.navigate(Location::Start, true, cx));
            return;
        }
        let tab = self.tabs.remove(ix);
        let loc = tab.read(cx).location.clone();
        if let (Some(dir), Some(w)) = (loc.dir(), self.watcher.as_mut()) {
            w.unwatch(dir);
        }
        self.closed.push(loc);
        let next = if self.active > ix || self.active >= self.tabs.len() {
            self.active.saturating_sub(1)
        } else {
            self.active
        };
        self.activate(next.min(self.tabs.len() - 1), window, cx);
    }

    pub fn reopen_closed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(loc) = self.closed.pop() {
            self.add_tab(loc, true, window, cx);
        }
    }

    fn save_session(&self, cx: &mut Context<Self>) {
        let tabs: Vec<Option<PathBuf>> = self
            .tabs
            .iter()
            .map(|t| t.read(cx).dir().map(Path::to_path_buf))
            .collect();
        let active = self.active;
        Store::update(cx, |s| {
            s.session.tabs = tabs;
            s.session.active = active;
        });
    }

    fn on_tab_event(
        &mut self,
        tab: &Entity<Tab>,
        event: &TabEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            TabEvent::Navigated { old, new } => {
                if let Some(w) = self.watcher.as_mut() {
                    if let Some(o) = old {
                        w.unwatch(o);
                    }
                    if let Some(n) = new {
                        w.watch(n);
                    }
                }
                if self.tabs.get(self.active) == Some(tab) {
                    self.end_path_edit(false, window, cx);
                    self.filter_field
                        .update(cx, |f, cx| f.set_text("", false, cx));
                    self.context_menu = None;
                }
                if new.is_none() && tab.read(cx).location == Location::Start {
                    refresh_start_data(cx);
                }
                self.save_session(cx);
                cx.notify();
            }
            TabEvent::OpenInTabs(dirs) => {
                // Keep the selection's order: a, b, c open as a, b, c.
                let first = self.active + 1;
                for (at, d) in (first..).zip(dirs.iter().cloned()) {
                    self.add_tab_at(Location::Dir(d), at, false, window, cx);
                }
            }
            TabEvent::ContextMenu { position, on_item } => {
                self.open_context_menu(*position, *on_item, window, cx);
            }
            TabEvent::Drop { paths, dest, copy } => {
                self.drop_files(paths.clone(), dest.clone(), *copy, window, cx);
            }
        }
    }

    fn reload_dirs(&mut self, dirs: &[PathBuf], cx: &mut Context<Self>) {
        for tab in &self.tabs {
            tab.update(cx, |t, cx| {
                let hit = t.dir().is_some_and(|d| dirs.iter().any(|x| x == d));
                // Columns view also caches ancestor/peek listings, including
                // subfolders that may just have been removed or renamed.
                t.column_cache
                    .retain(|k, _| !dirs.iter().any(|d| k.starts_with(d)));
                if hit {
                    t.reload(cx);
                }
                cx.notify();
            });
        }
        self.check_tab_dirs_exist(cx);
    }

    /// Deleting a folder is reported for its *parent* only: a tab showing the
    /// folder (or anything inside it) moves up to the nearest folder that
    /// still exists, like Finder. Every tab is checked (the reported parent
    /// may be a canonical path that doesn't prefix-match the tab's), off the
    /// UI thread, and one check at a time.
    fn check_tab_dirs_exist(&mut self, cx: &mut Context<Self>) {
        let shown: Vec<PathBuf> = self
            .tabs
            .iter()
            .filter_map(|t| t.read(cx).dir().map(Path::to_path_buf))
            .collect();
        for dir in shown {
            // One check per folder at a time: a folder on a hung mount blocks
            // only its own check (and doesn't pile up blocked threads), while
            // other tabs keep being checked.
            if !self.exists_inflight.insert(dir.clone()) {
                continue;
            }
            let check = cx.background_executor().spawn({
                let dir = dir.clone();
                async move {
                    if dir.is_dir() {
                        return None;
                    }
                    let mut up = dir.parent();
                    while let Some(p) = up
                        && !p.is_dir()
                    {
                        up = p.parent();
                    }
                    up.map(Path::to_path_buf)
                }
            });
            cx.spawn(async move |this, cx| {
                let up = check.await;
                let _ = this.update(cx, |ws, cx| {
                    ws.exists_inflight.remove(&dir);
                    let Some(up) = up else { return };
                    for tab in &ws.tabs {
                        tab.update(cx, |t, cx| {
                            if t.dir() == Some(dir.as_path()) {
                                t.navigate(Location::Dir(up.clone()), true, cx);
                            }
                        });
                    }
                });
            })
            .detach();
        }
    }

    // ---------------------------------------------------------------- navigation

    pub fn navigate_to(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let target = path.to_path_buf();
        let resolved = pathres::resolve(&target.to_string_lossy(), Path::new("/"));
        let tab = self.active_tab().clone();
        match resolved {
            Ok(Resolved::Dir(dir)) => {
                tab.update(cx, |t, cx| t.navigate(Location::Dir(dir), true, cx))
            }
            Ok(Resolved::File { parent, file }) => {
                tab.update(cx, |t, cx| t.reveal(parent, vec![file], cx))
            }
            Err(e) => self.show_toast(e, true, cx),
        }
        self.close_overlay(window, cx);
        window.focus(&tab.read(cx).focus.clone());
    }

    // ---------------------------------------------------------------- path bar

    pub fn begin_path_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tab = self.active_tab().read(cx);
        let text = match &tab.location {
            Location::Dir(d) => {
                let mut s = d.display().to_string();
                if !s.ends_with('/') {
                    s.push('/');
                }
                s
            }
            _ => format!("{}/", haven_core::home_dir().display()),
        };
        self.path_editing = true;
        self.path_error = None;
        self.completions.clear();
        self.completion_ix = None;
        self.path_field
            .update(cx, |f, cx| f.set_text(text, true, cx));
        window.focus(&self.path_field.focus_handle(cx));
        cx.notify();
    }

    pub fn end_path_edit(&mut self, refocus: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.path_editing {
            return;
        }
        self.path_editing = false;
        self.completions.clear();
        self.completion_ix = None;
        self.completion_task = None;
        self.path_error = None;
        // The path field is no longer rendered once editing ends, so never
        // leave focus on it (or on nothing): key bindings would stop firing.
        let stranded = match window.focused(cx) {
            None => true,
            Some(f) => f == self.path_field.focus_handle(cx),
        };
        if refocus || stranded {
            let focus = self.active_tab().read(cx).focus.clone();
            window.focus(&focus);
        }
        cx.notify();
    }

    fn cwd(&self, cx: &App) -> PathBuf {
        self.active_tab()
            .read(cx)
            .dir()
            .map(Path::to_path_buf)
            .unwrap_or_else(haven_core::home_dir)
    }

    fn on_path_event(
        &mut self,
        field: &Entity<TextField>,
        event: &TextFieldEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            TextFieldEvent::Changed => {
                self.path_error = None;
                self.completion_ix = None;
                let text = field.read(cx).text().to_string();
                let cwd = self.cwd(cx);
                let hidden = Store::get(cx).show_hidden;
                let task = cx
                    .background_executor()
                    .spawn(async move { complete::complete(&text, &cwd, hidden, 12) });
                self.completion_task = Some(cx.spawn(async move |this, cx| {
                    let c = task.await;
                    let _ = this.update(cx, |ws, cx| {
                        ws.completions = c;
                        cx.notify();
                    });
                }));
            }
            TextFieldEvent::Down => {
                if !self.completions.is_empty() {
                    self.completion_ix = Some(
                        self.completion_ix
                            .map(|i| (i + 1) % self.completions.len())
                            .unwrap_or(0),
                    );
                    cx.notify();
                }
            }
            TextFieldEvent::Up => {
                if !self.completions.is_empty() {
                    let n = self.completions.len();
                    self.completion_ix =
                        Some(self.completion_ix.map(|i| (i + n - 1) % n).unwrap_or(n - 1));
                    cx.notify();
                }
            }
            TextFieldEvent::Complete => {
                let current = field.read(cx).text().to_string();
                let replacement = match self.completion_ix {
                    Some(ix) => self.completions.get(ix).map(|c| c.replacement.clone()),
                    None => complete::tab_completion(&current, &self.completions),
                };
                if let Some(r) = replacement {
                    field.update(cx, |f, cx| {
                        f.set_text(r, false, cx);
                        cx.emit(TextFieldEvent::Changed);
                    });
                }
            }
            TextFieldEvent::Submit => {
                let text = match self.completion_ix.and_then(|i| self.completions.get(i)) {
                    Some(c) => c.replacement.clone(),
                    None => field.read(cx).text().to_string(),
                };
                let cwd = self.cwd(cx);
                match pathres::resolve(&text, &cwd) {
                    Ok(Resolved::Dir(dir)) => {
                        self.end_path_edit(true, window, cx);
                        self.active_tab()
                            .clone()
                            .update(cx, |t, cx| t.navigate(Location::Dir(dir), true, cx));
                    }
                    Ok(Resolved::File { parent, file }) => {
                        self.end_path_edit(true, window, cx);
                        self.active_tab()
                            .clone()
                            .update(cx, |t, cx| t.reveal(parent, vec![file], cx));
                    }
                    Err(e) => {
                        self.path_error = Some(e);
                        cx.notify();
                    }
                }
            }
            TextFieldEvent::Cancel => self.end_path_edit(true, window, cx),
            TextFieldEvent::Blur => {}
        }
    }

    fn on_filter_event(
        &mut self,
        field: &Entity<TextField>,
        event: &TextFieldEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tab = self.active_tab().clone();
        match event {
            TextFieldEvent::Changed => {
                let text = field.read(cx).text().to_string();
                tab.update(cx, |t, cx| t.set_filter(&text, cx));
            }
            TextFieldEvent::Submit | TextFieldEvent::Down => {
                window.focus(&tab.read(cx).focus.clone());
                if tab.read(cx).selection.is_empty() {
                    tab.update(cx, |t, cx| t.move_cursor(1, false, cx));
                }
            }
            TextFieldEvent::Cancel => {
                field.update(cx, |f, cx| f.set_text("", false, cx));
                tab.update(cx, |t, cx| t.set_filter("", cx));
                window.focus(&tab.read(cx).focus.clone());
            }
            _ => {}
        }
    }

    // ---------------------------------------------------------------- overlays

    pub fn push_undo(&mut self, undo: Undo) {
        self.undo_stack.push(undo);
    }

    pub fn close_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.overlay_subs.clear();
        let had_overlay = self.overlay.take().is_some();
        if had_overlay || self.context_menu.take().is_some() {
            let focus = self.active_tab().read(cx).focus.clone();
            window.focus(&focus);
            cx.notify();
        }
    }

    /// Ejects the volume mounted at `mount`. Plans first (in the background):
    /// when other volumes share its disk, asks before taking them too, like
    /// Finder. `confirmed` skips that question.
    pub fn eject(&mut self, mount: PathBuf, confirmed: bool, cx: &mut Context<Self>) {
        if !self.ejecting.insert(mount.clone()) {
            return;
        }
        cx.notify();
        let m = mount.clone();
        let plan = cx
            .background_executor()
            .spawn(async move { volumes::eject_plan(&m) });
        cx.spawn(async move |this, cx| {
            let plan = plan.await;
            let _ = this.update(cx, |ws, cx| {
                if plan.siblings.is_empty() || confirmed {
                    ws.run_eject(plan, cx);
                } else {
                    ws.ejecting.remove(&mount);
                    let others: Vec<String> =
                        plan.siblings.iter().map(|p| volume_name(p)).collect();
                    ws.overlay = Some(Overlay::Confirm {
                        title: format!(
                            "Eject “{}” and the other volumes on its disk?",
                            volume_name(&mount)
                        ),
                        body: format!(
                            "“{}” is on the same disk, so it will be ejected too.",
                            others.join("”, “")
                        ),
                        confirm: "Eject All".into(),
                        action: Box::new(crate::actions::EjectVolumeConfirmed(mount.clone())),
                    });
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Carries out a planned eject. Tabs showing any affected volume go to
    /// the start page first: a listing, watcher or size scan still open on it
    /// would keep it busy and make the eject fail.
    fn run_eject(&mut self, plan: volumes::EjectPlan, cx: &mut Context<Self>) {
        let mount = plan.mount.clone();
        let name = volume_name(&mount);
        let affected: Vec<PathBuf> = std::iter::once(mount.clone())
            .chain(plan.siblings.iter().cloned())
            .collect();
        for tab in self.tabs.clone() {
            let on_volume = {
                let t = tab.read(cx);
                affected.iter().any(|m| {
                    t.dir().is_some_and(|d| d.starts_with(m))
                        || t.column_root.as_ref().is_some_and(|r| r.starts_with(m))
                })
            };
            if on_volume {
                tab.update(cx, |t, cx| {
                    t.column_root = None;
                    t.navigate(Location::Start, false, cx)
                });
            }
        }
        // (Navigating emits `Navigated`, which unwatches the old folders.)
        self.show_toast(format!("Ejecting “{name}”…"), false, cx);
        let task = cx
            .background_executor()
            .spawn(async move { volumes::eject(&plan) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |ws, cx| {
                ws.ejecting.remove(&mount);
                match result {
                    Ok(volumes::EjectOutcome::Ejected) => {
                        ws.show_toast(format!("“{name}” can be safely removed"), false, cx)
                    }
                    Ok(volumes::EjectOutcome::Unmounted) => {
                        ws.show_toast(format!("“{name}” was unmounted"), false, cx)
                    }
                    Err(e) => ws.show_toast(format!("Couldn't eject “{name}”: {e}"), true, cx),
                }
                refresh_start_data(cx);
            });
        })
        .detach();
    }

    /// ⌘E: ejects the volume of the selected drive, or of the current folder.
    pub fn eject_current(&mut self, cx: &mut Context<Self>) {
        let vols = cx.global::<StartData>().volumes.clone();
        let tab = self.active_tab().read(cx);
        let target = tab
            .selected_paths()
            .into_iter()
            .chain(tab.dir().map(Path::to_path_buf))
            .find_map(|p| volumes::ejectable_volume_for(&p, &vols).map(|v| v.mount.clone()));
        match target {
            Some(mount) => self.eject(mount, false, cx),
            None => self.show_toast("Nothing to eject here", false, cx),
        }
    }

    pub fn show_toast(&mut self, message: impl Into<String>, error: bool, cx: &mut Context<Self>) {
        self.toast = Some(Toast {
            message: message.into(),
            error,
            at: Instant::now(),
        });
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(2600))
                .await;
            let _ = this.update(cx, |ws, cx| {
                if ws
                    .toast
                    .as_ref()
                    .is_some_and(|t| t.at.elapsed() >= Duration::from_millis(2500))
                {
                    ws.toast = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub fn open_context_menu(
        &mut self,
        position: Point<Pixels>,
        on_item: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tab = self.active_tab().read(cx);
        let selected: Vec<Entry> = tab.selected_entries().into_iter().cloned().collect();
        let has_clip = self.clipboard.is_some();
        let in_dir = tab.dir().is_some();
        let items = crate::overlays::build_context_menu(&selected, on_item, in_dir, has_clip, cx);
        self.context_menu = Some(ContextMenu { position, items });
        // Take focus so Esc reaches the workspace's Dismiss binding.
        window.focus(&self.focus);
        cx.notify();
    }

    pub fn open_menu_at(
        &mut self,
        position: Point<Pixels>,
        items: Vec<MenuEntry>,
        cx: &mut Context<Self>,
    ) {
        self.context_menu = Some(ContextMenu { position, items });
        cx.notify();
    }

    pub fn dispatch_menu_action(
        &mut self,
        action: Box<dyn Action>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.context_menu = None;
        let focus = self.active_tab().read(cx).focus.clone();
        window.focus(&focus);
        window.dispatch_action(action, cx);
        cx.notify();
    }

    // ---------------------------------------------------------------- file operations

    pub fn targets(&self, cx: &App) -> Vec<PathBuf> {
        self.active_tab().read(cx).selected_paths()
    }

    /// Finder drop semantics for every drop target (list, sidebar, tab strip,
    /// path bar): move within a volume, copy across volumes or with ⌥.
    pub fn drop_files(
        &mut self,
        paths: Vec<PathBuf>,
        dest: PathBuf,
        copy: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |this, cx| {
            let plan = cx
                .background_executor()
                .spawn(async move { plan_drop(paths, dest, copy) })
                .await;
            if let Some((kind, paths, dest)) = plan {
                let _ = this.update_in(cx, |ws, window, cx| {
                    ws.start_transfer(kind, paths, dest, window, cx)
                });
            }
        })
        .detach();
    }

    pub fn start_transfer(
        &mut self,
        kind: OpKind,
        sources: Vec<PathBuf>,
        dest: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if sources.is_empty() {
            return;
        }
        if kind == OpKind::Move && sources.iter().all(|s| s.parent() == Some(dest.as_path())) {
            return;
        }
        let conflicts = ops::conflicts(&sources, &dest);
        if !conflicts.is_empty() {
            self.overlay = Some(Overlay::Conflict {
                kind,
                sources,
                dest,
                count: conflicts.len(),
            });
            window.focus(&self.focus);
            cx.notify();
            return;
        }
        self.run_transfer(kind, sources, dest, Conflict::KeepBoth, cx);
    }

    pub fn run_transfer(
        &mut self,
        kind: OpKind,
        sources: Vec<PathBuf>,
        dest: PathBuf,
        policy: Conflict,
        cx: &mut Context<Self>,
    ) {
        if kind == OpKind::Move && matches!(&self.clipboard, Some((cut, true)) if *cut == sources) {
            self.clipboard = None;
        }
        let progress = Progress::new();
        let label = format!(
            "{} {} to “{}”",
            kind.verb(),
            if sources.len() == 1 {
                sources[0]
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            } else {
                format!("{} items", sources.len())
            },
            dest.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| dest.display().to_string())
        );
        let task = cx.background_executor().spawn({
            let progress = progress.clone();
            let dest = dest.clone();
            async move { ops::transfer(kind, &sources, &dest, policy, &progress) }
        });
        self.track_job(label, progress, task, cx);
    }

    fn track_job(
        &mut self,
        label: String,
        progress: Arc<Progress>,
        task: Task<ops::OpResult>,
        cx: &mut Context<Self>,
    ) {
        let id = self.next_job;
        self.next_job += 1;
        self.jobs.push(Job {
            id,
            label,
            progress: progress.clone(),
            done_at: None,
        });
        cx.notify();
        // Progress ticker.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                let finished = progress.finished.load(std::sync::atomic::Ordering::Relaxed);
                if this.update(cx, |_, cx| cx.notify()).is_err() || finished {
                    break;
                }
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |ws, cx| {
                if let Some(job) = ws.jobs.iter_mut().find(|j| j.id == id) {
                    job.done_at = Some(Instant::now());
                }
                let undo = match result {
                    Ok(undo) => undo,
                    Err(e) => {
                        ws.show_toast(e.to_string(), true, cx);
                        e.undo
                    }
                };
                // Select newly created items when they landed in the active tab's folder.
                let created: Vec<PathBuf> = match &undo {
                    Undo::Created(p) => p.clone(),
                    Undo::Batch(steps) => steps
                        .iter()
                        .filter_map(|s| match s {
                            Undo::Created(p) => Some(p.clone()),
                            _ => None,
                        })
                        .flatten()
                        .collect(),
                    _ => Vec::new(),
                };
                if !undo.is_empty() {
                    ws.undo_stack.push(undo);
                }
                ws.reload_all(cx);
                let tab = ws.active_tab().clone();
                let in_view = created.first().and_then(|p| p.parent()) == tab.read(cx).dir();
                if in_view {
                    tab.update(cx, |t, cx| t.reload_and_select(created, cx));
                }
                cx.notify();
            });
            cx.background_executor()
                .timer(Duration::from_millis(1800))
                .await;
            let _ = this.update(cx, |ws, cx| {
                ws.jobs.retain(|j| {
                    j.done_at
                        .is_none_or(|t| t.elapsed() < Duration::from_millis(1700))
                });
                cx.notify();
            });
        })
        .detach();
    }

    fn reload_all(&mut self, cx: &mut Context<Self>) {
        for tab in &self.tabs {
            tab.update(cx, |t, cx| t.reload(cx));
        }
    }

    pub fn cancel_job(&mut self, id: usize, cx: &mut Context<Self>) {
        if let Some(job) = self.jobs.iter().find(|j| j.id == id) {
            job.progress
                .cancel
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
        cx.notify();
    }

    pub fn trash_selected(&mut self, cx: &mut Context<Self>) {
        let paths = self.targets(cx);
        if paths.is_empty() {
            return;
        }
        let progress = Progress::new();
        let label = if paths.len() == 1 {
            format!(
                "Moving “{}” to Trash",
                paths[0]
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            )
        } else {
            format!("Moving {} items to Trash", paths.len())
        };
        // Keep the cursor near where the items were.
        let tab = self.active_tab().clone();
        let next = tab.read(cx).selection.iter().next().copied();
        let task = cx.background_executor().spawn({
            let progress = progress.clone();
            async move { ops::trash(&paths, &progress) }
        });
        self.track_job(label, progress, task, cx);
        if let Some(ix) = next {
            let survivor = tab
                .read(cx)
                .entries
                .iter()
                .enumerate()
                .find(|(i, _)| *i >= ix && !tab.read(cx).selection.contains(i))
                .map(|(_, e)| e.path.clone());
            if let Some(p) = survivor {
                tab.update(cx, |t, cx| {
                    let dir = t.dir().map(Path::to_path_buf);
                    if let Some(d) = dir {
                        t.reveal(d, vec![p], cx)
                    }
                });
            }
        }
    }

    pub fn undo(&mut self, cx: &mut Context<Self>) {
        // One undo at a time: they move files through temporary names and
        // must not interleave (a fast second ⌘Z would race the first).
        if self.undo_running {
            self.show_toast("Still undoing…", false, cx);
            return;
        }
        let Some(undo) = self.undo_stack.pop() else {
            self.show_toast("Nothing to undo", false, cx);
            return;
        };
        let label = undo.describe();
        let record = undo.clone();
        self.undo_running = true;
        let task = cx.background_executor().spawn(async move { undo.apply() });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |ws, cx| {
                ws.undo_running = false;
                match result {
                    Ok(_) => ws.show_toast(label, false, cx),
                    Err(e) => {
                        // A failed undo rolls itself back. Keep the record only
                        // when the user can fix the cause (something is in the
                        // way); otherwise (item gone, Trash emptied, volume
                        // ejected) it would block every older undo forever.
                        if e.kind() == std::io::ErrorKind::AlreadyExists {
                            ws.undo_stack.push(record);
                        }
                        ws.show_toast(format!("Undo failed: {e}"), true, cx)
                    }
                }
                ws.reload_all(cx);
            });
        })
        .detach();
    }

    pub fn paste(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((paths, cut)) = self.clipboard.clone() else {
            return;
        };
        let Some(dest) = self.active_tab().read(cx).dir().map(Path::to_path_buf) else {
            return;
        };
        // A cut is consumed when the move actually runs (see run_transfer),
        // so cancelling the conflict dialog keeps it for another try.
        self.start_transfer(
            if cut { OpKind::Move } else { OpKind::Copy },
            paths,
            dest,
            window,
            cx,
        );
    }

    pub fn copy_files(&mut self, cut: bool, cx: &mut Context<Self>) {
        let paths = self.targets(cx);
        if paths.is_empty() {
            return;
        }
        let text = paths
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        let n = paths.len();
        self.clipboard = Some((paths, cut));
        self.show_toast(
            format!(
                "{} {} item{}",
                if cut { "Cut" } else { "Copied" },
                n,
                if n == 1 { "" } else { "s" }
            ),
            false,
            cx,
        );
    }

    pub fn copy_path(&mut self, cx: &mut Context<Self>) {
        let mut paths = self.targets(cx);
        if paths.is_empty() {
            paths.push(self.cwd(cx));
        }
        let text = paths
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
        self.show_toast(
            format!(
                "Copied {}",
                if paths.len() == 1 {
                    tildify(&paths[0])
                } else {
                    format!("{} paths", paths.len())
                }
            ),
            false,
            cx,
        );
    }

    pub fn duplicate(&mut self, cx: &mut Context<Self>) {
        let paths = self.targets(cx);
        let Some(dest) = self.active_tab().read(cx).dir().map(Path::to_path_buf) else {
            return;
        };
        if !paths.is_empty() {
            self.run_transfer(OpKind::Copy, paths, dest, Conflict::KeepBoth, cx);
        }
    }

    pub fn new_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dir) = self.active_tab().read(cx).dir().map(Path::to_path_buf) else {
            return;
        };
        match ops::new_folder(&dir) {
            Ok(path) => {
                self.undo_stack.push(Undo::Created(vec![path.clone()]));
                let tab = self.active_tab().clone();
                tab.update(cx, |t, cx| t.reload_and_select(vec![path.clone()], cx));
                self.open_rename(Some(path), window, cx);
            }
            Err(e) => self.show_toast(e.to_string(), true, cx),
        }
    }

    pub fn open_rename(
        &mut self,
        path: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = match path {
            Some(p) => p,
            None => {
                let targets = self.targets(cx);
                match targets.len() {
                    0 => return,
                    1 => targets[0].clone(),
                    _ => return self.open_bulk_rename(window, cx),
                }
            }
        };
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let is_dir = path.is_dir();
        let field = cx.new(|cx| {
            let mut f = TextField::new("Name", cx);
            f.set_text(name, true, cx);
            if !is_dir {
                f.select_stem(cx);
            }
            f
        });
        self.subscribe_overlay_field(&field, window, cx);
        window.focus(&field.focus_handle(cx));
        self.overlay = Some(Overlay::Rename {
            field,
            path,
            error: None,
        });
        cx.notify();
    }

    pub fn open_bulk_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = self.targets(cx);
        if paths.len() < 2 {
            return self.open_rename(None, window, cx);
        }
        let overlay = crate::overlays::BulkRenameState::new(paths, window, cx);
        for f in overlay.fields.clone() {
            self.subscribe_overlay_field(&f, window, cx);
        }
        self.overlay = Some(Overlay::BulkRename(overlay));
        cx.notify();
    }

    pub fn subscribe_overlay_field(
        &mut self,
        field: &Entity<TextField>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let sub = cx.subscribe_in(field, window, |ws, _, ev: &TextFieldEvent, window, cx| {
            ws.on_overlay_event(ev, window, cx)
        });
        self.overlay_subs.push(sub);
    }

    pub fn submit_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Overlay::Rename { field, path, .. }) = &self.overlay else {
            return;
        };
        let new_name = field.read(cx).text().trim().to_string();
        let path = path.clone();
        match ops::rename(&path, &new_name) {
            Ok(undo) => {
                if let Undo::Moved(pairs) = &undo
                    && let Some((_, to)) = pairs.first()
                {
                    let to = to.clone();
                    let tab = self.active_tab().clone();
                    tab.update(cx, |t, cx| t.reload_and_select(vec![to], cx));
                }
                self.undo_stack.push(undo);
                self.close_overlay(window, cx);
            }
            Err(e) => {
                if let Some(Overlay::Rename { error, .. }) = &mut self.overlay {
                    *error = Some(e.to_string());
                }
                cx.notify();
            }
        }
    }

    pub fn apply_tag(&mut self, tag: Tag, on: bool, cx: &mut Context<Self>) {
        let paths = self.targets(cx);
        if paths.is_empty() {
            return;
        }
        Store::update(cx, |s| {
            s.learn_tag(&tag);
        });
        // Off the UI thread, in order (see `tag_tx`).
        // Count before sending: the worker may finish (and decrement)
        // before `try_send` even returns.
        PENDING_TAG_WRITES.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if self.tag_tx.try_send((paths, tag, on)).is_err() {
            PENDING_TAG_WRITES.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    pub fn toggle_tag_on_selection(&mut self, name: &str, cx: &mut Context<Self>) {
        let selected: Vec<Entry> = self
            .active_tab()
            .read(cx)
            .selected_entries()
            .into_iter()
            .cloned()
            .collect();
        if selected.is_empty() {
            return;
        }
        let all_have = selected
            .iter()
            .all(|e| e.tags.iter().any(|t| t.name.eq_ignore_ascii_case(name)));
        let color = Store::get(cx).tag_color(name).unwrap_or_default();
        self.apply_tag(Tag::new(name, color), !all_have, cx);
    }

    pub fn pin_selection(&mut self, sidebar: bool, cx: &mut Context<Self>) {
        let tab = self.active_tab().read(cx);
        let mut paths: Vec<PathBuf> = tab
            .selected_entries()
            .into_iter()
            .filter(|e| !sidebar || e.is_dir)
            .map(|e| e.path.clone())
            .collect();
        if paths.is_empty()
            && let Some(d) = tab.dir()
        {
            paths.push(d.to_path_buf());
        }
        Store::update(cx, |s| {
            for p in &paths {
                if sidebar {
                    s.toggle_sidebar_pin(p)
                } else {
                    s.toggle_start_pin(p)
                }
            }
        });
        refresh_pins(cx);
        cx.notify();
    }

    pub fn open_in_terminal(&mut self, cx: &mut Context<Self>) {
        let tab = self.active_tab().read(cx);
        let dir = tab
            .cursor_entry()
            .filter(|e| e.is_navigable() && tab.selection.len() == 1)
            .map(|e| e.path.clone())
            .or_else(|| tab.dir().map(Path::to_path_buf))
            .unwrap_or_else(haven_core::home_dir);
        let app = Store::get(cx).terminal_app.clone();
        if let Err(e) = std::process::Command::new("/usr/bin/open")
            .arg("-a")
            .arg(&app)
            .arg(&dir)
            .spawn()
        {
            self.show_toast(format!("Couldn't open {app}: {e}"), true, cx);
        }
    }

    pub fn open_in_editor(&mut self, cx: &mut Context<Self>) {
        let mut paths = self.targets(cx);
        if paths.is_empty() {
            paths.push(self.cwd(cx));
        }
        let cmd = Store::get(cx).editor_command.clone();
        let mut command = std::process::Command::new("/bin/zsh");
        command.arg("-lc").arg(format!("{cmd} \"$@\"")).arg("haven");
        command.args(&paths);
        if let Err(e) = command.spawn() {
            self.show_toast(format!("Couldn't run {cmd}: {e}"), true, cx);
        }
    }

    pub fn set_default_viewer(&mut self, enable: bool, cx: &mut Context<Self>) {
        let task = cx
            .background_executor()
            .spawn(async move { haven_core::system::set_default_viewer(enable) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |ws, cx| match result {
                Ok(()) if enable => {
                    ws.show_toast("Haven is now the default file viewer", false, cx)
                }
                Ok(()) => ws.show_toast("Finder restored as the default file viewer", false, cx),
                Err(e) => ws.show_toast(e, true, cx),
            });
        })
        .detach();
    }

    pub fn on_overlay_event(
        &mut self,
        ev: &TextFieldEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        crate::overlays::handle_overlay_event(self, ev, window, cx);
    }
}

impl Focusable for Workspace {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// What a drop of `paths` onto `dest` should do, or `None` to ignore it.
/// Dropping a folder onto itself or onto a folder inside it (its tab, a
/// sidebar row, a crumb, a row in its own listing) does nothing, whichever
/// way either path is spelled. Touches the filesystem: run it off the UI
/// thread.
fn plan_drop(
    paths: Vec<PathBuf>,
    dest: PathBuf,
    copy: bool,
) -> Option<(OpKind, Vec<PathBuf>, PathBuf)> {
    if paths.is_empty() || ops::source_containing(&paths, &dest).is_some() {
        return None;
    }
    let kind = if copy || !same_volume(&paths, &dest) {
        OpKind::Copy
    } else {
        OpKind::Move
    };
    Some((kind, paths, dest))
}

fn same_volume(paths: &[PathBuf], dest: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Ok(d) = std::fs::metadata(dest) else {
        return true;
    };
    paths.iter().all(|p| {
        std::fs::symlink_metadata(p)
            .map(|m| m.dev() == d.dev())
            .unwrap_or(true)
    })
}

/// Reloads drives and recents in the background.
/// Tag writes queued or in flight (see `Workspace::tag_tx`).
static PENDING_TAG_WRITES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Called on Quit: give queued tag writes up to 2 s to reach the disk.
pub fn wait_for_tag_writes() {
    let deadline = Instant::now() + Duration::from_secs(2);
    while PENDING_TAG_WRITES.load(std::sync::atomic::Ordering::SeqCst) > 0
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn volume_name(mount: &Path) -> String {
    mount
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| mount.display().to_string())
}

pub fn refresh_start_data(cx: &mut App) {
    let fresh = cx
        .global::<StartData>()
        .recents_loaded
        .is_some_and(|t| t.elapsed() < Duration::from_secs(20));
    let history = Store::get(cx).history_recents();
    let task = cx.background_executor().spawn(async move {
        let vols = volumes::list_volumes();
        let places = crate::workspace_view::places();
        let rec = if fresh {
            None
        } else {
            let merged = recents::merge(history, recents::spotlight_recents(14), 40);
            let entries: Vec<Option<Entry>> = merged
                .iter()
                .map(|r| Entry::from_path(&r.path).ok())
                .collect();
            Some((merged, entries))
        };
        (vols, rec, places)
    });
    cx.spawn(async move |cx| {
        let (vols, rec, places) = task.await;
        let _ = cx.update(|cx| {
            let data = cx.global_mut::<StartData>();
            data.volumes = vols;
            data.places = Some(places);
            if let Some((r, entries)) = rec {
                data.recents = r;
                data.recent_entries = entries;
                data.recents_loaded = Some(Instant::now());
            }
        });
    })
    .detach();
}

/// Re-reads metadata for start-page pins.
pub fn refresh_pins(cx: &mut App) {
    let pins = Store::get(cx).start_pins.clone();
    let task = cx.background_executor().spawn(async move {
        pins.iter()
            .filter_map(|p| Entry::from_path(p).ok())
            .collect::<Vec<_>>()
    });
    cx.spawn(async move |cx| {
        let entries = task.await;
        let _ = cx.update(|cx| cx.global_mut::<StartData>().pinned = entries);
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{TestAppContext, VisualTestContext};

    fn init(cx: &mut TestAppContext) {
        cx.update(|cx| {
            crate::store::init_test_env();
            crate::actions::bind_keys(cx);
            crate::text_field::bind_keys(cx);
            cx.set_global(crate::theme::Theme::dark());
            Store::init(cx);
            crate::icons::IconCache::init(cx);
            cx.set_global(StartData::default());
        });
    }

    fn fixture() -> PathBuf {
        let root = std::fs::canonicalize(tempfile::tempdir().unwrap().keep()).unwrap();
        std::fs::create_dir_all(root.join("projects/haven")).unwrap();
        std::fs::write(root.join("projects/readme.md"), b"# readme").unwrap();
        root
    }

    fn open(cx: &mut TestAppContext, loc: Location) -> (Entity<Workspace>, &mut VisualTestContext) {
        let (ws, vcx) = cx.add_window_view(|window, cx| Workspace::new(vec![loc], 0, window, cx));
        vcx.run_until_parked();
        (ws, vcx)
    }

    fn submit_path(ws: &Entity<Workspace>, text: &str, cx: &mut VisualTestContext) {
        ws.update_in(cx, |ws, window, cx| {
            ws.begin_path_edit(window, cx);
            let field = ws.path_field.clone();
            field.update(cx, |f, cx| {
                f.set_text(text.to_string(), false, cx);
                cx.emit(TextFieldEvent::Submit);
            });
        });
        cx.run_until_parked();
    }

    #[gpui::test]
    async fn path_bar_resolves_dirs_files_relative_and_errors(cx: &mut TestAppContext) {
        init(cx);
        let root = fixture();
        let (ws, cx) = open(cx, Location::Dir(root.clone()));

        submit_path(&ws, "projects/haven", cx);
        ws.read_with(cx, |ws, cx| {
            assert_eq!(
                ws.active_tab().read(cx).dir(),
                Some(root.join("projects/haven").as_path())
            );
            assert!(!ws.path_editing);
        });

        submit_path(&ws, "../readme.md", cx);
        ws.read_with(cx, |ws, cx| {
            let tab = ws.active_tab().read(cx);
            assert_eq!(tab.dir(), Some(root.join("projects").as_path()));
            assert_eq!(
                tab.cursor_entry().map(|e| e.name.clone()).as_deref(),
                Some("readme.md")
            );
        });

        submit_path(&ws, "does/not/exist", cx);
        ws.read_with(cx, |ws, _| {
            assert!(ws.path_editing);
            assert!(
                ws.path_error
                    .as_deref()
                    .unwrap_or("")
                    .starts_with("No such file")
            );
        });
    }

    #[gpui::test]
    async fn tabs_open_close_and_reopen(cx: &mut TestAppContext) {
        init(cx);
        let root = fixture();
        let (ws, cx) = open(cx, Location::Dir(root.clone()));
        ws.update_in(cx, |ws, window, cx| {
            ws.add_tab(Location::Dir(root.join("projects")), true, window, cx);
            assert_eq!(ws.tabs.len(), 2);
            assert_eq!(ws.active, 1);
            ws.close_tab(1, window, cx);
            assert_eq!(ws.tabs.len(), 1);
            ws.reopen_closed(window, cx);
            assert_eq!(ws.tabs.len(), 2);
            assert_eq!(
                ws.active_tab().read(cx).dir(),
                Some(root.join("projects").as_path())
            );
        });
        cx.run_until_parked();
        let session = cx.update(|_, cx| Store::get(cx).session.clone());
        assert_eq!(session.tabs.len(), 2);
    }

    #[gpui::test]
    async fn new_folder_rename_and_undo(cx: &mut TestAppContext) {
        init(cx);
        let root = fixture();
        let (ws, cx) = open(cx, Location::Dir(root.clone()));
        ws.update_in(cx, |ws, window, cx| ws.new_folder(window, cx));
        cx.run_until_parked();
        assert!(root.join("untitled folder").is_dir());
        ws.update_in(cx, |ws, window, cx| {
            let Some(Overlay::Rename { field, .. }) = &ws.overlay else {
                panic!("rename overlay")
            };
            field
                .clone()
                .update(cx, |f, cx| f.set_text("assets", false, cx));
            ws.submit_rename(window, cx);
            assert!(ws.overlay.is_none());
        });
        cx.run_until_parked();
        assert!(root.join("assets").is_dir() && !root.join("untitled folder").exists());
        ws.update_in(cx, |ws, _, cx| ws.undo(cx));
        cx.run_until_parked();
        assert!(root.join("untitled folder").is_dir());
    }

    #[gpui::test]
    async fn initial_tab_events_reach_the_workspace(cx: &mut TestAppContext) {
        init(cx);
        let root = fixture();
        let (ws, cx) = open(cx, Location::Dir(root.clone()));
        // Context menu from the tab created by `Workspace::new`.
        ws.update_in(cx, |ws, window, cx| {
            let tab = ws.active_tab().clone();
            tab.update(cx, |t, cx| {
                t.context_item(0, gpui::point(gpui::px(10.), gpui::px(10.)), window, cx)
            });
        });
        cx.run_until_parked();
        ws.read_with(cx, |ws, _| {
            assert!(ws.context_menu.is_some(), "context menu opened")
        });
        // Esc (Dismiss) closes it and refocuses the tab.
        ws.update_in(cx, |ws, window, cx| ws.close_overlay(window, cx));
        ws.read_with(cx, |ws, _| assert!(ws.context_menu.is_none()));
        // Navigation events update the persisted session.
        ws.update_in(cx, |ws, _, cx| {
            let tab = ws.active_tab().clone();
            tab.update(cx, |t, cx| {
                t.navigate(Location::Dir(root.join("projects")), true, cx)
            });
        });
        cx.run_until_parked();
        let session = cx.update(|_, cx| Store::get(cx).session.clone());
        assert_eq!(session.tabs, vec![Some(root.join("projects"))]);
    }

    #[gpui::test]
    async fn path_bar_tab_completes_prefix_match(cx: &mut TestAppContext) {
        init(cx);
        let root = fixture();
        std::fs::create_dir(root.join("tools")).unwrap();
        std::fs::create_dir(root.join("lpturmel-portfolio")).unwrap();
        let (ws, cx) = open(cx, Location::Dir(root.clone()));
        let input = format!("{}/too", root.display());
        ws.update_in(cx, |ws, window, cx| {
            ws.begin_path_edit(window, cx);
            let field = ws.path_field.clone();
            field.update(cx, |f, cx| {
                f.set_text(input.clone(), false, cx);
                cx.emit(TextFieldEvent::Changed);
            });
        });
        cx.run_until_parked();
        ws.update_in(cx, |ws, _, cx| {
            assert!(
                ws.completions.iter().any(|c| c.rank > 0),
                "fuzzy candidates listed"
            );
            ws.path_field
                .clone()
                .update(cx, |_, cx| cx.emit(TextFieldEvent::Complete));
        });
        cx.run_until_parked();
        ws.read_with(cx, |ws, cx| {
            assert_eq!(
                ws.path_field.read(cx).text(),
                format!("{}/tools/", root.display())
            );
        });
    }

    #[gpui::test]
    async fn trash_then_undo_and_multi_key_sort(cx: &mut TestAppContext) {
        init(cx);
        let root = fixture();
        std::fs::write(root.join("b.txt"), b"bb").unwrap();
        std::fs::write(root.join("a.txt"), b"bb").unwrap();
        let (ws, cx) = open(cx, Location::Dir(root.clone()));
        ws.update_in(cx, |ws, _, cx| {
            ws.active_tab().clone().update(cx, |t, cx| {
                t.click_sort(haven_core::SortKey::Size, false, cx);
                t.click_sort(haven_core::SortKey::Name, true, cx);
                assert_eq!(t.sort.keys.len(), 2);
                let files: Vec<_> = t
                    .entries
                    .iter()
                    .filter(|e| !e.is_dir)
                    .map(|e| e.name.clone())
                    .collect();
                assert_eq!(files, ["a.txt", "b.txt"], "size tie broken by name");
                let ix = t.entries.iter().position(|e| e.name == "b.txt").unwrap();
                t.select_only(ix, cx);
            });
        });
        ws.update_in(cx, |ws, _, cx| ws.trash_selected(cx));
        cx.run_until_parked();
        if root.join("b.txt").exists() {
            return; // No Trash on this volume (sandboxed CI).
        }
        ws.update_in(cx, |ws, _, cx| ws.undo(cx));
        cx.run_until_parked();
        assert!(
            root.join("b.txt").exists(),
            "undo restored the trashed file"
        );
    }

    fn active_tab_focused(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> bool {
        ws.update_in(cx, |ws, window, cx| {
            ws.active_tab().read(cx).focus.is_focused(window)
        })
    }

    #[gpui::test]
    async fn escape_key_closes_context_menu_not_selection(cx: &mut TestAppContext) {
        init(cx);
        let root = fixture();
        let (ws, cx) = open(cx, Location::Dir(root.clone()));
        let tab = ws.read_with(cx, |ws, _| ws.active_tab().clone());
        tab.update_in(cx, |t, window, cx| {
            t.context_item(0, gpui::point(gpui::px(80.), gpui::px(120.)), window, cx)
        });
        cx.run_until_parked();
        ws.read_with(cx, |ws, _| assert!(ws.context_menu.is_some()));
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        ws.read_with(cx, |ws, _| assert!(ws.context_menu.is_none()));
        // Esc went to the menu, not to the list's ClearSelection.
        assert_eq!(tab.read_with(cx, |t, _| t.selection.len()), 1);
        assert!(active_tab_focused(&ws, cx));
    }

    #[gpui::test]
    async fn restored_active_tab_gets_focus(cx: &mut TestAppContext) {
        init(cx);
        let root = fixture();
        let locs = vec![
            Location::Dir(root.clone()),
            Location::Dir(root.join("projects")),
        ];
        let (ws, cx) = cx.add_window_view(|window, cx| Workspace::new(locs, 1, window, cx));
        cx.run_until_parked();
        assert_eq!(ws.read_with(cx, |ws, _| ws.active), 1);
        assert!(active_tab_focused(&ws, cx));
    }

    #[gpui::test]
    async fn ending_path_edit_never_strands_focus(cx: &mut TestAppContext) {
        init(cx);
        let root = fixture();
        let (ws, cx) = open(cx, Location::Dir(root.clone()));
        ws.update_in(cx, |ws, window, cx| {
            ws.begin_path_edit(window, cx);
            assert!(ws.path_field.focus_handle(cx).is_focused(window));
            ws.end_path_edit(false, window, cx);
        });
        assert!(active_tab_focused(&ws, cx));
        // Shortcuts still fire: Cmd-L re-enters path editing.
        cx.simulate_keystrokes("cmd-l");
        cx.run_until_parked();
        assert!(ws.read_with(cx, |ws, _| ws.path_editing));
    }

    #[gpui::test]
    async fn moving_focus_to_filter_while_editing_path_keeps_it(cx: &mut TestAppContext) {
        init(cx);
        let root = fixture();
        let (ws, cx) = open(cx, Location::Dir(root.clone()));
        // Focus listeners compare the focus of consecutive drawn frames (of an
        // active window), so activate and draw after each step like the app.
        cx.update(|window, _| window.activate_window());
        draw(cx);
        ws.update_in(cx, |ws, window, cx| ws.begin_path_edit(window, cx));
        draw(cx);
        cx.simulate_keystrokes("cmd-f");
        draw(cx);
        ws.update_in(cx, |ws, window, cx| {
            assert!(!ws.path_editing, "leaving the path field ends editing");
            assert!(
                ws.filter_field
                    .read(cx)
                    .focus_handle_ref()
                    .is_focused(window),
                "the filter keeps the focus it was given"
            );
        });
    }

    fn draw(cx: &mut VisualTestContext) {
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.run_until_parked();
    }

    #[gpui::test]
    async fn path_edit_survives_window_deactivation(cx: &mut TestAppContext) {
        init(cx);
        let root = fixture();
        let (ws, cx) = open(cx, Location::Dir(root.clone()));
        cx.update(|window, _| window.activate_window());
        draw(cx);
        ws.update_in(cx, |ws, window, cx| ws.begin_path_edit(window, cx));
        draw(cx);
        // Another app takes the front, then Haven comes back.
        cx.deactivate_window();
        draw(cx);
        assert!(ws.read_with(cx, |ws, _| ws.path_editing));
        cx.update(|window, _| window.activate_window());
        draw(cx);
        ws.update_in(cx, |ws, window, cx| {
            assert!(ws.path_editing);
            assert!(ws.path_field.focus_handle(cx).is_focused(window));
        });
        // Typing still works: Escape cancels and focus returns to the files.
        cx.simulate_keystrokes("escape");
        draw(cx);
        assert!(!ws.read_with(cx, |ws, _| ws.path_editing));
        assert!(active_tab_focused(&ws, cx));
    }

    fn tab_dirs(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> Vec<PathBuf> {
        ws.read_with(cx, |ws, cx| {
            ws.tabs
                .iter()
                .map(|t| t.read(cx).dir().unwrap().to_path_buf())
                .collect()
        })
    }

    #[gpui::test]
    async fn session_tabs_restore_in_order_and_open_in_tabs_keeps_order(cx: &mut TestAppContext) {
        init(cx);
        let root = fixture();
        for d in ["a", "b", "c", "x", "y", "z"] {
            std::fs::create_dir(root.join(d)).unwrap();
        }
        let locs: Vec<Location> = ["a", "b", "c"]
            .iter()
            .map(|d| Location::Dir(root.join(d)))
            .collect();
        let (ws, cx) = cx.add_window_view(|window, cx| Workspace::new(locs, 1, window, cx));
        cx.run_until_parked();
        assert_eq!(
            tab_dirs(&ws, cx),
            vec![root.join("a"), root.join("b"), root.join("c")]
        );
        assert_eq!(ws.read_with(cx, |ws, _| ws.active), 1);
        // "Open in tabs" from tab b: x, y, z land right after it, in order.
        let tab = ws.read_with(cx, |ws, _| ws.active_tab().clone());
        tab.update(cx, |_, cx| {
            cx.emit(TabEvent::OpenInTabs(vec![
                root.join("x"),
                root.join("y"),
                root.join("z"),
            ]))
        });
        cx.run_until_parked();
        let names: Vec<String> = tab_dirs(&ws, cx)
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["a", "b", "x", "y", "z", "c"]);
        assert_eq!(ws.read_with(cx, |ws, _| ws.active), 1);
    }

    #[gpui::test]
    async fn cmd_z_in_a_text_field_never_undoes_file_operations(cx: &mut TestAppContext) {
        init(cx);
        let root = fixture();
        let (ws, cx) = open(cx, Location::Dir(root.clone()));
        std::fs::write(root.join("made.txt"), b"x").unwrap();
        ws.update(cx, |ws, _| {
            ws.undo_stack
                .push(Undo::Created(vec![root.join("made.txt")]))
        });
        cx.simulate_keystrokes("cmd-f n cmd-z");
        cx.run_until_parked();
        assert!(root.join("made.txt").exists(), "file undo must not run");
        assert_eq!(ws.read_with(cx, |ws, _| ws.undo_stack.len()), 1);
        let filter = ws.read_with(cx, |ws, cx| ws.filter_field.read(cx).text().to_string());
        assert_eq!(filter, "", "⌘Z undid the typing instead");
    }

    #[gpui::test]
    async fn cancelled_conflict_keeps_the_cut(cx: &mut TestAppContext) {
        init(cx);
        let root = fixture();
        std::fs::create_dir(root.join("dst")).unwrap();
        std::fs::write(root.join("dst/readme.md"), b"other").unwrap();
        let (ws, cx) = open(cx, Location::Dir(root.join("dst")));
        let cut = vec![root.join("projects/readme.md")];
        ws.update_in(cx, |ws, window, cx| {
            ws.clipboard = Some((cut.clone(), true));
            ws.paste(window, cx);
            assert!(matches!(ws.overlay, Some(Overlay::Conflict { .. })));
            ws.close_overlay(window, cx);
        });
        assert_eq!(
            ws.read_with(cx, |ws, _| ws.clipboard.clone()),
            Some((cut, true))
        );
        assert!(root.join("projects/readme.md").exists());
    }

    #[gpui::test]
    async fn space_continues_type_to_select(cx: &mut TestAppContext) {
        init(cx);
        let root = fixture();
        std::fs::write(root.join("my file.txt"), b"").unwrap();
        std::fs::write(root.join("my-notes.txt"), b"").unwrap();
        let (ws, cx) = open(cx, Location::Dir(root.clone()));
        let before = cx.update(|_, cx| Store::get(cx).preview_visible);
        cx.simulate_keystrokes("m y space f");
        cx.run_until_parked();
        let name = ws.read_with(cx, |ws, cx| {
            ws.active_tab()
                .read(cx)
                .cursor_entry()
                .map(|e| e.name.clone())
        });
        assert_eq!(name.as_deref(), Some("my file.txt"));
        assert_eq!(cx.update(|_, cx| Store::get(cx).preview_visible), before);
        // ⌥⌘P right after typing still toggles (it isn't the Space key).
        cx.simulate_keystrokes("m cmd-alt-p");
        assert_eq!(cx.update(|_, cx| Store::get(cx).preview_visible), !before);
        // A plain Space (no type-ahead in progress) toggles too.
        // (type-ahead expires on the real clock, not the test executor's)
        std::thread::sleep(std::time::Duration::from_millis(850));
        cx.simulate_keystrokes("space");
        assert_eq!(cx.update(|_, cx| Store::get(cx).preview_visible), before);
    }

    #[gpui::test]
    async fn quick_successive_tag_changes_all_apply(cx: &mut TestAppContext) {
        init(cx);
        let root = fixture();
        let f = root.join("projects/readme.md");
        let (ws, cx) = open(cx, Location::Dir(root.join("projects")));
        let tab = ws.read_with(cx, |ws, _| ws.active_tab().clone());
        tab.update(cx, |t, cx| {
            let ix = t
                .entries
                .iter()
                .position(|e| e.name == "readme.md")
                .unwrap();
            t.select_only(ix, cx);
        });
        ws.update(cx, |ws, cx| {
            ws.apply_tag(Tag::new("Red", haven_core::TagColor::Red), true, cx);
            ws.apply_tag(Tag::new("Blue", haven_core::TagColor::Blue), true, cx);
            ws.apply_tag(Tag::new("Red", haven_core::TagColor::Red), false, cx);
            ws.apply_tag(Tag::new("Green", haven_core::TagColor::Green), true, cx);
        });
        cx.run_until_parked();
        let names: Vec<String> = haven_core::tags::read_tags(&f)
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert_eq!(names, ["Blue", "Green"]);
    }

    #[gpui::test]
    async fn failed_undo_keeps_only_fixable_records(cx: &mut TestAppContext) {
        init(cx);
        let root = fixture();
        let (ws, cx) = open(cx, Location::Dir(root.clone()));
        // Unfixable: the created item is already gone. Dropped, so older
        // undos stay reachable.
        ws.update(cx, |ws, cx| {
            ws.undo_stack
                .push(Undo::Created(vec![root.join("gone.txt")]));
            ws.undo(cx);
        });
        cx.run_until_parked();
        assert_eq!(ws.read_with(cx, |ws, _| ws.undo_stack.len()), 0);
        // Fixable: something is in the way of the restore. Kept for a retry.
        std::fs::write(root.join("in-the-way.txt"), b"x").unwrap();
        std::fs::write(root.join("landed.txt"), b"y").unwrap();
        ws.update(cx, |ws, cx| {
            ws.undo_stack.push(Undo::Trashed(vec![(
                root.join("in-the-way.txt"),
                root.join("landed.txt"),
            )]));
            ws.undo(cx);
            // A second ⌘Z while the first runs is refused, not interleaved.
            ws.undo(cx);
        });
        cx.run_until_parked();
        assert_eq!(ws.read_with(cx, |ws, _| ws.undo_stack.len()), 1);
    }

    #[gpui::test]
    async fn deleting_the_shown_folder_moves_the_tab_up(cx: &mut TestAppContext) {
        init(cx);
        let root = fixture();
        let (ws, cx) = open(cx, Location::Dir(root.join("projects/haven")));
        std::fs::remove_dir_all(root.join("projects/haven")).unwrap();
        // The watcher reports the parent of a deleted folder.
        ws.update(cx, |ws, cx| ws.reload_dirs(&[root.join("projects")], cx));
        cx.run_until_parked();
        let dir = ws.read_with(cx, |ws, cx| {
            ws.active_tab().read(cx).dir().unwrap().to_path_buf()
        });
        assert_eq!(dir, root.join("projects"));
    }

    #[gpui::test]
    async fn conflict_dialog_takes_enter_and_blocks_the_palette(cx: &mut TestAppContext) {
        init(cx);
        let root = fixture();
        std::fs::create_dir(root.join("dst")).unwrap();
        std::fs::write(root.join("dst/readme.md"), b"other").unwrap();
        let (ws, cx) = open(cx, Location::Dir(root.join("dst")));
        ws.update_in(cx, |ws, window, cx| {
            ws.clipboard = Some((vec![root.join("projects/readme.md")], false));
            ws.paste(window, cx);
        });
        cx.run_until_parked();
        cx.simulate_keystrokes("cmd-k cmd-p");
        ws.read_with(cx, |ws, _| {
            assert!(matches!(ws.overlay, Some(Overlay::Conflict { .. })))
        });
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        ws.read_with(cx, |ws, _| assert!(ws.overlay.is_none()));
        // Enter = Keep Both.
        assert_eq!(std::fs::read(root.join("dst/readme.md")).unwrap(), b"other");
        assert_eq!(
            std::fs::read(root.join("dst/readme copy.md")).unwrap(),
            b"# readme"
        );
    }

    #[gpui::test]
    async fn tag_editor_keeps_an_existing_tags_colour(cx: &mut TestAppContext) {
        init(cx);
        let root = fixture();
        let (ws, cx) = open(cx, Location::Dir(root.join("projects")));
        let tab = ws.read_with(cx, |ws, _| ws.active_tab().clone());
        tab.update(cx, |t, cx| {
            let ix = t
                .entries
                .iter()
                .position(|e| e.name == "readme.md")
                .unwrap();
            t.select_only(ix, cx);
        });
        ws.update_in(cx, |ws, window, cx| {
            crate::overlays::open_tags(ws, window, cx)
        });
        cx.simulate_keystrokes("r e d enter");
        cx.run_until_parked();
        let tags = haven_core::tags::read_tags(&root.join("projects/readme.md"));
        assert_eq!(tags, vec![Tag::new("Red", haven_core::TagColor::Red)]);
    }

    #[gpui::test]
    async fn drops_move_within_a_volume_and_copy_with_option(cx: &mut TestAppContext) {
        init(cx);
        let root = fixture();
        std::fs::create_dir(root.join("dst")).unwrap();
        std::fs::write(root.join("a.txt"), b"a").unwrap();
        std::fs::write(root.join("b.txt"), b"b").unwrap();
        let (ws, cx) = open(cx, Location::Dir(root.clone()));
        ws.update_in(cx, |ws, window, cx| {
            ws.drop_files(
                vec![root.join("a.txt")],
                root.join("dst"),
                false,
                window,
                cx,
            );
            ws.drop_files(vec![root.join("b.txt")], root.join("dst"), true, window, cx);
        });
        cx.run_until_parked();
        // Same volume, no ⌥: moved. With ⌥: copied (original kept).
        assert!(!root.join("a.txt").exists() && root.join("dst/a.txt").exists());
        assert!(root.join("b.txt").exists() && root.join("dst/b.txt").exists());
        // Another volume (devfs here) is never "the same volume", so drops
        // there copy instead of deleting the originals.
        assert!(same_volume(&[root.join("b.txt")], &root.join("dst")));
        assert!(!same_volume(&[root.join("b.txt")], Path::new("/dev")));
    }

    #[gpui::test]
    async fn drops_into_a_dragged_folder_do_nothing(cx: &mut TestAppContext) {
        init(cx);
        let root = fixture();
        std::fs::create_dir_all(root.join("pinA/sub")).unwrap();
        std::fs::write(root.join("five.txt"), b"5").unwrap();
        // `alias` plays `/tmp`, `root` plays `/private/tmp`.
        std::os::unix::fs::symlink(&root, root.join("alias")).unwrap();
        let alias = root.join("alias");
        let (ws, cx) = open(cx, Location::Dir(root.clone()));
        ws.update_in(cx, |ws, window, cx| {
            // Onto a folder inside a dragged one, the file first.
            ws.drop_files(
                vec![alias.join("five.txt"), alias.join("pinA")],
                root.join("pinA/sub"),
                false,
                window,
                cx,
            );
            // Onto the dragged folder itself, spelled differently.
            ws.drop_files(
                vec![alias.join("pinA")],
                root.join("pinA"),
                true,
                window,
                cx,
            );
        });
        cx.run_until_parked();
        assert!(root.join("five.txt").exists());
        assert!(!root.join("pinA/sub/five.txt").exists());
        assert!(!root.join("pinA/sub/pinA").exists() && !root.join("pinA/pinA").exists());
        // Ignored silently: no job, no error toast, no conflict dialog.
        ws.update(cx, |ws, _| {
            assert!(ws.jobs.is_empty() && ws.toast.is_none() && ws.overlay.is_none());
        });
        assert_eq!(
            plan_drop(vec![alias.join("five.txt")], root.join("pinA"), false).map(|p| p.0),
            Some(OpKind::Move)
        );
    }

    #[gpui::test]
    async fn eject_moves_tabs_off_the_volume(cx: &mut TestAppContext) {
        init(cx);
        let root = fixture();
        let other = fixture();
        let (ws, cx) = open(cx, Location::Dir(root.join("projects")));
        ws.update_in(cx, |ws, window, cx| {
            ws.add_tab(Location::Dir(other.clone()), false, window, cx);
            // `root` stands in for a mounted volume (not a real mount point,
            // so no diskutil call is made).
            ws.eject(root.clone(), false, cx);
            assert!(ws.ejecting.contains(&root));
            // A second request while one is running is ignored.
            ws.eject(root.clone(), false, cx);
        });
        cx.run_until_parked();
        ws.read_with(cx, |ws, cx| {
            assert_eq!(ws.tabs[0].read(cx).location, Location::Start);
            assert_eq!(ws.tabs[1].read(cx).dir(), Some(other.as_path()));
            assert!(ws.ejecting.is_empty());
            let toast = ws.toast.as_ref().expect("result toast");
            assert!(!toast.error, "{}", toast.message);
            assert!(
                toast.message.contains("safely removed"),
                "{}",
                toast.message
            );
        });
    }

    #[gpui::test]
    async fn eject_shortcut_without_a_volume_says_so(cx: &mut TestAppContext) {
        init(cx);
        let root = fixture();
        let (ws, cx) = open(cx, Location::Dir(root.clone()));
        ws.update_in(cx, |ws, _, cx| ws.eject_current(cx));
        ws.read_with(cx, |ws, cx| {
            assert!(ws.ejecting.is_empty());
            assert_eq!(ws.active_tab().read(cx).dir(), Some(root.as_path()));
            assert_eq!(
                ws.toast.as_ref().map(|t| t.message.as_str()),
                Some("Nothing to eject here")
            );
        });
    }

    #[gpui::test]
    async fn settings_toggling_hidden_files_reloads_tabs(cx: &mut TestAppContext) {
        init(cx);
        let root = fixture();
        std::fs::write(root.join(".secret"), b"x").unwrap();
        cx.update(|cx| Store::update(cx, |s| s.show_hidden = false));
        let (ws, cx) = open(cx, Location::Dir(root.clone()));
        let names = |ws: &Entity<Workspace>, cx: &mut VisualTestContext| {
            ws.read_with(cx, |ws, cx| {
                ws.active_tab()
                    .read(cx)
                    .entries
                    .iter()
                    .map(|e| e.name.clone())
                    .collect::<Vec<_>>()
            })
        };
        assert!(!names(&ws, cx).contains(&".secret".to_string()));
        // What the Settings window does: write the store, nothing else.
        cx.update(|_, cx| Store::update(cx, |s| s.show_hidden = true));
        cx.run_until_parked();
        assert!(names(&ws, cx).contains(&".secret".to_string()));
        cx.update(|_, cx| Store::update(cx, |s| s.show_hidden = false));
        cx.run_until_parked();
        assert!(!names(&ws, cx).contains(&".secret".to_string()));
    }

    #[gpui::test]
    async fn settings_showing_the_preview_rebuilds_it(cx: &mut TestAppContext) {
        use crate::tab::Preview;
        init(cx);
        let root = fixture();
        std::fs::write(root.join("a.txt"), b"hello").unwrap();
        cx.update(|cx| Store::update(cx, |s| s.preview_visible = true));
        let (ws, cx) = open(cx, Location::Dir(root.clone()));
        let tab = ws.read_with(cx, |ws, _| ws.active_tab().clone());
        let ix = |name: &str, cx: &mut VisualTestContext| {
            tab.read_with(cx, |t, _| {
                t.entries.iter().position(|e| e.name == name).unwrap()
            })
        };
        let (a, projects) = (ix("a.txt", cx), ix("projects", cx));
        tab.update(cx, |t, cx| t.select_only(a, cx));
        cx.run_until_parked();
        // Hide the preview the way Settings does, change the selection,
        // then show it again: it must follow the new selection.
        cx.update(|_, cx| Store::update(cx, |s| s.preview_visible = false));
        cx.run_until_parked();
        tab.update(cx, |t, cx| t.select_only(projects, cx));
        cx.run_until_parked();
        cx.update(|_, cx| Store::update(cx, |s| s.preview_visible = true));
        cx.run_until_parked();
        tab.read_with(cx, |t, _| match &t.preview {
            Preview::Folder { entry, .. } => assert_eq!(entry.name, "projects"),
            Preview::Text(p) => panic!("stale preview of {}", p.path.display()),
            _ => panic!("expected the folder preview"),
        });
    }
}
