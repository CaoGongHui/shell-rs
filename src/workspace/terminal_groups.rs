//! 命令广播. Dock owns tab dragging. The workspace owns the explicitly
//! checked, live connections that share input, the command bar above them
//! and the split layout they are arranged in; reconnects never inherit
//! membership.

use gpui_kit::component::dock::{DockArea, DockPlacement, InsertTarget, PaneRef, PanelId};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{Placement, WindowExt as _};
use gpui_kit::*;

use super::Workspace;
use crate::app::{
    BroadcastSnippet, CenterTab, InterruptBroadcast, SendBroadcastCommand, SetBroadcastTarget,
    ToggleBroadcastKeys, ToggleTerminalBroadcast,
};
use crate::i18n::{join_list, t};
use crate::shared::RenamableTab as _;
use crate::terminal::{BroadcastSelection, TerminalInput, TerminalInputEvent, TerminalView};

#[derive(Default)]
pub(super) struct TerminalBroadcast {
    enabled: bool,
    /// In the order checked, which is the order of the split layout.
    selected: Vec<(CenterTab, u64)>,
    /// Typing in one checked terminal goes to the others as well. Off, only
    /// the command bar broadcasts, so a password typed at one prompt stays
    /// there.
    keys: bool,
}

impl TerminalBroadcast {
    fn reconcile(&mut self, available: &[(CenterTab, u64)]) {
        self.selected.retain(|target| available.contains(target));
    }

    /// At least two terminals checked: the command bar is up.
    fn active(&self) -> bool {
        self.enabled && self.selected.len() >= 2
    }
}

/// A checked terminal, as the command bar sends to it.
struct Recipient {
    view: Entity<TerminalView>,
    name: SharedString,
}

/// How many columns `count` terminals are laid out in: about a square.
fn grid_columns(count: usize) -> usize {
    (1..=count)
        .find(|columns| columns * columns >= count)
        .unwrap_or(1)
}

impl Workspace {
    fn terminal_view_for_tab(&self, tab: CenterTab, cx: &App) -> Option<Entity<TerminalView>> {
        match tab {
            CenterTab::Terminal(id) => Some(self.terminals.get(&id)?.read(cx).terminal().clone()),
            CenterTab::LocalTerminal(id) => {
                Some(self.local_terminals.get(&id)?.read(cx).terminal().clone())
            }
            _ => None,
        }
    }

    fn broadcast_candidates(&self, cx: &App) -> Vec<(CenterTab, u64)> {
        self.terminals
            .keys()
            .copied()
            .map(CenterTab::Terminal)
            .chain(
                self.local_terminals
                    .keys()
                    .copied()
                    .map(CenterTab::LocalTerminal),
            )
            .filter_map(|tab| {
                let view = self.terminal_view_for_tab(tab, cx)?;
                let view = view.read(cx);
                view.lifecycle(cx)
                    .accepts_input()
                    .then(|| (tab, view.generation(cx)))
            })
            .collect()
    }

    fn broadcast_recipients(&self, cx: &App) -> Vec<Recipient> {
        self.broadcast
            .selected
            .iter()
            .filter_map(|(tab, _)| match tab {
                CenterTab::Terminal(id) => {
                    let panel = self.terminals.get(id)?.read(cx);
                    Some(Recipient {
                        view: panel.terminal().clone(),
                        name: panel.tab_title(cx),
                    })
                }
                CenterTab::LocalTerminal(id) => {
                    let panel = self.local_terminals.get(id)?.read(cx);
                    Some(Recipient {
                        view: panel.terminal().clone(),
                        name: panel.display_title(cx),
                    })
                }
                _ => None,
            })
            .collect()
    }

    pub(super) fn sync_broadcast(&mut self, cx: &mut Context<Self>) {
        let available = self.broadcast_candidates(cx);
        self.broadcast.reconcile(&available);
        let target = self.focus_handle.clone();
        let selection = |tab| {
            self.broadcast.enabled.then(|| {
                BroadcastSelection::new(
                    self.broadcast
                        .selected
                        .iter()
                        .any(|(target, _)| *target == tab),
                    available.iter().any(|(target, _)| *target == tab),
                    target.clone(),
                )
            })
        };
        for (id, panel) in &self.terminals {
            let state = selection(CenterTab::Terminal(*id));
            panel.update(cx, |panel, cx| panel.set_broadcast_selection(state, cx));
        }
        for (id, panel) in &self.local_terminals {
            let state = selection(CenterTab::LocalTerminal(*id));
            panel.update(cx, |panel, cx| panel.set_broadcast_selection(state, cx));
        }
        let area = self.dock_area.read(cx);
        let mut first = None;
        if let Some(tree) = area.layout(DockPlacement::Center) {
            tree.root().walk(&mut |node| {
                if first.is_none()
                    && matches!(node.kind(), PaneRef::Tabs { .. })
                    && area.zoomed_group().is_none_or(|zoomed| zoomed == node.id())
                {
                    first = Some(node.id());
                }
            });
        }
        let hosts_open = (!area.is_zoomed() && area.is_dock_collapsible(DockPlacement::Left))
            .then(|| area.is_dock_open(DockPlacement::Left));
        self.skin.set_broadcast(
            first,
            hosts_open,
            self.broadcast.enabled,
            self.broadcast.selected.len(),
            cx,
        );
        self.skin.set_command_bar_shown(self.broadcast.active(), cx);
        let keys = self.broadcast.keys;
        self.broadcast_bar
            .update(cx, |bar, cx| bar.set_keys(keys, cx));
    }

