//! Fitting path-bar crumbs into the room the toolbar leaves them.
//!
//! When the whole path doesn't fit, the leading crumbs (the volume, then the
//! folders below it) collapse into one "…" crumb, so the current folder and
//! as many of its nearest parents as fit always stay visible. The current
//! folder itself is never hidden; the view ellipsizes it as a last resort.

use std::ops::Range;

/// The leading crumbs to hide behind a single "…" crumb, given each crumb's
/// width, the width a separator adds between two crumbs, the width of the
/// "…" crumb and the room available. Returns `0..k` for the smallest `k` that
/// makes the row fit (an empty range hides nothing), and never hides the last
/// crumb.
pub fn fit(widths: &[f32], sep: f32, more: f32, avail: f32) -> Range<usize> {
    let n = widths.len();
    let row = |shown: &[f32], with_more: bool| -> f32 {
        let items = shown.len() + usize::from(with_more);
        let more = if with_more { more } else { 0. };
        shown.iter().sum::<f32>() + more + sep * items.saturating_sub(1) as f32
    };
    // Unmeasured (first frame), a single crumb, or everything fits.
    if n < 2 || avail <= 0. || row(widths, false) <= avail {
        return 0..0;
    }
    let k = (1..n - 1)
        .find(|&k| row(&widths[k..], true) <= avail)
        .unwrap_or(n - 1);
    0..k
}

#[cfg(test)]
mod tests {
    use super::*;

    // Macintosh HD › Users › me › dev › haven, at made-up widths.
    const W: [f32; 5] = [110., 50., 30., 40., 60.];
    const SEP: f32 = 14.;
    const MORE: f32 = 22.;

    /// Width of the row with `0..k` collapsed into "…".
    fn width(k: usize) -> f32 {
        let (more, items) = if k == 0 { (0., 0) } else { (MORE, 1) };
        let shown = &W[k..];
        shown.iter().sum::<f32>() + more + SEP * (shown.len() + items - 1) as f32
    }

    #[test]
    fn everything_fits() {
        assert_eq!(fit(&W, SEP, MORE, width(0)), 0..0);
        assert_eq!(fit(&W, SEP, MORE, 2000.), 0..0);
    }

    #[test]
    fn unmeasured_or_single_crumb_hides_nothing() {
        assert_eq!(fit(&W, SEP, MORE, 0.), 0..0);
        assert_eq!(fit(&[400.], SEP, MORE, 100.), 0..0);
        assert_eq!(fit(&[], SEP, MORE, 100.), 0..0);
    }

    #[test]
    fn collapses_from_the_left_one_crumb_at_a_time() {
        // One point short: the volume goes first ("… › Users › me › dev › haven").
        assert_eq!(fit(&W, SEP, MORE, width(0) - 1.), 0..1);
        assert_eq!(fit(&W, SEP, MORE, width(2)), 0..2);
        assert_eq!(fit(&W, SEP, MORE, width(2) - 1.), 0..3);
        // "… › haven".
        assert_eq!(fit(&W, SEP, MORE, width(4)), 0..4);
    }

    #[test]
    fn the_current_folder_always_stays() {
        // Too narrow even for "… › haven": it stays (and the view ellipsizes it).
        assert_eq!(fit(&W, SEP, MORE, 30.), 0..4);
        assert_eq!(fit(&[300., 900.], SEP, MORE, 50.), 0..1);
    }

    #[test]
    fn hides_as_little_as_possible_and_shrinks_monotonically() {
        let mut last = 0;
        for avail in (1..=500).rev() {
            let avail = avail as f32;
            let h = fit(&W, SEP, MORE, avail);
            assert_eq!(h.start, 0);
            assert!(h.end < W.len(), "avail {avail}: hid the current folder");
            assert!(h.end >= last, "avail {avail}: showed more in less room");
            last = h.end;
            if h.end > 0 && width(h.end) <= avail {
                assert!(width(h.end - 1) > avail, "avail {avail}: hid too much");
            }
        }
    }
}
