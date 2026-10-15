//! The production Kubernetes browser, with injected SSH exec replies only.
use gpui_kit::{Focusable as _, SharedString};
use shellrs::{
    app::KubernetesAction,
    kubernetes::{KubernetesCommand, ResourceKind, ResourceOperation},
};

use crate::support::*;

/// A workspace with Kubernetes turned on in 侧栏功能, where it is off by
/// default.
fn open_workspace_with_kubernetes(
    cx: &mut TestAppContext,
    store: HostStore,
    factory: Arc<dyn TerminalTransportFactory>,
) -> (WindowHandle<Root>, Entity<Workspace>) {
    let (handle, workspace) = open_workspace_with_remote_factory(cx, store, factory);
    workspace.update(cx, |workspace, cx| {
        workspace.settings().update(cx, |settings, cx| {
            settings.update(
                |settings| {
                    settings
                        .sidebar_tools
                        .set(shellrs::app::ToolKind::Kubernetes, true)
                },
                cx,
            )
        });
    });
    (handle, workspace)
}

const SELECT_ALL: &str = if cfg!(target_os = "macos") {
    "cmd-a"
} else {
    "ctrl-a"
};

const CONTEXTS: &str =
    "prod-cluster\n\n__SHELLRS_CONTEXTS__\nprod-cluster\ntest-cluster\n\n__SHELLRS_K8S_EXIT__:0\n";
const NAMESPACES: &str = "{\"items\":[{\"metadata\":{\"name\":\"prod\"}},{\"metadata\":{\"name\":\"test\"}}]}\n__SHELLRS_K8S_EXIT__:0\n";

fn mutation_resource(kind: ResourceKind) -> serde_json::Value {
    let api = if matches!(
        kind,
        ResourceKind::Deployment | ResourceKind::StatefulSet | ResourceKind::DaemonSet
    ) {
        "apps/v1"
    } else {
        "v1"
    };
    let mut value = serde_json::json!({"apiVersion":api,"kind":kind.label(),"metadata":{"name":"managed","namespace":"prod","uid":"uid-original","resourceVersion":"42"}});
    if kind == ResourceKind::ConfigMap {
        value["data"] = serde_json::json!({"greeting":"old", "flag":"true"});
    } else if kind == ResourceKind::Pod {
        value["metadata"]["ownerReferences"] = serde_json::json!([{"apiVersion":"apps/v1","kind":"ReplicaSet","name":"managed-rs","uid":"rs-uid","controller":true}]);
        value["spec"] = serde_json::json!({"containers":[{"name":"app","image":"app:v1"}]});
        value["status"] = serde_json::json!({"phase":"Running"});
    } else {
        value["spec"] = serde_json::json!({"replicas":1,"template":{"metadata":{"labels":{"app":"managed"}},"spec":{"containers":[{"name":"app","image":"app:v1"}]}}});
    }
    value
}

fn mutation_wire(value: &serde_json::Value) -> String {
    format!("{value}\n__SHELLRS_K8S_EXIT__:0\n")
}

async fn open_mutation_resource(
    cx: &mut TestAppContext,
    kind: ResourceKind,
    operation: Option<ResourceOperation>,
) -> (WindowHandle<Root>, Arc<PendingFactory>) {
    let factory = Arc::new(PendingFactory::default());
    let (handle, _) = open_workspace_with_kubernetes(cx, HostStore::seed(), factory.clone());
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", INITIAL_WEB_TERMINAL), cx)
    });
    in_frame(cx, handle, |window, cx| window.click("tool-kubernetes", cx));
    reply(cx, handle, &factory, CONTEXTS).await;
    reply(cx, handle, &factory, NAMESPACES).await;
    reply(cx, handle, &factory, PODS).await;
    label(cx, handle, "k8s-count", "2 项资源 · 2 个命名空间").await;
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-search", cx);
        if kind != ResourceKind::Pod {
            window.dispatch_action(
                Box::new(KubernetesAction(KubernetesCommand::Kind(kind))),
                cx,
            );
        } else {
            window.dispatch_action(Box::new(KubernetesAction(KubernetesCommand::Refresh)), cx);
        }
    });
    if kind == ResourceKind::Pod {
        reply(cx, handle, &factory, CONTEXTS).await;
        reply(cx, handle, &factory, NAMESPACES).await;
    }
    let value = mutation_resource(kind);
    let list = if kind == ResourceKind::ConfigMap {
        serde_json::json!([value["metadata"]])
    } else {
        serde_json::json!({"items":[value]})
    };
    reply(cx, handle, &factory, &mutation_wire(&list)).await;
    label(cx, handle, "k8s-count", "1 项资源 · 2 个命名空间").await;
    in_frame(cx, handle, |window, cx| {
        if let Some(operation) = operation {
            // Menus dispatch this action. Avoid constructing popup menus in UI tests.
            window.dispatch_action(
                Box::new(KubernetesAction(KubernetesCommand::Manage {
                    kind,
                    namespace: "prod".into(),
                    name: "managed".into(),
                    operation,
                })),
                cx,
            );
        } else {
            let resource = match kind {
                ResourceKind::Pod => "pods",
                ResourceKind::ConfigMap => "configmaps",
                ResourceKind::Deployment => "deployments",
                ResourceKind::StatefulSet => "statefulsets",
                _ => unreachable!(),
            };
            window.click(
                SharedString::from(format!("k8s-resource:{resource}:prod/managed:open")),
                cx,
            );
        }
    });
    reply(
        cx,
        handle,
        &factory,
        &mutation_wire(&mutation_resource(kind)),
    )
    .await;
    (handle, factory)
}

async fn wait_for_confirmation(cx: &mut TestAppContext, handle: WindowHandle<Root>) {
    cx.wait_for(handle.into(), Duration::from_secs(5), |window, _| {
        if let Some(error) = window.try_find("k8s-operation-error") {
            panic!("confirmation was rejected: {:?}", error.label());
        }
        window.try_find("ok").is_some()
    })
    .await;
}

fn type_yaml(cx: &mut TestAppContext, handle: WindowHandle<Root>, text: &str) {
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-detail-yaml", cx);
        window.press(SELECT_ALL, cx);
        window.input(text, cx);
    });
}

#[gpui_kit::test]
async fn kubernetes_closing_details_asks_before_dropping_changed_yaml(cx: &mut TestAppContext) {
    let (handle, _factory) =
        open_mutation_resource(cx, ResourceKind::ConfigMap, Some(ResourceOperation::Edit)).await;
    cx.wait_for(handle.into(), Duration::from_secs(5), |window, _| {
        window.try_find("k8s-save-yaml").is_some()
    })
    .await;
    let mut draft = mutation_resource(ResourceKind::ConfigMap);
    draft["data"]["greeting"] = serde_json::json!("new");
    type_yaml(cx, handle, &serde_yaml_ng::to_string(&draft).unwrap());

    in_frame(cx, handle, |window, cx| {
        window.click("k8s-detail-close", cx)
    });
    wait_for_confirmation(cx, handle).await;
    in_frame(cx, handle, |window, cx| window.click("cancel", cx));
    in_frame(cx, handle, |window, _| {
        assert!(
            window.try_find("k8s-save-yaml").is_some(),
            "keeping the changes keeps the details open"
        )
    });

    in_frame(cx, handle, |window, cx| {
        window.click("k8s-detail-yaml", cx);
        window.press("escape", cx);
    });
    wait_for_confirmation(cx, handle).await;
    in_frame(cx, handle, |window, cx| window.click("ok", cx));
    cx.run_until_parked();
    in_frame(cx, handle, |window, _| {
        assert!(window.try_find("k8s-detail-yaml").is_none());
        assert!(window.try_find("ok").is_none());
    });
}

#[gpui_kit::test]
async fn kubernetes_yaml_save_requires_confirmation_and_conflicts_keep_the_draft(
    cx: &mut TestAppContext,
) {
    let (handle, factory) =
        open_mutation_resource(cx, ResourceKind::ConfigMap, Some(ResourceOperation::Edit)).await;
    cx.wait_for(handle.into(), Duration::from_secs(5), |window, _| {
        window.try_find("k8s-save-yaml").is_some()
    })
    .await;
    let before = factory.commands.lock().unwrap().len();
    let mut draft = mutation_resource(ResourceKind::ConfigMap);
    draft["data"]["greeting"] = serde_json::json!("new");
    let text = serde_yaml_ng::to_string(&draft).unwrap();
    type_yaml(cx, handle, &text);
    in_frame(cx, handle, |window, cx| window.click("k8s-save-yaml", cx));
    wait_for_confirmation(cx, handle).await;
    assert_eq!(
        factory.commands.lock().unwrap().len(),
        before,
        "preparing YAML never writes"
    );
    in_frame(cx, handle, |window, cx| window.click("cancel", cx));
    assert_eq!(factory.commands.lock().unwrap().len(), before);
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-detail-yaml", cx);
        window.press(
            if cfg!(target_os = "macos") {
                "cmd-s"
            } else {
                "ctrl-s"
            },
            cx,
        )
    });
    wait_for_confirmation(cx, handle).await;
    in_frame(cx, handle, |window, cx| window.click("ok", cx));
    cx.wait_for(handle.into(), Duration::from_secs(5), |_, _| {
        factory.pending.lock().unwrap().len() == 1
    })
    .await;
    assert_eq!(factory.commands.lock().unwrap().len(), before + 1);
    let command = script(factory.commands.lock().unwrap().last().unwrap());
    assert!(command.contains("replace --raw='/api/v1/namespaces/prod/configmaps/managed' -f -"));
    assert!(
        command.contains("\"greeting\":\"new\"") && command.contains("\"resourceVersion\":\"42\"")
    );
    cx.executor().advance_clock(Duration::from_secs(20));
    cx.run_until_parked();
    assert_eq!(
        factory.commands.lock().unwrap().len(),
        before + 1,
        "a pending write cannot duplicate"
    );
    reply(
        cx,
        handle,
        &factory,
        "Conflict: object has been modified\n__SHELLRS_K8S_EXIT__:1\n",
    )
    .await;
    label(
        cx,
        handle,
        "k8s-operation-error",
        "Conflict: object has been modified (exit 1)",
    )
    .await;
    in_frame(cx, handle, |window, cx| {
        assert!(window.find("k8s-save-yaml").visible());
        window.click("k8s-detail-yaml", cx);
        window.press(SELECT_ALL, cx);
        window.press(
            if cfg!(target_os = "macos") {
                "cmd-c"
            } else {
                "ctrl-c"
            },
            cx,
        );
    });
    cx.update(|cx| assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), text));
    type_yaml(cx, handle, "kind: [}");
    in_frame(cx, handle, |window, cx| window.click("k8s-save-yaml", cx));
    cx.wait_for(handle.into(), Duration::from_secs(5), |window, _| {
        window
            .try_find("k8s-operation-error")
            .is_some_and(|element| {
                element
                    .label()
                    .is_some_and(|label| label.starts_with("YAML 无效"))
            })
    })
    .await;
    assert_eq!(factory.commands.lock().unwrap().len(), before + 1);
    in_frame(cx, handle, |window, cx| window.click("k8s-cancel-edit", cx));
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-detail-close", cx)
    });
}

