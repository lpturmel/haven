# Runbook: hover, spacing and List columns

## Conventions to check against (in the diff and on screen)

Rows that highlight on hover follow one recipe:

| Surface | Height | Inset (px) | Icon to label gap | Radius | Hover fill |
|---|---|---|---|---|---|
| Sidebar rows (home, pins, places, drives, tags) | 28 | 8 | 8 (icon or tag dot in a 15px column) | 6 | `sidebar_hover` (7% white, over the frosted material) |
| List rows | 26 | 4 + 6 cell padding (icon under the "Name" header) | 8 | 6 | `hover` |
| Column rows | 24 | 8 | 8 | 6 | `hover` |
| Palette, fuzzy, tag-editor rows | 28–32 | 10 | 10 | 6 | `hover` |
| Small inline buttons (sidebar and Start page eject, unpin) | 20×20 | n/a | n/a | 4 | `control_hover` (14%) |
| Toolbar segments, tabs, menu items, completions | n/a | n/a | n/a | 6 | `hover` |

Font sizes are whole pixels only (no `.5`). Every sidebar label starts at
the same x, tags included, and long labels end in "…".

Rules that a validator caught once, so check them on every change:
- **Hover never replaces a "selected" or "on" fill.** In gpui a `.hover()`
  refinement replaces the base `bg`, so guard it with `when(!selected, …)`,
  or re-assert the fill inside the hover.
- **Disabled controls don't hover.** No fill, default cursor.
- **No 1px shifts.** Borders that come and go (active tab, cursor ring) are
  always present and transparent when off.
- **Fades apply to content, not to fills.** Hidden items fade their icon and
  label, so their hover and selection fills match other rows.
- **Grid labels** are at most two lines, laid out by `grid::label_lines`,
  with the second ending in "…". Never use `line_clamp(2)` with a centred
  ellipsis: gpui truncates it as one double-width line, so line 2
  overflows.

## List columns (the narrow-window bug)

Before the fix, a narrow list squeezed the Name column to nothing, and dates
drew over the icons. Now `views::list::Columns::for_width` drops Kind, then
Date Modified, then Size, keeping Name at least 180px wide.

- Headless: `cargo test -p haven list::tests`.
- In the app: launch with `HAVEN_WINDOW_SIZE=900x560` and the preview pane on
  (list about 300px wide). Expect Name and Size only, names readable, with
  ellipsis. At the default 1240×780: Name, Date Modified, Size, Kind. Hide
  the preview (⌥⌘P) or widen the window: columns come back. Headers always
  sit exactly over their cells, and row icons line up under "Name".

## Hover walk (screen free)

Move the pointer slowly down each surface and check:

1. Sidebar: every row's highlight has the same height, inset and corner
   radius, and labels don't shift on hover. Pins show the unpin ✕ only on
   hover, as a 20px rounded button. Drive rows always show the eject
   button, and hovering it highlights the button, not just the row.
   Tooltips ("Eject", "Remove from Sidebar") appear in the UI font.
2. List, grid and columns: the highlight spans the row with even margins on
   both sides, hover and selection never stack into a third colour, and the
   highlight follows the pointer while scrolling with the wheel (no stale
   hovered row).
3. Toolbar, tab strip, path-bar crumbs, context menu and palette: hover
   fills are consistent and nothing shifts by a pixel when hovered.
4. Capture a crop of each with the pointer over a row
   (`screencapture -l <wid>`). Zoom 3× (nearest neighbour) to compare the
   insets.
