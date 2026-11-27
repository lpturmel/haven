# Runbook: drag and drop in and out

Code:
- In: `views::file_drop_target` (`crates/haven/src/views/mod.rs`) accepts
  `DraggedFiles` (from Haven) and `ExternalPaths` (from Finder and other
  apps) with one handler and one highlight. It's used by list, grid and
  column rows, the column background, sidebar rows, the pin drop zone, tabs
  and path-bar crumbs.
- Out: `crates/haven/src/native_drag.rs`. When an in-app drag crosses the
  window edge, `haven_macos::begin_file_drag` starts a native `NSDraggingSession`
  (file URLs plus Finder icons), and the gpui drag is cancelled.
- gpui patch: `vendor/gpui/src/platform/mac/window.rs`. Without it, the
  synthetic-drag replay never stops after a native drag. See
  `vendor/gpui/HAVEN_PATCHES.md`.

Drop rules (same for both sources): move within a volume, copy across
volumes, ⌥ forces a copy. Dropping a folder onto itself is ignored.

## Fixture

```bash
SRC=$(mktemp -d /tmp/haven-dnd-src.XXXXXX); DST=$(mktemp -d /tmp/haven-dnd-dst.XXXXXX)
mkdir -p "$DST/inbox"; for f in one two three; do echo $f > "$SRC/$f.txt"; done
open -g "$SRC"        # a Finder window to drag from and to
```

Launch Haven on `$DST` (see SKILL.md). The interactive steps need
full-screen control: only when the screen is free.

## Drag in (Finder → Haven)

For each target, drag `one.txt` (then a two-file selection) from Finder:

| Target | Expect |
|---|---|
| List view background | file lands in `$DST` (moved: `$SRC` is on the same volume) |
| List row `inbox` | row highlights while hovered; file lands in `$DST/inbox` |
| Grid tile `inbox` (⌥⌘2) | same, tile highlights |
| Columns view (⌥⌘3) current column, and folder rows | lands in the folder |
| Sidebar place or pin row | row highlights; lands in that folder |
| Sidebar "Pinned" header area with a folder | folder gets pinned |
| A tab in the tab strip | lands in that tab's folder |
| A path-bar crumb | lands in that ancestor |

Hold ⌥ while dropping: the source file stays (copy). Check every result
with `ls`. The folder view refreshes live with no ⌘R.

## Drag out (Haven → other apps)

1. Drag `one.txt` from the list out of the window onto the Finder window
   showing `$SRC`. While the cursor is inside Haven you see Haven's own
   label preview. Once it leaves, the Finder icon follows the cursor, and
   the drop moves the file into `$SRC` (same volume, Finder's rule).
2. Drag a multi-selection out onto the Desktop: the stacked icons follow,
   and all items land.
3. Drag out, then back into Haven and drop on a folder: it works (it arrives
   as a native drop).
4. Drag out and release over nothing (or press Esc): the drag cancels and
   nothing moves.
5. **Runaway check** (the gpui patch): after each drag-out, Haven should go
   idle. `ps -o %cpu= -p <pid>` stays near 0 a few seconds later. A
   constant 5–20% means the synthetic drag never ended. Clicking and
   hovering rows must behave normally afterwards.
6. Dragging a sidebar pin (reordering) must never start a native drag.

Headless coverage when the screen is busy: `cargo test -p haven` covers
`native_drag::tests` (window-edge detection) and the drop rules
(`drops_move_within_a_volume_and_copy_with_option`). The native hand-off
itself needs a real mouse, so mark it **pending** rather than guessing.