#[gpui_kit::test]
async fn kubernetes_saving_yaml_refreshes_the_resource_and_returns_to_read_only(
    cx: &mut TestAppContext,
) {
    let (handle, factory) =
        open_mutation_resource(cx, ResourceKind::ConfigMap, Some(ResourceOperation::Edit)).await;
    cx.wait_for(handle.into(), Duration::from_secs(5), |window, _| {
        window.try_find("k8s-save-yaml").is_some()
    })
    .await;
    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(
            Box::new(KubernetesAction(KubernetesCommand::ConfirmMutation(
                "not-issued".into(),
            ))),
            cx,
        )
    });
    assert!(
        factory.pending.lock().unwrap().is_empty(),
        "no confirmation ticket means no write"
    );
    let mut edited = mutation_resource(ResourceKind::ConfigMap);
    edited["data"]["greeting"] = serde_json::json!("saved");
    type_yaml(cx, handle, &serde_yaml_ng::to_string(&edited).unwrap());
    in_frame(cx, handle, |window, cx| window.click("k8s-save-yaml", cx));
    wait_for_confirmation(cx, handle).await;
    in_frame(cx, handle, |window, cx| window.click("ok", cx));
    edited["metadata"]["resourceVersion"] = serde_json::json!("43");
    reply(cx, handle, &factory, &mutation_wire(&edited)).await;
    cx.wait_for(handle.into(), Duration::from_secs(5), |_, _| {
        factory.pending.lock().unwrap().len() == 2
    })
    .await;
    for _ in 0..2 {
        let request = factory.pending.lock().unwrap().pop_front().unwrap();
        let command = script(&request.command);
        let response = if command.contains("jsonpath-as-json") {
            mutation_wire(&serde_json::json!([edited["metadata"]]))
        } else {
            assert!(command.contains(" get configmaps 'managed'"));
            mutation_wire(&edited)
        };
        request.reply.send(Ok(response)).unwrap();
    }
    cx.run_until_parked();
    cx.wait_for(handle.into(), Duration::from_secs(5), |window, _| {
        window.try_find("k8s-save-yaml").is_none()
            && window.try_find("k8s-details-reading").is_none()
            && window
                .try_find("k8s-edit")
                .is_some_and(|button| button.disabled() != Some(true))
    })
    .await;
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-detail-yaml", cx);
        window.press(SELECT_ALL, cx);
        window.input("cannot change read-only YAML", cx);
        window.press(
            if cfg!(target_os = "macos") {
                "cmd-c"
            } else {
                "ctrl-c"
            },
            cx,
        );
    });
    cx.update(|cx| {
        let copied = cx.read_from_clipboard().unwrap().text().unwrap();
        let parsed: serde_json::Value = serde_yaml_ng::from_str(&copied).unwrap();
        assert_eq!(parsed["data"]["greeting"], "saved");
        assert_eq!(parsed["metadata"]["resourceVersion"], "43");
    });
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-detail-close", cx)
    });
}

#[gpui_kit::test]
async fn kubernetes_restart_and_delete_always_confirm_and_target_one_resource(
    cx: &mut TestAppContext,
) {
    for kind in [
        ResourceKind::Deployment,
        ResourceKind::StatefulSet,
        ResourceKind::Pod,
        ResourceKind::ConfigMap,
    ] {
        let operation = if kind == ResourceKind::ConfigMap {
            ResourceOperation::Delete
        } else {
            ResourceOperation::Restart
        };
        let (handle, factory) = open_mutation_resource(cx, kind, Some(operation)).await;
        wait_for_confirmation(cx, handle).await;
        let before = factory.commands.lock().unwrap().len();
        assert!(factory.pending.lock().unwrap().is_empty());
        in_frame(cx, handle, |window, cx| window.click("cancel", cx));
        assert_eq!(factory.commands.lock().unwrap().len(), before);
        in_frame(cx, handle, |window, cx| {
            window.click(
                if operation == ResourceOperation::Delete {
                    "k8s-delete"
                } else {
                    "k8s-restart"
                },
                cx,
            )
        });
        wait_for_confirmation(cx, handle).await;
        in_frame(cx, handle, |window, cx| window.click("ok", cx));
        cx.wait_for(handle.into(), Duration::from_secs(5), |_, _| {
            !factory.pending.lock().unwrap().is_empty()
        })
        .await;
        let command = script(factory.commands.lock().unwrap().last().unwrap());
        assert!(command.contains("--context='prod-cluster'"));
        assert!(command.contains("uid-original") && command.contains("42"));
        if matches!(kind, ResourceKind::Deployment | ResourceKind::StatefulSet) {
            assert!(
                command.contains(" patch ")
                    && command.contains("--namespace='prod'")
                    && command.contains("'managed'")
            );
            assert!(
                command.contains("/metadata/uid") && command.contains("/metadata/resourceVersion")
            );
        } else {
            assert!(
                command.contains(" delete --raw=")
                    && command.contains("/namespaces/prod/")
                    && command.contains("/managed'")
            );
        }
        reply(cx, handle, &factory, "Forbidden\n__SHELLRS_K8S_EXIT__:1\n").await;
        label(cx, handle, "k8s-operation-error", "Forbidden (exit 1)").await;
        assert_eq!(factory.commands.lock().unwrap().len(), before + 1);
        in_frame(cx, handle, |window, cx| window.click("k8s-delete", cx));
        wait_for_confirmation(cx, handle).await;
        in_frame(cx, handle, |window, cx| window.click("ok", cx));
        reply(
            cx,
            handle,
            &factory,
            "{\"kind\":\"Status\",\"status\":\"Success\"}\n__SHELLRS_K8S_EXIT__:0\n",
        )
        .await;
        label(cx, handle, "k8s-operation-submitted", "删除请求已提交").await;
        let empty = if kind == ResourceKind::ConfigMap {
            serde_json::json!([])
        } else {
            serde_json::json!({"items":[]})
        };
        reply(cx, handle, &factory, &mutation_wire(&empty)).await;
        label(cx, handle, "k8s-count", "0 项资源 · 2 个命名空间").await;
        in_frame(cx, handle, |window, cx| {
            window.click("k8s-detail-close", cx)
        });
    }
}

#[gpui_kit::test]
async fn kubernetes_terminating_pods_cannot_restart(cx: &mut TestAppContext) {
    let (handle, factory) = open_mutation_resource(cx, ResourceKind::Pod, None).await;
    label(cx, handle, "k8s-overview:k8s.name", "managed").await;
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-detail-refresh", cx)
    });
    let mut pod = mutation_resource(ResourceKind::Pod);
    pod["metadata"]["deletionTimestamp"] = serde_json::json!("2026-10-15T01:00:00Z");
    reply(cx, handle, &factory, &mutation_wire(&pod)).await;
    cx.wait_for(handle.into(), Duration::from_secs(5), |window, _| {
        window.try_find("k8s-details-reading").is_none()
    })
    .await;
    let before = factory.commands.lock().unwrap().len();
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-restart", cx);
    });
    assert_eq!(factory.commands.lock().unwrap().len(), before);
    in_frame(cx, handle, |window, cx| {
        assert!(window.try_find("ok").is_none());
        assert!(window.try_find("k8s-operation-error").is_none());
        window.click("k8s-detail-close", cx);
    });
}

#[gpui_kit::test]
async fn kubernetes_restart_cancels_with_escape_and_finishes_after_closing_its_details(
    cx: &mut TestAppContext,
) {
    let (handle, factory) =
        open_mutation_resource(cx, ResourceKind::Pod, Some(ResourceOperation::Restart)).await;
    wait_for_confirmation(cx, handle).await;
    let before = factory.commands.lock().unwrap().len();
    in_frame(cx, handle, |window, cx| window.press("escape", cx));
    in_frame(cx, handle, |window, cx| {
        assert!(window.try_find("ok").is_none());
        assert!(window.find("k8s-details").visible());
        assert!(factory.pending.lock().unwrap().is_empty());
        assert_eq!(factory.commands.lock().unwrap().len(), before);
        window.click("k8s-restart", cx);
    });
    wait_for_confirmation(cx, handle).await;
    in_frame(cx, handle, |window, cx| window.click("ok", cx));
    cx.wait_for(handle.into(), Duration::from_secs(5), |_, _| {
        factory.pending.lock().unwrap().len() == 1
    })
    .await;
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-detail-close", cx)
    });
    next_option(cx, handle, "k8s-context");
    cx.wait_for(handle.into(), Duration::from_secs(5), |_, _| {
        factory.pending.lock().unwrap().len() == 2
    })
    .await;
    let submitted = factory.pending.lock().unwrap().pop_front().unwrap();
    let command = script(&submitted.command);
    assert!(command.contains("--context='prod-cluster'"));
    assert!(command.contains("delete --raw='/api/v1/namespaces/prod/pods/managed'"));
    submitted
        .reply
        .send(Ok(mutation_wire(
            &serde_json::json!({"kind":"Status","status":"Success"}),
        )))
        .expect("closing details must still observe an already submitted write");
    cx.run_until_parked();
    cx.wait_for(handle.into(), Duration::from_secs(5), |window, cx| {
        window.notifications(cx).len() == 1
    })
    .await;
    assert_eq!(
        factory.commands.lock().unwrap().len(),
        before + 2,
        "the previous cluster's result must not refresh the new scope"
    );
    reply(cx, handle, &factory, CONTEXTS).await;
    reply(cx, handle, &factory, NAMESPACES).await;
    reply(cx, handle, &factory, PODS).await;
    label(cx, handle, "k8s-count", "2 项资源 · 2 个命名空间").await;
    in_frame(cx, handle, |window, _| {
        assert!(window.try_find("k8s-details").is_none());
        assert_eq!(window.find("k8s-context").value(), Some("test-cluster"));
    });
    assert_eq!(factory.commands.lock().unwrap().len(), before + 4);
}

#[gpui_kit::test]
async fn kubernetes_unused_or_failed_does_not_interrupt_terminal_input(cx: &mut TestAppContext) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[
        "kubectl: command not found\n__SHELLRS_K8S_EXIT__:127\n",
    ]));
    let (handle, workspace) =
        open_workspace_with_kubernetes(cx, HostStore::seed(), factory.clone());
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", INITIAL_WEB_TERMINAL), cx);
    });
    cx.wait_for(handle.into(), Duration::from_secs(5), |_, cx| {
        remote_lifecycle(&workspace, RemoteTerminalId(INITIAL_WEB_TERMINAL), cx)
            == TerminalLifecycle::Running
    })
    .await;
    in_frame(cx, handle, |window, cx| {
        let focus = workspace
            .read(cx)
            .remote_terminal(shellrs::terminal::RemoteTerminalId(INITIAL_WEB_TERMINAL))
            .unwrap()
            .read(cx)
            .focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        assert_eq!(
            window.find(("terminal", INITIAL_WEB_TERMINAL)).focused(),
            Some(true)
        );
        window.press("tab", cx);
    });
    cx.wait_for(handle.into(), Duration::from_secs(5), |_, _| {
        factory.written_text().contains('\t')
    })
    .await;
    in_frame(cx, handle, |window, cx| window.click("tool-snippets", cx));
    cx.executor().advance_clock(Duration::from_secs(60));
    cx.run_until_parked();
    assert_eq!(
        factory.exec_count(),
        0,
        "unused Kubernetes must issue no queries"
    );

    in_frame(cx, handle, |window, cx| window.click("tool-kubernetes", cx));
    label(
        cx,
        handle,
        "k8s-error",
        "kubectl: command not found (exit 127)",
    )
    .await;
    in_frame(cx, handle, |window, cx| {
        let focus = workspace
            .read(cx)
            .remote_terminal(shellrs::terminal::RemoteTerminalId(INITIAL_WEB_TERMINAL))
            .unwrap()
            .read(cx)
            .focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.press("shift-tab", cx);
        window.click("tool-kubernetes", cx);
        window.press("enter", cx);
    });
    cx.wait_for(handle.into(), Duration::from_secs(5), |_, _| {
        let text = factory.written_text();
        text.contains("\x1b[Z") && text.contains('\r')
    })
    .await;
    cx.executor().advance_clock(Duration::from_secs(60));
    cx.run_until_parked();
    assert_eq!(
        factory.exec_count(),
        1,
        "hidden Kubernetes must stop polling"
    );
}

