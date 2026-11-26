pub mod columns;
pub mod grid;
pub mod list;
pub mod preview;
pub mod start;

use gpui::{
    AnyElement, App, AppContext, Context, Entity, IntoElement, ParentElement, Styled, Window, div,
    img, px,
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
