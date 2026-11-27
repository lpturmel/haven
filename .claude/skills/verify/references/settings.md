# Runbook: Settings window and menu bar

Code: `crates/haven/src/settings.rs` (window, controls, `MainWindow` and
`SettingsWindow` globals), the menu in `crates/haven/src/main.rs` (`menus()`),
and the `OpenSettings` action bound to ⌘, globally. Every control writes
`Store` (persisted to `$HAVEN_STATE_DIR/state.json`, debounced about 750ms).
Workspaces observe the store.

## Headless

- `cargo test -p haven settings` covers the grid-icon stepper (clamping and
  snapping).
- `cargo test -p haven settings_toggling_hidden_files_reloads_tabs` checks
  that a store-only change (what the window does) reloads open tabs.
- Menu check without clicking: `osascript -e 'tell application "System Events" to
  tell (first process whose unix id is <pid>) to get name of every menu item of
  menu 1 of menu bar item 2 of menu bar 1'`. Expect `Settings…` after the app
  name item. This needs Accessibility permission for the terminal; skip it if
  that isn't granted.

## In the app (screen free)

1. Menu bar ▸ Haven: "Settings…" with ⌘, sits above "Make Haven the Default
   File Viewer…". Choosing it opens a "Settings" window, centered, not
   resizable, with traffic lights at the top left and a title bar that
   doesn't overlap them.
2. Press ⌘, again, from the main window and from Settings: the same window
   comes to the front (never a second one). ⌘W or Esc closes it. ⌘, after
   closing opens a fresh one.
3. Each control, checking the main window live (no restart) and
   `state.json` after about 1s:
   - Default view List / Grid / Columns: `default_view`. Folders without
     their own prefs open in that view (new tab ▸ some folder).
   - Show hidden files: dotfiles appear or disappear in every open tab
     (same as ⇧⌘.).
   - Show preview pane: the preview column toggles.
   - Fuzzy find respects .gitignore: `respect_gitignore`.
   - Font smoothing: text in both windows repaints immediately, heavier when
     on (see font-rendering.md for how to measure it).
   - Grid icon size −/+: steps of 8 between 48 and 128. The buttons dim at
     the limits, and grid view tiles resize.
   - Terminal app and Editor command fields: typing persists
     (`terminal_app`, `editor_command`). ⌥⌘T and ⌥⌘E use the new values.
   - Make Default… opens the confirmation in the main window. **Don't
     confirm it:** AGENTS.md forbids changing the default file viewer on
     the owner's machine. Use Finder only if you changed it.
4. Quit and relaunch with the same `HAVEN_STATE_DIR`: the values stick.
5. Close the main window (red button), then ⌘,: Settings still opens. Its
   default-viewer buttons do nothing harmful when no main window exists.
