# Haven — a GPUI file manager to replace Finder

## Context
`/Users/lpturmel/dev2/tools/haven` is empty. Goal: a native-feeling, keyboard-first, very fast macOS file manager in Rust + GPUI. It should replace Finder for daily browsing. The reference image is for **layout only**: a frosted sidebar, then a main card with tabs, a nav/path bar, the content, and a status bar. Requirements:
- Dark mode, rounded corners, frosted-glass sidebar.
- An editable path bar that resolves absolute paths.
- A start page with Pinned items, Recents and Drives.
- Native Finder tags.
- Sidebar pins.
- Multi-key sorting.
- List / grid / columns layouts, plus a preview pane with tree-sitter highlighting.
- Tabs, a command palette, fuzzy find and batch file operations.
- Registration as the system's default file viewer.

Decisions you made: native Finder tags (xattr); all layouts in v1; standalone app + default viewer (NSFileViewer); v1 power features are keyboard-first navigation, tabs, fuzzy find and batch ops.

## Stack (matches sibling projects, all on gpui 0.2.2)
- **Workspace (edition 2024)** modeled on `spaces/Cargo.toml`. Use `gpui = { version = "=0.2.2", default-features = false, features = ["font-kit", "runtime_shaders"] }`.
- **No gpui-component.** We need full control of the look, and its 9k-line input brings its own theme system. Instead we hand-roll a small theme and widget set (pattern from `spaces/crates/silt-ui/src/theme.rs`). The text field is adapted from gpui's `examples/input.rs` (`EntityInputHandler`).
- **Core crates:**
  - `jwalk` + `rayon`: parallel walks for folder sizes.
  - `ignore` + `nucleo`: fuzzy find.
  - `notify` 7: FSEvents watcher.
  - `trash` 5: using the NSFileManager method, which is fast and supports Put Back.
  - `xattr` + `plist`: tags.
  - `serde`/`serde_json`, `smol`, `anyhow`, `natord` (or a hand-written natural compare), `libc` (getmntinfo/statfs).
- **Highlighting:**
  - `tree-sitter` 0.25 + `tree-sitter-highlight`.
  - Grammars: rust, python, js, ts/tsx, json, toml, yaml, markdown, bash, go, c, cpp, html, css, lua, swift.
- **macOS:** `objc2` 0.6, `objc2-app-kit` 0.3, `objc2-foundation` 0.3. Used for the NSWorkspace icons and default-viewer registration.

## Crate layout
```
haven/
  Cargo.toml                 workspace
  crates/haven-core/         no gpui; unit-testable, benchmarked
    listing.rs   dir read → Vec<Entry>{name, path, kind, size, mtime, ctime, ext, hidden, symlink, tags}
    sort.rs      SortSpec = Vec<(SortKey, Dir)>; keys: Name(natural, case-insens), Size, Modified, Created, Kind, Ext, Tags; folders-first flag
    pathres.rs   resolve user input: ~, $VARS, relative to cwd, .., canonicalize; returns Dir | File(parent, select) | Err
    complete.rs  path-bar Tab completion (prefix + fuzzy among child dirs)
    tags.rs      read/write com.apple.metadata:_kMDItemUserTags (binary plist ["Name\n<color>"]); tag search via `mdfind kMDItemUserTags==`
    volumes.rs   getmntinfo + statfs → mounted local/removable volumes with capacity; eject via `diskutil eject`
    recents.rs   Haven open-history + background `mdfind kMDItemLastUsedDate >= $time.today(-14)` merged, deduped
    ops.rs       job queue: copy (std::fs::copy → APFS clonefile), move (rename, fallback copy+delete), trash, mkdir, rename, bulk rename; progress via atomics; conflict policy; undo records
    fuzzy.rs     ignore::WalkParallel feeding nucleo::Nucleo injector; cancellable
    dirsize.rs   jwalk recursive size with cancel/progress (Control struct pattern from prism-disk/src/scanner.rs:59)
    watch.rs     notify watcher, non-recursive on dirs shown in open tabs, 50ms debounce
    state.rs     persisted AppState (pins, sidebar pins, tag colors, per-folder view prefs, history, session tabs)
                 → ~/Library/Application Support/Haven/state.json, atomic write, debounced (pattern: lpdf/src/preferences.rs)
  crates/haven-highlight/    tree-sitter languages registry, ext/shebang detection,
                             highlight(text) → per-line Vec<(Range, Capture)>; capture → palette mapping
  crates/haven-macos/        objc2: file_icon(path|ext, size) → PNG bytes (adapt lpbar/src/icons.rs:92-110), icon cache keyed by UTI/ext (per-path only for .app/.icns), default-viewer registration
  crates/haven/              gpui app
    main.rs      Application::new().with_assets(Assets); fonts; key bindings; menus; on_open_urls; window
    theme.rs     dark palette tokens (+ light later)
    assets.rs    Lucide SVG icons via include_bytes (pattern: lpdf/src/assets.rs)
    workspace.rs root view: sidebar | main card (tabs, toolbar, content, status bar), overlays (palette, fuzzy, dialogs, ops HUD)
    sidebar.rs   Favorites/pins (drag-reorder), Drives, Tags; frosted
    tab.rs       Tab entity: cwd, history (back/fwd), selection, view mode, sort, listing Task, watcher sub
    pathbar.rs   breadcrumb ↔ editable TextField with completion dropdown
    text_field.rs  reusable single-line input (from gpui examples/input.rs)
    views/{start.rs, list.rs, grid.rs, columns.rs, preview.rs, tag_results.rs}
    palette.rs, fuzzy.rs, ops_hud.rs, rename.rs, context_menu.rs (deferred(anchored(..)) popover)
  scripts/bundle-macos.sh    release build → Haven.app + Info.plist + ad-hoc codesign (pattern: spaces/scripts/bundle-macos.sh)
  assets/Info.plist.template, icons/
```

