# Vendored gpui 0.2.2 (with Haven patches)

This is gpui 0.2.2 from crates.io (`src/`, `build.rs`, `resources/`, `docs/`,
licence), with the examples dropped. The workspace uses it through
`[patch.crates-io]` in the root `Cargo.toml`. It is excluded from the
workspace, so `cargo clippy --workspace` and `cargo test --workspace` skip it.

Every change is marked `HAVEN PATCH` in the source. The full diff against the
crates.io release is in `haven-patches.diff`:

```bash
diff -ru ~/.cargo/registry/src/*/gpui-0.2.2/src vendor/gpui/src
```

## 1. Font smoothing (sharper text, mainly on non-Retina displays)

`src/platform/mac/text_system.rs`, `src/text_system.rs`, `src/window.rs`

Stock gpui rasterizes glyphs into an alpha-only bitmap. CoreText never
applies font smoothing ("stem darkening") there, so gpui text comes out
thinner and softer than native AppKit text. The difference is most visible on
1x displays.

With the patch, glyphs are drawn white on black into an opaque RGB bitmap
with `CGContextSetShouldSmoothFonts(true)`. The brightest channel becomes the
coverage, and raster bounds grow by 1px per side to make room for the
dilation. Emoji are unchanged.

- `gpui::set_font_smoothing(bool)` and `gpui::font_smoothing()` toggle it at
  runtime. The flag is part of `RenderGlyphParams`, the glyph-atlas key, so
  toggling re-rasterizes glyphs instead of reusing stale ones.
- In Haven it's the Settings ▸ Appearance ▸ Font smoothing switch, and
  `HAVEN_FONT_SMOOTHING=0|1` overrides it for before/after captures.
- `scripts/text-render-compare.swift` compares stock, smoothed and AppKit
  rendering offscreen.

## 2. Synthetic drag stops when no button is held

`src/platform/mac/window.rs`, `handle_view_event`

While the mouse button is held, gpui replays the last drag event every 16ms
until a mouse-up. When Haven hands a file drag to AppKit (dragging files out
of the window, see `crates/haven/src/native_drag.rs`), the native drag session
swallows that mouse-up, and the replay would run forever. The loop now also
stops on any mouse move that has no button pressed.

## 3. Warning fix

`src/taffy.rs`: explicit `f32` literals (the `float_literal_f32_fallback`
future-incompatibility warning).

## Updating gpui

Copy the new release over `vendor/gpui`, re-apply the `HAVEN PATCH` hunks
(`haven-patches.diff`), drop the `[[example]]` tables from `Cargo.toml`, then
run the font check in `.claude/skills/verify/references/font-rendering.md`.
