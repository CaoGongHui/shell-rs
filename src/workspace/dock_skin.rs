use std::{cell::Cell, rc::Rc, sync::Arc};

use crate::app::{CatalogIcon, ToggleHostPanel, ToggleTerminalBroadcast};
use crate::i18n::t;
use gpui_kit::component::dock::{
    BasePanelView, DockArea, DockAreaRenderer, DockContext, DockSkin, DropIndicator, NodeId,
    PanelState, TabGroupContext, TabGroupRenderer, TilesRenderer,
};
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

/// What the leading center tab bar shows ahead of its tabs: the sidebar
/// switch and 命令广播's. Renderers consume this value, never the Workspace
/// entity being rendered.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct TabBarControls {
    node: Option<NodeId>,
    hosts_open: Option<bool>,
    enabled: bool,
    count: usize,
}

impl TabBarControls {
    fn controls(self, target: &FocusHandle, cx: &App) -> impl IntoElement {
        let hosts = target.clone();
        let broadcast = target.clone();
        let label = if !self.enabled {
            t!("workspace.broadcast.title")
        } else if self.count < 2 {
            t!("workspace.broadcast.choose")
        } else {
            t!("workspace.broadcast.active", count = self.count)
        };
        // Bordered like the tab bar's own prefix: the bar's bottom line runs
        // under it, and the first tab, drawn without a left border, is set
        // off from it.
        h_flex()
            .id("tab-bar-controls")
            .test_support()
            .flex_shrink_0()
            .self_stretch()
            .px_2()
            .gap_1()
            .border_b_1()
            .border_r_1()
            .border_color(cx.theme().border)
            .when_some(self.hosts_open, |row, open| {
                row.child(
                    Button::new("toggle-dock:Left")
                        .ghost()
                        .xsmall()
                        .icon(if open {
                            IconName::PanelLeft
                        } else {
                            IconName::PanelLeftOpen
                        })
                        .tooltip(if open {
                            t!("workspace.title_bar.hide_sidebar")
                        } else {
                            t!("workspace.title_bar.show_sidebar")
                        })
                        .on_click(move |_, window, cx| {
                            hosts.dispatch_action(&ToggleHostPanel, window, cx)
                        }),
                )
            })
            .child(
                Button::new("toggle-terminal-broadcast")
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(CatalogIcon::Megaphone))
                    .selected(self.enabled)
                    .accessibility_label(label)
                    .tooltip(t!("workspace.broadcast.title"))
                    .on_click(move |_, window, cx| {
                        broadcast.dispatch_action(&ToggleTerminalBroadcast, window, cx)
                    }),
            )
    }
}

/// The workspace's dock appearance: gpui-kit's `DockSkin`, plus a view that
/// fills the center while it holds no tab.
///
/// The dock draws nothing for an empty center and `DockAreaRenderer` has no
/// empty-center hook, so the placeholder rides on `center_frame`. It is drawn
/// deferred, above the empty layout the dock paints there, whenever the
/// workspace last said the center is empty (`set_center_empty`); the
/// workspace keeps that in step with `DockEvent::LayoutChanged`.
pub(super) struct WorkspaceDockSkin {
    skin: Rc<DockSkin>,
    // Only the leading center bar replaces the built-in sidebar toggle, so
    // its two controls can sit together. Other dock bars keep their skin.
    center_skin: Rc<DockSkin>,
    broadcast: Rc<Cell<TabBarControls>>,
    target: FocusHandle,
    area: WeakEntity<DockArea>,
    placeholder: AnyView,
    center_empty: Cell<bool>,
}

impl WorkspaceDockSkin {
    /// Build a `DockArea` wearing this skin, together with the handle its
    /// settings are changed through. `placeholder` is what the center shows
    /// while it is empty.
    pub(super) fn dock_area(
        id: &'static str,
        version: Option<usize>,
        placeholder: AnyView,
        target: FocusHandle,
        window: &mut Window,
        cx: &mut App,
    ) -> (Entity<DockArea>, Rc<Self>) {
        let mut skin = None;
        let area = cx.new(|cx| {
            let this = Rc::new(Self {
                skin: DockSkin::new(cx),
                center_skin: DockSkin::new(cx),
                broadcast: Rc::default(),
                target,
                area: cx.weak_entity(),
                placeholder,
                center_empty: Cell::new(false),
            });
            skin = Some(this.clone());
            DockArea::new(id, version, window, cx).with_renderer(this)
        });
        // The closure above runs before `cx.new` returns.
        let skin = skin.expect("the skin is built inside the constructor");
        skin.center_skin.set_toggle_button_visible(false, cx);
        (area, skin)
    }

    pub(super) fn is_center_empty(&self) -> bool {
        self.center_empty.get()
    }

