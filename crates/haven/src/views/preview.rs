//! Preview pane: tree-sitter highlighted text, images, folder and file info.

use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, ObjectFit, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, StyledImage, StyledText, Window, div, img,
    prelude::FluentBuilder, px, uniform_list,
};
use haven_core::kind::kind_label;
use haven_core::{Entry, fmt, tildify};

use super::entry_icon_el;
use crate::store::Store;
use crate::tab::{Preview, Tab};
use crate::theme::{MONO_FONT, theme};
use crate::ui::{empty_state, tag_dot};

pub fn render(tab: &mut Tab, _window: &mut Window, cx: &mut Context<Tab>) -> AnyElement {
    let t = theme(cx).clone();
    let body: AnyElement = match &tab.preview {
        Preview::None => empty_state("eye", "Select an item to preview", cx),
        Preview::Loading => div().size_full().into_any_element(),
        Preview::Multi { count, size } => empty_state(
            "copy",
            format!("{count} items selected · {}", fmt::size(*size)),
            cx,
        ),
        Preview::Text(tp) => {
            let lines = tp.lines.clone();
            let runs = tp.runs.clone();
            let gutter_w = (lines.len().to_string().len() as f32) * 7.5 + 16.;
            let header = format!(
                "{} · {} lines · {}{}",
                tp.lang.unwrap_or("Plain text"),
                lines.len(),
                haven_core::fmt::size(tp.bytes),
                if tp.truncated {
                    " (preview truncated)"
                } else {
                    ""
                }
            );
            div()
                .size_full()
                .flex()
                .flex_col()
                .child(
                    div()
                        .flex_none()
                        .px(px(12.))
                        .py(px(6.))
                        .text_size(px(11.))
                        .text_color(t.text_faint)
                        .border_b_1()
                        .border_color(t.divider)
                        .child(header),
                )
                .child(
                    uniform_list("preview-lines", lines.len(), move |range, _w, cx| {
                        let t = theme(cx);
                        range
                            .map(|i| {
                                let line = lines[i].clone();
                                let text: AnyElement = match runs.as_ref().and_then(|r| r.get(i)) {
                                    Some(r) if !line.is_empty() => StyledText::new(line)
                                        .with_runs(r.clone())
                                        .into_any_element(),
                                    _ => div().child(line).into_any_element(),
                                };
                                div()
                                    .flex()
                                    .h(px(18.))
                                    .child(
                                        div()
                                            .w(px(gutter_w))
                                            .flex_none()
                                            .pr(px(10.))
                                            .flex()
                                            .justify_end()
                                            .text_color(t.gutter)
                                            .child(SharedString::from((i + 1).to_string())),
                                    )
                                    .child(div().whitespace_nowrap().child(text))
                                    .into_any_element()
                            })
                            .collect::<Vec<_>>()
                    })
                    .track_scroll(tab.preview_scroll.clone())
                    .flex_1()
                    .py(px(6.))
                    .font_family(MONO_FONT)
                    .text_size(px(12.))
                    .line_height(px(18.))
                    .text_color(t.text)
                    .bg(t.code_bg),
                )
                .into_any_element()
        }
        Preview::Image(entry) => {
            let entry = entry.clone();
            div()
                .size_full()
                .flex()
                .flex_col()
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .p(px(16.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(if haven_core::kind::is_image_ext(&entry.ext) {
                            img(entry.path.clone())
                                .size_full()
                                .object_fit(ObjectFit::Contain)
                                .rounded(px(6.))
                                .into_any_element()
                        } else {
                            // HEIC / RAW / PSD: a native ImageIO thumbnail.
                            match crate::icons::entry_icon(
                                &entry,
                                crate::icons::IconSize::Preview,
                                cx,
                            ) {
                                Some(image) => img(image)
                                    .size_full()
                                    .object_fit(ObjectFit::Contain)
                                    .rounded(px(6.))
                                    .into_any_element(),
                                None => entry_icon_el(&entry, 96., cx),
                            }
                        }),
                )
                .child(info_table(&entry, cx))
                .into_any_element()
        }
        Preview::Folder { entry, count } => {
            let entry = entry.clone();
            let count = *count;
            hero(
                &entry,
                count.map(|c| {
                    if c == 1 {
                        "1 item".to_string()
                    } else {
                        format!("{c} items")
                    }
                }),
                cx,
            )
        }
        Preview::Info(entry) => {
            let entry = entry.clone();
            hero(&entry, None, cx)
        }
    };
    div()
        .id("preview")
        .size_full()
        .overflow_hidden()
        .bg(t.card_bg)
        .child(body)
        .into_any_element()
}

fn hero(entry: &Entry, subtitle: Option<String>, cx: &mut Context<Tab>) -> AnyElement {
    let t = theme(cx).clone();
    div()
        .id("preview-hero")
        .size_full()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .items_center()
        .pt(px(36.))
        .gap(px(8.))
        .child(entry_icon_el(entry, 96., cx))
        .child(
            div()
                .px(px(16.))
                .text_size(px(15.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.text)
                .text_center()
                .child(entry.name.clone()),
        )
        .when_some(subtitle, |d, s| {
            d.child(div().text_size(px(12.)).text_color(t.text_muted).child(s))
        })
        .child(div().w_full().mt(px(12.)).child(info_table(entry, cx)))
        .into_any_element()
}

fn info_table(entry: &Entry, cx: &mut Context<Tab>) -> AnyElement {
    let t = theme(cx).clone();
    let store_tags = Store::get(cx).tags.clone();
    let mut rows: Vec<(&str, String)> = vec![("Kind", kind_label(entry))];
    if let Some(size) = entry.effective_size() {
        rows.push(("Size", format!("{} ({} bytes)", fmt::size(size), size)));
    }
    rows.push(("Created", fmt::date(entry.created)));
    rows.push(("Modified", fmt::date(entry.modified)));
    if let Some(parent) = entry.path.parent() {
        rows.push(("Where", tildify(parent)));
    }
    div()
        .w_full()
        .px(px(16.))
        .py(px(10.))
        .flex()
        .flex_col()
        .gap(px(6.))
        .border_t_1()
        .border_color(t.divider)
        .text_size(px(12.))
        .children(rows.into_iter().map(|(k, v)| {
            div()
                .flex()
                .gap(px(10.))
                .child(
                    div()
                        .w(px(70.))
                        .flex_none()
                        .text_color(t.text_faint)
                        .child(k),
                )
                .child(div().flex_1().min_w_0().text_color(t.text).child(v))
        }))
        .when(!entry.tags.is_empty(), |d| {
            d.child(
                div()
                    .flex()
                    .gap(px(10.))
                    .child(
                        div()
                            .w(px(70.))
                            .flex_none()
                            .text_color(t.text_faint)
                            .child("Tags"),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(8.))
                            .children(entry.tags.iter().map(|tag| {
                                let color = store_tags
                                    .iter()
                                    .find(|s| s.name == tag.name)
                                    .map(|s| s.color)
                                    .unwrap_or(tag.color);
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(4.))
                                    .child(tag_dot(t.tag(color), px(8.)))
                                    .child(div().text_color(t.text).child(tag.name.clone()))
                            })),
                    ),
            )
        })
        .into_any_element()
}