## Look & window
- `WindowOptions`:
  - `titlebar: TitlebarOptions { appears_transparent: true, traffic_light_position: Some(point(px(14.), px(16.))) }`
  - `window_background: WindowBackgroundAppearance::Blurred`
  - `window_min_size` 900×560
- On macOS 12+, gpui 0.2.2 puts an NSVisualEffectView behind the content (`gpui/src/platform/mac/window.rs:1257-1310`). The system gives rounded corners and a shadow.
- The sidebar paints a **semi-transparent** bg, so the blur shows through: `hsla(240°, 6%, 10%, 0.55)` plus a 1px right hairline `white/6%`.
- The main area is an **opaque card**: bg `#1c1c1f`, inset 6px top/right/bottom, `rounded(10px)`, border `white/7%`. That gives the "panel on glass" look from the image.
- Draft palette:
  - text `#ececf1`, muted `#9a9aa5`, faint `#6b6b75`
  - accent `#0a84ff`, selection `accent/22%`, hover `white/5%`
  - danger `#ff453a`
  - Finder tag colors: red, orange, yellow, green, blue, purple, gray
- Fonts:
  - UI: `.SystemUIFont`, falling back to bundled Inter (registered as in `alula/crates/alula/src/main.rs:9072`)
  - Mono: bundled JetBrains Mono
- Corner radii: 6 on rows and buttons, 8 on popovers, 10 on the card.

## Features → design
**Path bar.** It shows breadcrumbs of the *canonical* absolute path; a symlinked cwd shows the resolved path, and a tooltip shows the alias. A click on empty space or Cmd+L turns it into a TextField pre-filled with the absolute path, all selected.
- Accepted input: `~`, `$HOME`, relative paths, `..`, and pasted `file://` URLs.
- Tab and ↓ open a completion dropdown.
- Enter calls `pathres::resolve`. A directory navigates. A file navigates to its parent and selects the file. An error shows a red outline and an inline message.
- Esc reverts.
- A right-click on a crumb opens a menu of its sibling folders.

**Start page** (the default for a new tab, also reachable via Home in the sidebar):
- **Pinned:** large tiles for files and folders. Clicking a file opens it with the system app.
- **Drives:** cards with capacity bars, as in the image.
- **Recent files:** name, location and relative time. Clicking reveals the file in Haven; Enter opens it.

All sections load asynchronously and render placeholders first.

**Sidebar:**
- Home.
- Pinned folders (drag to reorder, drop a folder to pin, context menu to unpin).
- Standard places (Desktop, Documents, Downloads, Pictures, Movies, Music, Applications, iCloud Drive if it exists).
- Drives (with an eject button).
- Tags (color dot, opens the tag results view, plus "New tag…").
- Collapsible sections.

**Tags.**
- Read them in the listing pass, as one `getxattr` per entry, done in the background after the first paint for big directories.
- Show them as colored dots in the Name column and grid tiles.
- Edit them in a popover (opened with **Ctrl+T** or from the context menu). It offers checkbox tags, a "create" field, and the 7 Finder colors.
- Writes keep unknown tags intact.
- Tag results use `mdfind`, streamed into the same list view, so a tag works across the disk just like Finder.

