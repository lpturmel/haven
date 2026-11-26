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

## Rules
- Never do filesystem work on the UI thread. Use `cx.spawn` plus `cx.background_executor().spawn`.
- Every list is virtualized with `uniform_list`.
- Keep `haven-core` free of gpui so it stays unit-testable.
- Run `cargo fmt --all` and `cargo clippy --workspace --all-targets -- -D warnings` before every commit.
- Performance budgets live in `crates/*/tests/perf.rs`, which are `#[ignore]`d. Run them with `cargo test --release -p <crate> --test perf -- --ignored`.
- To take screenshots, bundle the app first (`PROFILE=debug scripts/bundle-macos.sh`, then `open target/bundle/Haven.app`). The bundle id is `dev.haven.app`. The bare binary does not appear in the app list that screenshot tools use.
- Before any visual check, make sure no other Haven is running (`pgrep -fl MacOS/haven`). Every build shares the bundle id, so screenshot tools attach to whichever instance came first, which may be a stale build. Launch with `open -n --env HAVEN_STATE_DIR=$(mktemp -d) target/bundle/Haven.app --args <folder>` so the validation never touches the real state in `~/Library/Application Support/Haven`.
- The sidebar's frosted look comes from a native `NSVisualEffectView` (`.sidebar` material) installed behind gpui's content (`haven_macos::install_sidebar_material`). Don't switch back to gpui's `WindowBackgroundAppearance::Blurred`: on current macOS it only tints, so whatever is behind the window shows through sharp.

## Workflow: PRs and 3-tier validation
1. Work on a feature branch and open a PR against `main`.
2. **After creating a PR, start a fresh session (with no context from the authoring session) to validate the feature.** The validator runs all three tiers and posts the results as a PR comment:
   1. **Unit tests:** `cargo test --workspace`. The PR must add or extend tests for the logic it changes, mostly in `haven-core`.
   2. **Integration tests:** GPUI view tests using gpui `test-support` / `TestAppContext` for the view state it touches (navigation, selection, path bar, and so on), plus `cargo build --release`.
   3. **Visual validation:**
      - Run `scripts/visual-check.sh`. It must exit 0. It captures `target/visual/{start,list,grid,columns}.png` composited over the real desktop, plus a `.window.png` of the window alone. It fails if the sidebar is too bright for its text. If another app covers Haven during a capture, the composite is skipped rather than recording someone else's screen. Look at every capture, and never publish one that shows other apps.
      - Then drive the app by hand (keyboard input needs Haven to be the key window, so a screen-control agent needs full-screen control) through the feature and this regression checklist:
        - The sidebar is frosted: windows behind it are blurred, never readable. Its text is legible over a bright wallpaper. The content card is opaque and rounded, and the traffic lights don't overlap content.
        - List view: Date, Size and Kind sit under their headers on every row, and rows span the full width (Columns view too).
        - Path bar (⌘L): the completion dropdown paints above the page. Tab completes a unique prefix match. Enter navigates. A bad path shows an error.
        - Right-click and Ctrl-click open the context menu in the first tab (not only in ⌘T tabs). Esc closes it.
        - After switching to another app and back, shortcuts (⌘K, ⌘L) still work without clicking first.
        - Folders refresh live in every tab when files change on disk.
        - ⇧⌘N and rename select the renamed item. ⌘⌫ then ⌘Z restores it.
      - Describe each screenshot in the PR comment.
3. Fix whatever the validator finds on the same branch. Re-validate if the fixes are substantive.
4. Agents may **create and merge PRs on their own** once all three tiers pass and CI (if any) is green. Squash-merge, then delete the branch.