const PODS: &str = r#"{"items":[
{"metadata":{"name":"api-0","namespace":"prod","labels":{"app":"api"}},"spec":{"containers":[{"name":"api"},{"name":"sidecar"}],"nodeName":"node-1"},"status":{"phase":"Running","containerStatuses":[{"ready":true},{"ready":true}]}},
{"metadata":{"name":"worker-0","namespace":"test"},"spec":{"containers":[{"name":"worker"}]},"status":{"phase":"Pending"}}]}
__SHELLRS_K8S_EXIT__:0
"#;
const PROD_PODS: &str = r#"{"items":[{"metadata":{"name":"api-0","namespace":"prod"},"spec":{"containers":[{"name":"api"},{"name":"sidecar"}]},"status":{"phase":"Running"}}]}
__SHELLRS_K8S_EXIT__:0
"#;
const DEPLOYMENTS: &str = r#"{"items":[{"metadata":{"name":"api","namespace":"prod"},"spec":{"replicas":3},"status":{"readyReplicas":2}}]}
__SHELLRS_K8S_EXIT__:0
"#;
const POD_DETAIL: &str = r#"{"kind":"Pod","metadata":{"name":"api-0","namespace":"prod"},"spec":{"containers":[{"name":"api","image":"api:v1"},{"name":"sidecar","image":"sidecar:v1"}],"initContainers":[{"name":"init"}],"ephemeralContainers":[{"name":"debug"}]},"status":{"phase":"Running","podIP":"10.1.0.2"}}
__SHELLRS_K8S_EXIT__:0
"#;
const DEPLOY_DETAIL: &str = r#"{"kind":"Deployment","metadata":{"name":"api","namespace":"prod"},"spec":{"replicas":3,"selector":{"matchLabels":{"app":"api"}}},"status":{"readyReplicas":2}}
__SHELLRS_K8S_EXIT__:0
"#;
const LOGS: &str = "2026-10-12T01:00:00Z INFO ready\n2026-10-12T01:00:01Z ERROR 请求失败\n2026-10-12T01:00:02Z WARN retry\n\n__SHELLRS_K8S_EXIT__:0\n";
const SIDECAR_LOGS: &str = "sidecar ready\nsidecar healthy\n\n__SHELLRS_K8S_EXIT__:0\n";
const PREVIOUS_LOGS: &str = "previous container exited\n\n__SHELLRS_K8S_EXIT__:0\n";

#[gpui_kit::test]
async fn kubernetes_details_show_key_properties_and_keep_yaml_in_its_own_tab(
    cx: &mut TestAppContext,
) {
    let mut value: serde_json::Value =
        serde_json::from_str(POD_DETAIL.split("\n__SHELLRS").next().unwrap()).unwrap();
    value["spec"]["containers"][0]["resources"] =
        serde_json::json!({"requests":{"cpu":"100m","memory":"128Mi"},"limits":{"memory":"512Mi"}});
    value["status"]["containerStatuses"] = serde_json::json!([{"name":"api","ready":true,"restartCount":2},{"name":"sidecar","ready":false,"restartCount":1}]);
    let output = format!("{value}\n__SHELLRS_K8S_EXIT__:0\n");
    let factory = Arc::new(FakeTerminalFactory::answering(&[
        CONTEXTS, NAMESPACES, PODS, &output,
    ]));
    let handle = open_kubernetes(cx, factory);
    label(cx, handle, "k8s-count", "2 项资源 · 2 个命名空间").await;
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-resource:pods:prod/api-0:open", cx)
    });
    label(cx, handle, "k8s-overview:k8s.restarts", "3").await;
    label(cx, handle, "k8s-overview:k8s.ready_containers", "1/2").await;
    in_frame(cx, handle, |window, _| {
        assert!(window.try_find("k8s-detail-yaml").is_none());
        assert!(
            window
                .find("k8s-overview:k8s.resource_requests")
                .label()
                .unwrap()
                .contains("100m")
        );
        let name = window.find("k8s-overview:k8s.name").bounds();
        let namespace = window.find("k8s-overview:k8s.namespace").bounds();
        assert_eq!(name.left(), namespace.left());
        assert_eq!(name.right(), namespace.right());
        assert!(name.bottom() <= namespace.top());
        assert!(window.try_find("k8s-copy-field:k8s.name").is_none());
    });
    in_frame(cx, handle, |window, cx| {
        window.within("k8s-detail-tabs").click(4usize, cx)
    });
    in_frame(cx, handle, |window, _| {
        assert!(window.find("k8s-detail-yaml").visible())
    });
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-detail-close", cx)
    });
}

#[gpui_kit::test]
async fn kubernetes_relation_query_is_canceled_when_details_close(cx: &mut TestAppContext) {
    let factory = Arc::new(PendingFactory::default());
    let (handle, _) = open_workspace_with_kubernetes(cx, HostStore::seed(), factory.clone());
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", INITIAL_WEB_TERMINAL), cx)
    });
    in_frame(cx, handle, |window, cx| window.click("tool-kubernetes", cx));
    reply(cx, handle, &factory, CONTEXTS).await;
    reply(cx, handle, &factory, NAMESPACES).await;
    reply(cx, handle, &factory, PODS).await;
    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(
            Box::new(KubernetesAction(KubernetesCommand::Kind(
                ResourceKind::Deployment,
            ))),
            cx,
        )
    });
    reply(cx, handle, &factory, DEPLOYMENTS).await;
    label(cx, handle, "k8s-count", "1 项资源 · 2 个命名空间").await;
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-resource:deployments:prod/api:open", cx)
    });
    reply(cx, handle, &factory, GRAPH_DEPLOY).await;
    in_frame(cx, handle, |window, cx| {
        window.within("k8s-detail-tabs").click(3usize, cx)
    });
    cx.wait_for(handle.into(), Duration::from_secs(5), |_, _| {
        !factory.pending.lock().unwrap().is_empty()
    })
    .await;
    let count = factory.commands.lock().unwrap().len();
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-detail-close", cx)
    });
    reply(cx, handle, &factory, DEPLOYMENTS).await;
    in_frame(cx, handle, |window, _| {
        assert!(window.try_find("k8s-details").is_none())
    });
    assert_eq!(factory.commands.lock().unwrap().len(), count);
}
const GRAPH_DEPLOY: &str = r#"{"kind":"Deployment","metadata":{"name":"api","namespace":"prod","uid":"deploy-uid"}}
__SHELLRS_K8S_EXIT__:0
"#;

async fn graph_replies(
    cx: &mut TestAppContext,
    handle: WindowHandle<Root>,
    factory: &PendingFactory,
    count: usize,
) {
    for _ in 0..count {
        cx.wait_for(handle.into(), Duration::from_secs(5), |_, _| {
            !factory.pending.lock().unwrap().is_empty()
        })
        .await;
        let wire = factory
            .pending
            .lock()
            .unwrap()
            .front()
            .unwrap()
            .command
            .clone();
        let command = script(&wire);
        let value = if command.contains("/deployments?") {
            serde_json::json!({"items":[serde_json::from_str::<serde_json::Value>(GRAPH_DEPLOY.split("\n__SHELLRS").next().unwrap()).unwrap()]})
        } else if command.contains("/pods?") {
            let mut items = vec![
                serde_json::json!({"kind":"Pod","metadata":{"name":"api-0","namespace":"prod","uid":"pod-uid","labels":{"tier":"backend"},"ownerReferences":[{"kind":"ReplicaSet","name":"api-rs","uid":"rs-uid"}]},"spec":{"containers":[{"name":"api","image":"api:v1"}]}}),
            ];
            for ix in 0..7 {
                items[0]["spec"]["containers"]
                    .as_array_mut()
                    .unwrap()
                    .push(serde_json::json!({"name":format!("extra{ix}"),"image":"sidecar:v1"}));
            }
            for ix in 0..120 {
                items.push(serde_json::json!({"kind":"Pod","metadata":{"name":format!("other-{ix}"),"namespace":"prod","labels":{"tier":"backend"}},"spec":{"containers":[{"name":"worker"}]}}));
            }
            serde_json::json!({"items":items})
        } else if command.contains("/services?") {
            serde_json::json!({"items":[{"kind":"Service","metadata":{"name":"shared","namespace":"prod"},"spec":{"selector":{"tier":"backend"}}}]})
        } else if command.contains("/poddisruptionbudgets?") {
            serde_json::json!({"items":[{"kind":"PodDisruptionBudget","metadata":{"name":"shared","namespace":"prod"},"spec":{"selector":{}}}]})
        } else if command.contains("/replicasets?") {
            serde_json::json!({"items":[{"kind":"ReplicaSet","metadata":{"name":"api-rs","namespace":"prod","uid":"rs-uid","ownerReferences":[{"kind":"Deployment","name":"api","uid":"deploy-uid"}]}}]})
        } else if command.contains("get secrets ") {
            serde_json::json!([])
        } else {
            serde_json::json!({"items":[]})
        };
        if command.contains("/statefulsets?") {
            reply(
                cx,
                handle,
                factory,
                "Forbidden: statefulsets\n__SHELLRS_K8S_EXIT__:1\n",
            )
            .await;
        } else {
            reply(
                cx,
                handle,
                factory,
                &format!("{value}\n__SHELLRS_K8S_EXIT__:0\n"),
            )
            .await;
        }
    }
    cx.wait_for(handle.into(), Duration::from_secs(5), |window, _| {
        window.try_find("k8s-relations-reading").is_none()
    })
    .await;
}

#[gpui_kit::test]
async fn kubernetes_graph_container_opens_container_details_and_returns_to_pod(
    cx: &mut TestAppContext,
) {
    let factory = Arc::new(PendingFactory::default());
    let (handle, _) = open_workspace_with_kubernetes(cx, HostStore::seed(), factory.clone());
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", INITIAL_WEB_TERMINAL), cx)
    });
    in_frame(cx, handle, |window, cx| window.click("tool-kubernetes", cx));
    reply(cx, handle, &factory, CONTEXTS).await;
    reply(cx, handle, &factory, NAMESPACES).await;
    reply(cx, handle, &factory, PODS).await;
    label(cx, handle, "k8s-count", "2 项资源 · 2 个命名空间").await;
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-resource:pods:prod/api-0:open", cx)
    });
    let mut graph_detail: serde_json::Value =
        serde_json::from_str(POD_DETAIL.split("\n__SHELLRS").next().unwrap()).unwrap();
    graph_detail["metadata"]["ownerReferences"] =
        serde_json::json!([{"kind":"ReplicaSet","name":"api-rs","uid":"rs-uid"}]);
    for ix in 0..7 {
        graph_detail["spec"]["containers"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"name":format!("extra{ix}"),"image":"sidecar:v1"}));
    }
    reply(
        cx,
        handle,
        &factory,
        &format!("{graph_detail}\n__SHELLRS_K8S_EXIT__:0\n"),
    )
    .await;
    in_frame(cx, handle, |window, cx| {
        window.within("k8s-detail-tabs").click(3usize, cx)
    });
    graph_replies(cx, handle, &factory, 0).await;
    let requests = factory.commands.lock().unwrap().len();
    in_frame(cx, handle, |window, cx| {
        assert!(window.try_find("k8s-relation-viewport").is_none());
        assert!(window.try_find("k8s-relation-edges").is_none());
        assert!(
            window
                .try_find("k8s-graph-node:k8s-resource:pods:prod/api-0:container:extra2")
                .is_none()
        );
        let parent = window
            .find("k8s-graph-node:k8s-resource:replicasets:prod/api-rs")
            .bounds();
        let current = window
            .find("k8s-graph-node:k8s-resource:pods:prod/api-0")
            .bounds();
        let child = window
            .find("k8s-graph-node:k8s-resource:pods:prod/api-0:container:api")
            .bounds();
        assert!(parent.right() < current.left() && current.right() < child.left());
        assert_eq!(parent.top(), current.top());
        assert_eq!(current.top(), child.top());
        window.click("k8s-graph-downstream-next", cx);
    });
    in_frame(cx, handle, |window, cx| {
        assert!(
            window
                .try_find("k8s-graph-node:k8s-resource:pods:prod/api-0:container:api")
                .is_none()
        );
        assert!(
            window
                .find("k8s-graph-node:k8s-resource:pods:prod/api-0:container:extra2")
                .visible()
        );
        assert!(
            window
                .try_find("k8s-graph-node:k8s-resource:pods:prod/api-0:container:extra5")
                .is_none()
        );
        window.click("k8s-graph-downstream-previous", cx);
    });
    assert_eq!(
        factory.commands.lock().unwrap().len(),
        requests,
        "paging must not query SSH"
    );
    use gpui_kit::component::{Theme, ThemeMode};
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        for zoom in [12., 20.] {
            cx.update_window(handle.into(), |_, window, cx| {
                Theme::change(mode, Some(window), cx);
                Theme::global_mut(cx).font_size = px(zoom);
            })
            .unwrap();
            cx.simulate_window_resize(handle.into(), size(px(960.), px(600.)));
            cx.run_until_parked();
            in_frame(cx, handle, |window, _| {
                assert_eq!(window.viewport_size().width, px(960.));
                let graph = window.find("k8s-relation-graph").bounds();
                assert!(graph.left() >= px(0.) && graph.right() <= px(960.));
                for key in [
                    "k8s-graph-node:k8s-resource:replicasets:prod/api-rs",
                    "k8s-graph-node:k8s-resource:pods:prod/api-0",
                    "k8s-graph-node:k8s-resource:pods:prod/api-0:container:api",
                ] {
                    let node = window.find(key);
                    assert!(node.visible(), "{key} at {zoom}");
                    assert!(
                        node.bounds().left() >= graph.left()
                            && node.bounds().right() <= graph.right()
                    );
                }
                assert!(window.find("k8s-detail-close").visible());
            });
        }
    }
    cx.update_window(handle.into(), |_, _, cx| {
        Theme::global_mut(cx).font_size = px(16.);
    })
    .unwrap();
    cx.simulate_window_resize(handle.into(), size(px(1280.), px(800.)));
    cx.run_until_parked();
    in_frame(cx, handle, |window, cx| {
        window.click(
            "k8s-graph-node:k8s-resource:pods:prod/api-0:container:api",
            cx,
        )
    });
    reply(cx, handle, &factory, POD_DETAIL).await;
    label(cx, handle, "k8s-overview:k8s.name", "api").await;
    label(cx, handle, "k8s-overview:k8s.pod", "api-0").await;
    in_frame(cx, handle, |window, cx| window.click("k8s-detail-back", cx));
    reply(cx, handle, &factory, POD_DETAIL).await;
    label(cx, handle, "k8s-overview:k8s.name", "api-0").await;
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-detail-close", cx)
    });
}