    /// What was typed in one terminal, to the other checked ones when the
    /// keyboard is shared.
    pub(super) fn broadcast_input(
        &mut self,
        source: CenterTab,
        event: &TerminalInputEvent,
        cx: &mut Context<Self>,
    ) {
        // Every keystroke comes through here: the cheap checks first.
        if !self.broadcast.enabled || !self.broadcast.keys {
            return;
        }
        self.sync_broadcast(cx);
        if !self.broadcast.active()
            || !self
                .broadcast
                .selected
                .contains(&(source, event.generation()))
        {
            return;
        }
        // receive_input never emits Input, so recipients cannot form a loop.
        let recipients: Vec<_> = self
            .broadcast
            .selected
            .iter()
            .filter(|(tab, _)| *tab != source)
            .filter_map(|(tab, _)| self.terminal_view_for_tab(*tab, cx))
            .collect();
        for view in recipients {
            view.update(cx, |view, cx| {
                view.receive_input(event.input(), cx);
            });
        }
    }

    /// Put `command` on the input line of every checked terminal and with
    /// `run` run it. After whatever is typed there, not in its place as
    /// 命令片段 does for one: as Xshell's and SecureCRT's send to all, and
    /// clearing with Ctrl-E Ctrl-U rings bash's bell on an empty line and
    /// means other things to network devices. Says which terminals could not
    /// take it, and returns whether any did.
    fn broadcast_command(
        &mut self,
        command: &str,
        run: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.sync_broadcast(cx);
        if !self.broadcast.active() {
            return false;
        }
        let mut sent = false;
        // Why not, with the terminals it applies to, in the order met.
        let mut refused: Vec<(SharedString, Vec<SharedString>)> = Vec::new();
        for recipient in self.broadcast_recipients(cx) {
            match recipient
                .view
                .update(cx, |view, cx| view.enter_command(command, run, false, cx))
            {
                Ok(()) => sent = true,
                Err(why) => match refused.iter_mut().find(|(reason, _)| *reason == why) {
                    Some((_, names)) => names.push(recipient.name),
                    None => refused.push((why, vec![recipient.name])),
                },
            }
        }
        for (why, names) in refused {
            let message = t!(
                "workspace.broadcast.refused",
                names = join_list(&names),
                error = why
            );
            window.push_notification(Notification::error(message), cx);
        }
        sent
    }

    /// Lay the checked terminals out side by side, in the order checked, in
    /// about as many rows as columns. The first stays in its tab group,
    /// with whatever else is there; the others are gathered there first, so
    /// that every arrangement starts alike, then split out of it.
    fn arrange_broadcast(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let panels: Vec<PanelId> = self
            .broadcast
            .selected
            .iter()
            .filter_map(|(tab, _)| match tab {
                CenterTab::Terminal(id) => Some(self.terminals.get(id)?.entity_id()),
                CenterTab::LocalTerminal(id) => Some(self.local_terminals.get(id)?.entity_id()),
                _ => None,
            })
            .map(PanelId::from)
            .collect();
        let Some((&first, rest)) = panels.split_first() else {
            return;
        };
        if rest.is_empty() {
            return;
        }
        let columns = grid_columns(panels.len());
        self.dock_area.update(cx, |area, cx| {
            let group_of = |area: &DockArea, panel| {
                area.layout(DockPlacement::Center)
                    .and_then(|tree| tree.find_panel_node(panel))
            };
            for &panel in rest {
                if let Some(node) = group_of(area, first) {
                    area.move_panel(
                        panel,
                        InsertTarget::Tabs {
                            node,
                            ix: None,
                            activate: false,
                        },
                        window,
                        cx,
                    );
                }
            }
            // The first row left to right, then each below the one above.
            for (ix, &panel) in panels.iter().enumerate().skip(1) {
                let (beside, placement) = if ix < columns {
                    (panels[ix - 1], Placement::Right)
                } else {
                    (panels[ix - columns], Placement::Bottom)
                };
                if let Some(node) = group_of(area, beside) {
                    area.move_panel(
                        panel,
                        InsertTarget::Split {
                            node,
                            placement,
                            size: None,
                        },
                        window,
                        cx,
                    );
                }
            }
        });
        // The first in front of the tabs it shares its group with.
        match self.broadcast.selected.first().map(|(tab, _)| *tab) {
            Some(CenterTab::Terminal(id)) => {
                if let Some(panel) = self.terminals.get(&id).cloned() {
                    let group = panel.read(cx).tab_group();
                    self.activate_tab(group, panel.entity_id(), window, cx);
                }
            }
            Some(CenterTab::LocalTerminal(id)) => {
                if let Some(panel) = self.local_terminals.get(&id).cloned() {
                    let group = panel.read(cx).tab_group();
                    self.activate_tab(group, panel.entity_id(), window, cx);
                }
            }
            _ => {}
        }
    }

