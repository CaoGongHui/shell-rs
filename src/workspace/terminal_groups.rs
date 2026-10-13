//! Dock owns tab dragging. The workspace owns the explicitly checked, live
//! connections that share interactive input; reconnects never inherit membership.

use super::Workspace;
use crate::app::{CenterTab, SetBroadcastTarget, ToggleTerminalBroadcast};
use crate::terminal::{BroadcastSelection, TerminalInputEvent, TerminalView};
use gpui_kit::component::dock::{DockPlacement, PaneRef};
use gpui_kit::*;

#[derive(Default)]
pub(super) struct TerminalBroadcast {
    enabled: bool,
    selected: Vec<(CenterTab, u64)>,
}

impl TerminalBroadcast {
    fn reconcile(&mut self, available: &[(CenterTab, u64)]) {
        self.selected.retain(|target| available.contains(target));
    }
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

    pub(super) fn sync_broadcast(&mut self, cx: &mut Context<Self>) {
        let available = self.broadcast_candidates(cx);
        self.broadcast.reconcile(&available);
        let selection = |tab| {
            self.broadcast.enabled.then(|| {
                BroadcastSelection::new(
                    self.broadcast
                        .selected
                        .iter()
                        .any(|(target, _)| *target == tab),
                    available.iter().any(|(target, _)| *target == tab),
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
    }

    pub(super) fn broadcast_input(
        &mut self,
        source: CenterTab,
        event: &TerminalInputEvent,
        cx: &mut Context<Self>,
    ) {
        self.sync_broadcast(cx);
        if !self.broadcast.enabled
            || self.broadcast.selected.len() < 2
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

    pub(super) fn on_toggle_terminal_broadcast(
        &mut self,
        _: &ToggleTerminalBroadcast,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.broadcast = TerminalBroadcast {
            enabled: !self.broadcast.enabled,
            selected: Vec::new(),
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
        self.broadcast
            .selected
            .retain(|(tab, _)| *tab != action.tab());
        if action.is_selected()
            && let Some(target) = self
                .broadcast_candidates(cx)
                .into_iter()
                .find(|(tab, _)| *tab == action.tab())
        {
            self.broadcast.selected.push(target);
        }
        self.sync_broadcast(cx);
        self.focus_center(window, cx);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::TerminalBroadcast;
    use crate::{app::CenterTab, terminal::RemoteTerminalId};
    #[test]
    fn closed_or_reconnected_terminals_need_to_be_checked_again() {
        let a = CenterTab::Terminal(RemoteTerminalId(1));
        let b = CenterTab::Terminal(RemoteTerminalId(2));
        for available in [vec![(a, 1)], vec![(a, 1), (b, 2)]] {
            let mut broadcast = TerminalBroadcast {
                enabled: true,
                selected: vec![(a, 1), (b, 1)],
            };
            broadcast.reconcile(&available);
            assert_eq!(broadcast.selected, vec![(a, 1)]);
            broadcast.reconcile(&[(a, 1), (b, 1)]);
            assert_eq!(broadcast.selected, vec![(a, 1)]);
        }
    }
}