fn open_kubernetes(
    cx: &mut TestAppContext,
    factory: Arc<FakeTerminalFactory>,
) -> WindowHandle<Root> {
    let (handle, _) = open_workspace_with_kubernetes(cx, HostStore::seed(), factory);
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", INITIAL_WEB_TERMINAL), cx)
    });
    in_frame(cx, handle, |window, cx| window.click("tool-kubernetes", cx));
    handle
}

async fn label(cx: &mut TestAppContext, handle: WindowHandle<Root>, id: &str, expected: &str) {
    let id = SharedString::from(id.to_owned());
    cx.wait_for(handle.into(), Duration::from_secs(5), |window, _| {
        window
            .try_find(id.clone())
            .is_some_and(|element| element.label() == Some(expected))
    })
    .await;
}

async fn container(cx: &mut TestAppContext, handle: WindowHandle<Root>, expected: &str) {
    cx.wait_for(handle.into(), Duration::from_secs(5), |window, _| {
        window
            .try_find("k8s-container")
            .is_some_and(|element| element.value() == Some(expected))
    })
    .await;
}

#[gpui_kit::test]
async fn kubernetes_result_rows_share_spines_and_height_across_zoom_and_theme(
    cx: &mut TestAppContext,
) {
    use gpui_kit::component::{Theme, ThemeMode};
    let factory = Arc::new(FakeTerminalFactory::answering(&[
        CONTEXTS, NAMESPACES, PODS,
    ]));
    let handle = open_kubernetes(cx, factory.clone());
    label(cx, handle, "k8s-count", "2 项资源 · 2 个命名空间").await;
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        for zoom in [12., 16., 20.] {
            cx.update_window(handle.into(), |_, window, cx| {
                Theme::change(mode, Some(window), cx);
                Theme::global_mut(cx).font_size = px(zoom);
            })
            .unwrap();
            cx.run_until_parked();
            in_frame(cx, handle, |window, _| {
                let context = window.find("k8s-context").bounds();
                for id in ["k8s-namespace", "k8s-kind"] {
                    let field = window.find(id).bounds();
                    assert_eq!(context.left(), field.left(), "{id}: {mode:?} / {zoom}");
                    assert_eq!(context.right(), field.right(), "{id}: {mode:?} / {zoom}");
                }
                let first = window.find("k8s-resource:pods:prod/api-0").bounds();
                let second = window.find("k8s-resource:pods:test/worker-0").bounds();
                assert_eq!(first.size.height, second.size.height);
                assert!(first.bottom() <= second.top());
                for key in [
                    "k8s-resource:pods:prod/api-0",
                    "k8s-resource:pods:test/worker-0",
                ] {
                    let name = window
                        .find(SharedString::from(format!("{key}:name")))
                        .bounds();
                    let summary = window
                        .find(SharedString::from(format!("{key}:summary")))
                        .bounds();
                    assert_eq!(name.left(), summary.left(), "{mode:?} / {zoom}");
                    assert!(name.bottom() <= summary.top());
                }
            });
        }
    }
    assert_eq!(factory.exec_count(), 3, "zoom and theme changes are local");
}

#[gpui_kit::test]
async fn kubernetes_fields_support_native_text_copy_and_pod_metrics_show_real_usage(
    cx: &mut TestAppContext,
) {
    let detail = "{\"metadata\":{\"name\":\"api-0\",\"namespace\":\"prod\",\"labels\":{\"app\":\"api\"}},\"spec\":{\"containers\":[{\"name\":\"api\",\"resources\":{\"limits\":{\"cpu\":\"1\",\"memory\":\"512Mi\"}}},{\"name\":\"sidecar\"}]}}\n__SHELLRS_K8S_EXIT__:0\n";
    let usage = "{\"metadata\":{\"name\":\"api-0\",\"namespace\":\"prod\"},\"containers\":[{\"name\":\"api\",\"usage\":{\"cpu\":\"250m\",\"memory\":\"128Mi\"}},{\"name\":\"sidecar\",\"usage\":{\"cpu\":\"50m\",\"memory\":\"32Mi\"}}]}\n__SHELLRS_K8S_EXIT__:0\n";
    let factory = Arc::new(FakeTerminalFactory::answering(&[
        CONTEXTS, NAMESPACES, PODS, detail, usage,
    ]));
    let handle = open_kubernetes(cx, factory.clone());
    label(cx, handle, "k8s-count", "2 项资源 · 2 个命名空间").await;
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-resource:pods:prod/api-0:open", cx)
    });
    label(cx, handle, "k8s-overview:k8s.namespace", "prod").await;
    in_frame(cx, handle, |window, cx| {
        assert!(window.try_find("k8s-overview:k8s.labels").is_none());
        let value = window
            .find((
                gpui_kit::ElementId::from("k8s-overview:k8s.namespace"),
                "value-cell",
            ))
            .bounds();
        window.drag(
            point(value.left() + px(1.), value.center().y),
            point(value.right() - px(1.), value.center().y),
            cx,
        );
        window.press(
            if cfg!(target_os = "macos") {
                "cmd-c"
            } else {
                "ctrl-c"
            },
            cx,
        );
    });
    cx.update(|cx| {
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().as_deref(),
            Some("prod")
        )
    });
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-more-properties", cx)
    });
    label(cx, handle, "k8s-overview:k8s.labels", "app=api").await;
    in_frame(cx, handle, |window, cx| {
        window.within("k8s-detail-tabs").click(5usize, cx)
    });
    label(cx, handle, "k8s-cpu-usage", "—").await;
    label(cx, handle, "k8s-cpu-amount", "0.300").await;
    in_frame(cx, handle, |window, _| {
        assert_eq!(window.find("k8s-memory-usage").label(), Some("—"));
        assert!(window.try_find("k8s-disk-usage").is_none());
    });
    assert_eq!(factory.exec_count(), 5);
    assert!(
        script(&factory.exec_commands()[4])
            .contains("/apis/metrics.k8s.io/v1beta1/namespaces/prod/pods/api%2D0")
    );
    in_frame(cx, handle, |window, cx| {
        window.within("k8s-detail-tabs").click(0usize, cx)
    });
    cx.executor().advance_clock(Duration::from_secs(30));
    cx.run_until_parked();
    assert_eq!(
        factory.exec_count(),
        5,
        "performance queries stop outside their page"
    );
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-detail-close", cx)
    });
}

#[gpui_kit::test]
async fn kubernetes_node_metrics_are_on_demand_serial_and_stop_on_tab_change(
    cx: &mut TestAppContext,
) {
    let factory = Arc::new(PendingFactory::default());
    let (handle, _) = open_workspace_with_kubernetes(cx, HostStore::seed(), factory.clone());
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", INITIAL_WEB_TERMINAL), cx)
    });
    in_frame(cx, handle, |window, cx| window.click("tool-kubernetes", cx));
    reply(cx, handle, &factory, CONTEXTS).await;
    reply(cx, handle, &factory, NAMESPACES).await;
    reply(cx, handle, &factory, PODS).await;
    in_frame(cx, handle, |window, cx| {
        window.within("k8s-sections").click(0usize, cx)
    });
    reply(
        cx,
        handle,
        &factory,
        "{\"items\":[{\"metadata\":{\"name\":\"node-1\"}}]}\n__SHELLRS_K8S_EXIT__:0\n",
    )
    .await;
    label(cx, handle, "k8s-count", "1 项资源 · 2 个命名空间").await;
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-resource:nodes:/node-1:open", cx)
    });
    reply(cx, handle, &factory, "{\"metadata\":{\"name\":\"node-1\"},\"status\":{\"capacity\":{\"cpu\":\"8\",\"memory\":\"16Gi\"}}}\n__SHELLRS_K8S_EXIT__:0\n").await;
    label(cx, handle, "k8s-overview:k8s.name", "node-1").await;
    let before = factory.commands.lock().unwrap().len();
    assert!(factory.pending.lock().unwrap().is_empty());
    in_frame(cx, handle, |window, cx| {
        window.within("k8s-detail-tabs").click(5usize, cx)
    });
    reply(cx, handle, &factory, "{\"metadata\":{\"name\":\"node-1\"},\"usage\":{\"cpu\":\"2\",\"memory\":\"4Gi\"}}\n__SHELLRS_K8S_EXIT__:0\n").await;
    label(cx, handle, "k8s-cpu-usage", "25.0%").await;
    label(cx, handle, "k8s-memory-usage", "25.0%").await;
    reply(cx, handle, &factory, "{\"node\":{\"nodeName\":\"node-1\",\"fs\":{\"usedBytes\":250,\"capacityBytes\":1000}}}\n__SHELLRS_K8S_EXIT__:0\n").await;
    label(cx, handle, "k8s-disk-usage", "25.0%").await;
    assert_eq!(factory.commands.lock().unwrap().len(), before + 2);
    cx.executor().advance_clock(Duration::from_secs(10));
    cx.run_until_parked();
    cx.wait_for(handle.into(), Duration::from_secs(5), |_, _| {
        factory.pending.lock().unwrap().len() == 1
    })
    .await;
    let count = factory.commands.lock().unwrap().len();
    cx.executor().advance_clock(Duration::from_secs(40));
    cx.run_until_parked();
    assert_eq!(
        factory.commands.lock().unwrap().len(),
        count,
        "slow reads never overlap"
    );
    in_frame(cx, handle, |window, cx| {
        window.within("k8s-detail-tabs").click(0usize, cx)
    });
    reply(cx, handle, &factory, "{\"metadata\":{\"name\":\"node-1\"},\"usage\":{\"cpu\":\"8\",\"memory\":\"16Gi\"}}\n__SHELLRS_K8S_EXIT__:0\n").await;
    cx.executor().advance_clock(Duration::from_secs(60));
    cx.run_until_parked();
    assert_eq!(factory.commands.lock().unwrap().len(), count);
    in_frame(cx, handle, |window, cx| {
        window.within("k8s-detail-tabs").click(5usize, cx)
    });
    reply(cx, handle, &factory, "Forbidden\n__SHELLRS_K8S_EXIT__:1\n").await;
    label(cx, handle, "k8s-metrics-error", "Forbidden (exit 1)").await;
    label(cx, handle, "k8s-cpu-usage", "25.0%").await;
    reply(cx, handle, &factory, "Forbidden\n__SHELLRS_K8S_EXIT__:1\n").await;
    label(cx, handle, "k8s-disk-error", "Forbidden (exit 1)").await;
    let count = factory.commands.lock().unwrap().len();
    cx.executor().advance_clock(Duration::from_secs(30));
    cx.run_until_parked();
    assert_eq!(
        factory.commands.lock().unwrap().len(),
        count,
        "failed metrics back off"
    );
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-detail-close", cx)
    });
    in_frame(cx, handle, |window, cx| window.click("tool-kubernetes", cx));
    let count = factory.commands.lock().unwrap().len();
    cx.executor().advance_clock(Duration::from_secs(60));
    cx.run_until_parked();
    assert_eq!(
        factory.commands.lock().unwrap().len(),
        count,
        "hidden Kubernetes stops polling"
    );
}

