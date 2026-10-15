//! A fixed three-column view; navigation reveals the next hop.
use super::{
    model::{KubernetesCommand, ResourceKind},
    panel::defer_dispatch,
    relation_scope::{self, PAGE_SIZE},
    relations::{Graph, GraphNode},
};
use crate::{app::KubernetesAction, i18n::t};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

pub(super) fn render(
    graph: &Graph,
    selected: &str,
    dispatch: &FocusHandle,
    pages: [usize; 3],
    cx: &App,
) -> AnyElement {
    let upstream = relation_scope::neighbors(graph, selected, true);
    let downstream = relation_scope::neighbors(graph, selected, false);
    let dependencies = relation_scope::dependencies(graph, selected);
    let accessible = upstream
        .iter()
        .map(|key| {
            format!(
                "{} → {}",
                graph.nodes[key].resource.name, graph.nodes[selected].resource.name
            )
        })
        .chain(downstream.iter().chain(&dependencies).map(|key| {
            format!(
                "{} → {}",
                graph.nodes[selected].resource.name,
                graph.nodes[key]
                    .container
                    .as_deref()
                    .unwrap_or(&graph.nodes[key].resource.name)
            )
        }))
        .collect::<Vec<_>>()
        .join("\n");
    let accessible = graph
        .edges
        .iter()
        .filter(|(from, to, _)| {
            (to == selected && upstream.contains(from))
                || (from == selected && (downstream.contains(to) || dependencies.contains(to)))
        })
        .map(|(_, _, relation)| t!(relation.label()).to_string())
        .fold(accessible, |text, label| format!("{text}\n{label}"));
    let body = h_flex()
        .w_full()
        .items_start()
        .gap_3()
        .child(column(graph, &upstream, 0, pages[0], dispatch, cx))
        .child(
            div().mt(rems(3.5)).child(
                Icon::new(IconName::ArrowRight)
                    .small()
                    .text_color(cx.theme().muted_foreground),
            ),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_3()
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(t!("k8s.graph.current")),
                )
                .when_some(graph.nodes.get(selected), |body, node| {
                    body.child(card(selected, node, true, dispatch, cx))
                }),
        )
        .child(
            div().mt(rems(3.5)).child(
                Icon::new(IconName::ArrowRight)
                    .small()
                    .text_color(cx.theme().muted_foreground),
            ),
        )
        .child(column(graph, &downstream, 1, pages[1], dispatch, cx));
    v_flex()
        .gap_3()
        .child(
            div()
                .id("k8s-relation-graph")
                .test_support()
                .aria_label(accessible)
                .w_full()
                .p_4()
                .border_1()
                .border_color(cx.theme().border)
                .rounded(cx.theme().radius)
                .child(body),
        )
        .when(!dependencies.is_empty(), |body| {
            body.child(column(graph, &dependencies, 2, pages[2], dispatch, cx))
        })
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(t!("k8s.relations_tree_note")),
        )
        .into_any_element()
}

