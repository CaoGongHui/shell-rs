use serde_json::json;

use super::{
    commands::{self, LogOptions},
    details::filter,
    model::{self, Resource, ResourceKind},
};

#[test]
fn memory_and_resource_times_are_formatted_without_changing_manifests() {
    use super::{format, logs::LogLine};
    assert_eq!(format::memory("16777216Ki"), "16.00 G");
    assert_eq!(format::memory("512Mi"), "0.50 G");
    assert_eq!(format::memory("1G"), "0.93 G");
    assert_eq!(format::memory("1Ki"), "<0.01 G");
    assert_eq!(format::memory("0"), "0.00 G");
    assert_eq!(format::memory("—"), "—");
    assert_eq!(
        format::time("2026-10-14T20:30:00Z"),
        "2026-10-15 04:30:00 +08:00"
    );
    assert_eq!(
        format::time("2026-10-15T01:30:00-04:00"),
        "2026-10-15 13:30:00 +08:00"
    );
    assert_eq!(format::time("unknown"), "unknown");
    let node = json!({"metadata":{"name":"node","creationTimestamp":"2026-10-14T20:30:00Z"}, "status":{"capacity":{"memory":"16777216Ki"},"allocatable":{"memory":"15Gi"}}});
    let overview = model::overview(&node, ResourceKind::Node);
    assert!(overview.contains(&("k8s.memory_capacity", "16.00 G".into())));
    assert!(overview.contains(&("k8s.memory_allocatable", "15.00 G".into())));
    assert!(overview.contains(&("k8s.created", "2026-10-15 04:30:00 +08:00".into())));
    assert!(super::nodes::summary(&node).contains("16.00 G"));
    assert_eq!(node["status"]["capacity"]["memory"], "16777216Ki");
    let job = json!({"metadata":{"creationTimestamp":"2026-10-14T20:30:00Z"},"status":{"startTime":"2026-10-14T20:31:00Z","completionTime":"2026-10-14T20:32:00Z"}});
    let overview = model::overview(&job, ResourceKind::Job);
    assert!(overview.contains(&("k8s.started", "2026-10-15 04:31:00 +08:00".into())));
    assert!(overview.contains(&("k8s.completed", "2026-10-15 04:32:00 +08:00".into())));
    let pod_value = json!({"spec":{"containers":[{"name":"app","resources":{"requests":{"memory":"128Mi"},"limits":{"memory":"512Mi"}}}]}});
    let overview = model::overview(&pod_value, ResourceKind::Pod);
    assert!(overview.contains(&("k8s.resource_requests", "app: memory=0.12 G".into())));
    assert!(overview.contains(&("k8s.resource_limits", "app: memory=0.50 G".into())));
    let raw = "2026-10-14T20:30:00.123456789Z INFO 中文 2026-10-14T20:30:00Z";
    let line = LogLine::new(raw);
    assert_eq!(line.raw, raw);
    assert_eq!(
        line.text,
        "2026-10-15T04:30:00.123456789+08:00 INFO 中文 2026-10-14T20:30:00Z"
    );
    assert_eq!(filter(raw, "04:30", false).unwrap().1, 1);
    assert_eq!(filter(raw, r"04:30:00.*\+08:00", true).unwrap().1, 1);
    assert_eq!(LogLine::new("INFO 没有时间戳").text, "INFO 没有时间戳");
}

#[test]
fn resource_search_includes_status_and_ignores_surrounding_spaces() {
    let resource = Resource {
        status: Some(("Progressing".into(), model::Tone::Warning)),
        ..pod()
    };
    assert!(resource.matches("  PROGRESSING  "));
    assert!(resource.matches("production"));
    assert!(!resource.matches("ready"));
}

#[test]
fn editable_yaml_round_trips_strings_and_excludes_server_owned_fields() {
    use super::mutation;
    let original = json!({"apiVersion":"v1","kind":"ConfigMap","metadata":{"name":"api-0","namespace":"production","uid":"uid-1","resourceVersion":"12","managedFields":[{"manager":"kubectl"}]},"data":{"flag":"true","port":"8080","script":"echo ready\necho done\n"},"status":{"phase":"Active"}});
    let mut resource = pod();
    resource.name = "api-0".into();
    resource.kind = ResourceKind::ConfigMap;
    let yaml = mutation::yaml(&mutation::manifest(&original)).unwrap();
    let parsed: serde_json::Value = serde_yaml_ng::from_str(&yaml).unwrap();
    assert_eq!(parsed["data"], original["data"]);
    assert!(parsed.get("status").is_none());
    assert!(parsed["metadata"].get("managedFields").is_none());
    let mut changed = parsed;
    changed["data"]["port"] = json!("9090");
    let edited =
        mutation::edited(&mutation::yaml(&changed).unwrap(), &original, &resource).unwrap();
    assert_eq!(edited["data"]["port"], "9090");
    assert_eq!(edited["metadata"]["uid"], "uid-1");
    assert_eq!(edited["metadata"]["resourceVersion"], "12");
    assert!(mutation::edited(&yaml, &original, &resource).is_err());
}

#[test]
fn yaml_edits_cannot_change_identity_or_submit_lists_status_and_oversized_content() {
    use super::mutation;
    let original = json!({"apiVersion":"v1","kind":"Pod","metadata":{"name":"api-0","namespace":"production","uid":"uid-1","resourceVersion":"12"},"spec":{"containers":[{"name":"app","image":"app:v1"}]}});
    let mut resource = pod();
    resource.name = "api-0".into();
    for path in [
        vec!["apiVersion"],
        vec!["kind"],
        vec!["metadata", "name"],
        vec!["metadata", "namespace"],
        vec!["metadata", "uid"],
        vec!["metadata", "resourceVersion"],
    ] {
        let mut value = original.clone();
        let mut field = &mut value;
        for key in path {
            field = &mut field[key];
        }
        *field = json!("changed");
        assert!(mutation::edited(&mutation::yaml(&value).unwrap(), &original, &resource).is_err());
    }
    for text in [
        "[]",
        "null",
        "spec: [",
        "---\nkind: Pod\n---\nkind: Pod",
        "status: {}",
    ] {
        assert!(
            mutation::edited(text, &original, &resource).is_err(),
            "{text}"
        );
    }
    assert!(mutation::edited(&"a".repeat(mutation::EDIT_LIMIT + 1), &original, &resource).is_err());
    let mut changed = original.clone();
    changed["spec"]["containers"][0]["image"] = json!("app:v2");
    let valid = mutation::yaml(&changed).unwrap();
    assert!(mutation::edited(&format!("---\n{valid}---\n{valid}"), &original, &resource).is_err());
    let duplicated = valid.replace("image: app:v2", "image: app:v2\n    image: app:v3");
    assert!(
        mutation::edited(&duplicated, &original, &resource).is_err(),
        "duplicate YAML keys must not silently overwrite values"
    );
}