fn next_option(cx: &mut TestAppContext, handle: WindowHandle<Root>, id: &str) {
    let id = SharedString::from(id.to_owned());
    in_frame(cx, handle, |window, cx| {
        window.within(id).click("input", cx)
    });
    for key in ["down", "enter"] {
        in_frame(cx, handle, |window, cx| window.press(key, cx));
    }
}

fn script(wire: &str) -> String {
    let encoded = wire
        .strip_prefix("sh -c 'printf \"%b\" \"")
        .unwrap()
        .strip_suffix("\" | sh'")
        .unwrap();
    String::from_utf8(
        encoded
            .as_bytes()
            .as_chunks::<5>()
            .0
            .iter()
            .map(|chunk| u8::from_str_radix(std::str::from_utf8(&chunk[2..]).unwrap(), 8).unwrap())
            .collect(),
    )
    .unwrap()
}

#[gpui_kit::test]
async fn kubernetes_discovers_namespaces_and_filters_resource_queries(cx: &mut TestAppContext) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[
        CONTEXTS,
        NAMESPACES,
        PODS,
        PROD_PODS,
        DEPLOYMENTS,
    ]));
    let handle = open_kubernetes(cx, factory.clone());
    label(cx, handle, "k8s-count", "2 项资源 · 2 个命名空间").await;
    in_frame(cx, handle, |window, _| {
        assert_eq!(window.find("k8s-context").value(), Some("prod-cluster"));
        assert_eq!(window.find("k8s-namespace").value(), Some("全部命名空间"));
        assert!(window.find("k8s-resource:pods:prod/api-0").visible());
        assert!(window.find("k8s-resource:pods:test/worker-0").visible());
        assert!(
            window.find("k8s-context").bounds().bottom()
                <= window.find("k8s-namespace").bounds().top()
        );
    });
    assert!(script(&factory.exec_commands()[2]).contains("/api/v1/pods?limit=100"));
    next_option(cx, handle, "k8s-namespace");
    label(cx, handle, "k8s-count", "1 项资源 · 2 个命名空间").await;
    in_frame(cx, handle, |window, _| {
        assert_eq!(window.find("k8s-namespace").value(), Some("prod"));
        assert!(window.try_find("k8s-resource:pods:test/worker-0").is_none());
    });
    assert!(script(&factory.exec_commands()[3]).contains("/api/v1/namespaces/prod/pods?limit=100"));
    next_option(cx, handle, "k8s-kind");
    label(
        cx,
        handle,
        "k8s-resource:deployments:prod/api",
        "prod/api · 2/3",
    )
    .await;
    assert!(script(&factory.exec_commands()[4]).contains("/deployments?limit=100"));
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-search", cx);
        window.input("missing", cx);
    });
    in_frame(cx, handle, |window, _| {
        assert_eq!(
            window.find("k8s-count").label(),
            Some("0 项资源 · 2 个命名空间")
        );
    });
    assert_eq!(factory.exec_count(), 5); // Text filtering runs locally.
}

#[gpui_kit::test]
async fn kubernetes_searches_namespace_and_context_by_value(cx: &mut TestAppContext) {
    let test_pods = "{\"items\":[{\"metadata\":{\"name\":\"worker-0\",\"namespace\":\"test\"}}]}\n__SHELLRS_K8S_EXIT__:0\n";
    let factory = Arc::new(FakeTerminalFactory::answering(&[
        CONTEXTS, NAMESPACES, PODS, test_pods, CONTEXTS, NAMESPACES, PODS,
    ]));
    let handle = open_kubernetes(cx, factory.clone());
    label(cx, handle, "k8s-count", "2 项资源 · 2 个命名空间").await;
    in_frame(cx, handle, |window, cx| {
        window.within("k8s-namespace").click("input", cx)
    });
    in_frame(cx, handle, |window, cx| window.input("test", cx));
    for key in ["down", "enter"] {
        in_frame(cx, handle, |window, cx| window.press(key, cx));
    }
    label(cx, handle, "k8s-count", "1 项资源 · 2 个命名空间").await;
    in_frame(cx, handle, |window, _| {
        assert_eq!(window.find("k8s-namespace").value(), Some("test"))
    });
    assert!(script(&factory.exec_commands()[3]).contains("/namespaces/test/pods?"));
    in_frame(cx, handle, |window, cx| {
        window.within("k8s-context").click("input", cx)
    });
    in_frame(cx, handle, |window, cx| window.input("test-", cx));
    for key in ["down", "enter"] {
        in_frame(cx, handle, |window, cx| window.press(key, cx));
    }
    label(cx, handle, "k8s-count", "2 项资源 · 2 个命名空间").await;
    in_frame(cx, handle, |window, _| {
        assert_eq!(window.find("k8s-context").value(), Some("test-cluster"));
        assert_eq!(window.find("k8s-namespace").value(), Some("全部命名空间"));
    });
    assert!(script(&factory.exec_commands()[6]).contains("--context='test-cluster'"));
}

#[gpui_kit::test]
async fn kubernetes_sidebar_setting_hides_stops_and_reopens_the_tool(cx: &mut TestAppContext) {
    use shellrs::app::{OpenSettings, ToggleTool, ToolKind};
    let factory = Arc::new(FakeTerminalFactory::answering(&[
        CONTEXTS, NAMESPACES, PODS, CONTEXTS, NAMESPACES, PODS, POD_DETAIL, LOGS, LOGS,
    ]));
    let (handle, workspace) =
        open_workspace_with_kubernetes(cx, HostStore::seed(), factory.clone());
    // Match the settings sidebar's integration fixture so its virtualized
    // category and both tool rows are rendered together.
    cx.simulate_window_resize(handle.into(), size(px(1280.), px(1400.)));
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", INITIAL_WEB_TERMINAL), cx)
    });
    in_frame(cx, handle, |window, cx| window.click("tool-kubernetes", cx));
    label(cx, handle, "k8s-count", "2 项资源 · 2 个命名空间").await;
    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(Box::new(OpenSettings), cx)
    });
    in_frame(cx, handle, |window, cx| {
        window.within("settings").click("0-1", cx)
    });
    in_frame(cx, handle, |window, cx| {
        window.within("settings").click("0-1-2", cx)
    });
    in_frame(cx, handle, |window, cx| {
        window
            .within("sidebar-tools-shown")
            .click("sidebar-tool-kubernetes", cx);
    });
    cx.update(|cx| {
        assert!(
            !workspace
                .read(cx)
                .settings()
                .read(cx)
                .settings()
                .sidebar_tools
                .shows(ToolKind::Kubernetes)
        )
    });
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", INITIAL_WEB_TERMINAL), cx)
    });
    in_frame(cx, handle, |window, cx| {
        assert!(window.try_find("tool-kubernetes").is_none());
        assert!(window.try_find("kubernetes").is_none());
        window.dispatch_action(Box::new(ToggleTool(ToolKind::Kubernetes)), cx);
        window.dispatch_action(Box::new(KubernetesAction(KubernetesCommand::Refresh)), cx);
    });
    cx.executor().advance_clock(Duration::from_secs(60));
    cx.run_until_parked();
    assert_eq!(
        factory.exec_count(),
        3,
        "disabled tools cannot refresh or reopen through actions"
    );
    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(Box::new(OpenSettings), cx)
    });
    in_frame(cx, handle, |window, cx| {
        window.within("settings").click("0-1", cx)
    });
    in_frame(cx, handle, |window, cx| {
        window.within("settings").click("0-1-2", cx)
    });
    in_frame(cx, handle, |window, cx| {
        window
            .within("sidebar-tools-hidden")
            .click("sidebar-tool-kubernetes", cx)
    });
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", INITIAL_WEB_TERMINAL), cx)
    });
    in_frame(cx, handle, |window, cx| window.click("tool-kubernetes", cx));
    label(cx, handle, "k8s-count", "2 项资源 · 2 个命名空间").await;
    assert_eq!(
        factory.exec_count(),
        6,
        "re-enabled tools rediscover their scope"
    );
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-resource:pods:prod/api-0:open", cx)
    });
    in_frame(cx, handle, |window, cx| {
        window.within("k8s-detail-tabs").click(1usize, cx)
    });
    container(cx, handle, "api").await;
    label(cx, handle, "k8s-log-count", "匹配 3 / 3 行").await;
    in_frame(cx, handle, |window, cx| window.click("k8s-live", cx));
    cx.update(|cx| {
        let settings = workspace.read(cx).settings().clone();
        settings.update(cx, |store, cx| {
            store.update(
                |settings| settings.sidebar_tools.set(ToolKind::Kubernetes, false),
                cx,
            )
        });
    });
    cx.run_until_parked();
    let count = factory.exec_count();
    cx.executor().advance_clock(Duration::from_secs(60));
    cx.run_until_parked();
    assert_eq!(
        factory.exec_count(),
        count,
        "disabling also stops a retained inspector's live reads"
    );
}

#[gpui_kit::test]
async fn kubernetes_pod_logs_support_container_previous_and_regex_search(cx: &mut TestAppContext) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[
        CONTEXTS,
        NAMESPACES,
        PODS,
        POD_DETAIL,
        LOGS,
        SIDECAR_LOGS,
        PREVIOUS_LOGS,
    ]));
    let handle = open_kubernetes(cx, factory.clone());
    label(cx, handle, "k8s-count", "2 项资源 · 2 个命名空间").await;
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-resource:pods:prod/api-0:open", cx)
    });
    // Open from a real row, wait for the real detail query to discover containers.
    in_frame(cx, handle, |window, cx| {
        window.within("k8s-detail-tabs").click(1usize, cx)
    });
    container(cx, handle, "api").await;
    label(cx, handle, "k8s-log-count", "匹配 3 / 3 行").await;
    in_frame(cx, handle, |window, _| {
        assert!(
            window
                .find("k8s-log-output")
                .label()
                .unwrap()
                .contains("2026-10-12T09:00:00+08:00")
        );
    });
    assert!(script(&factory.exec_commands()[4]).contains("--container='api'"));
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-log-search", cx);
        window.input("ERROR", cx);
    });
    in_frame(cx, handle, |window, _| {
        assert_eq!(window.find("k8s-log-count").label(), Some("匹配 1 / 3 行"));
        assert!(
            window
                .find("k8s-log-output")
                .label()
                .unwrap()
                .contains("请求失败")
        );
        assert!(
            !window
                .find("k8s-log-output")
                .label()
                .unwrap()
                .contains("INFO")
        );
    });
    in_frame(cx, handle, |window, cx| {
        window.press(SELECT_ALL, cx);
        window.input("ERROR|WARN", cx);
        window.click("k8s-regex", cx);
    });
    in_frame(cx, handle, |window, _| {
        assert_eq!(window.find("k8s-log-count").label(), Some("匹配 2 / 3 行"))
    });
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-log-search", cx);
        window.press(SELECT_ALL, cx);
        window.input("[", cx);
    });
    in_frame(cx, handle, |window, _| {
        assert!(window.find("k8s-regex-error").visible())
    });
    in_frame(cx, handle, |window, cx| {
        window.press(SELECT_ALL, cx);
        window.press("backspace", cx);
    });
    assert_eq!(factory.exec_count(), 5);
    next_option(cx, handle, "k8s-container");
    label(cx, handle, "k8s-log-count", "匹配 2 / 2 行").await;
    assert!(script(&factory.exec_commands()[5]).contains("--container='sidecar'"));
    in_frame(cx, handle, |window, cx| window.click("k8s-previous", cx));
    label(cx, handle, "k8s-log-count", "匹配 1 / 1 行").await;
    let query = script(&factory.exec_commands()[6]);
    assert!(query.contains("--container='sidecar'"));
    assert!(query.contains("--previous=true"));
    in_frame(cx, handle, |window, cx| {
        window.within("k8s-detail-tabs").click(2usize, cx)
    });
    in_frame(cx, handle, |window, cx| {
        let command = window
            .find("k8s-command:k8s.command.exec")
            .label()
            .unwrap()
            .to_owned();
        assert!(command.contains("--context='prod-cluster'"));
        assert!(command.contains("--namespace='prod' --container='sidecar' -- /bin/sh"));
        window.click("k8s-copy:k8s.command.exec", cx);
    });
    cx.update(|cx| {
        assert!(
            cx.read_from_clipboard()
                .unwrap()
                .text()
                .unwrap()
                .contains("--container='sidecar'")
        )
    });
}

