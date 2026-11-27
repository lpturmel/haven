# Haven — agent guide

Haven is a macOS file manager written in Rust with GPUI (`gpui = "=0.2.2"`). It is meant to replace Finder.
The design plan lives in `docs/PLAN.md`.

## Autonomy
Agents working on this repo are autonomous and **do not need the owner's approval** to proceed. That covers:
- picking up and fixing bugs;
- branching and committing;
- pushing;
- opening, validating and merging PRs;
- starting validation sessions.

Don't stop to ask "should I continue?" or "ok to merge?". Decide, act, and report what you did. The guardrails that still apply are the workflow below (every PR passes the 3-tier validation before merging) and these limits:
- never force-push `main`;
- never touch files outside the repo and scratch folders under `/tmp`;
- never change system settings such as the default file viewer on the owner's machine.

## Layout
- `crates/haven-core`: filesystem logic. It has no gpui dependency, covering listing, sorting, path resolution, tags, volumes, recents, file operations, fuzzy find, watching and persisted state.
- `crates/haven-highlight`: tree-sitter syntax highlighting for the preview pane.
- `crates/haven-macos`: objc2 / AppKit glue for file icons, trash and default-viewer registration.
- `crates/haven`: the GPUI app (binary `haven`).
- `scripts/bundle-macos.sh`: builds `target/bundle/Haven.app`.
- `scripts/visual-check.sh`: bundles, launches and screenshots the app per view, and checks sidebar legibility (tier 3).
- `scripts/text-render-compare.swift`: offscreen comparison of stock gpui, Haven (font smoothing) and AppKit text, with a softness metric.
- `vendor/gpui`: gpui 0.2.2 with small, marked patches (font smoothing; synthetic drag ends when no button is held), used through `[patch.crates-io]` and excluded from the workspace. Read `vendor/gpui/HAVEN_PATCHES.md` before touching or upgrading it.
- `.claude/skills/verify`: the `/verify` skill. It covers 3-tier validation plus per-feature runbooks (eject, drag and drop, settings, fonts, hover and layout).

## Rules
- Never do filesystem work on the UI thread. Use `cx.spawn` plus `cx.background_executor().spawn`.
- Every list is virtualized with `uniform_list`.
- Keep `haven-core` free of gpui so it stays unit-testable.
- Run `cargo fmt --all` and `cargo clippy --workspace --all-targets -- -D warnings` before every commit.
- Performance budgets live in `crates/*/tests/perf.rs`, which are `#[ignore]`d. Run them with `cargo test --release -p <crate> --test perf -- --ignored`.
- To take screenshots, bundle the app first (`PROFILE=debug scripts/bundle-macos.sh`, then `open target/bundle/Haven.app`). The bundle id is `dev.haven.app`. The bare binary does not appear in the app list that screenshot tools use.
- Every build shares the bundle id, and the owner may have an installed copy running from `/Applications`. Never quit by bundle id (`quit app id "dev.haven.app"`), because that hits whichever instance came first. Stop your own instance by pid. `scripts/visual-check.sh` handles this already: it finds, captures and quits its instance by this checkout's bundle path and pid, so it runs fine next to other Havens. Screen-control tools (computer-use) are different: they also attach to the first `dev.haven.app` instance, which may be the owner's copy or a stale build. Before driving the app by hand, check `pgrep -fl MacOS/haven` and confirm which window you are acting on. Launch with `open -n --env HAVEN_STATE_DIR=$(mktemp -d) target/bundle/Haven.app --args <folder>` so the validation never touches the real state in `~/Library/Application Support/Haven`.
- Dev env vars: `HAVEN_STATE_DIR` (isolated state), `HAVEN_DISPLAY=<n>` (open on the n-th display, e.g. a secondary screen while the main one is busy), `HAVEN_WINDOW_SIZE=<w>x<h>` (initial size, e.g. `900x560` for the narrow List view), `HAVEN_FONT_SMOOTHING=0|1` (overrides Settings, for before/after captures), and `HAVEN_OPEN_SETTINGS=1` (opens the Settings window at launch, on the `HAVEN_DISPLAY` screen, so it can be captured without keystrokes). Add `open -g` so a validation launch doesn't steal focus from whatever the owner is doing.
- `haven_core::volumes::eject` must only ever receive a real mount point: `diskutil` resolves any path to the whole disk that holds it.
- Files dragged out of the window are handed to AppKit as a native drag (`crates/haven/src/native_drag.rs`). Inside the window, drops go through `views::file_drop_target`, which accepts both `DraggedFiles` and `ExternalPaths`. Use it for any new drop target.
- The sidebar's frosted look comes from a native `NSVisualEffectView` (`.sidebar` material) installed behind gpui's content (`haven_macos::install_sidebar_material`). Don't switch back to gpui's `WindowBackgroundAppearance::Blurred`: on current macOS it only tints, so whatever is behind the window shows through sharp.

## Workflow: PRs and 3-tier validation
1. Work on a feature branch and open a PR against `main`.
2. **After creating a PR, start a fresh session (with no context from the authoring session) to validate the feature.** The validator follows the `/verify` skill (`.claude/skills/verify/SKILL.md`), runs all three tiers and posts the results as a PR comment. When a PR bundles several features, start one validation session per feature so each gets an unanchored look:
   1. **Unit tests:** `cargo test --workspace`. The PR must add or extend tests for the logic it changes, mostly in `haven-core`.
   2. **Integration tests:** GPUI view tests using gpui `test-support` / `TestAppContext` for the view state it touches (navigation, selection, path bar, and so on), plus `cargo build --release`.
   3. **Visual validation:**
      - Run `scripts/visual-check.sh`. It must exit 0. Other running Havens don't need to be quit. It captures `target/visual/{start,list,grid,columns}.png` composited over the real desktop, plus a `.window.png` of the window alone. It fails if the sidebar is too bright for its text. If another app covers Haven during a capture, the composite is skipped rather than recording someone else's screen. Look at every capture, and never publish one that shows other apps.
      - Then drive the app by hand (keyboard input needs Haven to be the key window, so a screen-control agent needs full-screen control) through the feature and this regression checklist:
        - The sidebar is frosted: windows behind it are blurred, never readable. Its text is legible over a bright wallpaper. The content card is opaque and rounded, and the traffic lights don't overlap content.
        - List view: Date, Size and Kind sit under their headers on every row, and rows span the full width (Columns view too).
        - Path bar (⌘L): the completion dropdown paints above the page. Tab completes a unique prefix match. Enter navigates. A bad path shows an error.
        - Right-click and Ctrl-click open the context menu in the first tab (not only in ⌘T tabs). Esc closes it.
        - After switching to another app and back, shortcuts (⌘K, ⌘L) still work without clicking first.
        - Folders refresh live in every tab when files change on disk.
        - ⇧⌘N and rename select the renamed item. ⌘⌫ then ⌘Z restores it.
        - Drives: the eject button, the right-click "Eject" and ⌘E all eject a mounted disk image, and tabs on it go to the start page. A busy volume shows "in use by another app".
        - Haven ▸ Settings… (⌘,) opens one Settings window. Its switches take effect live (hidden files, preview, font smoothing).
        - Files drag in from Finder (list, grid, columns, sidebar, tabs, path bar) and drag out to Finder or the Desktop.
        - List view at 900×560 (`HAVEN_WINDOW_SIZE`): names stay readable and the optional columns drop out.
      - Describe each screenshot in the PR comment.
3. Fix whatever the validator finds on the same branch. Re-validate if the fixes are substantive.
4. Agents may **create and merge PRs on their own** once all three tiers pass and CI (if any) is green. Squash-merge, then delete the branch.
