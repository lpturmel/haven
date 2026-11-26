# Haven — agent guide

Haven is a macOS file manager written in Rust with GPUI (`gpui = "=0.2.2"`). It is meant to replace Finder.
The design plan lives in `docs/PLAN.md`.

## Layout
- `crates/haven-core`: filesystem logic. It has no gpui dependency, covering listing, sorting, path resolution, tags, volumes, recents, file operations, fuzzy find, watching and persisted state.
- `crates/haven-highlight`: tree-sitter syntax highlighting for the preview pane.
- `crates/haven-macos`: objc2 / AppKit glue for file icons, trash and default-viewer registration.
- `crates/haven`: the GPUI app (binary `haven`).
- `scripts/bundle-macos.sh`: builds `target/bundle/Haven.app`.

## Rules
- Never do filesystem work on the UI thread. Use `cx.spawn` plus `cx.background_executor().spawn`.
- Every list is virtualized with `uniform_list`.
- Keep `haven-core` free of gpui so it stays unit-testable.
- Run `cargo fmt --all` and `cargo clippy --workspace --all-targets -- -D warnings` before every commit.

## Workflow: PRs and 3-tier validation
1. Work on a feature branch and open a PR against `main`.
2. **After creating a PR, start a fresh session (with no context from the authoring session) to validate the feature.** The validator runs all three tiers and posts the results as a PR comment:
   1. **Unit tests:** `cargo test --workspace`. The PR must add or extend tests for the logic it changes, mostly in `haven-core`.
   2. **Integration tests:** GPUI view tests using gpui `test-support` / `TestAppContext` for the view state it touches (navigation, selection, path bar, and so on), plus `cargo build --release`.
   3. **Visual validation:** build and launch the app (`cargo run -p haven` or the bundled `Haven.app`) and drive it through the feature. Take screenshots that show the feature working, and attach them or describe them in the PR comment, including dark-mode look, blur and layout.
3. Fix whatever the validator finds on the same branch. Re-validate if the fixes are substantive.
4. Agents may **create and merge PRs on their own** once all three tiers pass and CI (if any) is green. Squash-merge, then delete the branch.