#[gpui_kit::test]
async fn kubernetes_workloads_link_to_namespaced_pods_and_logs(cx: &mut TestAppContext) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[
        CONTEXTS,
        NAMESPACES,
        PODS,
        DEPLOYMENTS,
        DEPLOY_DETAIL,
        "{\"items\":[{\"metadata\":{\"name\":\"api-rs\",\"namespace\":\"prod\",\"uid\":\"rs\",\"ownerReferences\":[{\"kind\":\"Deployment\",\"name\":\"api\"}]}}]}\n__SHELLRS_K8S_EXIT__:0\n",
        "{\"items\":[{\"metadata\":{\"name\":\"api-0\",\"namespace\":\"prod\",\"ownerReferences\":[{\"kind\":\"ReplicaSet\",\"name\":\"api-rs\",\"uid\":\"rs\"}]},\"spec\":{\"containers\":[{\"name\":\"api\"}]}}]}\n__SHELLRS_K8S_EXIT__:0\n",
        POD_DETAIL,
        LOGS,
    ]));
    let handle = open_kubernetes(cx, factory.clone());
    label(cx, handle, "k8s-count", "2 项资源 · 2 个命名空间").await;
    next_option(cx, handle, "k8s-kind");
    label(cx, handle, "k8s-count", "1 项资源 · 2 个命名空间").await;
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-resource:deployments:prod/api:open", cx)
    });
    cx.wait_for(handle.into(), Duration::from_secs(5), |window, _| {
        window.try_find("k8s-related:api-0").is_some()
    })
    .await;
    assert!(script(&factory.exec_commands()[5]).contains("/replicasets?"));
    assert!(!script(&factory.exec_commands()[5]).contains("labelSelector"));
    let query = script(&factory.exec_commands()[6]);
    assert!(query.contains("/namespaces/prod/"));
    assert!(query.contains("labelSelector=app%3Dapi"));
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-related:api-0", cx)
    });
    label(
        cx,
        handle,
        "k8s-detail-scope",
        "Pod · prod/api-0 · prod-cluster",
    )
    .await;
    in_frame(cx, handle, |window, cx| {
        window.within("k8s-detail-tabs").click(1usize, cx)
    });
    container(cx, handle, "api").await;
    label(cx, handle, "k8s-log-count", "匹配 3 / 3 行").await;
}

#[gpui_kit::test]
async fn kubernetes_details_close_and_reopen_from_every_tab(cx: &mut TestAppContext) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[
        CONTEXTS, NAMESPACES, PODS, POD_DETAIL, POD_DETAIL, POD_DETAIL, LOGS, POD_DETAIL, LOGS,
        POD_DETAIL, POD_DETAIL,
    ]));
    let handle = open_kubernetes(cx, factory);
    label(cx, handle, "k8s-count", "2 项资源 · 2 个命名空间").await;
    for tab in 0usize..3 {
        for pointer in [true, false] {
            in_frame(cx, handle, |window, cx| {
                window.click("k8s-resource:pods:prod/api-0:open", cx)
            });
            label(
                cx,
                handle,
                "k8s-overview:k8s.images",
                "api: api:v1\nsidecar: sidecar:v1\ninit: \ndebug: ",
            )
            .await;
            in_frame(cx, handle, |window, cx| {
                window.within("k8s-detail-tabs").click(tab, cx)
            });
            if tab == 1 {
                label(cx, handle, "k8s-log-count", "匹配 3 / 3 行").await;
                in_frame(cx, handle, |window, cx| {
                    window.click("k8s-log-search", cx);
                    window.input("ERROR", cx);
                });
            } else if tab == 2 {
                in_frame(cx, handle, |window, cx| {
                    window.click("k8s-copy:k8s.command.exec", cx)
                });
            }
            in_frame(cx, handle, |window, cx| {
                if pointer {
                    window.click("k8s-detail-close", cx);
                } else {
                    window.press("escape", cx);
                }
            });
            in_frame(cx, handle, |window, _| {
                assert!(
                    window.try_find("k8s-details").is_none(),
                    "tab {tab} must close via {}",
                    if pointer { "close button" } else { "Escape" }
                );
                assert!(window.find("k8s-resource:pods:prod/api-0:open").visible());
                assert_eq!(window.find("kubernetes").focused(), Some(true));
            });
        }
    }
}

#[gpui_kit::test]
async fn kubernetes_details_close_button_works_if_focus_returns_to_terminal(
    cx: &mut TestAppContext,
) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[
        CONTEXTS, NAMESPACES, PODS, POD_DETAIL,
    ]));
    let (handle, workspace) = open_workspace_with_kubernetes(cx, HostStore::seed(), factory);
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", INITIAL_WEB_TERMINAL), cx)
    });
    in_frame(cx, handle, |window, cx| window.click("tool-kubernetes", cx));
    label(cx, handle, "k8s-count", "2 项资源 · 2 个命名空间").await;
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-resource:pods:prod/api-0:open", cx)
    });
    label(
        cx,
        handle,
        "k8s-overview:k8s.images",
        "api: api:v1\nsidecar: sidecar:v1\ninit: \ndebug: ",
    )
    .await;
    in_frame(cx, handle, |window, cx| {
        window.within("k8s-detail-tabs").click(2usize, cx)
    });
    in_frame(cx, handle, |window, cx| {
        let focus = workspace
            .read(cx)
            .remote_terminal(shellrs::terminal::RemoteTerminalId(INITIAL_WEB_TERMINAL))
            .unwrap()
            .read(cx)
            .focus_handle(cx);
        window.focus(&focus, cx);
        window.click("k8s-detail-close", cx);
    });
    in_frame(cx, handle, |window, _| {
        assert!(window.try_find("k8s-details").is_none());
        assert_eq!(window.find("kubernetes").focused(), Some(true));
    });
}

#[gpui_kit::test]
async fn kubernetes_rbac_failures_are_visible_without_partial_or_automatic_queries(
    cx: &mut TestAppContext,
) {
    let denied = "Error from server (Forbidden): namespaces is forbidden\n__SHELLRS_K8S_EXIT__:1\n";
    let factory = Arc::new(FakeTerminalFactory::answering(&[CONTEXTS, denied]));
    let handle = open_kubernetes(cx, factory.clone());
    label(
        cx,
        handle,
        "k8s-error",
        "Error from server (Forbidden): namespaces is forbidden (exit 1)",
    )
    .await;
    cx.executor().advance_clock(Duration::from_secs(60));
    cx.run_until_parked();
    assert_eq!(factory.exec_count(), 2);
    in_frame(cx, handle, |window, _| {
        assert!(window.try_find("k8s-resource:pods:prod/api-0").is_none())
    });
}

#[derive(Default)]
struct PendingFactory {
    pending: Arc<Mutex<std::collections::VecDeque<shellrs::terminal::ExecRequest>>>,
    commands: Arc<Mutex<Vec<String>>>,
}

struct PendingTransport {
    pending: Arc<Mutex<std::collections::VecDeque<shellrs::terminal::ExecRequest>>>,
    commands: Arc<Mutex<Vec<String>>>,
}

impl TerminalTransportFactory for PendingFactory {
    fn create(&self) -> Box<dyn TerminalTransport> {
        Box::new(PendingTransport {
            pending: self.pending.clone(),
            commands: self.commands.clone(),
        })
    }
}

impl TerminalTransport for PendingTransport {
    fn run(
        self: Box<Self>,
        _: TerminalSize,
        commands: mpsc::Receiver<TerminalTransportCommand>,
        events: async_channel::Sender<TerminalTransportEvent>,
    ) -> anyhow::Result<()> {
        events.send_blocking(TerminalTransportEvent::Started)?;
        while let Ok(command) = commands.recv() {
            match command {
                TerminalTransportCommand::Exec(request) => {
                    self.commands.lock().unwrap().push(request.command.clone());
                    self.pending.lock().unwrap().push_back(request);
                }
                TerminalTransportCommand::Shutdown => break,
                _ => {}
            }
        }
        Ok(())
    }
}

async fn reply(
    cx: &mut TestAppContext,
    handle: WindowHandle<Root>,
    factory: &PendingFactory,
    output: &str,
) {
    cx.wait_for(handle.into(), Duration::from_secs(5), |_, _| {
        !factory.pending.lock().unwrap().is_empty()
    })
    .await;
    let pending = factory.pending.lock().unwrap().pop_front().unwrap();
    // An obsolete query intentionally has no receiver left.
    let _ = pending.reply.send(Ok(output.to_owned()));
    cx.run_until_parked();
}

#[gpui_kit::test]
async fn kubernetes_live_logs_are_serial_cancelable_and_copyable(cx: &mut TestAppContext) {
    let factory = Arc::new(PendingFactory::default());
    let (handle, _) = open_workspace_with_kubernetes(cx, HostStore::seed(), factory.clone());
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", INITIAL_WEB_TERMINAL), cx)
    });
    in_frame(cx, handle, |window, cx| window.click("tool-kubernetes", cx));
    reply(cx, handle, &factory, CONTEXTS).await;
    reply(cx, handle, &factory, NAMESPACES).await;
    reply(cx, handle, &factory, PODS).await;
    label(cx, handle, "k8s-count", "2 项资源 · 2 个命名空间").await;
    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(
            Box::new(KubernetesAction(KubernetesCommand::CopyValue(
                "api-0".into(),
            ))),
            cx,
        )
    });
    cx.update(|cx| {
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().as_deref(),
            Some("api-0")
        )
    });
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-resource:pods:prod/api-0:open", cx)
    });
    reply(cx, handle, &factory, POD_DETAIL).await;
    label(cx, handle, "k8s-overview:k8s.pod_ip", "10.1.0.2").await;
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-copy-overview", cx)
    });
    cx.update(|cx| {
        assert!(
            cx.read_from_clipboard()
                .unwrap()
                .text()
                .unwrap()
                .contains("10.1.0.2")
        );
    });
    in_frame(cx, handle, |window, cx| {
        window.within("k8s-detail-tabs").click(1usize, cx)
    });
    reply(cx, handle, &factory, LOGS).await;
    label(cx, handle, "k8s-log-count", "匹配 3 / 3 行").await;
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-log-search", cx);
        window.input("ERROR", cx);
        window.click("k8s-live", cx);
    });
    const LIVE_LOGS: &str = "2026-10-12T01:00:03Z ERROR {\"message\":\"新日志\",\"code\":500}\n2026-10-12T01:00:04Z INFO ok\n\n__SHELLRS_K8S_EXIT__:0\n";
    reply(cx, handle, &factory, LIVE_LOGS).await;
    label(cx, handle, "k8s-log-count", "匹配 1 / 2 行").await;
    in_frame(cx, handle, |window, cx| window.click("k8s-copy-logs", cx));
    cx.update(|cx| {
        let copy = cx.read_from_clipboard().unwrap().text().unwrap();
        assert!(copy.contains("新日志"));
        assert!(!copy.contains("INFO"));
    });
    cx.executor().advance_clock(Duration::from_secs(3));
    cx.run_until_parked();
    cx.wait_for(handle.into(), Duration::from_secs(5), |_, _| {
        factory.pending.lock().unwrap().len() == 1
    })
    .await;
    let queries = factory.commands.lock().unwrap().len();
    // A slow request never causes another request to be queued.
    cx.executor().advance_clock(Duration::from_secs(20));
    cx.run_until_parked();
    assert_eq!(factory.commands.lock().unwrap().len(), queries);
    in_frame(cx, handle, |window, cx| window.click("k8s-live", cx));
    reply(
        cx,
        handle,
        &factory,
        "late ERROR reply\n__SHELLRS_K8S_EXIT__:0\n",
    )
    .await;
    in_frame(cx, handle, |window, _| {
        assert_eq!(window.find("k8s-live").checked(), Some(false));
        assert!(
            window
                .find("k8s-log-output")
                .label()
                .unwrap()
                .contains("新日志")
        );
        assert!(
            !window
                .find("k8s-log-output")
                .label()
                .unwrap()
                .contains("late")
        );
    });
    cx.executor().advance_clock(Duration::from_secs(60));
    cx.run_until_parked();
    assert_eq!(factory.commands.lock().unwrap().len(), queries);
    // Leaving the page also cancels its timer; returning resumes live mode.
    in_frame(cx, handle, |window, cx| window.click("k8s-live", cx));
    reply(cx, handle, &factory, LIVE_LOGS).await;
    in_frame(cx, handle, |window, cx| {
        window.within("k8s-detail-tabs").click(2usize, cx)
    });
    let queries = factory.commands.lock().unwrap().len();
    cx.executor().advance_clock(Duration::from_secs(60));
    cx.run_until_parked();
    assert_eq!(factory.commands.lock().unwrap().len(), queries);
    in_frame(cx, handle, |window, cx| {
        window.within("k8s-detail-tabs").click(1usize, cx)
    });
    reply(cx, handle, &factory, "Forbidden\n__SHELLRS_K8S_EXIT__:1\n").await;
    label(cx, handle, "k8s-logs-error", "Forbidden (exit 1)").await;
    in_frame(cx, handle, |window, _| {
        assert_eq!(window.find("k8s-live").checked(), Some(false))
    });
    // Closing during an outstanding live read rejects its reply and timer.
    in_frame(cx, handle, |window, cx| window.click("k8s-live", cx));
    cx.wait_for(handle.into(), Duration::from_secs(5), |_, _| {
        !factory.pending.lock().unwrap().is_empty()
    })
    .await;
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-detail-close", cx)
    });
    reply(cx, handle, &factory, LIVE_LOGS).await;
    let queries = factory.commands.lock().unwrap().len();
    cx.executor().advance_clock(Duration::from_secs(60));
    cx.run_until_parked();
    assert_eq!(
        factory.commands.lock().unwrap().len(),
        queries,
        "the list doesn't refresh itself"
    );
    in_frame(cx, handle, |window, _| {
        assert!(window.try_find("k8s-details").is_none())
    });
}

