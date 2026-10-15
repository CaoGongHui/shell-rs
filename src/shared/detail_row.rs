//! Aligned, selectable values for resource inspectors.
use gpui_kit::base::SelectableText;
use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::*;

pub fn detail_row(
    id: ElementId,
    label: SharedString,
    value: SharedString,
    cx: &App,
) -> impl IntoElement + InteractiveElement + ParentElement + Styled + use<> {
    h_flex()
        .id(id.clone())
        .test_support()
        .aria_label(value.clone())
        .items_start()
        .gap_3()
        .px_3()
        .py_2()
        .text_sm()
        .child(
            div()
                .w(rems(9.))
                .flex_shrink_0()
                .text_color(cx.theme().muted_foreground)
                .child(label),
        )
        .child(
            div()
                .id((id.clone(), "value-cell"))
                .test_support()
                .flex_1()
                .min_w_0()
                .child(
                    SelectableText::new((id, "value"), value).selection_color(cx.theme().selection),
                ),
        )
}
