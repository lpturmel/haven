---
name: verify
description: Validate a Haven change end to end with the 3-tier check from AGENTS.md (unit tests, gpui integration tests and a release build, then visual and interactive checks in the real app), and post the results on the PR. Use it for /verify, when validating a PR or branch, or when checking that a feature (drive eject, drag and drop, Settings, font rendering, hover and layout, list columns) works in the running app. Includes per-feature runbooks under references/.
---

# Verify a Haven change

You are validating, not authoring. Judge the change from its diff and from
the app itself, not from the author's description. Report what you saw,
including failures, with evidence (test output, screenshots, measurements).

## 0. Scope

1. Find the PR and the feature you were asked to check:
   `gh pr view <n> --json title,body,headRefName,files`, then `git diff main...HEAD`.
2. Pick the runbook(s) that apply:

   | Feature | Runbook |
   |---|---|
   | Drive eject | [references/eject.md](references/eject.md) |
   | Drag and drop in and out | [references/drag-and-drop.md](references/drag-and-drop.md) |
   | Settings window and menu bar | [references/settings.md](references/settings.md) |
   | Text sharpness, font smoothing, gpui patches | [references/font-rendering.md](references/font-rendering.md) |
   | Hover states, spacing, alignment, List columns | [references/hover-and-layout.md](references/hover-and-layout.md) |

   Anything else gets the generic tier-3 checklist in AGENTS.md.

## 1. Tier 1: unit tests

```bash
cargo test --workspace
```

All suites must pass. Check the PR adds or extends tests for the logic it
changes (mostly `haven-core`). A test that fails once and passes on rerun is
still a finding: name it in the report, and fix it if the PR touched it.

## 2. Tier 2: integration and build

- gpui view tests (`#[gpui::test]`, `TestAppContext` / `VisualTestContext`)
  cover the view state the change touches. They live next to the code, for
  example `crates/haven/src/workspace.rs` `mod tests`.
- `cargo build --release` succeeds.
- `cargo fmt --all --check` and
  `cargo clippy --workspace --all-targets -- -D warnings` are clean.

## 3. Tier 3: the real app

### Ground rules (read before launching anything)

- **Don't disturb the owner.** Before any GUI step, check what's in front:
  `lsappinfo info -only name $(lsappinfo front)`. If it's a game, a video
  call or a full-screen app, the owner is busy. Don't take full-screen
  control, don't send keystrokes or clicks, and launch only with `open -g`
  (background). Do everything that works headlessly: window captures,
  scripts, tests. List the interactive checks as **pending: screen was in
  use** in your report rather than grabbing the mouse.
- **Never take a full-screen capture to publish.** It shows whatever the
  owner has open. Capture Haven's own window (`screencapture -l <wid>`).
- **Stale instances.** Every build shares bundle id `dev.haven.app`:
  `pgrep -fl MacOS/haven`. Don't kill instances you didn't start. Target
  yours by PID (the helper below) instead.
- **Isolated state.** Always `--env HAVEN_STATE_DIR=$(mktemp -d)`.
- **Scratch only.** Fixtures, disk images and captures go under `/tmp` or
  your scratchpad, never into the repo or the owner's folders.

### Launch and capture your own build

```bash
PROFILE=debug scripts/bundle-macos.sh
FIX=$(mktemp -d /tmp/haven-verify.XXXXXX); mkdir -p "$FIX/docs" "$FIX/src"; echo hi > "$FIX/a.txt"
open -g -n --env HAVEN_STATE_DIR=$(mktemp -d) --env HAVEN_DISPLAY=1 \
  target/bundle/Haven.app --args "$FIX"
sleep 4
PID=$(pgrep -n -f "$PWD/target/bundle/Haven.app/Contents/MacOS/haven")
read WID BOUNDS < <(swift scripts/haven-window.swift bounds "$PID")
screencapture -x -o -l "$WID" /tmp/haven-verify-window.png
```

Dev env vars:
- `HAVEN_DISPLAY=<n>`: open on display n (0-based). Use a screen the owner
  isn't using. A window hidden behind a full-screen app stops repainting, and
  its captures show a stale first frame (an endless "Loading…" is the tell).
- `HAVEN_WINDOW_SIZE=<w>x<h>`: initial size in points (minimum 900×560).
- `HAVEN_FONT_SMOOTHING=0|1`: overrides the Settings switch.

`scripts/visual-check.sh` (required, must exit 0) finds, captures and quits
its own instance by bundle path and PID, so it runs next to other Havens.
Never quit by bundle id (`quit app id "dev.haven.app"`): that can hit the
owner's copy. Note that the script brings its window forward, so the screen
rule above applies to it too.

### Driving the app

Keyboard input only reaches gpui when Haven is the key window, which needs
full-screen control. Only do this when the screen is free (see the ground
rules), then release control as soon as you're done. Walk the feature's
runbook, then the regression checklist in AGENTS.md.

## 4. Report

Post one PR comment per validation session
(`gh pr comment <n> --body-file report.md`):

- **Verdict:** pass, fail, or pass with pending items.
- Tier 1 and tier 2 results (test counts, any flake).
- Tier 3: each check as pass, fail or pending, with what you saw. Describe
  each screenshot; attach only window captures.
- Bugs found: steps to reproduce, expected and actual behaviour.

Per AGENTS.md, bugs you find get fixed on the same branch (with tests), then
re-verified.