#[test]
fn writes_quote_scope_preserve_payload_and_delete_with_uid_and_version_preconditions() {
    use super::mutation;
    let mut resource = pod();
    resource.name = "api-0".into();
    resource.kind = ResourceKind::ConfigMap;
    let original = json!({"apiVersion":"v1","kind":"ConfigMap","metadata":{"name":"api-0","namespace":"production","uid":"uid-1","resourceVersion":"12"},"data":{"script":"kubectl get pods; $(echo bad)\n__SHELLRS_K8S_INPUT__\n'quote'"}});
    let wire = mutation::replace("prod'; echo bad", &resource, &original, &original).unwrap();
    let program = script(&wire);
    assert!(program.contains("--context='prod'\\''; echo bad'"));
    assert!(
        program.contains("replace --raw='/api/v1/namespaces/production/configmaps/api%2D0' -f -")
    );
    let body = program
        .split("<<'__SHELLRS_K8S_INPUT__'\n")
        .nth(1)
        .unwrap()
        .split("\n__SHELLRS_K8S_INPUT__")
        .next()
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(body).unwrap(),
        original,
        "timeout insertion must not modify ConfigMap text"
    );
    let deletion = script(&mutation::delete("prod", &resource, &original).unwrap());
    assert!(
        deletion.contains("delete --raw='/api/v1/namespaces/production/configmaps/api%2D0' -f -")
    );
    assert!(deletion.contains("\"uid\":\"uid-1\""));
    assert!(deletion.contains("\"resourceVersion\":\"12\""));
    assert!(!deletion.contains("--force") && !deletion.contains("--all"));
}

#[test]
fn saves_too_long_for_one_shell_argument_are_refused() {
    use super::mutation;
    let mut resource = pod();
    resource.name = "api-0".into();
    resource.kind = ResourceKind::ConfigMap;
    let original = json!({"apiVersion":"v1","kind":"ConfigMap","metadata":{"name":"api-0","namespace":"production","uid":"uid-1","resourceVersion":"12"}});
    let with = |size: usize| {
        let mut value = original.clone();
        value["data"] = json!({"text": "x".repeat(size)});
        value
    };
    let fits = mutation::replace("prod", &resource, &original, &with(20_000)).unwrap();
    assert!(fits.len() <= super::commands::COMMAND_LIMIT);
    assert_eq!(
        mutation::replace("prod", &resource, &original, &with(30_000)),
        Err(crate::i18n::t!("k8s.edit_too_large").to_string()),
        "Linux takes at most 128 KiB in the login shell's one argument"
    );
}

#[test]
fn restart_is_scoped_guarded_and_only_offered_for_actual_workloads() {
    use super::mutation;
    let mut resource = pod();
    resource.name = "api-0".into();
    let mut value = json!({"apiVersion":"v1","kind":"Pod","metadata":{"name":"api-0","namespace":"production","uid":"uid-1","resourceVersion":"12"},"status":{"phase":"Running"}});
    assert!(
        mutation::restart("prod", &resource, &value).is_err(),
        "standalone Pod cannot be recreated by a controller"
    );
    value["metadata"]["ownerReferences"] = json!([{"kind":"ReplicaSet","controller":true}]);
    assert!(
        script(&mutation::restart("prod", &resource, &value).unwrap()).contains("delete --raw=")
    );
    for (kind, name) in [
        (ResourceKind::Deployment, "Deployment"),
        (ResourceKind::StatefulSet, "StatefulSet"),
        (ResourceKind::DaemonSet, "DaemonSet"),
    ] {
        resource.kind = kind;
        value["kind"] = json!(name);
        value["spec"] = json!({"template":{"metadata":{"annotations":{"existing":"keep"}}}});
        let program = script(&mutation::restart("prod", &resource, &value).unwrap());
        assert!(
            program.contains("--type=json")
                && program.contains("/metadata/uid")
                && program.contains("/metadata/resourceVersion")
        );
        assert!(
            program
                .contains("/spec/template/metadata/annotations/kubectl.kubernetes.io~1restartedAt")
        );
        value["spec"]["updateStrategy"]["type"] = json!("OnDelete");
        assert!(mutation::restart("prod", &resource, &value).is_err());
    }
    assert!(!mutation::restart_kind(ResourceKind::ConfigMap));
    assert!(!mutation::editable(ResourceKind::Secret));
}

#[test]
fn terminating_workloads_cannot_restart() {
    use super::mutation;
    let mut resource = pod();
    resource.name = "api-0".into();
    for kind in [
        ResourceKind::Pod,
        ResourceKind::Deployment,
        ResourceKind::StatefulSet,
        ResourceKind::DaemonSet,
    ] {
        resource.kind = kind;
        let api_version = if kind == ResourceKind::Pod {
            "v1"
        } else {
            "apps/v1"
        };
        let mut value = json!({"apiVersion":api_version,"kind":kind.label(),"metadata":{"name":"api-0","namespace":"production","uid":"uid-1","resourceVersion":"12","ownerReferences":[{"kind":"ReplicaSet","controller":true}]},"status":{"phase":"Running"},"spec":{"template":{"metadata":{}}}});
        assert!(mutation::restart("prod", &resource, &value).is_ok());
        value["metadata"]["deletionTimestamp"] = json!("2026-10-15T01:00:00Z");
        assert!(
            mutation::restart_blocker(kind, &value).is_some(),
            "{kind:?}"
        );
        assert!(
            mutation::restart("prod", &resource, &value).is_err(),
            "{kind:?}"
        );
    }
}

#[test]
fn resources_waiting_for_deletion_show_a_transition_state_instead_of_ready() {
    let value = json!({"metadata":{"deletionTimestamp":"2026-10-14T01:00:00Z"},"status":{"phase":"Running","readyReplicas":3},"spec":{"replicas":3}});
    for kind in [
        ResourceKind::Pod,
        ResourceKind::Deployment,
        ResourceKind::StatefulSet,
        ResourceKind::ConfigMap,
    ] {
        assert_eq!(model::status(&value, kind).unwrap().1, model::Tone::Warning);
    }
}

#[test]
fn metrics_quantities_cover_cpu_and_storage_without_invalid_values() {
    use super::metrics::quantity;
    for (text, expected) in [
        ("125m", 0.125),
        ("250000000n", 0.25),
        ("250000u", 0.25),
        ("2", 2.),
        ("128Mi", 134217728.),
        ("1.5Gi", 1610612736.),
        ("1e3", 1000.),
        ("1G", 1e9),
    ] {
        assert_eq!(quantity(text), Some(expected), "{text}");
    }
    for invalid in ["", "unknown", "NaN", "inf", "-1", "1e999"] {
        assert_eq!(quantity(invalid), None, "{invalid}");
    }
}