fn column(
    graph: &Graph,
    keys: &[String],
    column: usize,
    page: usize,
    dispatch: &FocusHandle,
    cx: &App,
) -> AnyElement {
    let page_count = keys.len().div_ceil(PAGE_SIZE).max(1);
    let page = page.min(page_count - 1);
    let mut body = v_flex().flex_1().min_w_0().gap_3().child(
        h_flex()
            .gap_2()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(match column {
                0 => t!("k8s.graph.upstream"),
                1 => t!("k8s.graph.downstream"),
                _ => t!("k8s.graph.dependencies"),
            })
            .child(keys.len().to_string()),
    );
    if keys.is_empty() {
        body = body.child(
            div()
                .h(rems(4.))
                .flex()
                .items_start()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(t!("k8s.graph.empty")),
        );
    }
    let mut cards = div()
        .flex()
        .gap_3()
        .when(column != 2, |cards| cards.flex_col());
    for key in keys.iter().skip(page * PAGE_SIZE).take(PAGE_SIZE) {
        cards = cards.child(div().flex_1().min_w_0().child(card(
            key,
            &graph.nodes[key],
            false,
            dispatch,
            cx,
        )));
    }
    body = body.child(cards);
    if page_count > 1 {
        let side = ["upstream", "downstream", "dependencies"][column];
        let previous = dispatch.clone();
        let next = dispatch.clone();
        body = body.child(
            h_flex()
                .justify_between()
                .gap_1()
                .child(
                    Button::new(SharedString::from(format!("k8s-graph-{side}-previous")))
                        .ghost()
                        .small()
                        .icon(IconName::ChevronLeft)
                        .accessibility_label(t!("k8s.graph.previous"))
                        .tooltip(t!("k8s.graph.previous"))
                        .disabled(page == 0)
                        .on_click(move |_, window, cx| {
                            defer_dispatch(
                                &previous,
                                &KubernetesAction(KubernetesCommand::RelationPage {
                                    column,
                                    page: page.saturating_sub(1),
                                }),
                                window,
                                cx,
                            )
                        }),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("{} / {}", page + 1, page_count)),
                )
                .child(
                    Button::new(SharedString::from(format!("k8s-graph-{side}-next")))
                        .ghost()
                        .small()
                        .icon(IconName::ChevronRight)
                        .accessibility_label(t!("k8s.graph.next"))
                        .tooltip(t!("k8s.graph.next"))
                        .disabled(page + 1 == page_count)
                        .on_click(move |_, window, cx| {
                            defer_dispatch(
                                &next,
                                &KubernetesAction(KubernetesCommand::RelationPage {
                                    column,
                                    page: page + 1,
                                }),
                                window,
                                cx,
                            )
                        }),
                ),
        );
    }
    body.into_any_element()
}

fn card(
    key: &str,
    node: &GraphNode,
    selected: bool,
    dispatch: &FocusHandle,
    cx: &App,
) -> AnyElement {
    let kind = if node.container.is_some() {
        t!("k8s.container").to_string()
    } else {
        node.resource.kind.label().into()
    };
    let name = node.container.as_deref().unwrap_or(&node.resource.name);
    let scope = if node.resource.namespace.is_empty() {
        t!("k8s.cluster_scope").to_string()
    } else {
        node.resource.namespace.clone()
    };
    let hint = if node.fetched {
        format!("{kind} · {name} · {scope}")
    } else {
        format!("{kind} · {name} · {scope} · {}", t!("k8s.reference_only"))
    };
    let accent = if node.container.is_some() {
        cx.theme().green
    } else {
        match node.resource.kind {
            ResourceKind::Pod => cx.theme().cyan,
            ResourceKind::Service | ResourceKind::Ingress | ResourceKind::EndpointSlice => {
                cx.theme().magenta
            }
            ResourceKind::ConfigMap | ResourceKind::Secret | ResourceKind::ServiceAccount => {
                cx.theme().yellow
            }
            ResourceKind::PersistentVolumeClaim
            | ResourceKind::PersistentVolume
            | ResourceKind::StorageClass => cx.theme().blue_light,
            _ => cx.theme().blue,
        }
    };
    let handle = dispatch.clone();
    let action = KubernetesAction(KubernetesCommand::RelationNode(key.into()));
    Button::new(SharedString::from(format!("k8s-graph-node:{key}")))
        .outline()
        .small()
        .w_full()
        .h(rems(4.))
        .bg(if selected {
            cx.theme().primary.opacity(0.08)
        } else {
            cx.theme().background
        })
        .border_color(if selected {
            cx.theme().primary
        } else {
            cx.theme().border
        })
        .accessibility_label(hint.clone())
        .tooltip(hint)
        .child(
            v_flex()
                .w_full()
                .min_w_0()
                .gap_1()
                .child(
                    h_flex()
                        .gap_1p5()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(div().size_1p5().rounded_full().bg(accent))
                        .child(kind),
                )
                .child(div().w_full().text_sm().truncate().child(name.to_owned())),
        )
        .on_click(move |_, window, cx| defer_dispatch(&handle, &action, window, cx))
        .into_any_element()
}
