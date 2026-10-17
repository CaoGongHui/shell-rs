//! The bar above the terminals of 命令广播: a command line sent to every
//! checked terminal, the snippets, Ctrl+C for all of them, and whether
//! typing in one checked terminal goes to the others as well. Every control
//! dispatches to the workspace, which owns the broadcast.

use std::rc::Rc;

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::DropdownMenu as _,
    switch::Switch,
    tooltip::Tooltip,
};
use gpui_kit::*;

use crate::app::{
    BroadcastSnippet, CatalogIcon, InterruptBroadcast, SendBroadcastCommand, ToggleBroadcastKeys,
    ToggleTerminalBroadcast,
};
use crate::host::{HostStore, SnippetId};
use crate::i18n::{UiLocale, t};

/// A category's name (none for the uncategorized) and its snippets.
type SnippetGroup = (Option<SharedString>, Vec<(SnippetId, SharedString)>);

pub(super) struct BroadcastBar {
    input: Entity<InputState>,
    store: Entity<HostStore>,
    /// The workspace's: the bar's commands go there.
    dispatch: FocusHandle,
    /// Typing in a checked terminal goes to the others too.
    keys: bool,
    _subscriptions: Vec<Subscription>,
}

impl BroadcastBar {
    pub(super) fn new(
        store: Entity<HostStore>,
        dispatch: FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("workspace.broadcast.placeholder"))
        });
        let subscriptions = vec![
            // The snippets menu lists what the store has.
            cx.observe(&store, |_, _, cx| cx.notify()),
            cx.observe_global_in::<UiLocale>(window, |this, window, cx| {
                this.input.update(cx, |input, cx| {
                    input.set_placeholder(t!("workspace.broadcast.placeholder"), window, cx)
                });
            }),
            // Through the window, which queues it: the handler reads the
            // line, so it must not run while this bar is being updated. The
            // line has the focus, inside the workspace.
            cx.subscribe_in(&input, window, |_, _, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    window.dispatch_action(Box::new(SendBroadcastCommand), cx);
                }
            }),
        ];
        Self {
            input,
            store,
            dispatch,
            keys: false,
            _subscriptions: subscriptions,
        }
    }

    /// The line typed.
    pub(super) fn command(&self, cx: &App) -> String {
        self.input.read(cx).value().to_string()
    }

    /// Empty the line, once it went out.
    pub(super) fn clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.input
            .update(cx, |input, cx| input.set_value("", window, cx));
    }

    pub(super) fn set_keys(&mut self, keys: bool, cx: &mut Context<Self>) {
        if self.keys != keys {
            self.keys = keys;
            cx.notify();
        }
    }

    pub(super) fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| input.focus(window, cx));
    }

    /// The snippets by category, the uncategorized first, as 命令片段
    /// lists them.
    fn snippet_groups(&self, cx: &App) -> Vec<SnippetGroup> {
        let store = self.store.read(cx);
        let of = |category| {
            store
                .snippets()
                .iter()
                .filter(|snippet| snippet.category == category)
                .map(|snippet| (snippet.id, snippet.name.clone()))
                .collect::<Vec<_>>()
        };
        std::iter::once((None, of(None)))
            .chain(
                store
                    .snippet_categories()
                    .iter()
                    .map(|category| (Some(category.name.clone()), of(Some(category.id)))),
            )
            .filter(|(_, snippets)| !snippets.is_empty())
            .collect()
    }
}

impl Render for BroadcastBar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let groups = Rc::new(self.snippet_groups(cx));
        let no_snippets = groups.is_empty();
        // Picking a snippet hands the keyboard back to the command line.
        let input_focus = self.input.read(cx).focus_handle(cx);
        let send = |action: &'static dyn Action, dispatch: &FocusHandle| {
            let dispatch = dispatch.clone();
            move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                dispatch.dispatch_action(action, window, cx)
            }
        };
        let keys_dispatch = self.dispatch.clone();
        h_flex()
            .id("broadcast-bar")
            .test_support()
            .flex_shrink_0()
            .w_full()
            .gap_2()
            .px_2()
            .py_1()
            .bg(cx.theme().tokens.tab_bar)
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Input::new(&self.input).small()),
            )
            .child(
                Button::new("broadcast-snippets")
                    .small()
                    .outline()
                    .icon(Icon::new(CatalogIcon::CodeXml))
                    .label(t!("workspace.broadcast.snippets"))
                    .tooltip(if no_snippets {
                        t!("workspace.broadcast.no_snippets")
                    } else {
                        t!("workspace.broadcast.snippets_tooltip")
                    })
                    .disabled(no_snippets)
                    .dropdown_menu(move |mut menu, _, _| {
                        menu = menu
                            .action_context(input_focus.clone())
                            .max_h(px(420.))
                            .scrollable(true);
                        for (ix, (category, snippets)) in groups.iter().enumerate() {
                            if ix > 0 {
                                menu = menu.separator();
                            }
                            if let Some(name) = category {
                                menu = menu.label(name.clone());
                            }
                            for (id, name) in snippets {
                                menu = menu.menu(name.clone(), Box::new(BroadcastSnippet(*id)));
                            }
                        }
                        menu
                    }),
            )
            .child(
                // A switch, like 目录跟随's: the knob shows the state, which
                // a pressed toggle on the bar's own gray hardly does. The
                // tooltip goes around it, label included: the switch's own
                // shows over the knob alone.
                div()
                    .id("broadcast-keys-tip")
                    .flex_shrink_0()
                    .tooltip(|window, cx| {
                        Tooltip::new(t!("workspace.broadcast.keys_tooltip")).build(window, cx)
                    })
                    .child(
                        Switch::new("broadcast-keys")
                            .small()
                            .checked(self.keys)
                            .label(t!("workspace.broadcast.keys"))
                            .on_click(move |_, window, cx| {
                                keys_dispatch.dispatch_action(&ToggleBroadcastKeys, window, cx)
                            }),
                    ),
            )
            .child(
                Button::new("broadcast-send")
                    .small()
                    .primary()
                    .icon(Icon::new(CatalogIcon::Send))
                    .label(t!("workspace.broadcast.send"))
                    .tooltip(t!("workspace.broadcast.send_tooltip"))
                    .on_click(send(&SendBroadcastCommand, &self.dispatch)),
            )
            .child(
                Button::new("broadcast-interrupt")
                    .small()
                    .outline()
                    .label("Ctrl+C")
                    .tooltip(t!("workspace.broadcast.interrupt"))
                    .on_click(send(&InterruptBroadcast, &self.dispatch)),
            )
            .child(
                Button::new("broadcast-end")
                    .small()
                    .ghost()
                    .icon(IconName::Close)
                    .tooltip(t!("workspace.broadcast.end"))
                    .on_click(send(&ToggleTerminalBroadcast, &self.dispatch)),
            )
    }
}