#[test]
fn metrics_only_compute_shares_when_every_observed_container_has_a_limit() {
    use super::metrics;
    let mut resource = pod();
    resource.namespace = "prod".into();
    let detail = json!({"spec":{"containers":[
        {"name":"api","resources":{"limits":{"cpu":"1","memory":"512Mi"}}},
        {"name":"sidecar","resources":{"requests":{"cpu":"100m","memory":"64Mi"}}}
    ]}});
    let sample = json!({"metadata":{"name":resource.name,"namespace":"prod"},"containers":[
        {"name":"api","usage":{"cpu":"250m","memory":"128Mi"}},
        {"name":"sidecar","usage":{"cpu":"50m","memory":"32Mi"}}
    ]});
    let usage = metrics::usage(&sample, &detail, &resource, None).unwrap();
    assert_eq!(usage.cpu, Some(0.3));
    assert_eq!(usage.memory, Some(160 * 1024 * 1024));
    assert_eq!(usage.cpu_total, None, "requests are not limits");
    assert_eq!(usage.memory_total, None, "partial limits are not totals");
    let selected = metrics::usage(&sample, &detail, &resource, Some("api")).unwrap();
    assert_eq!(selected.cpu, Some(0.25));
    assert_eq!(selected.cpu_total, Some(1.));
    assert_eq!(selected.memory_total, Some(512 * 1024 * 1024));
    assert!(metrics::usage(&sample, &detail, &resource, Some("removed")).is_none());
    resource.namespace = "another".into();
    assert!(metrics::usage(&sample, &detail, &resource, None).is_none());
}

#[test]
fn node_metrics_and_disk_are_validated_independently() {
    use super::metrics;
    let mut resource = pod();
    resource.kind = ResourceKind::Node;
    resource.name = "node-1".into();
    resource.namespace.clear();
    let sample = json!({"metadata":{"name":"node-1"},"usage":{"cpu":"500m","memory":"4Gi"}});
    let detail = json!({"status":{"capacity":{"cpu":"8","memory":"16Gi"}}});
    let usage = metrics::usage(&sample, &detail, &resource, None).unwrap();
    assert_eq!(usage.cpu_total, Some(8.));
    assert_eq!(usage.memory_total, Some(16 * 1024 * 1024 * 1024));
    let summary = json!({"node":{"nodeName":"node-1","fs":{"usedBytes":250,"capacityBytes":1000}}});
    let disk = metrics::disk(&summary, "node-1").unwrap();
    assert_eq!((disk.used, disk.total), (250, 1000));
    assert!(metrics::disk(&summary, "node-2").is_none());
    assert!(
        metrics::disk(
            &json!({"node":{"nodeName":"node-1","fs":{"usedBytes":0,"capacityBytes":0}}}),
            "node-1"
        )
        .is_none()
    );
}

fn object(kind: &str, name: &str, ns: &str) -> serde_json::Value {
    json!({"kind":kind,"metadata":{"name":name,"namespace":ns,"uid":format!("{ns}-{name}")}})
}

#[test]
fn deployment_graph_does_not_fan_out_through_shared_services_pdb_or_ingress() {
    use super::relations;
    let ns = "production";
    let mut root = pod();
    root.kind = ResourceKind::Deployment;
    root.name = "api".into();
    let mut rs = object("ReplicaSet", "api-rs", ns);
    rs["metadata"]["ownerReferences"] =
        json!([{"kind":"Deployment","name":"api","uid":"production-api"}]);
    let mut own_pod = object("Pod", "api-0", ns);
    own_pod["metadata"]["ownerReferences"] =
        json!([{"kind":"ReplicaSet","name":"api-rs","uid":"production-api-rs"}]);
    own_pod["metadata"]["labels"] = json!({"tier":"backend"});
    own_pod["spec"] = json!({"nodeName":"shared","containers":[{"name":"api"}]});
    let mut service = object("Service", "shared", ns);
    service["spec"]["selector"] = json!({"tier":"backend"});
    let mut pdb = object("PodDisruptionBudget", "shared", ns);
    pdb["spec"]["selector"] = json!({});
    let mut ingress = object("Ingress", "shared", ns);
    ingress["spec"]["rules"] = json!([{"http":{"paths":[{"backend":{"service":{"name":"shared"}}},{"backend":{"service":{"name":"other"}}}]}}]);
    let mut values = vec![
        (ResourceKind::Deployment, object("Deployment", "api", ns)),
        (ResourceKind::ReplicaSet, rs),
        (ResourceKind::Pod, own_pod),
        (ResourceKind::Service, service),
        (ResourceKind::PodDisruptionBudget, pdb),
        (ResourceKind::Ingress, ingress),
        (ResourceKind::Service, object("Service", "other", ns)),
    ];
    for ix in 0..120 {
        let mut unrelated = object("Pod", &format!("other-{ix}"), ns);
        unrelated["metadata"]["labels"] = json!({"tier":"backend"});
        unrelated["spec"] = json!({"nodeName":"shared","containers":[{"name":"unrelated"}]});
        values.push((ResourceKind::Pod, unrelated));
    }
    let graph = relations::build(&root, &values);
    assert!(!graph.clipped);
    assert!(graph.nodes.len() < 12);
    assert!(
        !graph
            .nodes
            .values()
            .any(|node| node.resource.name.starts_with("other"))
    );
    for kind in [
        ResourceKind::ReplicaSet,
        ResourceKind::Pod,
        ResourceKind::Service,
        ResourceKind::Ingress,
        ResourceKind::PodDisruptionBudget,
    ] {
        assert!(graph.nodes.values().any(|node| node.resource.kind == kind));
    }
    // Selecting the shared Service itself does show all its direct targets.
    let mut service_root = root.clone();
    service_root.kind = ResourceKind::Service;
    service_root.name = "shared".into();
    let service_graph = relations::build(&service_root, &values);
    assert!(
        service_graph
            .nodes
            .values()
            .any(|node| node.resource.name == "other-119")
    );
}

#[test]
fn pod_graph_follows_ancestors_without_opening_sibling_replicas_or_controllers() {
    use super::relations;
    let mut root = pod();
    root.name = "api-0".into();
    let mut values = vec![(
        ResourceKind::Deployment,
        object("Deployment", "api", "production"),
    )];
    for (rs_name, pod_name) in [("api-rs", "api-0"), ("old-rs", "old-0")] {
        let mut rs = object("ReplicaSet", rs_name, "production");
        rs["metadata"]["ownerReferences"] =
            json!([{"kind":"Deployment","name":"api","uid":"production-api"}]);
        let mut child = object("Pod", pod_name, "production");
        child["metadata"]["ownerReferences"] =
            json!([{"kind":"ReplicaSet","name":rs_name,"uid":format!("production-{rs_name}")}]);
        values.extend([(ResourceKind::ReplicaSet, rs), (ResourceKind::Pod, child)]);
    }
    let graph = relations::build(&root, &values);
    assert!(
        graph
            .nodes
            .values()
            .any(|node| node.resource.kind == ResourceKind::Deployment)
    );
    assert!(
        !graph
            .nodes
            .values()
            .any(|node| node.resource.name.starts_with("old"))
    );
}