**Sorting.**
- Click a column header to sort by it. Click it again to reverse.
- **Shift+click adds a secondary key**, shown with a "1"/"2" badge.
- Also offered in the toolbar sort menu: Name, Kind, Size, Date Modified, Date Created, Extension, Tags.
- "Folders first" toggle. Natural ordering for names.
- Folder sizes are "--" until computed. Sorting by size offers "Calculate folder sizes", which runs `dirsize` in the background and re-sorts as results stream in.
- Sort and view are persisted per folder, with a global default.

**Layouts** (switched with Cmd+1/2/3 or the toolbar segmented control). Each layout's state is kept per tab.
- **List:** `uniform_list` rows with resizable columns (Name, Modified, Size, Kind, Tags). The preview pane is a toggle (Space or Cmd+Shift+P) with a resizable splitter. Preview off gives the plain dense list.
- **Grid:** uniform_list of rows of N tiles, where N comes from the width and an icon-size slider. Images show downscaled thumbnails through `img(path)` with a `RetainAllImageCache`; everything else shows NSWorkspace icons.
- **Columns (Miller):** horizontal stack of per-directory uniform_lists. ←/→ move between columns. The last column is the preview when a file is selected.

**Preview pane** (debounced 40ms on selection change; stale tasks are cancelled):
- **Text/code:** sniff the first 8KB for NUL bytes, read up to 2 MB, and show the plain text immediately. `haven-highlight` then runs on the background executor and swaps in the highlighting. Render a uniform_list of lines, each `StyledText::with_runs` with precomputed `TextRun`s, plus a line-number gutter. LRU cache keyed by (path, mtime, len), 32 entries.
- **Images:** `img(path).object_fit(Contain)` plus dimensions.
- **Folders:** item count, async total size, newest items.
- **Everything else:** large icon plus metadata (size, created, modified, kind/UTI, permissions, tags) and an "Open with…" list.

**Keyboard-first:**
- Actions + `KeyBinding`s scoped by `key_context` (pattern from `lpdf/src/ui.rs:243-261`).
- Selection movement: ↑/↓ and optional j/k; Shift extends the selection, Cmd+A selects all.
- Opening: Return opens (a folder enters, a file opens with the system app); Cmd+↓ does the same.
- Navigation: Cmd+↑ goes to the parent; Cmd+[ and Cmd+] go back/forward; Cmd+L focuses the path bar.
- Search: typing selects by prefix (resets after 700ms); Cmd+K opens the command palette; Cmd+P or Cmd+F opens fuzzy find.
- Tabs: Cmd+T / Cmd+W / Cmd+Shift+T, Ctrl+Tab, Cmd+1…9 jumps to a tab. View modes move to Cmd+Opt+1/2/3 to avoid a clash.
- File operations: Cmd+C / Cmd+X / Cmd+V on files; Cmd+Shift+C copies the path; Cmd+⌫ trashes; Cmd+Z undoes.
- Other: F2 or Shift+Return renames; Cmd+Shift+N makes a new folder; Cmd+Shift+. toggles hidden files.
- Keymap overrides go in `~/.config/haven/keymap.json` (later).

**Tabs.** A tab strip at the top of the card, like the image. Each Tab is its own entity with its own history and watcher. The session (open tabs plus cwd) is restored on launch.

**Fuzzy find (Cmd+P).** An overlay scoped to the cwd.
- `ignore::WalkParallel` respects .gitignore (toggle in the overlay) and skips `node_modules`/`.git`/`target` by default. It streams into a `nucleo` injector.
- The UI ticks nucleo every frame while the walk is active, with the top 200 shown in a uniform_list with match highlighting.
- Return reveals the result; Cmd+Return opens it.

**Command palette (Cmd+K).** Fuzzy search (nucleo) over all actions with their shortcuts, pins, recents and drives.

**Batch ops.**
- Multi-select (click, Shift-range, Cmd-toggle).
- Copy/move via the clipboard or internal drag-and-drop between tabs and sidebar pins. Files dropped from other apps arrive as `ExternalPaths`.
- The `ops` queue runs on background threads. An ops HUD in the bottom-right shows per-job progress and cancel.
- Conflict dialog: Replace / Keep both / Skip, with "apply to all".
- Trash with Put Back. Undo stack for rename/move/trash/new-folder.
- Bulk rename dialog: find/replace (regex), prefix/suffix, numbering, case, and a live preview table.
- Open in Terminal/editor uses commands configured in state (defaults: Ghostty if installed, else Terminal; `$EDITOR` or `code`).
- Known GPUI limitation: **dragging files *out* to other apps is not possible in gpui 0.2.2** (there is no NSDraggingSource). For v1, Cmd+Shift+C copies the paths and "Reveal/Open with" covers it. A native drag-out shim via objc2 can come later.

