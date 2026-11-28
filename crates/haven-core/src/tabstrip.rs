//! Tab strip geometry.

/// Width of each of `count` tabs sharing `free` points of room.
///
/// This reproduces the old flexbox layout, where `count` tabs and one
/// trailing spacer split the strip's free room equally (`flex: 1` each) and
/// each tab was held between `min` and `max`: every tab gets
/// `free / (count + 1)`, clamped to `min..=max`. Past `min` the tabs no
/// longer fit and the strip scrolls instead of shrinking them further.
///
/// No tabs, a negative or non-finite `free` give `min`.
pub fn tab_width(free: f32, count: usize, min: f32, max: f32) -> f32 {
    if count == 0 || !free.is_finite() || free < 0. {
        return min;
    }
    let share = free / (count as f32 + 1.);
    // `max(min)` keeps `clamp` from panicking on a min above max.
    share.clamp(min, max.max(min))
}

/// Total width of `count` tabs of width `tab_w` with `gap` between them.
pub fn content_width(count: usize, tab_w: f32, gap: f32) -> f32 {
    if count == 0 {
        return 0.;
    }
    count as f32 * tab_w + (count - 1) as f32 * gap
}

/// Width of the scrolling viewport that holds the tabs, in a strip `inner`
/// wide that also holds the "+" button (`new_tab_w`) and a trailing spacer,
/// each `gap` apart: the tabs' own width while they fit, else what is left.
pub fn viewport_width(inner: f32, count: usize, tab_w: f32, gap: f32, new_tab_w: f32) -> f32 {
    content_width(count, tab_w, gap)
        .min(inner - new_tab_w - 2. * gap)
        .max(0.)
}

/// The horizontal scroll offset (0 or negative, as gpui's `ScrollHandle`
/// keeps it) that brings tab `index` fully into a viewport `viewport_w` wide,
/// moving the current `offset` as little as possible and never past the
/// first or last tab. A tab wider than the viewport lines up with its left
/// edge.
pub fn reveal_offset(
    offset: f32,
    index: usize,
    count: usize,
    tab_w: f32,
    gap: f32,
    viewport_w: f32,
) -> f32 {
    if index >= count || !(offset.is_finite() && tab_w.is_finite() && viewport_w.is_finite()) {
        return offset;
    }
    let left = index as f32 * (tab_w + gap);
    let right = left + tab_w;
    let mut o = offset;
    if left + o < 0. || tab_w > viewport_w {
        o = -left;
    } else if right + o > viewport_w {
        o = viewport_w - right;
    }
    let max = (content_width(count, tab_w, gap) - viewport_w).max(0.);
    o.clamp(-max, 0.)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: f32 = 100.;
    const MAX: f32 = 200.;

    #[test]
    fn few_tabs_with_room_to_spare_get_the_max() {
        assert_eq!(tab_width(2000., 3, MIN, MAX), MAX);
        assert_eq!(tab_width(5000., 1, MIN, MAX), MAX);
    }

    #[test]
    fn mid_range_shares_the_room_with_the_spacer() {
        assert_eq!(tab_width(600., 3, MIN, MAX), 150.);
        let w = tab_width(700., 4, MIN, MAX);
        assert!((w - 140.).abs() < 1e-4, "{w}");
    }

    #[test]
    fn many_tabs_or_little_room_get_the_min() {
        assert_eq!(tab_width(700., 20, MIN, MAX), MIN);
        assert_eq!(tab_width(150., 1, MIN, MAX), MIN);
        assert_eq!(tab_width(0., 5, MIN, MAX), MIN);
    }

    #[test]
    fn degenerate_inputs_give_the_min() {
        assert_eq!(tab_width(800., 0, MIN, MAX), MIN);
        assert_eq!(tab_width(-50., 3, MIN, MAX), MIN);
        assert_eq!(tab_width(f32::NAN, 3, MIN, MAX), MIN);
        assert_eq!(tab_width(f32::INFINITY, 3, MIN, MAX), MIN);
        // A min above the max doesn't panic.
        assert_eq!(tab_width(100., 3, 60., 40.), 60.);
    }

    #[test]
    fn viewport_is_the_tabs_while_they_fit_then_the_rest_of_the_strip() {
        assert_eq!(content_width(0, 100., 4.), 0.);
        assert_eq!(content_width(3, 200., 4.), 608.);
        // 3 × 200 + 2 × 4 fits in 2000 - 26 - 8.
        assert_eq!(viewport_width(2000., 3, 200., 4., 26.), 608.);
        // 20 × 100 doesn't fit in 652: the viewport takes 652 - 26 - 8.
        assert_eq!(viewport_width(652., 20, 100., 4., 26.), 618.);
        assert_eq!(viewport_width(10., 20, 100., 4., 26.), 0.);
    }

    #[test]
    fn reveal_scrolls_as_little_as_possible() {
        let (n, w, gap, vw) = (20, 100., 4., 618.);
        // Already visible: nothing moves.
        assert_eq!(reveal_offset(0., 2, n, w, gap, vw), 0.);
        assert_eq!(reveal_offset(-104., 3, n, w, gap, vw), -104.);
        // Off to the right: its right edge lines up with the viewport's.
        let o = reveal_offset(0., 10, n, w, gap, vw);
        assert_eq!(o, vw - (10. * 104. + 100.));
        // Off to the left: its left edge lines up with the viewport's.
        assert_eq!(reveal_offset(-800., 2, n, w, gap, vw), -208.);
        // The first and the last tab sit at the two ends.
        assert_eq!(reveal_offset(-900., 0, n, w, gap, vw), 0.);
        let max = content_width(n, w, gap) - vw;
        assert_eq!(reveal_offset(0., 19, n, w, gap, vw), -max);
    }

    #[test]
    fn reveal_stays_in_range() {
        // Everything fits: always 0, even from a stale offset.
        assert_eq!(reveal_offset(-300., 2, 3, 200., 4., 608.), 0.);
        // A stale offset past the end is pulled back.
        let max = content_width(20, 100., 4.) - 618.;
        assert_eq!(reveal_offset(-5000., 19, 20, 100., 4., 618.), -max);
        // A viewport narrower than a tab shows the tab's start.
        assert_eq!(reveal_offset(0., 5, 20, 100., 4., 60.), -520.);
        // Out-of-range or non-finite input leaves the offset alone.
        assert_eq!(reveal_offset(-10., 20, 20, 100., 4., 618.), -10.);
        assert_eq!(reveal_offset(-10., 1, 20, 100., 4., f32::NAN), -10.);
    }
}
