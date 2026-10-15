use gpui_kit::component::{
    ActiveTheme as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    menu::PopupMenu,
    spinner::Spinner,
};
use gpui_kit::*;

use super::{
    model::{KubernetesCommand, ResourceKind, Tone},
    mutation::{self, ResourceOperation},
    panel::defer_dispatch,
};
use crate::{
    app::{CatalogIcon, KubernetesAction},
    i18n::t,
};

pub(super) fn resource_menu(
    mut menu: PopupMenu,
    kind: ResourceKind,
    namespace: &str,
    name: &str,
    dispatch: &FocusHandle,
) -> PopupMenu {
    menu = menu.action_context(dispatch.clone());
    for operation in [
        ResourceOperation::Edit,
        ResourceOperation::Restart,
        ResourceOperation::Delete,
    ] {
        let supported = match operation {
            ResourceOperation::Edit => mutation::editable(kind),
            ResourceOperation::Restart => mutation::restart_kind(kind),
            ResourceOperation::Delete => mutation::deletable(kind),
        };
        if !supported {
            continue;
        }
        if operation == ResourceOperation::Delete {
            menu = menu.separator();
        }
        let icon = match operation {
            ResourceOperation::Edit => CatalogIcon::Pencil,
            ResourceOperation::Restart => CatalogIcon::RotateCw,
            ResourceOperation::Delete => CatalogIcon::Trash,
        };
        menu = menu.menu_with_icon(
            operation.label(),
            icon,
            Box::new(KubernetesAction(KubernetesCommand::Manage {
                kind,
                namespace: namespace.to_owned(),
                name: name.to_owned(),
                operation,
            })),
        );
    }
    menu.separator().menu_with_icon(
        t!("k8s.copy_name"),
        IconName::Copy,
        Box::new(KubernetesAction(KubernetesCommand::CopyValue(
            name.to_owned(),
        ))),
    )
}

pub(super) fn tone_color(tone: Tone, cx: &App) -> Hsla {
    match tone {
        Tone::Good => cx.theme().success,
        Tone::Warning => cx.theme().warning,
        Tone::Danger => cx.theme().danger,
        Tone::Quiet => cx.theme().muted_foreground,
    }
}

pub(super) fn reading(id: &'static str, cx: &App) -> AnyElement {
    h_flex()
        .id(id)
        .test_support()
        .gap_2()
        .justify_center()
        .py_3()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(Spinner::new().small())
        .child(t!("tools.reading"))
        .into_any_element()
}

pub(super) fn copy_button(
    id: impl Into<ElementId>,
    value: String,
    dispatch: &FocusHandle,
) -> Button {
    let handle = dispatch.clone();
    Button::new(id)
        .ghost()
        .small()
        .icon(IconName::Copy)
        .tooltip(t!("history.menu.copy"))
        .accessibility_label(t!("history.menu.copy"))
        .on_click(move |_, window, cx| {
            defer_dispatch(
                &handle,
                &KubernetesAction(KubernetesCommand::CopyValue(value.clone())),
                window,
                cx,
            )
        })
}
