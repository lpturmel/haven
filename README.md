# Haven

A fast, keyboard-first file manager for macOS, built in Rust with [GPUI](https://www.gpui.rs/).

- Frosted-glass sidebar, a rounded dark main card, and tabs.
- **Path bar** showing breadcrumbs of the resolved absolute path.
  - Click it or press `⌘L` to edit. It accepts `~`, `$VARS`, `..`, relative paths and `file://` URLs, with Tab completion.
- **Start page** with pinned files and folders, drives with capacity bars, and recently used files.
  - Recents come from Spotlight plus Haven's own history.
- **Native Finder tags**, stored in the `_kMDItemUserTags` xattr.
  - Edit them with `⌃T`. A tag view lists every tagged file through Spotlight.
- **Views:** list with multi-key sorting (⇧-click a header to add a secondary key), grid with thumbnails, and Miller columns.
  - The preview pane uses **tree-sitter** highlighting for 18 languages.
- **Power features:**
  - `⌘K` command palette and `⌘P` gitignore-aware fuzzy find.
  - Type-to-select and a `⌘F` folder filter.
  - Batch copy/move/trash with a progress HUD and conflict handling.
  - Undo, bulk rename (regex, numbering, case), and open in Terminal/editor.
- Can be the system's default file viewer (`NSFileViewer`) and folder handler.

## Build

```bash
cargo run -p haven                  # dev
scripts/bundle-macos.sh             # release Haven.app in target/bundle/
cp -R target/bundle/Haven.app /Applications/
scripts/haven ~/some/folder         # CLI shim (put it on your PATH)
```

The bundle uses `assets/AppIcon.icns` for Finder and the Dock. The transparent
1024×1024 master is `assets/AppIcon.png`; update both files when changing the icon.

## Test

```bash
cargo test --workspace
cargo test --release -p haven-core --test perf -- --ignored --nocapture
cargo test --release -p haven-highlight --test perf -- --ignored --nocapture
```

## Keys

| Keys | Action |
|---|---|
| `↑ ↓`, `⇧↑ ⇧↓` | Move the selection / extend it |
| `↩`, `⌘↓` | Open |
| `⌘↑`, `⌘[ ⌘]` | Enclosing folder / back and forward |
| `⌘L` | Edit the path |
| `⌘F` | Filter the folder |
| `⌘P` | Fuzzy find |
| `⌘K` | Command palette |
| `⌘T`, `⌘W`, `⇧⌘T`, `⌘1…9` | New tab / close tab / reopen tab / jump to tab |
| `⌥⌘1/2/3` | List / grid / columns |
| `Space` | Toggle preview |
| `⇧⌘.` | Show hidden files |
| `F2` or `⇧↩` | Rename |
| `⇧⌘R` | Bulk rename |
| `⌃T` | Edit tags |
| `⌃⌘T` | Pin to sidebar |
| `⌘C ⌘X ⌘V` | Copy, cut and paste files |
| `⌘D` | Duplicate |
| `⌘⌫` | Move to Trash |
| `⌘Z` | Undo |
| `⇧⌘C` | Copy path |
| `⌥⌘T`, `⌥⌘E` | Open in Terminal / editor |

## Known limitations

- GPUI 0.2.2 cannot start drags *out* to other apps. Drops *into* Haven and drags inside it work.
- The `public.folder` handler change takes effect the next time LaunchServices refreshes, which may require logging out and back in.