#[gpui_kit::test]
async fn kubernetes_log_buffer_bounds_and_virtualizes_large_results(cx: &mut TestAppContext) {
    let logs = (0..20_000)
        .map(|ix| format!("INFO line-{ix}\n"))
        .collect::<String>()
        + "__SHELLRS_K8S_EXIT__:0\n";
    let factory = Arc::new(FakeTerminalFactory::answering(&[
        CONTEXTS, NAMESPACES, PODS, POD_DETAIL, &logs,
    ]));
    let handle = open_kubernetes(cx, factory);
    label(cx, handle, "k8s-count", "2 项资源 · 2 个命名空间").await;
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-resource:pods:prod/api-0:open", cx)
    });
    label(cx, handle, "k8s-overview:k8s.pod_ip", "10.1.0.2").await;
    in_frame(cx, handle, |window, cx| {
        window.within("k8s-detail-tabs").click(1usize, cx)
    });
    label(cx, handle, "k8s-log-count", "匹配 10000 / 10000 行").await;
    in_frame(cx, handle, |window, cx| {
        // Thousands of off-screen rows have no rendered element.
        assert!(window.try_find("k8s-copy-line:5000").is_none());
        window.click("k8s-copy-logs", cx);
    });
    cx.update(|cx| {
        let copy = cx.read_from_clipboard().unwrap().text().unwrap();
        assert_eq!(copy.lines().count(), 10_000);
        assert!(copy.starts_with("INFO line-10000\n"));
        assert!(copy.ends_with("INFO line-19999"));
    });
}

#[gpui_kit::test]
async fn kubernetes_details_close_while_loading_but_not_on_backdrop(cx: &mut TestAppContext) {
    let factory = Arc::new(PendingFactory::default());
    let (handle, _) = open_workspace_with_kubernetes(cx, HostStore::seed(), factory.clone());
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", INITIAL_WEB_TERMINAL), cx)
    });
    in_frame(cx, handle, |window, cx| window.click("tool-kubernetes", cx));
    reply(cx, handle, &factory, CONTEXTS).await;
    reply(cx, handle, &factory, NAMESPACES).await;
    reply(cx, handle, &factory, PODS).await;
    label(cx, handle, "k8s-count", "2 项资源 · 2 个命名空间").await;
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-resource:pods:prod/api-0:open", cx)
    });
    in_frame(cx, handle, |window, cx| {
        assert!(window.find("k8s-details").visible());
        window.click("k8s-detail-close", cx);
    });
    reply(cx, handle, &factory, POD_DETAIL).await;
    in_frame(cx, handle, |window, _| {
        assert!(window.try_find("k8s-details").is_none());
        assert!(window.find("k8s-resource:pods:prod/api-0:open").visible());
    });
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-resource:pods:prod/api-0:open", cx)
    });
    reply(cx, handle, &factory, POD_DETAIL).await;
    label(
        cx,
        handle,
        "k8s-overview:k8s.images",
        "api: api:v1\nsidecar: sidecar:v1\ninit: \ndebug: ",
    )
    .await;
    in_frame(cx, handle, |window, cx| {
        window.click_at(
            "dialog",
            gpui_kit::point(gpui_kit::px(8.), gpui_kit::px(100.)),
            cx,
        );
    });
    in_frame(cx, handle, |window, cx| {
        assert!(
            window.find("k8s-details").visible(),
            "a click beside the details doesn't close them"
        );
        window.click("k8s-detail-close", cx);
    });
    in_frame(cx, handle, |window, _| {
        assert!(window.try_find("k8s-details").is_none());
    });
}

#[gpui_kit::test]
async fn kubernetes_finishes_namespace_discovery_when_resource_type_changes(
    cx: &mut TestAppContext,
) {
    let factory = Arc::new(PendingFactory::default());
    let (handle, _) = open_workspace_with_kubernetes(cx, HostStore::seed(), factory.clone());
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", INITIAL_WEB_TERMINAL), cx)
    });
    in_frame(cx, handle, |window, cx| window.click("tool-kubernetes", cx));
    reply(cx, handle, &factory, CONTEXTS).await;
    cx.wait_for(handle.into(), Duration::from_secs(5), |_, _| {
        factory.pending.lock().unwrap().len() == 1
    })
    .await;
    next_option(cx, handle, "k8s-kind");
    cx.wait_for(handle.into(), Duration::from_secs(5), |_, _| {
        factory.pending.lock().unwrap().len() == 2
    })
    .await;
    reply(cx, handle, &factory, NAMESPACES).await; // Cancelled discovery.
    reply(cx, handle, &factory, CONTEXTS).await;
    reply(cx, handle, &factory, NAMESPACES).await;
    reply(cx, handle, &factory, DEPLOYMENTS).await;
    label(cx, handle, "k8s-count", "1 项资源 · 2 个命名空间").await;
    next_option(cx, handle, "k8s-namespace");
    reply(cx, handle, &factory, DEPLOYMENTS).await;
    label(cx, handle, "k8s-count", "1 项资源 · 2 个命名空间").await;
    in_frame(cx, handle, |window, _| {
        assert_eq!(window.find("k8s-kind").value(), Some("Deployment"));
        assert_eq!(window.find("k8s-namespace").value(), Some("prod"));
    });
    let queries = factory.commands.lock().unwrap();
    let query = script(queries.last().unwrap());
    assert!(query.contains("/deployments?limit=100"));
    assert!(query.contains("/namespaces/prod/"));
}

#[gpui_kit::test]
async fn kubernetes_rejects_old_scope_replies_and_explicitly_switches_context(
    cx: &mut TestAppContext,
) {
    let factory = Arc::new(PendingFactory::default());
    let (handle, _) = open_workspace_with_kubernetes(cx, HostStore::seed(), factory.clone());
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", INITIAL_WEB_TERMINAL), cx)
    });
    in_frame(cx, handle, |window, cx| window.click("tool-kubernetes", cx));
    reply(cx, handle, &factory, CONTEXTS).await;
    reply(cx, handle, &factory, NAMESPACES).await;
    reply(cx, handle, &factory, PODS).await;
    label(cx, handle, "k8s-count", "2 项资源 · 2 个命名空间").await;
    next_option(cx, handle, "k8s-namespace");
    cx.wait_for(handle.into(), Duration::from_secs(5), |_, _| {
        factory.pending.lock().unwrap().len() == 1
    })
    .await;
    next_option(cx, handle, "k8s-kind");
    cx.wait_for(handle.into(), Duration::from_secs(5), |_, _| {
        factory.pending.lock().unwrap().len() == 2
    })
    .await;
    reply(cx, handle, &factory, PROD_PODS).await; // Previous Pod scope.
    in_frame(cx, handle, |window, _| {
        assert!(window.try_find("k8s-resource:pods:prod/api-0").is_none())
    });
    reply(cx, handle, &factory, DEPLOYMENTS).await;
    label(
        cx,
        handle,
        "k8s-resource:deployments:prod/api",
        "prod/api · 2/3",
    )
    .await;
    next_option(cx, handle, "k8s-context");
    reply(cx, handle, &factory, CONTEXTS).await;
    reply(cx, handle, &factory, NAMESPACES).await;
    reply(cx, handle, &factory, DEPLOYMENTS).await;
    label(cx, handle, "k8s-count", "1 项资源 · 2 个命名空间").await;
    in_frame(cx, handle, |window, _| {
        assert_eq!(window.find("k8s-context").value(), Some("test-cluster"));
        assert_eq!(window.find("k8s-namespace").value(), Some("全部命名空间"));
    });
    let queries = factory.commands.lock().unwrap();
    let query = script(queries.last().unwrap());
    assert!(query.contains("--context='test-cluster'"));
    assert!(query.contains("/apis/apps/v1/deployments?limit=100"));
}

fn compressed_fixture(body: &str) -> String {
    use base64::Engine as _;
    use std::io::Write as _;
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(body.as_bytes()).unwrap();
    format!(
        "__SHELLRS_K8S_GZIP__\n{}",
        base64::engine::general_purpose::STANDARD.encode(encoder.finish().unwrap())
    )
}

#[gpui_kit::test]
async fn kubernetes_large_lists_retry_smaller_pages_and_show_first_page_early(
    cx: &mut TestAppContext,
) {
    let factory = Arc::new(PendingFactory::default());
    let (handle, _) = open_workspace_with_kubernetes(cx, HostStore::seed(), factory.clone());
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", INITIAL_WEB_TERMINAL), cx)
    });
    in_frame(cx, handle, |window, cx| window.click("tool-kubernetes", cx));
    reply(cx, handle, &factory, CONTEXTS).await;
    reply(cx, handle, &factory, NAMESPACES).await;
    reply(cx, handle, &factory, &"x".repeat(1 << 20)).await;
    cx.wait_for(handle.into(), Duration::from_secs(5), |_, _| {
        !factory.pending.lock().unwrap().is_empty()
    })
    .await;
    assert!(
        script(factory.commands.lock().unwrap().last().unwrap()).contains("/api/v1/pods?limit=50")
    );
    let too_large = compressed_fixture(&format!(
        "{{\"data\":\"{}\"}}\n__SHELLRS_K8S_EXIT__:0\n",
        "x".repeat(17 << 20)
    ));
    reply(cx, handle, &factory, &too_large).await;
    cx.wait_for(handle.into(), Duration::from_secs(5), |_, _| {
        !factory.pending.lock().unwrap().is_empty()
    })
    .await;
    assert!(
        script(factory.commands.lock().unwrap().last().unwrap()).contains("/api/v1/pods?limit=25")
    );
    let first = serde_json::json!({"metadata":{"continue":"next+/=&","resourceVersion":"42"},"items":[{"metadata":{"name":"first","namespace":"prod","annotations":{"kubectl.kubernetes.io/last-applied-configuration":"annotation".repeat(150_000)}},"spec":{"containers":[{"name":"api"}]},"status":{"phase":"Running"}}]});
    let first = format!("{first}\n__SHELLRS_K8S_EXIT__:0\n");
    assert!(first.len() > 1 << 20);
    let wire = compressed_fixture(&first);
    assert!(wire.len() < 1 << 20);
    reply(cx, handle, &factory, &wire).await;
    label(cx, handle, "k8s-count", "1 项资源 · 2 个命名空间").await;
    cx.wait_for(handle.into(), Duration::from_secs(5), |_, _| {
        !factory.pending.lock().unwrap().is_empty()
    })
    .await;
    in_frame(cx, handle, |window, _| {
        assert!(window.find("k8s-resource:pods:prod/first").visible())
    });
    let next = script(factory.commands.lock().unwrap().last().unwrap());
    assert!(next.contains("limit=25&continue=next%2B%2F%3D%26"));
    let last = r#"{"metadata":{"resourceVersion":"42"},"items":[{"metadata":{"name":"last","namespace":"prod"},"spec":{"containers":[{"name":"worker"}]}}]}