    pub(super) fn on_toggle_terminal_broadcast(
        &mut self,
        _: &ToggleTerminalBroadcast,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.broadcast = TerminalBroadcast {
            enabled: !self.broadcast.enabled,
            ..TerminalBroadcast::default()
        };
        self.sync_broadcast(cx);
        self.focus_center(window, cx);
        cx.notify();
    }

    pub(super) fn on_set_broadcast_target(
        &mut self,
        action: &SetBroadcastTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.broadcast.enabled {
            return;
        }
        let was_active = self.broadcast.active();
        self.broadcast
            .selected
            .retain(|(tab, _)| *tab != action.tab());
        let mut added = false;
        if action.is_selected()
            && let Some(target) = self
                .broadcast_candidates(cx)
                .into_iter()
                .find(|(tab, _)| *tab == action.tab())
        {
            self.broadcast.selected.push(target);
            added = true;
        }
        self.sync_broadcast(cx);
        if added && self.broadcast.active() {
            self.arrange_broadcast(window, cx);
        }
        if self.broadcast.active() && !was_active {
            // The bar has just come up: type the command there. A tab the
            // dock moved takes the focus as it becomes active, which its
            // group tells it from a spawned task; this one comes after.
            let bar = self.broadcast_bar.clone();
            cx.spawn_in(window, async move |_, cx| {
                _ = cx.update(|window, cx| bar.update(cx, |bar, cx| bar.focus(window, cx)));
            })
            .detach();
        } else {
            self.focus_center(window, cx);
        }
        cx.notify();
    }

    pub(super) fn on_send_broadcast_command(
        &mut self,
        _: &SendBroadcastCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let command = self.broadcast_bar.read(cx).command(cx);
        // What could not be sent anywhere stays, to send again.
        if self.broadcast_command(&command, true, window, cx) {
            self.broadcast_bar
                .update(cx, |bar, cx| bar.clear(window, cx));
        }
    }

    pub(super) fn on_broadcast_snippet(
        &mut self,
        action: &BroadcastSnippet,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(snippet) = self.store.read(cx).snippet(action.0).cloned() else {
            return;
        };
        self.broadcast_command(&snippet.command, snippet.run_on_click, window, cx);
    }

    pub(super) fn on_interrupt_broadcast(
        &mut self,
        _: &InterruptBroadcast,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sync_broadcast(cx);
        if !self.broadcast.active() {
            return;
        }
        for recipient in self.broadcast_recipients(cx) {
            recipient.view.update(cx, |view, cx| {
                view.receive_input(&TerminalInput::Bytes(vec![0x03]), cx);
            });
        }
    }

    pub(super) fn on_toggle_broadcast_keys(
        &mut self,
        _: &ToggleBroadcastKeys,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.broadcast.keys = !self.broadcast.keys;
        self.sync_broadcast(cx);
    }
}

#[cfg(test)]
mod tests {
    use super::{TerminalBroadcast, grid_columns};
    use crate::{app::CenterTab, terminal::RemoteTerminalId};

    #[test]
    fn closed_or_reconnected_terminals_need_to_be_checked_again() {
        let a = CenterTab::Terminal(RemoteTerminalId(1));
        let b = CenterTab::Terminal(RemoteTerminalId(2));
        for available in [vec![(a, 1)], vec![(a, 1), (b, 2)]] {
            let mut broadcast = TerminalBroadcast {
                enabled: true,
                selected: vec![(a, 1), (b, 1)],
                keys: false,
            };
            broadcast.reconcile(&available);
            assert_eq!(broadcast.selected, vec![(a, 1)]);
            broadcast.reconcile(&[(a, 1), (b, 1)]);
            assert_eq!(broadcast.selected, vec![(a, 1)]);
        }
    }

    #[test]
    fn the_layout_is_about_square() {
        let columns: Vec<usize> = (1..=10).map(grid_columns).collect();
        assert_eq!(columns, [1, 2, 2, 2, 3, 3, 3, 3, 3, 4]);
    }
}