#[test]
fn relationship_view_shows_only_direct_core_neighbors() {
    use super::{relation_scope, relations};
    let mut root = pod();
    root.kind = ResourceKind::Deployment;
    root.name = "api".into();
    let mut values = vec![(
        ResourceKind::Deployment,
        object("Deployment", "api", "production"),
    )];
    for ix in 0..8 {
        let name = format!("api-{ix}");
        let mut child = object("Pod", &name, "production");
        child["metadata"]["ownerReferences"] =
            json!([{"kind":"Deployment","name":"api","uid":"production-api"}]);
        child["spec"] = json!({"containers":[{"name":"api"},{"name":"sidecar"}],"volumes":[{"configMap":{"name":"shared"}}]});
        values.push((ResourceKind::Pod, child));
    }
    let graph = relations::build(&root, &values);
    let upstream = relation_scope::neighbors(&graph, &root.key(), true);
    let downstream = relation_scope::neighbors(&graph, &root.key(), false);
    assert!(
        upstream.is_empty(),
        "namespace must not clutter workload context"
    );
    assert_eq!(downstream.len(), 8);
    assert!(
        downstream
            .iter()
            .all(|key| graph.nodes[key].container.is_none()
                && graph.nodes[key].resource.kind == ResourceKind::Pod)
    );
    assert_eq!(downstream.chunks(relation_scope::PAGE_SIZE).count(), 3);
    let pod_key = downstream[0].clone();
    assert_eq!(
        relation_scope::neighbors(&graph, &pod_key, true),
        vec![root.key()]
    );
    assert_eq!(relation_scope::neighbors(&graph, &pod_key, false).len(), 2);
}

#[test]
fn details_prioritize_real_operational_properties_without_filling_missing_usage() {
    let deployment = json!({"metadata":{"name":"api","namespace":"production"},"spec":{"replicas":3,"strategy":{"type":"RollingUpdate"},"selector":{"matchLabels":{"app":"api"}},"template":{"spec":{"containers":[{"name":"api","resources":{"requests":{"cpu":"100m","memory":"128Mi"},"limits":{"memory":"512Mi"}},"ports":[{"containerPort":8080}]}]}}},"status":{"readyReplicas":2,"updatedReplicas":3,"availableReplicas":2}});
    let rows = model::overview(&deployment, ResourceKind::Deployment);
    for (key, expected) in [
        ("k8s.desired_replicas", "3"),
        ("k8s.ready_replicas", "2"),
        ("k8s.update_strategy", "RollingUpdate"),
        ("k8s.selector", "app=api"),
    ] {
        assert!(
            rows.iter()
                .any(|(label, value)| *label == key && value == expected)
        );
    }
    assert!(
        rows.iter()
            .any(|(key, value)| *key == "k8s.resource_requests"
                && value.contains("100m")
                && value.contains("0.12 G"))
    );
    assert!(!rows.iter().any(|(key, _)| *key == "k8s.restarts"));
    let missing = model::overview(
        &json!({"spec":{"containers":[{"name":"api"}]}}),
        ResourceKind::Pod,
    );
    assert!(
        !missing
            .iter()
            .any(|(key, _)| matches!(*key, "k8s.restarts" | "k8s.ready_containers"))
    );
}

#[test]
fn relationships_trace_controllers_network_storage_and_all_container_types() {
    use super::relations::{self, Relation};
    let mut pod_value = object("Pod", "web-0", "production");
    pod_value["metadata"]["ownerReferences"] =
        json!([{"kind":"ReplicaSet","name":"web-rs","uid":"production-web-rs"}]);
    pod_value["metadata"]["labels"] = json!({"app":"web"});
    pod_value["spec"] = json!({"nodeName":"node-1","serviceAccountName":"web-sa",
        "containers":[{"name":"web","image":"web:v1","envFrom":[{"configMapRef":{"name":"settings"}}]}],
        "initContainers":[{"name":"init"}],"ephemeralContainers":[{"name":"debug"}],
        "volumes":[{"persistentVolumeClaim":{"claimName":"data"}},{"secret":{"secretName":"token"}}]});
    let mut rs = object("ReplicaSet", "web-rs", "production");
    rs["metadata"]["ownerReferences"] =
        json!([{"kind":"Deployment","name":"web","uid":"production-web"}]);
    let mut service = object("Service", "web", "production");
    service["spec"]["selector"] = json!({"app":"web"});
    let mut ingress = object("Ingress", "web", "production");
    ingress["spec"]["rules"] = json!([{"http":{"paths":[{"backend":{"service":{"name":"web"}}}]}}]);
    let mut claim = object("PersistentVolumeClaim", "data", "production");
    claim["spec"] = json!({"volumeName":"pv-1","storageClassName":"fast"});
    let graph = relations::build(
        &pod(),
        &[
            (ResourceKind::Pod, pod_value),
            (ResourceKind::ReplicaSet, rs),
            (
                ResourceKind::Deployment,
                object("Deployment", "web", "production"),
            ),
            (ResourceKind::Service, service),
            (ResourceKind::Ingress, ingress),
            (ResourceKind::PersistentVolumeClaim, claim),
        ],
    );
    for name in [
        "web-rs", "web", "node-1", "web-sa", "settings", "token", "data", "pv-1", "fast",
    ] {
        assert!(
            graph.nodes.values().any(|node| node.resource.name == name),
            "missing {name}"
        );
    }
    for name in ["web", "init", "debug"] {
        assert!(
            graph
                .nodes
                .values()
                .any(|node| node.container.as_deref() == Some(name))
        );
    }
    assert!(
        graph
            .edges
            .iter()
            .any(|(_, _, relation)| *relation == Relation::Owns)
    );
    assert!(
        graph
            .edges
            .iter()
            .any(|(_, _, relation)| *relation == Relation::Routes)
    );
    assert!(
        graph
            .edges
            .iter()
            .any(|(_, _, relation)| *relation == Relation::Mounts)
    );
}

#[test]
fn relationships_do_not_expand_shared_infrastructure_or_recreated_owners() {
    use super::relations;
    let mut first = object("Pod", "web-0", "production");
    first["spec"] = json!({"nodeName":"shared","volumes":[{"configMap":{"name":"shared"}}]});
    first["metadata"]["ownerReferences"] =
        json!([{"kind":"ReplicaSet","name":"old","uid":"old-uid"}]);
    let mut other = object("Pod", "unrelated", "production");
    other["spec"] = first["spec"].clone();
    let graph = relations::build(
        &pod(),
        &[
            (ResourceKind::Pod, first),
            (ResourceKind::Pod, other),
            (
                ResourceKind::ReplicaSet,
                object("ReplicaSet", "old", "production"),
            ),
        ],
    );
    assert!(
        !graph
            .nodes
            .values()
            .any(|node| node.resource.name == "unrelated" || node.resource.name == "old")
    );
}

#[test]
fn relation_graph_is_bounded_without_recursing_on_owner_cycles() {
    use super::relations;
    let mut root = pod();
    root.kind = ResourceKind::Namespace;
    root.namespace.clear();
    root.name = "production".into();
    let values = (0..200)
        .map(|ix| {
            (
                ResourceKind::Pod,
                object("Pod", &format!("pod-{ix}"), "production"),
            )
        })
        .collect::<Vec<_>>();
    let graph = relations::build(&root, &values);
    assert!(graph.clipped);
    assert!(graph.nodes.len() <= relations::NODE_LIMIT);
}

