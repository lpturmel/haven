# Runbook: text sharpness and font smoothing

Background: stock gpui 0.2.2 rasterizes glyphs without CoreText font
smoothing, so its text is thinner and softer than native AppKit text. You
see it most on 1× (non-Retina) displays; the owner's main screen is a
3440×1440 display at 1×. Haven's vendored gpui adds font smoothing (see
`vendor/gpui/HAVEN_PATCHES.md`), and Settings ▸ Appearance ▸ Font smoothing
toggles it. The UI also avoids fractional font sizes (11.5 or 12.5 px blur
at 1×). Check `grep -rn "text_size(px([0-9]*\.5))" crates/haven/src` is
empty.

## 1. Offscreen harness (no screen needed)

```bash
swift scripts/text-render-compare.swift /tmp/text-1x.png 1
ZOOM=4 swift scripts/text-render-compare.swift /tmp/text-1x-zoom.png 1
swift scripts/text-render-compare.swift /tmp/text-2x.png 2
```

It prints a per-section "softness" (share of partially covered ink pixels,
lower is crisper), inked pixels and mean coverage for stock gpui, Haven
(font smoothing) and AppKit (offscreen). Expect Haven's section to be at
least as crisp as stock with at least as much ink. Reference run
(2026-11-26, 1×): stock 0.620 softness / 4313 inked, Haven 0.597 / 4386,
AppKit offscreen 0.656 / 3810. The offscreen harness understates the
on-screen gain (step 2 measured +18% ink in the real app). Don't use
pixel-snapping as a fix: it measured softer (0.69).

## 2. Real app, A/B at 1×

Launch two copies of the same build, smoothing on and off, on a 1× display
(`HAVEN_DISPLAY` picks it; check the capture's pixel size equals the
window's point size, which means 1×):

```bash
for s in 1 0; do
  open -g -n --env HAVEN_STATE_DIR=$(mktemp -d) --env HAVEN_DISPLAY=1 \
    --env HAVEN_FONT_SMOOTHING=$s target/bundle/Haven.app --args "$FIX"; sleep 4
done
# capture each by PID (SKILL.md), then compare the same crop of each
```

Measure the list text region on both captures: count pixels whose green
channel sits between the card background `#1c1c1f` and text `#ececf1`.
The smoothed capture should carry noticeably more ink. On 2026-11-26 it was
+18% (5874 vs 4971 summed coverage over the fixture listing). Make a 3×
nearest-neighbour side-by-side crop for the PR. The smoothed version shows
solid one-pixel stems where the stock one shows faint two-pixel grey ones.

Visually: no clipped glyph edges (the patch grows raster bounds by 1px for
the dilation), emoji unchanged and still in colour, and selection or
coloured text (accent links, tag names) still the right colour.

## 3. Live toggle

With the screen free: open Settings, flip Font smoothing. Every window
repaints on the spot, with no restart. Flip back and capture to confirm both
states match the A/B captures (the glyph cache is keyed on the flag, so no
stale glyphs).

## 4. Retina sanity

Repeat a capture on a 2× display (pixel size = 2 × points). Text should be
crisp and only slightly heavier than before. If it looks bold or smeared at
2×, report it: smoothing could be limited to 1× displays in
`smoothing_applies` (vendor/gpui).