**Watcher.** Each visible tab's dir is watched. On an event, re-list in the background, then diff-apply while keeping the selection and scroll by path.

**Default viewer.**
- The Info.plist declares a `CFBundleDocumentTypes` entry for `public.folder` (role Viewer).
- `cx.on_open_urls` (gpui handles `application:openURLs:`, `mac/platform.rs:143`) opens incoming file URLs in a new tab. A file URL selects the file in its parent.
- Settings → "Make Haven the default file viewer" runs these, after a confirmation prompt, with a matching "Restore Finder":
  - `defaults write -g NSFileViewer -string dev.haven.app`
  - the LaunchServices `public.folder` handler
- A CLI shim `haven <path>` uses `open -b dev.haven.app <path>`.
- Risk to check: which apps honor NSFileViewer for "Reveal in Finder". Many do via NSWorkspace; Finder-hardcoded ones won't.

## Performance rules
- No filesystem calls on the UI thread. Everything goes through `cx.spawn` + `background_executor().spawn` (pattern: `spaces/crates/silt-ui/src/state.rs:346-394`).
- Every list is virtualized: `uniform_list` everywhere.
- Listing uses `read_dir` with `symlink_metadata` done in parallel through rayon when there are more than 1,000 entries. Tags and icons come in a second pass, so the first paint has names, sizes and dates. `getattrlistbulk` can replace this later behind the same function if the benchmarks call for it.
- Sorting uses precomputed sort keys (lowercase/natural tokens are cached on the Entry). Re-sorting 100k entries should take under 30ms.
- Icon cache keyed by UTI/extension; thumbnails are decoded downscaled.
- Budgets:
  - first frame under 150ms
  - 10k-entry dir shown in under 50ms
  - arrow-key repeat holds 120fps
  - preview text shown in under 16ms, with highlighting following in under 50ms for files under 200KB
- Criterion benches in haven-core (listing, sort, pathres) against a generated 100k-file tempdir.

## Milestones (each one runnable)
1. **Scaffold & shell.** Workspace, window (blur/transparent titlebar), theme, frosted sidebar plus opaque card, icon assets, bundle script.
2. **Browsing core.** listing/sort/watch, List view, nav history, path bar (breadcrumbs, editable, completion), status bar, keyboard navigation, hidden-files toggle, persistence.
3. **Start page & sidebar.** Pins (start page and sidebar), recents, drives with capacity bars and eject.
4. **Views & preview.** Grid, Columns, preview pane with tree-sitter, images, NSWorkspace icons.
5. **Tags.** xattr read/write, tag popover, sidebar tags, mdfind tag results.
6. **Power features.** Tabs and session restore, command palette, fuzzy find, ops queue/HUD, conflicts, undo, bulk rename, clipboard, open in terminal/editor, context menus.
7. **System integration.** Info.plist document types, on_open_urls, default-viewer toggle, CLI shim, dock menu.

## Verification
- **Unit tests in `haven-core`:**
  - natural and multi-key sort
  - `pathres` (`~`, `..`, symlinks, file vs dir, bad paths)
  - tag plist encode/decode round-trip, plus a real xattr on a tempfile, cross-checked with `mdls -name kMDItemUserTags`
  - ops: copy/move/trash/rename, conflicts, undo
  - completion
  - state migration and defaults
- **GPUI tests** (`gpui` `test-support`, as lpdf does) for Tab navigation/history and selection-preservation logic.
- **Benchmarks** against the budgets above: `cargo bench -p haven-core`.
- **Manual run:**
  - `cargo run -p haven` first, then `scripts/bundle-macos.sh` and launch `Haven.app`.
  - Screenshot and drive it via computer-use to check: the blur shows through the sidebar, the card is opaque with rounded corners, path-bar editing works, tags show in Finder and Finder's tags show in Haven, all 3 layouts and the preview highlighting work, and fuzzy find works on a large repo (e.g. `~/dev2`).
  - Check the default-viewer flow with "Reveal in Finder" from VS Code, then restore Finder.