#[test]
fn related_pod_lists_are_complete_beyond_the_graph_limit() {
    use super::relations;
    let root = Resource {
        kind: ResourceKind::Deployment,
        name: "web".into(),
        ..pod()
    };
    let deployment = object("Deployment", "web", "production");
    let mut replica_set = object("ReplicaSet", "web-rs", "production");
    replica_set["metadata"]["ownerReferences"] = json!([{
        "apiVersion":"apps/v1", "kind":"Deployment", "name":"web",
        "uid":deployment["metadata"]["uid"], "controller":true
    }]);
    let mut values = vec![
        (ResourceKind::Deployment, deployment),
        (ResourceKind::ReplicaSet, replica_set.clone()),
    ];
    for ix in 0..200 {
        let mut value = object("Pod", &format!("web-{ix:03}"), "production");
        value["metadata"]["ownerReferences"] = json!([{
            "apiVersion":"apps/v1", "kind":"ReplicaSet", "name":"web-rs",
            "uid":replica_set["metadata"]["uid"], "controller":true
        }]);
        value["spec"] = json!({"containers":[{"name":"app"},{"name":"sidecar"}]});
        values.push((ResourceKind::Pod, value));
    }
    let mut unrelated = values.last().unwrap().1.clone();
    unrelated["metadata"]["name"] = json!("unrelated");
    unrelated["metadata"]["ownerReferences"][0]["uid"] = json!("recreated-rs");
    values.push((ResourceKind::Pod, unrelated));
    let graph = relations::build(&root, &values);
    assert!(graph.clipped);
    let pods = relations::related_pods(&root, &values).unwrap();
    assert_eq!(pods.len(), 200);
    assert!(pods.iter().any(|pod| pod.name == "web-199"));
    assert!(!pods.iter().any(|pod| pod.name == "unrelated"));
}

#[test]
fn cluster_queries_omit_namespace_and_secret_queries_only_request_metadata() {
    let decoded = script(&commands::list(
        "prod",
        Some("production"),
        ResourceKind::Node,
        None,
    ));
    assert!(!decoded.contains("--namespace") && !decoded.contains("/api/v1/services?limit=100"));
    let node = model::resources(
        r#"{"items":[{"metadata":{"name":"node-1"}}]}"#,
        ResourceKind::Node,
    )
    .unwrap();
    assert_eq!(node[0].namespace, "");
    let decoded = script(&commands::detail(
        "prod",
        &Resource {
            kind: ResourceKind::Secret,
            ..pod()
        },
    ));
    assert!(decoded.contains("jsonpath-as-json='{.metadata}'"));
    let secret = model::resources(
        r#"[{"name":"token","namespace":"production"}]"#,
        ResourceKind::Secret,
    )
    .unwrap();
    assert_eq!(secret[0].name, "token");
}

#[test]
fn secret_details_do_not_display_data_embedded_in_last_applied_annotations() {
    let metadata = json!([{"name":"token", "namespace":"production", "uid":"secret-uid",
        "resourceVersion":"7", "annotations":{"note":"metadata", "kubectl.kubernetes.io/last-applied-configuration":"{\"data\":{\"password\":\"hidden\"}}"},
        "managedFields":[{"manager":"kubectl"}]}]);
    let value = model::secret_detail(&metadata).unwrap();
    assert_eq!(value["metadata"]["uid"], "secret-uid");
    assert_eq!(value["metadata"]["annotations"]["note"], "metadata");
    assert!(value["metadata"].get("managedFields").is_none());
    assert!(
        value["metadata"]["annotations"]
            .get("kubectl.kubernetes.io/last-applied-configuration")
            .is_none()
    );
    assert!(!super::mutation::yaml(&value).unwrap().contains("hidden"));
    for malformed in [json!([]), json!([null]), json!({})] {
        assert!(model::secret_detail(&malformed).is_err());
    }
}

fn pod() -> Resource {
    Resource {
        kind: ResourceKind::Pod,
        name: "web-0".into(),
        namespace: "production".into(),
        summary: String::new(),
        labels: String::new(),
        status: None,
    }
}

#[test]
fn status_tags_require_ready_containers_and_distinguish_workload_readiness() {
    use model::Tone;
    let mut value = json!({"status":{"phase":"Running","containerStatuses":[{"ready":true}]}});
    assert_eq!(
        model::status(&value, ResourceKind::Pod),
        Some(("Running".into(), Tone::Good))
    );
    value["status"]["containerStatuses"][0]["ready"] = json!(false);
    assert_eq!(
        model::status(&value, ResourceKind::Pod),
        Some(("Running".into(), Tone::Warning))
    );
    value["status"]["containerStatuses"][0]["state"] =
        json!({"waiting":{"reason":"CrashLoopBackOff"}});
    assert_eq!(
        model::status(&value, ResourceKind::Pod),
        Some(("CrashLoopBackOff".into(), Tone::Danger))
    );
    assert_eq!(
        model::status(
            &json!({"spec":{"replicas":3},"status":{"readyReplicas":2}}),
            ResourceKind::Deployment
        ),
        Some(("2/3".into(), Tone::Warning))
    );
    assert_eq!(
        model::status(&json!({"spec":{"replicas":0}}), ResourceKind::StatefulSet),
        Some(("0/0".into(), Tone::Good))
    );
    assert_eq!(
        model::status(
            &json!({"status":{"phase":"Lost"}}),
            ResourceKind::PersistentVolumeClaim
        ),
        Some(("Lost".into(), Tone::Danger))
    );
}

#[test]
fn overview_exposes_copyable_names_addresses_and_node() {
    let value = json!({"metadata":{"name":"api-0","namespace":"prod"},"spec":{"nodeName":"node-1"},"status":{"podIP":"10.1.0.2","hostIP":"192.168.1.1"}});
    let rows = model::overview(&value, ResourceKind::Pod);
    for row in [
        ("k8s.name", "api-0"),
        ("k8s.namespace", "prod"),
        ("k8s.pod_ip", "10.1.0.2"),
        ("k8s.host_ip", "192.168.1.1"),
        ("k8s.node", "node-1"),
    ] {
        assert!(
            rows.iter()
                .any(|(key, value)| *key == row.0 && value == row.1)
        );
    }
}

#[test]
fn framed_replies_distinguish_rbac_errors_empty_output_and_truncation() {
    assert_eq!(
        commands::answer("\n__SHELLRS_K8S_EXIT__:0\n"),
        Ok(String::new())
    );
    assert_eq!(
        commands::answer("Error from server (Forbidden)\n__SHELLRS_K8S_EXIT__:1\n"),
        Err("Error from server (Forbidden) (exit 1)".into())
    );
    assert!(commands::answer("{\"items\":[]}").is_err());
    // Marker-like content inside a log line is not the final status.
    assert_eq!(
        commands::answer("__SHELLRS_K8S_EXIT__:1\ntext\n__SHELLRS_K8S_EXIT__:0\n"),
        Ok("__SHELLRS_K8S_EXIT__:1\ntext".into())
    );
}

