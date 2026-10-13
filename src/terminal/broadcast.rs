//! Controlled presentation of broadcast membership, owned by the workspace.

use crate::app::{CenterTab, SetBroadcastTarget};
use crate::i18n::t;
use gpui_kit::component::{Disableable as _, Sizable as _, checkbox::Checkbox, h_flex};
use gpui_kit::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct BroadcastSelection {
    selected: bool,
    available: bool,
}

impl BroadcastSelection {
    pub(crate) fn new(selected: bool, available: bool) -> Self {
        Self {
            selected,
            available,
        }
    }

    pub(super) fn render(self, tab: CenterTab, name: SharedString) -> impl IntoElement {
        let id = match tab {
            CenterTab::Terminal(id) => ("broadcast-remote", id.0),
            CenterTab::LocalTerminal(id) => ("broadcast-local", id.0),
            _ => unreachable!("only terminals have broadcast checkboxes"),
        };
        let label = t!("workspace.broadcast.target", name = name);
        h_flex().size_5().justify_center().flex_shrink_0().child(
            Checkbox::new(id)
                .small()
                .checked(self.selected)
                .disabled(!self.available)
                .accessibility_label(label.clone())
                .tooltip(if self.available {
                    label
                } else {
                    t!("workspace.broadcast.unavailable")
                })
                // Membership must not select or start dragging the tab.
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_change(move |selected, window, cx| {
                    cx.stop_propagation();
                    window.dispatch_action(Box::new(SetBroadcastTarget::new(tab, *selected)), cx);
                }),
        )
    }
}
