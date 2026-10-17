//! Controlled presentation of broadcast membership, owned by the workspace:
//! a checkbox ahead of a terminal tab's mark, which keeps telling whether
//! the terminal is connected.

use crate::app::{CenterTab, SetBroadcastTarget};
use crate::i18n::t;
use gpui_kit::component::{Disableable as _, Sizable as _, checkbox::Checkbox, h_flex};
use gpui_kit::*;

#[derive(Clone, PartialEq)]
pub(crate) struct BroadcastSelection {
    selected: bool,
    available: bool,
    /// The workspace's, which handles the checkbox: focus may be nowhere
    /// when it is clicked, right after a dialog closed.
    target: FocusHandle,
}

impl BroadcastSelection {
    pub(crate) fn new(selected: bool, available: bool, target: FocusHandle) -> Self {
        Self {
            selected,
            available,
            target,
        }
    }

    /// A tab's leading `mark`, after its checkbox while 命令广播 is on.
    pub(super) fn lead(
        selection: Option<Self>,
        tab: CenterTab,
        name: SharedString,
        mark: impl IntoElement,
    ) -> AnyElement {
        match selection {
            Some(selection) => h_flex()
                .gap_1()
                .child(selection.render(tab, name))
                .child(mark)
                .into_any_element(),
            None => mark.into_any_element(),
        }
    }

    fn render(self, tab: CenterTab, name: SharedString) -> impl IntoElement {
        let target = self.target;
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
                    target.dispatch_action(&SetBroadcastTarget::new(tab, *selected), window, cx);
                }),
        )
    }
}
