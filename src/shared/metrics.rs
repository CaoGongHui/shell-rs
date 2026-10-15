//! Resource indicators shared by system monitoring and Kubernetes.
use super::{format_bytes, format_percent};
use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable as _,
    group_box::{GroupBox, GroupBoxVariants as _},
    h_flex,
    progress::{Progress, ProgressCircle},
    v_flex,
};
use gpui_kit::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Load {
    Normal,
    High,
    Critical,
}
impl Load {
    pub fn of(percent: f32) -> Self {
        if percent >= 90. {
            Self::Critical
        } else if percent >= 70. {
            Self::High
        } else {
            Self::Normal
        }
    }
}

/// A card: its icon and title, with `meta` at the end of the title row.
pub fn card(
    id: &'static str,
    icon: impl Into<Icon>,
    title: impl Into<SharedString>,
    meta: Option<AnyElement>,
    cx: &App,
) -> GroupBox {
    GroupBox::new().id(id).fill().child(
        h_flex()
            .gap_2()
            .child(icon.into().small().text_color(cx.theme().muted_foreground))
            .child(
                div()
                    .flex_1()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child(title.into()),
            )
            .children(meta),
    )
}

pub fn load_color(percent: f32, cx: &App) -> Hsla {
    match Load::of(percent) {
        Load::Normal => cx.theme().success,
        Load::High => cx.theme().warning,
        Load::Critical => cx.theme().danger,
    }
}

/// Numbers that change every reading: monospaced, so they hold still.
pub fn numeric(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .font_family(cx.theme().mono_font_family.clone())
        .child(text.into())
}

pub fn gauge(id: &'static str, percent: Option<f32>, cx: &App) -> impl IntoElement {
    let color = percent.map_or(cx.theme().muted_foreground, |percent| {
        load_color(percent, cx)
    });
    ProgressCircle::new(id)
        .value(percent.unwrap_or(0.))
        .color(color)
        .size_16()
        .child(
            v_flex()
                .items_center()
                .text_color(color)
                .child(
                    numeric(
                        percent.map_or("—".into(), |p| format!("{}", p.round() as u32)),
                        cx,
                    )
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD),
                )
                .child(div().text_xs().child("%")),
        )
}

/// A labelled bar: the label and the share above it.
pub fn meter(
    id: &'static str,
    label: SharedString,
    percent: Option<f32>,
    cx: &App,
) -> impl IntoElement {
    let color = percent.map_or(cx.theme().muted_foreground, |percent| {
        load_color(percent, cx)
    });
    let text = percent.map_or("—".into(), format_percent);
    v_flex()
        .gap_1p5()
        .child(
            h_flex().justify_between().text_sm().child(label).child(
                numeric(text.clone(), cx)
                    .id(id)
                    .test_support()
                    .aria_label(text)
                    .text_color(color)
                    .font_weight(FontWeight::SEMIBOLD),
            ),
        )
        .child(
            Progress::new(SharedString::from(format!("{id}-bar")))
                .small()
                .value(percent.unwrap_or(0.))
                .color(color),
        )
}

pub fn amount(id: &'static str, used: u64, total: u64, cx: &App) -> impl IntoElement {
    let (used, total) = (format_bytes(used), format_bytes(total));
    h_flex()
        .id(id)
        .test_support()
        .aria_label(format!("{used} / {total}"))
        .gap_1()
        .text_xs()
        .child(numeric(used, cx))
        .child(numeric(format!("/ {total}"), cx).text_color(cx.theme().muted_foreground))
}