    pub(super) fn set_broadcast(
        &self,
        node: Option<NodeId>,
        hosts_open: Option<bool>,
        enabled: bool,
        count: usize,
        cx: &mut App,
    ) {
        let state = TabBarControls {
            node,
            hosts_open,
            enabled,
            count,
        };
        if self.broadcast.replace(state) != state {
            _ = self.area.update(cx, |_, cx| cx.notify());
        }
    }

    /// Show (`true`) or hide the placeholder. The skin is not an entity, so
    /// it redraws the area itself when the value changes.
    pub(super) fn set_center_empty(&self, empty: bool, cx: &mut App) {
        if self.center_empty.replace(empty) != empty {
            _ = self.area.update(cx, |_, cx| cx.notify());
        }
    }
}

impl DockAreaRenderer for WorkspaceDockSkin {
    fn frame(&self, window: &mut Window, cx: &mut App) -> Stateful<Div> {
        self.skin.frame(window, cx)
    }

    fn split_frame(
        &self,
        node: NodeId,
        axis: Axis,
        window: &mut Window,
        cx: &mut App,
    ) -> Stateful<Div> {
        self.skin.split_frame(node, axis, window, cx)
    }

    fn center_frame(&self, window: &mut Window, cx: &mut App) -> Stateful<Div> {
        self.skin
            .center_frame(window, cx)
            .relative()
            .when(self.center_empty.get(), |frame| {
                // Deferred, so it paints after the empty layout's own
                // background, which the dock draws later in the same frame.
                frame.child(deferred(
                    div().absolute().inset_0().child(self.placeholder.clone()),
                ))
            })
    }

    fn render_dock(
        &self,
        dock: &DockContext,
        content: AnyElement,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        self.skin.render_dock(dock, content, window, cx)
    }

    fn build_placeholder(
        &self,
        state: &PanelState,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Arc<dyn BasePanelView>> {
        self.skin.build_placeholder(state, window, cx)
    }

    fn tab_group_renderer(&self) -> Rc<dyn TabGroupRenderer> {
        Rc::new(WorkspaceTabBar {
            skin: self.skin.tab_group_renderer(),
            center_skin: self.center_skin.tab_group_renderer(),
            broadcast: self.broadcast.clone(),
            target: self.target.clone(),
        })
    }

    fn tiles_renderer(&self) -> Rc<dyn TilesRenderer> {
        self.skin.tiles_renderer()
    }
}

/// Add the permanent broadcast toggle ahead of the leading center tab strip.
/// Dock's renderer still owns tabs, dragging, overflow, menus and focus.
struct WorkspaceTabBar {
    skin: Rc<dyn TabGroupRenderer>,
    center_skin: Rc<dyn TabGroupRenderer>,
    broadcast: Rc<Cell<TabBarControls>>,
    target: FocusHandle,
}

impl TabGroupRenderer for WorkspaceTabBar {
    fn frame(&self, group: &TabGroupContext, window: &mut Window, cx: &mut App) -> Stateful<Div> {
        self.skin.frame(group, window, cx)
    }
    fn content_frame(
        &self,
        group: &TabGroupContext,
        window: &mut Window,
        cx: &mut App,
    ) -> Stateful<Div> {
        self.skin.content_frame(group, window, cx)
    }
    fn render_tab_bar(
        &self,
        group: &TabGroupContext,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        // 撰写窗格 draws its own title bar.
        if let [panel] = group.panels()
            && panel.panel_name(cx) == "ComposePane"
        {
            return Empty.into_any_element();
        }
        let state = self.broadcast.get();
        if state.node != Some(group.node()) {
            return self.skin.render_tab_bar(group, window, cx);
        }
        let tabs = self.center_skin.render_tab_bar(group, window, cx);
        h_flex()
            .w_full()
            .bg(cx.theme().tokens.tab_bar)
            .child(state.controls(&self.target, cx))
            // A column, as base lays out the other bars: in a block `div`
            // the bar's height is indefinite, so the toolbar box on its
            // right, `h_full`, shrinks to its buttons and its bottom border
            // sits above the bar's (#44).
            .child(
                v_flex()
                    .id("center-tab-bar")
                    .test_support()
                    .flex_1()
                    .min_w_0()
                    .child(tabs),
            )
            .into_any_element()
    }
    fn render_active_panel(
        &self,
        panel: AnyView,
        group: &TabGroupContext,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        self.skin.render_active_panel(panel, group, window, cx)
    }
    fn render_drop_indicator(
        &self,
        indicator: DropIndicator,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        self.skin.render_drop_indicator(indicator, window, cx)
    }
    fn render_empty(
        &self,
        group: &TabGroupContext,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        self.skin.render_empty(group, window, cx)
    }
}
