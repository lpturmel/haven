//! Global persisted state with debounced background saves.

use std::time::Duration;

use gpui::{App, Global};
use haven_core::state::AppState;

pub struct Store {
    pub state: AppState,
    dirty: bool,
}

impl Global for Store {}

impl Store {
    pub fn init(cx: &mut App) {
        let state = AppState::load();
        cx.set_global(Store {
            state,
            dirty: false,
        });
        // Debounced saver: writes at most every 750ms when something changed.
        cx.spawn(async move |cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(750))
                    .await;
                let snapshot = cx
                    .update(|cx| {
                        // Only touch the global mutably when there is work:
                        // `global_mut` notifies observers (a full re-render).
                        if !cx.global::<Store>().dirty {
                            return None;
                        }
                        let store = cx.global_mut::<Store>();
                        store.dirty = false;
                        Some(store.state.clone())
                    })
                    .ok()
                    .flatten();
                if let Some(state) = snapshot {
                    cx.background_executor()
                        .spawn(async move {
                            let _ = state.save();
                        })
                        .await;
                }
            }
        })
        .detach();
    }

    pub fn get(cx: &App) -> &AppState {
        &cx.global::<Store>().state
    }

    /// Mutates state, marks it dirty and notifies observers.
    pub fn update<R>(cx: &mut App, f: impl FnOnce(&mut AppState) -> R) -> R {
        let store = cx.global_mut::<Store>();
        let r = f(&mut store.state);
        store.dirty = true;
        r
    }

    /// Synchronous save (used on quit).
    pub fn flush(cx: &mut App) {
        let store = cx.global_mut::<Store>();
        let _ = store.state.save();
        store.dirty = false;
    }
}

/// Points the store at a per-process temp dir exactly once (tests run on
/// parallel threads, so the env var must not be rewritten per test).
#[cfg(test)]
pub fn init_test_env() {
    static ONCE: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("haven-test-state-{}", std::process::id()));
        // SAFETY: runs once, before any test reads the variable.
        unsafe { std::env::set_var("HAVEN_STATE_DIR", &dir) };
    });
}
