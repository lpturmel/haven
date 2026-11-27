pub mod columns;
pub mod grid;
pub mod list;
pub mod preview;
pub mod start;

use std::path::PathBuf;
use std::rc::Rc;

use gpui::{
    AnyElement, App, AppContext, Context, Entity, ExternalPaths, Hsla, InteractiveElement,
    IntoElement, ParentElement, Styled, Window, div, img, px,
};
use haven_core::Entry;

use crate::icons::{IconSize, entry_icon};
use crate::tab::{DraggedFiles, Tab};
use crate::theme::theme;
use crate::ui::{fallback_icon, icon};

/// Native icon with a vector fallback while it loads.
pub fn entry_icon_el(entry: &Entry, size: f32, cx: &mut App) -> AnyElement {
    let class = if size > 32. {
        IconSize::Large
    } else {
        IconSize::Small
    };
    match entry_icon(entry, class, cx) {
        Some(image) => img(image).size(px(size)).flex_none().into_any_element(),
        None => {
            let color = if entry.is_dir {
                theme(cx).folder
            } else {
                theme(cx).text_muted
            };
            div()
                .size(px(size))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    icon(fallback_icon(entry.is_dir, &entry.ext))
                        .size(px(size * 0.85))
                        .text_color(color),
                )
                .into_any_element()
        }
    }
}

/// Floating label shown under the cursor while dragging files.
pub struct DragPreview {
    pub label: String,
}

impl gpui::Render for DragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        div()
            .px(px(10.))
            .py(px(4.))
            .rounded(px(6.))
            .bg(t.accent)
            .text_color(gpui::white())
            .text_size(px(12.))
            .child(self.label.clone())
    }
}

pub fn drag_preview(files: &DraggedFiles, cx: &mut App) -> Entity<DragPreview> {
    let label = match files.paths.as_slice() {
        [one] => one
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        many => format!("{} items", many.len()),
    };
    cx.new(|_| DragPreview { label })
}

/// The selection (or the single item under the cursor) as a drag payload.
pub fn drag_payload(tab: &Tab, ix: usize) -> DraggedFiles {
    if tab.selection.contains(&ix) {
        DraggedFiles {
            paths: tab.selected_paths(),
        }
    } else {
        DraggedFiles {
            paths: tab
                .entries
                .get(ix)
                .map(|e| vec![e.path.clone()])
                .unwrap_or_default(),
        }
    }
}

/// Files dropped on a target.
pub struct Dropped {
    pub paths: Vec<PathBuf>,
    /// The drop must copy (⌥ held, or the source app doesn't allow moving
    /// its files). Otherwise it moves within a volume and copies across.
    pub copy: bool,
}

/// Copy instead of move? ⌥ always copies. Drops from other apps also copy
/// when their source doesn't allow moving (an editor's file tree, a mail
/// attachment): only Finder-like sources hand their files over.
pub fn must_copy(option_held: bool, external: bool, source_allows_move: bool) -> bool {
    option_held || (external && !source_allows_move)
}

fn dropped(paths: Vec<PathBuf>, external: bool, window: &Window) -> Dropped {
    // gpui's cached modifiers miss ⌥ during a drag from another app (Haven
    // isn't active then), so also ask AppKit for the live state.
    let option = window.modifiers().alt || haven_macos::option_key_down();
    let allows_move = !external || gpui::external_drag_allows_move();
    Dropped {
        paths,
        copy: must_copy(option, external, allows_move),
    }
}

/// Makes `el` accept files dropped from inside Haven (`DraggedFiles`) and
/// from other apps such as Finder (`ExternalPaths`), tinted with `highlight`
/// while a drag hovers it. Both kinds go through the same `on_drop`, so drops
/// follow the same move/copy rules wherever they come from.
pub fn file_drop_target<E, V>(
    el: E,
    highlight: Hsla,
    cx: &Context<V>,
    on_drop: impl Fn(&mut V, Dropped, &mut Window, &mut Context<V>) + 'static,
) -> E
where
    E: InteractiveElement,
    V: 'static,
{
    let on_drop = Rc::new(on_drop);
    let internal = on_drop.clone();
    el.drag_over::<DraggedFiles>(move |s, _, _, _| s.bg(highlight))
        .drag_over::<ExternalPaths>(move |s, _, _, _| s.bg(highlight))
        .on_drop(cx.listener(move |v, files: &DraggedFiles, window, cx| {
            cx.stop_propagation();
            internal(v, dropped(files.paths.clone(), false, window), window, cx)
        }))
        .on_drop(cx.listener(move |v, ext: &ExternalPaths, window, cx| {
            cx.stop_propagation();
            on_drop(v, dropped(ext.paths().to_vec(), true, window), window, cx)
        }))
}

#[cfg(test)]
mod tests {
    use super::must_copy;

    #[test]
    fn drops_copy_with_option_or_from_copy_only_sources() {
        // In-app drags: move unless ⌥.
        assert!(!must_copy(false, false, false));
        assert!(must_copy(true, false, false));
        // From Finder (allows move): move unless ⌥.
        assert!(!must_copy(false, true, true));
        assert!(must_copy(true, true, true));
        // From an app that only lends a copy: always copy.
        assert!(must_copy(false, true, false));
    }
}