__SHELLRS_K8S_EXIT__:0
"#;
    reply(cx, handle, &factory, &compressed_fixture(last)).await;
    label(cx, handle, "k8s-count", "2 项资源 · 2 个命名空间").await;
    assert_eq!(factory.commands.lock().unwrap().len(), 6);
}

#[gpui_kit::test]
async fn kubernetes_repeated_page_tokens_stop_with_visible_error(cx: &mut TestAppContext) {
    let page = r#"{"metadata":{"continue":"same","resourceVersion":"42"},"items":[{"metadata":{"name":"api","namespace":"prod"}}]}
__SHELLRS_K8S_EXIT__:0
"#;
    let factory = Arc::new(FakeTerminalFactory::answering(&[
        CONTEXTS, NAMESPACES, page, page,
    ]));
    let handle = open_kubernetes(cx, factory.clone());
    cx.wait_for(handle.into(), Duration::from_secs(5), |window, _| {
        factory.exec_count() == 4
            && window
                .try_find("k8s-error")
                .and_then(|element| element.label().map(str::to_owned))
                .is_some_and(|label| label.contains("返回数据不完整"))
    })
    .await;
    label(cx, handle, "k8s-count", "1 项资源 · 2 个命名空间").await;
    assert_eq!(factory.exec_count(), 4);
}

#[gpui_kit::test]
async fn kubernetes_relationship_navigation_reuses_cache_and_refresh_removes_old_children(
    cx: &mut TestAppContext,
) {
    let rs = r#"{"kind":"ReplicaSet","metadata":{"name":"api-rs","namespace":"prod","uid":"rs-uid","ownerReferences":[{"kind":"Deployment","name":"api","uid":"deploy-uid"}]}}
__SHELLRS_K8S_EXIT__:0
"#;
    let rs_list = format!(
        "{{\"items\":[{}]}}\n__SHELLRS_K8S_EXIT__:0\n",
        rs.split("\n__SHELLRS").next().unwrap()
    );
    let empty = "{\"items\":[]}\n__SHELLRS_K8S_EXIT__:0\n";
    let factory = Arc::new(FakeTerminalFactory::answering(&[
        CONTEXTS,
        NAMESPACES,
        PODS,
        DEPLOYMENTS,
        GRAPH_DEPLOY,
        &rs_list,
        rs,
        empty,
        GRAPH_DEPLOY,
        empty,
    ]));
    let handle = open_kubernetes(cx, factory.clone());
    label(cx, handle, "k8s-count", "2 项资源 · 2 个命名空间").await;
    next_option(cx, handle, "k8s-kind");
    label(cx, handle, "k8s-count", "1 项资源 · 2 个命名空间").await;
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-resource:deployments:prod/api:open", cx)
    });
    label(
        cx,
        handle,
        "k8s-detail-scope",
        "Deployment · prod/api · prod-cluster",
    )
    .await;
    in_frame(cx, handle, |window, cx| {
        window.within("k8s-detail-tabs").click(3usize, cx)
    });
    cx.wait_for(handle.into(), Duration::from_secs(5), |window, _| {
        factory.exec_count() == 6 && window.try_find("k8s-relations-reading").is_none()
    })
    .await;
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-graph-node:k8s-resource:replicasets:prod/api-rs", cx)
    });
    cx.wait_for(handle.into(), Duration::from_secs(5), |window, _| {
        factory.exec_count() == 8 && window.try_find("k8s-relations-reading").is_none()
    })
    .await;
    in_frame(cx, handle, |window, cx| window.click("k8s-detail-back", cx));
    cx.wait_for(handle.into(), Duration::from_secs(5), |window, _| {
        factory.exec_count() == 9 && window.try_find("k8s-relations-reading").is_none()
    })
    .await;
    in_frame(cx, handle, |window, cx| {
        assert!(
            window
                .find("k8s-graph-node:k8s-resource:replicasets:prod/api-rs")
                .visible()
        );
        window.click("k8s-relation-refresh", cx);
    });
    cx.wait_for(handle.into(), Duration::from_secs(5), |window, _| {
        factory.exec_count() == 10 && window.try_find("k8s-relations-reading").is_none()
    })
    .await;
    in_frame(cx, handle, |window, cx| {
        assert!(
            window
                .try_find("k8s-graph-node:k8s-resource:replicasets:prod/api-rs")
                .is_none()
        );
        window.click("k8s-detail-close", cx);
    });
    assert_eq!(factory.exec_count(), 10);
}

#[gpui_kit::test]
async fn kubernetes_configuration_cards_page_locally_and_open_details(cx: &mut TestAppContext) {
    let mut value: serde_json::Value =
        serde_json::from_str(POD_DETAIL.split("\n__SHELLRS").next().unwrap()).unwrap();
    value["spec"]["containers"][0]["envFrom"] = serde_json::json!(
        (0..4)
            .map(|ix| serde_json::json!({"configMapRef":{"name":format!("cfg-{ix}")}}))
            .collect::<Vec<_>>()
    );
    value["spec"]["containers"][1]["envFrom"] = serde_json::json!([{"secretRef":{"name":"token"}}]);
    let pod = mutation_wire(&value);
    let cm = mutation_wire(
        &serde_json::json!({"kind":"ConfigMap", "metadata":{"name":"cfg-0","namespace":"prod"}, "data":{"app.yaml":"port: 80"}}),
    );
    let empty = "{\"items\":[]}\n__SHELLRS_K8S_EXIT__:0\n";
    let factory = Arc::new(FakeTerminalFactory::answering(&[
        CONTEXTS, NAMESPACES, PODS, &pod, &cm, empty, empty, empty, empty, empty, empty, &pod,
    ]));
    let handle = open_kubernetes(cx, factory.clone());
    label(cx, handle, "k8s-count", "2 项资源 · 2 个命名空间").await;
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-resource:pods:prod/api-0:open", cx)
    });
    label(cx, handle, "k8s-overview:k8s.name", "api-0").await;
    in_frame(cx, handle, |window, cx| {
        window.within("k8s-detail-tabs").click(3usize, cx)
    });
    in_frame(cx, handle, |window, cx| {
        assert!(
            window
                .find("k8s-graph-node:k8s-resource:configmaps:prod/cfg-0")
                .visible()
        );
        assert!(
            window
                .try_find("k8s-graph-node:k8s-resource:configmaps:prod/cfg-3")
                .is_none()
        );
        window.click("k8s-graph-dependencies-next", cx);
    });
    in_frame(cx, handle, |window, cx| {
        assert!(
            window
                .find("k8s-graph-node:k8s-resource:secrets:prod/token")
                .visible()
        );
        window.click("k8s-graph-dependencies-previous", cx);
    });
    assert_eq!(factory.exec_count(), 4, "configuration pagination is local");
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-graph-node:k8s-resource:configmaps:prod/cfg-0", cx)
    });
    cx.wait_for(handle.into(), Duration::from_secs(5), |window, _| {
        factory.exec_count() == 11 && window.try_find("k8s-relations-reading").is_none()
    })
    .await;
    in_frame(cx, handle, |window, cx| {
        window.within("k8s-detail-tabs").click(0usize, cx)
    });
    label(cx, handle, "k8s-overview:k8s.config_keys", "app.yaml").await;
    assert!(
        script(&factory.exec_commands()[4]).contains("get configmaps 'cfg-0' --namespace='prod'")
    );
    in_frame(cx, handle, |window, cx| window.click("k8s-detail-back", cx));
    label(cx, handle, "k8s-overview:k8s.name", "api-0").await;
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-detail-close", cx)
    });
}

#[gpui_kit::test]
async fn kubernetes_owner_navigation_rejects_a_recreated_resource(cx: &mut TestAppContext) {
    let mut value: serde_json::Value =
        serde_json::from_str(POD_DETAIL.split("\n__SHELLRS").next().unwrap()).unwrap();
    value["metadata"]["ownerReferences"] = serde_json::json!([{"apiVersion":"apps/v1", "kind":"ReplicaSet", "name":"api-rs", "uid":"original"}]);
    let pod = mutation_wire(&value);
    let rs = mutation_wire(
        &serde_json::json!({"kind":"ReplicaSet", "metadata":{"name":"api-rs", "namespace":"prod", "uid":"recreated"}}),
    );
    let factory = Arc::new(FakeTerminalFactory::answering(&[
        CONTEXTS, NAMESPACES, PODS, &pod, &rs,
    ]));
    let handle = open_kubernetes(cx, factory.clone());
    label(cx, handle, "k8s-count", "2 项资源 · 2 个命名空间").await;
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-resource:pods:prod/api-0:open", cx)
    });
    label(cx, handle, "k8s-overview:k8s.name", "api-0").await;
    in_frame(cx, handle, |window, cx| {
        window.within("k8s-detail-tabs").click(3usize, cx)
    });
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-graph-node:k8s-resource:replicasets:prod/api-rs", cx)
    });
    label(
        cx,
        handle,
        "k8s-detail-error",
        "引用的资源已被删除或重建，请返回并刷新关系图。",
    )
    .await;
    assert_eq!(factory.exec_count(), 5);
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-detail-close", cx)
    });
}

#[gpui_kit::test]
async fn kubernetes_selectorless_services_query_only_their_endpoint_slices(
    cx: &mut TestAppContext,
) {
    let value = serde_json::json!({"kind":"Service", "metadata":{"name":"external", "namespace":"prod"}, "spec":{"ports":[{"port":80}]}});
    let service = mutation_wire(&value);
    let list = mutation_wire(&serde_json::json!({"items":[value]}));
    let slices = mutation_wire(
        &serde_json::json!({"items":[{"kind":"EndpointSlice", "metadata":{"name":"external-1", "namespace":"prod", "labels":{"kubernetes.io/service-name":"external"}}, "endpoints":[{"addresses":["192.0.2.1"]}]}]}),
    );
    let factory = Arc::new(FakeTerminalFactory::answering(&[
        CONTEXTS, NAMESPACES, PODS, &list, &service, &slices,
    ]));
    let handle = open_kubernetes(cx, factory.clone());
    label(cx, handle, "k8s-count", "2 项资源 · 2 个命名空间").await;
    in_frame(cx, handle, |window, cx| {
        window.dispatch_action(
            Box::new(KubernetesAction(KubernetesCommand::Kind(
                ResourceKind::Service,
            ))),
            cx,
        )
    });
    label(cx, handle, "k8s-count", "1 项资源 · 2 个命名空间").await;
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-resource:services:prod/external:open", cx)
    });
    label(cx, handle, "k8s-overview:k8s.name", "external").await;
    in_frame(cx, handle, |window, cx| {
        window.within("k8s-detail-tabs").click(3usize, cx)
    });
    cx.wait_for(handle.into(), Duration::from_secs(5), |window, _| {
        factory.exec_count() == 6 && window.try_find("k8s-relations-reading").is_none()
    })
    .await;
    let command = script(&factory.exec_commands()[5]);
    assert!(command.contains("/namespaces/prod/endpointslices?"));
    assert!(command.contains("labelSelector=kubernetes%2Eio%2Fservice%2Dname%3Dexternal"));
    in_frame(cx, handle, |window, cx| {
        assert!(
            window
                .find("k8s-graph-node:k8s-resource:endpointslices:prod/external-1")
                .visible()
        );
        window.click("k8s-detail-close", cx);
    });
}
