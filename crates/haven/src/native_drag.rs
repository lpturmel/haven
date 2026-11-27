//! Dragging files out of Haven into other apps.
//!
//! Inside the window, file drags are gpui drags (`DraggedFiles`): drop
//! targets, highlights and move/copy rules all live in Haven. When such a
//! drag leaves the window, it is handed to AppKit as a native file drag, so
//! Finder, the Desktop, Mail, editors… can receive it.

use gpui::{App, Pixels, Point, Size, Window};

use crate::tab::DraggedFiles;

/// True once `position` (window coordinates) is outside the window.
pub fn outside_window(position: Point<Pixels>, viewport: Size<Pixels>) -> bool {
    let zero = Pixels::ZERO;
    position.x < zero
        || position.y < zero
        || position.x >= viewport.width
        || position.y >= viewport.height
}

/// Hands the in-app drag to AppKit. Returns true if a native drag started
/// (the gpui drag must then be cancelled: AppKit now owns the mouse).
pub fn hand_off(files: &DraggedFiles, window: &Window, cx: &mut App) -> bool {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let _ = cx;
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return false;
    };
    let RawWindowHandle::AppKit(h) = handle.as_raw() else {
        return false;
    };
    // SAFETY: gpui hands out the window's live content NSView, and we are
    // on the main thread inside its mouse-drag event.
    unsafe { haven_macos::begin_file_drag(h.ns_view.as_ptr(), &files.paths) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{point, px, size};

    #[test]
    fn detects_leaving_the_window() {
        let vp = size(px(800.), px(600.));
        assert!(!outside_window(point(px(0.), px(0.)), vp));
        assert!(!outside_window(point(px(799.), px(599.)), vp));
        assert!(outside_window(point(px(-1.), px(10.)), vp));
        assert!(outside_window(point(px(10.), px(-0.5)), vp));
        assert!(outside_window(point(px(800.), px(10.)), vp));
        assert!(outside_window(point(px(10.), px(600.)), vp));
    }
}