#[test]
fn discovery_preserves_context_names_and_all_namespaces() {
    let (current, contexts) = commands::contexts(
        "prod\n\n__SHELLRS_CONTEXTS__\nqa\nprod\nprod\n\n__SHELLRS_K8S_EXIT__:0\n",
    )
    .unwrap();
    assert_eq!(current, "prod");
    assert_eq!(contexts, ["prod", "qa"]);
    assert_eq!(
        model::names(r#"{"items":[{"metadata":{"name":"z"}},{"metadata":{"name":"a"}}]}"#).unwrap(),
        ["a", "z"]
    );
    assert!(model::names(r#"{"items":[{}]}"#).is_err());
}

#[test]
fn queries_are_explicitly_scoped_and_shell_quoted() {
    let context = "prod';$(touch /tmp/oops)";
    let wire = commands::list(context, Some("team-a"), ResourceKind::Deployment, None);
    assert!(wire.starts_with("sh -c '"));
    let command = script(&wire);
    assert!(command.contains("deployments"));
    assert!(command.contains("/namespaces/team%2Da/deployments?limit=100"));
    assert!(!command.contains("/api/v1/services?limit=100"));
    assert!(
        script(&commands::list("prod", None, ResourceKind::Service, None))
            .contains("/api/v1/services?limit=100")
    );
    let generated = commands::suggestions(
        context,
        &pod(),
        &LogOptions {
            container: "api".into(),
            ..Default::default()
        },
    );
    assert!(
        generated
            .iter()
            .all(|(_, command)| command.contains("--context='prod'\\'';$(touch /tmp/oops)'"))
    );
    assert!(
        generated
            .iter()
            .all(|(_, command)| command.contains("--namespace='production'"))
    );
    assert!(
        generated
            .iter()
            .any(|(_, command)| command.contains("--container='api' -- /bin/sh"))
    );
}

#[test]
fn log_queries_bound_work_and_bind_container_previous_and_time() {
    let options = LogOptions {
        container: "init".into(),
        tail: 500,
        since: "15m".into(),
        previous: true,
    };
    let command = commands::logs("cluster", &pod(), &options, false).unwrap();
    for option in [
        "--container='init'",
        "--tail=500",
        "--limit-bytes=524288",
        "--since='15m'",
        "--previous=true",
        "--timestamps=true",
    ] {
        assert!(command.contains(option), "{option}: {command}");
    }
    assert!(!command.contains("--follow"));
    assert!(
        commands::logs("cluster", &pod(), &options, true)
            .unwrap()
            .contains("--follow=true")
    );
    let follow = commands::logs("cluster", &pod(), &options, true).unwrap();
    assert!(!follow.contains("--limit-bytes"));
    assert!(!follow.contains("--request-timeout"));
    assert!(commands::logs("cluster", &pod(), &LogOptions::default(), false).is_none());
}

#[test]
fn pods_report_readiness_restarts_waiting_reason_and_node() {
    let rows = model::resources(
        &json!({"items":[{"metadata":{"name":"web","namespace":"prod","labels":{"app":"web"}},
        "spec":{"containers":[{"name":"api"},{"name":"sidecar"}],"nodeName":"node-1"},
        "status":{"phase":"Running","containerStatuses":[{"ready":true,"restartCount":2},
            {"ready":false,"restartCount":3,"state":{"waiting":{"reason":"CrashLoopBackOff"}}}]}}]})
        .to_string(),
        ResourceKind::Pod,
    )
    .unwrap();
    assert_eq!(rows[0].summary, "CrashLoopBackOff · 1/2 · ↻ 5 · node-1");
    assert!(rows[0].matches("app=web"));
    assert!(rows[0].matches("CRASHLOOP"));
    assert!(!rows[0].matches("staging"));
}

#[test]
fn basic_resource_kinds_parse_without_hiding_empty_or_invalid_lists() {
    for kind in ResourceKind::ALL {
        assert!(
            model::resources(r#"{"items":[]}"#, kind)
                .unwrap()
                .is_empty()
        );
        let rows = model::resources(r#"{"items":[{"metadata":{"name":"app","namespace":"ns"},"spec":{"replicas":3},"status":{"readyReplicas":2}}]}"#, kind).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].namespace, "ns");
        assert_eq!(
            model::resources(r#"{"items":[{"metadata":{"name":"app"}}]}"#, kind).is_err(),
            kind.namespaced()
        );
    }
    let rows = model::resources(r#"{"items":[{"metadata":{"name":"web","namespace":"ns"},"spec":{"type":"ClusterIP","clusterIP":"10.0.0.1","ports":[{"port":80,"protocol":"TCP"}]}}]}"#, ResourceKind::Service).unwrap();
    assert_eq!(rows[0].summary, "ClusterIP · 10.0.0.1 · 80/TCP");
}

#[test]
fn container_discovery_includes_init_and_ephemeral_and_selectors_are_complete() {
    assert_eq!(
        model::containers(
            &json!({"spec":{"containers":[{"name":"api"},{"name":"sidecar"}],"initContainers":[{"name":"init"}],"ephemeralContainers":[{"name":"debug"}]}})
        ),
        ["api", "sidecar", "init", "debug"]
    );
    let selector = model::pod_selector(&json!({"spec":{"selector":{"matchLabels":{"app":"web"},"matchExpressions":[{"key":"tier","operator":"In","values":["api","worker"]},{"key":"disabled","operator":"DoesNotExist"}]}}}), ResourceKind::Deployment).unwrap();
    assert_eq!(selector, "app=web,tier in (api,worker),!disabled");
    assert_eq!(
        model::pod_selector(
            &json!({"spec":{"selector":{"app":"web"}}}),
            ResourceKind::Service
        ),
        Some("app=web".into())
    );
    assert_eq!(
        model::pod_selector(&json!({"spec":{}}), ResourceKind::Service),
        None
    );
}

#[test]
fn log_search_is_literal_or_regex_and_never_silently_accepts_invalid_patterns() {
    let logs = "INFO ready\nERROR 请求失败\nWARN retry\nerror lower case\n";
    assert_eq!(
        filter(logs, "ERROR", false).unwrap(),
        ("ERROR 请求失败".into(), 1)
    );
    assert_eq!(filter(logs, "ERROR|WARN", true).unwrap().1, 2);
    assert_eq!(filter(logs, "ERROR|WARN", false).unwrap().1, 0);
    assert_eq!(filter(logs, "请求", false).unwrap().1, 1);
    assert!(filter(logs, "[", true).is_err());
    assert_eq!(filter(logs, "", false).unwrap().1, 4);
    assert_eq!(filter("", "", false).unwrap(), (String::new(), 0));
}

fn script(wire: &str) -> String {
    let encoded = wire
        .strip_prefix("sh -c 'printf \"%b\" \"")
        .unwrap()
        .strip_suffix("\" | sh'")
        .unwrap();
    let bytes = encoded
        .as_bytes()
        .as_chunks::<5>()
        .0
        .iter()
        .map(|chunk| {
            assert_eq!(&chunk[..2], b"\\0");
            u8::from_str_radix(std::str::from_utf8(&chunk[2..]).unwrap(), 8).unwrap()
        })
        .collect::<Vec<_>>();
    String::from_utf8(bytes).unwrap()
}

#[cfg(unix)]
#[test]
fn remote_scripts_run_in_sh_and_capture_stderr_and_exit_status() {
    let wire = commands::remote("printf 'hello\\n'; printf 'Forbidden\\n' >&2; (exit 7)");
    let output = crate::testing::sh(&wire, None);
    let output = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        commands::answer(&output),
        Err("hello\nForbidden (exit 7)".into())
    );
    let context = "production'\"!$(printf INJECTED)";
    let wire = commands::remote(&format!(
        "kubectl() {{ printf '%s\\n' \"$@\"; }}; {} get pods",
        commands::kubectl(context)
    ));
    let output = crate::testing::sh(&wire, None);
    let output = commands::answer(&String::from_utf8(output.stdout).unwrap()).unwrap();
    assert!(
        output
            .lines()
            .any(|line| line == format!("--context={context}"))
    );
}

fn compressed_reply(text: &str) -> String {
    use base64::Engine as _;
    use std::io::Write as _;
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(text.as_bytes()).unwrap();
    let encoded = base64::engine::general_purpose::STANDARD.encode(encoder.finish().unwrap());
    format!(
        "{}{}",
        commands::COMPRESSED,
        encoded
            .as_bytes()
            .chunks(76)
            .map(|chunk| std::str::from_utf8(chunk).unwrap())
            .collect::<Vec<_>>()
            .join("\n")
    )
}

#[test]
fn compressed_reads_preserve_large_utf8_results_errors_and_reject_corruption() {
    let body = "容器配置\\n".repeat(180_000);
    let wire = compressed_reply(&format!("{body}\n__SHELLRS_K8S_EXIT__:0\n"));
    assert!(body.len() > commands::WIRE_LIMIT && wire.len() < commands::WIRE_LIMIT);
    assert_eq!(commands::answer(&wire).unwrap(), body);
    assert_eq!(
        commands::answer(&compressed_reply("Forbidden\n__SHELLRS_K8S_EXIT__:1\n")),
        Err("Forbidden (exit 1)".into())
    );
    assert!(commands::answer(&format!("{}broken", commands::COMPRESSED)).is_err());
    assert!(commands::answer(&wire[..wire.len() / 2]).is_err());
    let oversized = compressed_reply(&"x".repeat(commands::DECODED_LIMIT + 1));
    assert!(commands::answer(&oversized).is_err());
    let script = script(&commands::remote_data("printf 'test'"));
    assert!(
        script.contains("command -v gzip")
            && script.contains("gzip -1 -c | base64")
            && script.contains("else")
    );
}

#[test]
fn api_pages_encode_tokens_selectors_and_keep_cluster_scope() {
    let command = script(
        &commands::ListQuery::new("ctx';$()", Some("prod"), ResourceKind::Deployment)
            .selector(Some("app=api,tier in (web,worker)"))
            .command("token+/=&';$()", 50),
    );
    assert!(command.contains("/apis/apps/v1/namespaces/prod/deployments?limit=50"));
    assert!(command.contains("labelSelector=app%3Dapi%2Ctier%20in%20%28web%2Cworker%29"));
    assert!(command.contains("continue=token%2B%2F%3D%26%27%3B%24%28%29"));
    assert!(command.contains("--context='ctx'\\'';$()'"));
    let node = script(
        &commands::ListQuery::new("ctx", Some("ignored"), ResourceKind::Node).command("", 100),
    );
    assert!(node.contains("/api/v1/nodes?limit=100") && !node.contains("/namespaces/ignored"));
    let query = commands::ListQuery::new("ctx", None, ResourceKind::Pod);
    assert_eq!(
        super::query::smaller_limit(query, &"x".repeat(commands::WIRE_LIMIT), 100),
        Some(50)
    );
    assert_eq!(super::query::smaller_limit(query, "Forbidden", 100), None);
    assert_eq!(
        super::query::smaller_limit(query, &"x".repeat(commands::WIRE_LIMIT), 1),
        None
    );
}

#[test]
fn list_pages_keep_continuation_and_discard_verbose_list_only_metadata() {
    let reply = serde_json::json!({"metadata":{"continue":"next","resourceVersion":"42"},"items":[{"metadata":{"name":"api","namespace":"prod","managedFields":[{"fieldsV1":{"f:spec":{}}}],"annotations":{"kubectl.kubernetes.io/last-applied-configuration":"large","example.com/annotation":"api","example.com/annotationspace":"prod"}}}]});
    let page = super::query::page(
        &format!("{reply}\n__SHELLRS_K8S_EXIT__:0\n"),
        ResourceKind::Pod,
    )
    .unwrap();
    assert_eq!(page.next, "next");
    assert_eq!(page.version, "42");
    assert!(page.items[0]["metadata"]["managedFields"].is_null());
    assert!(page.items[0]["metadata"]["annotations"]["kubectl.kubernetes.io/last-applied-configuration"].is_null());
    assert_eq!(
        page.items[0]["metadata"]["annotations"]["example.com/annotation"],
        "api"
    );
    let metadata = super::query::page(
        "[{\"name\":\"config\",\"namespace\":\"prod\"}]\n__SHELLRS_K8S_EXIT__:0\n",
        ResourceKind::ConfigMap,
    )
    .unwrap();
    assert_eq!(metadata.items[0]["metadata"]["name"], "config");
    assert!(metadata.items[0]["data"].is_null());
}

#[test]
fn relationship_queries_fetch_only_the_next_hop() {
    use super::relation_scope::query_kinds;
    assert!(query_kinds(ResourceKind::Pod).is_empty());
    assert_eq!(
        query_kinds(ResourceKind::Deployment),
        &[ResourceKind::ReplicaSet]
    );
    assert_eq!(query_kinds(ResourceKind::StatefulSet), &[ResourceKind::Pod]);
    assert_eq!(query_kinds(ResourceKind::CronJob), &[ResourceKind::Job]);
    assert!(query_kinds(ResourceKind::Ingress).is_empty());
}

#[test]
fn structured_remote_script_preserves_failures_and_plain_fallback_in_local_sh() {
    #[cfg(windows)]
    let executable = {
        let path = std::path::Path::new("C:/Program Files/Git/bin/sh.exe");
        if !path.exists() {
            return;
        }
        path
    };
    #[cfg(not(windows))]
    let executable = std::path::Path::new("/bin/sh");
    let run = |wire: &str| {
        let mut process = std::process::Command::new(executable);
        #[cfg(windows)]
        process.env("PATH", "/usr/bin:/bin");
        process.args(["-c", wire]).output().unwrap()
    };
    let wire = commands::remote_data("printf 'hello\\n'; printf 'Forbidden\\n' >&2; (exit 7)");
    let output = run(&wire);
    assert!(output.status.success());
    assert_eq!(
        commands::answer(std::str::from_utf8(&output.stdout).unwrap()),
        Err("hello\nForbidden (exit 7)".into())
    );
    let fallback = script(&wire).replace("command -v gzip >/dev/null 2>&1", "false");
    let output = run(&fallback);
    let output = std::str::from_utf8(&output.stdout).unwrap();
    assert!(!output.starts_with(commands::COMPRESSED));
    assert_eq!(
        commands::answer(output),
        Err("hello\nForbidden (exit 7)".into())
    );
}

#[test]
fn configuration_references_are_visible_and_container_specific() {
    use super::{relation_scope, relations};
    let mut value = object("Pod", "web-0", "production");
    value["spec"] = json!({"imagePullSecrets":[{"name":"pull"}], "containers":[
        {"name":"app", "envFrom":[{"configMapRef":{"name":"settings"}}], "env":[{"valueFrom":{"secretKeyRef":{"name":"password", "key":"value"}}}], "volumeMounts":[{"name":"shared"}]},
        {"name":"sidecar", "envFrom":[{"secretRef":{"name":"sidecar-only"}}]}],
        "initContainers":[{"name":"init", "envFrom":[{"configMapRef":{"name":"init-settings"}}]}],
        "ephemeralContainers":[{"name":"debug", "envFrom":[{"secretRef":{"name":"debug-only"}}]}],
        "volumes":[{"name":"shared", "projected":{"sources":[{"configMap":{"name":"mounted"}},{"secret":{"name":"certificate"}}]}}, {"name":"data", "persistentVolumeClaim":{"claimName":"data"}}, {"name":"csi", "csi":{"nodePublishSecretRef":{"name":"csi-token"}}}]});
    let graph = relations::build(&pod(), &[(ResourceKind::Pod, value)]);
    let names = |key: &str| {
        relation_scope::dependencies(&graph, key)
            .iter()
            .map(|key| graph.nodes[key].resource.name.clone())
            .collect::<Vec<_>>()
    };
    let root = pod().key();
    for name in [
        "settings",
        "password",
        "pull",
        "mounted",
        "certificate",
        "data",
        "init-settings",
        "debug-only",
        "csi-token",
    ] {
        assert!(names(&root).contains(&name.into()), "missing {name}");
    }
    let app = format!("{root}:container:app");
    assert_eq!(
        names(&app),
        ["mounted", "settings", "certificate", "password"]
    );
    assert_eq!(
        names(&format!("{root}:container:sidecar")),
        ["sidecar-only"]
    );
    assert_eq!(relation_scope::neighbors(&graph, &app, true), [root]);
}

#[test]
fn explicit_references_reject_recreated_targets_and_foreign_api_groups() {
    use super::relations;
    let mut root = pod();
    root.kind = ResourceKind::EndpointSlice;
    root.name = "web-slice".into();
    let mut slice = object("EndpointSlice", "web-slice", "production");
    slice["endpoints"] =
        json!([{"targetRef":{"apiVersion":"v1", "kind":"Pod", "name":"web-0", "uid":"deleted"}}]);
    let graph = relations::build(
        &root,
        &[
            (root.kind, slice),
            (ResourceKind::Pod, object("Pod", "web-0", "production")),
        ],
    );
    assert!(!graph.nodes.contains_key(&pod().key()));
    let mut child = object("Pod", "web-0", "production");
    child["metadata"]["ownerReferences"] = json!([{"apiVersion":"custom.example/v1", "kind":"Deployment", "name":"fake", "uid":"fake"}]);
    let graph = relations::build(&pod(), &[(ResourceKind::Pod, child)]);
    assert!(
        !graph
            .nodes
            .values()
            .any(|node| node.resource.name == "fake")
    );
    let mut volume = object("PersistentVolume", "data", "");
    volume["spec"]["claimRef"] = json!({"name":"web-0", "namespace":"production", "uid":"deleted"});
    let mut claim = object("PersistentVolumeClaim", "web-0", "production");
    claim["spec"]["volumeName"] = json!("data");
    let mut root = pod();
    root.kind = ResourceKind::PersistentVolumeClaim;
    let graph = relations::build(
        &root,
        &[(root.kind, claim), (ResourceKind::PersistentVolume, volume)],
    );
    assert!(
        !graph
            .nodes
            .values()
            .any(|node| node.resource.kind == ResourceKind::PersistentVolume)
    );
}

#[test]
fn secret_consumers_include_tls_and_service_accounts_but_not_other_namespaces() {
    use super::{relation_scope, relations};
    let mut root = pod();
    root.kind = ResourceKind::Secret;
    root.name = "tls".into();
    let mut ingress = object("Ingress", "web", "production");
    ingress["spec"]["tls"] = json!([{"secretName":"tls"}]);
    let mut other = ingress.clone();
    other["metadata"]["namespace"] = json!("other");
    other["metadata"]["name"] = json!("unrelated");
    let mut sa = object("ServiceAccount", "puller", "production");
    sa["imagePullSecrets"] = json!([{"name":"tls"}]);
    let graph = relations::build(
        &root,
        &[
            (ResourceKind::Ingress, ingress),
            (ResourceKind::Ingress, other),
            (ResourceKind::ServiceAccount, sa),
        ],
    );
    let keys = relation_scope::neighbors(&graph, &root.key(), true);
    assert_eq!(keys.len(), 2);
    assert!(
        !graph
            .nodes
            .values()
            .any(|node| node.resource.name == "unrelated")
    );
    assert!(relation_scope::query_kinds(root.kind).contains(&ResourceKind::Ingress));
    assert!(relation_scope::query_kinds(root.kind).contains(&ResourceKind::ServiceAccount));
}

#[test]
fn configmap_details_list_text_and_binary_keys_without_exposing_secret_values() {
    let rows = model::overview(
        &json!({"data":{"app.yaml":"port: 80"}, "binaryData":{"logo":"AA=="}, "immutable":true}),
        ResourceKind::ConfigMap,
    );
    assert!(
        rows.iter()
            .any(|(key, value)| *key == "k8s.config_keys" && value == "app.yaml\nlogo")
    );
    assert!(
        rows.iter()
            .any(|(key, value)| *key == "k8s.immutable" && value == "true")
    );
}

#[test]
fn service_and_pdb_empty_selectors_follow_their_distinct_api_semantics() {
    use super::{relation_scope, relations};
    for (kind, selector, expected) in [
        (ResourceKind::Service, json!({}), 0),
        (ResourceKind::PodDisruptionBudget, json!(null), 0),
        (ResourceKind::PodDisruptionBudget, json!({}), 1),
    ] {
        let mut root = pod();
        root.kind = kind;
        root.name = "selection".into();
        let mut value = object(kind.label(), "selection", "production");
        value["spec"]["selector"] = selector;
        let graph = relations::build(
            &root,
            &[
                (kind, value),
                (ResourceKind::Pod, object("Pod", "web-0", "production")),
                (ResourceKind::Pod, object("Pod", "other", "staging")),
            ],
        );
        assert_eq!(
            relation_scope::neighbors(&graph, &root.key(), false).len(),
            expected,
            "{kind:?}"
        );
    }
    let mut root = pod();
    root.kind = ResourceKind::StorageClass;
    root.name = "fast".into();
    root.namespace.clear();
    let mut volume = object("PersistentVolume", "data", "");
    volume["spec"]["storageClassName"] = json!("fast");
    let graph = relations::build(&root, &[(ResourceKind::PersistentVolume, volume)]);
    assert_eq!(
        relation_scope::neighbors(&graph, &root.key(), true).len(),
        1
    );
}
