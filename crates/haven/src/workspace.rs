//! Root view: sidebar + main card (tabs, toolbar, content, status bar) + overlays.

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
    pub toast: Option<Toast>,
    pub sidebar_visible: bool,
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
            cx.observe_global::<Store>(|_, cx| cx.notify()),
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
            toast: None,
            sidebar_visible: true,
            watcher,
            overlay_subs: Vec::new(),
            _subs: Vec::new(),
        };
        let initial = if initial.is_empty() {
            vec![Location::Start]
        } else {
            initial
        };
        for loc in initial {
            ws.add_tab(loc, false, window, cx);
        }
        ws.active = active.min(ws.tabs.len() - 1);
        subs.push(cx.on_focus_out(
            &ws.path_field.focus_handle(cx),
            window,
            |ws, _, window, cx| {
                if ws.path_editing {
                    ws.end_path_edit(false, window, cx);
                }
            },
        ));
        ws._subs = subs;
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
        let tab = cx.new(|cx| Tab::new(location, cx));
        let sub = cx.subscribe_in(&tab, window, Self::on_tab_event);
        self._subs.push(sub);
        if let Some(dir) = tab.read(cx).dir().map(Path::to_path_buf)
            && let Some(w) = self.watcher.as_mut()
        {
            w.watch(&dir);
        }
        let insert_at = if self.tabs.is_empty() {
            0
        } else {
            self.active + 1
        };
        self.tabs.insert(insert_at.min(self.tabs.len()), tab);
        if activate || self.tabs.len() == 1 {
            self.activate(insert_at.min(self.tabs.len() - 1), window, cx);
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
                for d in dirs.clone() {
                    self.add_tab(Location::Dir(d), false, window, cx);
                }
            }
            TabEvent::ContextMenu { position, on_item } => {
                self.open_context_menu(*position, *on_item, window, cx);
            }
            TabEvent::Drop { paths, dest, copy } => {
                let kind = if *copy || !same_volume(paths, dest) {
                    OpKind::Copy
                } else {
                    OpKind::Move
                };
                self.start_transfer(kind, paths.clone(), dest.clone(), window, cx);
            }
        }
    }

    fn reload_dirs(&mut self, dirs: &[PathBuf], cx: &mut Context<Self>) {
        for tab in &self.tabs {
            tab.update(cx, |t, cx| {
                let hit = t.dir().is_some_and(|d| dirs.iter().any(|x| x == d));
                // Columns view also caches ancestor/peek listings.
                for d in dirs {
                    t.column_cache.remove(d);
                }
                if hit {
                    t.reload(cx);
                }
                cx.notify();
            });
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
        if refocus {
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
                let replacement = if let Some(ix) = self.completion_ix {
                    self.completions.get(ix).map(|c| c.replacement.clone())
                } else if self.completions.len() == 1 {
                    Some(self.completions[0].replacement.clone())
                } else {
                    complete::common_prefix(&self.completions).filter(|p| p.len() > current.len())
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
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tab = self.active_tab().read(cx);
        let selected: Vec<Entry> = tab.selected_entries().into_iter().cloned().collect();
        let has_clip = self.clipboard.is_some();
        let in_dir = tab.dir().is_some();
        let items = crate::overlays::build_context_menu(&selected, on_item, in_dir, has_clip, cx);
        self.context_menu = Some(ContextMenu { position, items });
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
        self.track_job(label, progress, task, vec![dest], cx);
    }

    fn track_job(
        &mut self,
        label: String,
        progress: Arc<Progress>,
        task: Task<std::io::Result<Undo>>,
        select: Vec<PathBuf>,
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
                match result {
                    Ok(undo) => {
                        if let Undo::Created(paths) = &undo {
                            // Select newly created items in the active tab.
                            let tab = ws.active_tab().clone();
                            let parent_matches = paths
                                .first()
                                .and_then(|p| p.parent())
                                .map(Path::to_path_buf);
                            if parent_matches.as_deref() == tab.read(cx).dir() {
                                let paths = paths.clone();
                                tab.update(cx, |t, cx| {
                                    t.reveal(t.dir().unwrap().to_path_buf(), paths, cx)
                                });
                            }
                        }
                        ws.undo_stack.push(undo);
                    }
                    Err(e) => ws.show_toast(e.to_string(), true, cx),
                }
                let _ = &select;
                ws.reload_active(cx);
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

    fn reload_active(&mut self, cx: &mut Context<Self>) {
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
        self.track_job(label, progress, task, vec![], cx);
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
        let Some(undo) = self.undo_stack.pop() else {
            self.show_toast("Nothing to undo", false, cx);
            return;
        };
        let label = undo.describe();
        let task = cx.background_executor().spawn(async move { undo.apply() });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |ws, cx| {
                match result {
                    Ok(_) => ws.show_toast(label, false, cx),
                    Err(e) => ws.show_toast(format!("Undo failed: {e}"), true, cx),
                }
                ws.reload_active(cx);
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
        if cut {
            self.clipboard = None;
        }
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
                tab.update(cx, |t, cx| {
                    t.reveal(dir.clone(), vec![path.clone()], cx);
                    t.reload(cx);
                });
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
                    tab.update(cx, |t, cx| {
                        if let Some(d) = t.dir().map(Path::to_path_buf) {
                            t.reveal(d, vec![to], cx);
                            t.reload(cx);
                        }
                    });
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
        if let Err(e) = haven_core::tags::set_tag(&paths, &tag, on) {
            self.show_toast(format!("Couldn't tag: {e}"), true, cx);
        }
        self.active_tab().clone().update(cx, |t, cx| t.reload(cx));
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
pub fn refresh_start_data(cx: &mut App) {
    let fresh = cx
        .global::<StartData>()
        .recents_loaded
        .is_some_and(|t| t.elapsed() < Duration::from_secs(20));
    let history = Store::get(cx).history_recents();
    let task = cx.background_executor().spawn(async move {
        let vols = volumes::list_volumes();
        let rec = if fresh {
            None
        } else {
            Some(recents::merge(history, recents::spotlight_recents(14), 40))
        };
        (vols, rec)
    });
    cx.spawn(async move |cx| {
        let (vols, rec) = task.await;
        let _ = cx.update(|cx| {
            let data = cx.global_mut::<StartData>();
            data.volumes = vols;
            if let Some(r) = rec {
                data.recents = r;
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
            let dir = tempfile::tempdir().unwrap().keep();
            // SAFETY: set before the store reads it; tests in this module share the value.
            unsafe { std::env::set_var("HAVEN_STATE_DIR", &dir) };
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
}
