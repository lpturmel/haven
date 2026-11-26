//! Dark palette tokens.

use gpui::{App, Global, Hsla, hsla, rgb, rgba};
use haven_core::TagColor;
use haven_highlight::Style;

#[derive(Clone)]
pub struct Theme {
    pub sidebar_bg: Hsla,
    /// Section headers and hints on the frosted sidebar.
    pub sidebar_label: Hsla,
    /// Row icons on the frosted sidebar.
    pub sidebar_icon: Hsla,
    pub card_bg: Hsla,
    pub card_border: Hsla,
    pub elevated: Hsla,
    pub popover_bg: Hsla,
    pub field_bg: Hsla,
    pub field_border: Hsla,
    pub divider: Hsla,
    pub text: Hsla,
    pub text_muted: Hsla,
    pub text_faint: Hsla,
    pub accent: Hsla,
    pub accent_text: Hsla,
    pub selection: Hsla,
    pub selection_inactive: Hsla,
    pub hover: Hsla,
    pub active: Hsla,
    pub danger: Hsla,
    pub success: Hsla,
    pub warning: Hsla,
    pub folder: Hsla,
    pub scrim: Hsla,
    pub code_bg: Hsla,
    pub gutter: Hsla,
}

impl Global for Theme {}

impl Theme {
    pub fn dark() -> Theme {
        Theme {
            // A light tint over the native sidebar material (installed in
            // main.rs), which does the blurring; together they keep sidebar
            // text legible over bright wallpapers.
            sidebar_bg: hsla(240. / 360., 0.06, 0.08, 0.35),
            sidebar_label: hsla(0., 0., 1., 0.62),
            sidebar_icon: hsla(0., 0., 1., 0.72),
            card_bg: rgb(0x1c1c1f).into(),
            card_border: hsla(0., 0., 1., 0.08),
            elevated: rgb(0x26262b).into(),
            popover_bg: rgb(0x2a2a30).into(),
            field_bg: rgb(0x141417).into(),
            field_border: hsla(0., 0., 1., 0.09),
            divider: hsla(0., 0., 1., 0.07),
            text: rgb(0xececf1).into(),
            text_muted: rgb(0x9a9aa5).into(),
            text_faint: rgb(0x6b6b75).into(),
            accent: rgb(0x0a84ff).into(),
            accent_text: rgb(0x4da3ff).into(),
            selection: rgba(0x0a84ff40).into(),
            selection_inactive: hsla(0., 0., 1., 0.09),
            hover: hsla(0., 0., 1., 0.05),
            active: hsla(0., 0., 1., 0.09),
            danger: rgb(0xff453a).into(),
            success: rgb(0x32d74b).into(),
            warning: rgb(0xffd60a).into(),
            folder: rgb(0x5ab0ff).into(),
            scrim: hsla(0., 0., 0., 0.45),
            code_bg: rgb(0x18181b).into(),
            gutter: rgb(0x55555e).into(),
        }
    }

    pub fn tag(&self, color: TagColor) -> Hsla {
        match color {
            TagColor::None => hsla(0., 0., 0.6, 0.0),
            TagColor::Red => rgb(0xff453a).into(),
            TagColor::Orange => rgb(0xff9f0a).into(),
            TagColor::Yellow => rgb(0xffd60a).into(),
            TagColor::Green => rgb(0x32d74b).into(),
            TagColor::Blue => rgb(0x0a84ff).into(),
            TagColor::Purple => rgb(0xbf5af2).into(),
            TagColor::Gray => rgb(0x98989d).into(),
        }
    }

    pub fn syntax(&self, style: Style) -> Hsla {
        let c = match style {
            Style::Keyword => 0xff7ab2,
            Style::String => 0xff8170,
            Style::Comment => 0x7f8c98,
            Style::Function => 0x67b7a4,
            Style::Type => 0x5dd8ff,
            Style::Constant => 0xd9c97c,
            Style::Number => 0xd9c97c,
            Style::Operator => 0xc8c8d0,
            Style::Punctuation => 0x9a9aa5,
            Style::Property => 0x8fd7e0,
            Style::Variable => 0xdfdfe6,
            Style::VariableBuiltin => 0xb281eb,
            Style::Tag => 0x5dd8ff,
            Style::Attribute => 0xffa14f,
            Style::Label => 0xffa14f,
            Style::Escape => 0xffa14f,
            Style::Constructor => 0xa167e6,
            Style::Module => 0xacf2e4,
            Style::Heading => 0x4da3ff,
            Style::Link => 0x67b7a4,
            Style::Emphasis => 0xdfdfe6,
            Style::Strong => 0xffffff,
            Style::Literal => 0xff8170,
        };
        rgb(c).into()
    }
}

pub fn theme(cx: &App) -> &Theme {
    cx.global::<Theme>()
}

pub const UI_FONT: &str = ".SystemUIFont";
pub const MONO_FONT: &str = "Menlo";
