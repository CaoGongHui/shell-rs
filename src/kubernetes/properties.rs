//! Frequently inspected properties, without fabricated defaults or secret values.
use super::model::ResourceKind;
use serde_json::Value;

fn text(value: &Value) -> Option<String> {
    match value {
        Value::String(value) if !value.is_empty() => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}
fn add(rows: &mut Vec<(&'static str, String)>, key: &'static str, value: &Value) {
    if let Some(value) = text(value) {
        let value = match key {
            "k8s.started" | "k8s.completed" | "k8s.last_schedule" => super::format::time(&value),
            _ => value,
        };
        rows.push((key, value));
    }
}
fn joined(values: impl Iterator<Item = String>) -> Option<String> {
    let values = values.filter(|value| !value.is_empty()).collect::<Vec<_>>();
    (!values.is_empty()).then(|| values.join("\n"))
}
fn add_joined(
    rows: &mut Vec<(&'static str, String)>,
    key: &'static str,
    values: impl Iterator<Item = String>,
) {
    if let Some(value) = joined(values) {
        rows.push((key, value));
    }
}

pub(super) fn resource(value: &Value, kind: ResourceKind) -> Vec<(&'static str, String)> {
    let mut rows = Vec::new();
    let spec = &value["spec"];
    let status = &value["status"];
    let pod_spec = match kind {
        ResourceKind::Pod => spec,
        ResourceKind::CronJob => &spec["jobTemplate"]["spec"]["template"]["spec"],
        _ => &spec["template"]["spec"],
    };
    if let Some(containers) = pod_spec["containers"].as_array() {
        add(
            &mut rows,
            "k8s.service_account",
            &pod_spec["serviceAccountName"],
        );
        let statuses = status["containerStatuses"].as_array();
        if let Some(statuses) = statuses {
            rows.push((
                "k8s.ready_containers",
                format!(
                    "{}/{}",
                    statuses
                        .iter()
                        .filter(|status| status["ready"] == true)
                        .count(),
                    containers.len()
                ),
            ));
            let restarts = statuses
                .iter()
                .filter_map(|item| item["restartCount"].as_u64())
                .collect::<Vec<_>>();
            if !restarts.is_empty() {
                rows.push(("k8s.restarts", restarts.iter().sum::<u64>().to_string()));
            }
        }
        add_joined(
            &mut rows,
            "k8s.resource_requests",
            containers
                .iter()
                .filter_map(|container| allocation(container, "requests")),
        );
        add_joined(
            &mut rows,
            "k8s.resource_limits",
            containers
                .iter()
                .filter_map(|container| allocation(container, "limits")),
        );
        add_joined(
            &mut rows,
            "k8s.ports",
            containers.iter().flat_map(|container| {
                container["ports"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(move |port| {
                        format!(
                            "{}: {}/{}",
                            container["name"].as_str().unwrap_or("—"),
                            port["containerPort"],
                            port["protocol"].as_str().unwrap_or("TCP")
                        )
                    })
            }),
        );
    }
    match kind {
        ResourceKind::Pod => {
            add(&mut rows, "k8s.phase", &status["phase"]);
            add(&mut rows, "k8s.qos", &status["qosClass"]);
            add(&mut rows, "k8s.started", &status["startTime"]);
            add(&mut rows, "k8s.priority_class", &spec["priorityClassName"]);
        }
        ResourceKind::Deployment
        | ResourceKind::StatefulSet
        | ResourceKind::ReplicaSet
        | ResourceKind::DaemonSet => {
            for (key, field) in [
                (
                    "k8s.desired_replicas",
                    if kind == ResourceKind::DaemonSet {
                        &status["desiredNumberScheduled"]
                    } else {
                        &spec["replicas"]
                    },
                ),
                (
                    "k8s.ready_replicas",
                    if kind == ResourceKind::DaemonSet {
                        &status["numberReady"]
                    } else {
                        &status["readyReplicas"]
                    },
                ),
                (
                    "k8s.updated_replicas",
                    if kind == ResourceKind::DaemonSet {
                        &status["updatedNumberScheduled"]
                    } else {
                        &status["updatedReplicas"]
                    },
                ),
                (
                    "k8s.available_replicas",
                    if kind == ResourceKind::DaemonSet {
                        &status["numberAvailable"]
                    } else {
                        &status["availableReplicas"]
                    },
                ),
                ("k8s.unavailable_replicas", &status["unavailableReplicas"]),
                (
                    "k8s.update_strategy",
                    spec.get("strategy")
                        .map_or(
                            &spec["updateStrategy"]["type"],
                            |strategy| &strategy["type"],
                        ),
                ),
                ("k8s.min_ready_seconds", &spec["minReadySeconds"]),
            ] {
                add(&mut rows, key, field);
            }
        }
        ResourceKind::Service => {
            add(&mut rows, "k8s.service_type", &spec["type"]);
            add(&mut rows, "k8s.external_name", &spec["externalName"]);
            add(
                &mut rows,
                "k8s.external_traffic_policy",
                &spec["externalTrafficPolicy"],
            );
            add_joined(
                &mut rows,
                "k8s.ports",
                spec["ports"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|port| {
                        let target = text(&port["targetPort"])?;
                        let node = text(&port["nodePort"])
                            .map(|node| format!(" · NodePort {node}"))
                            .unwrap_or_default();
                        Some(format!(
                            "{}/{} → {target}{node}",
                            port["port"],
                            port["protocol"].as_str().unwrap_or("TCP")
                        ))
                    }),
            );
        }
        ResourceKind::Ingress => {
            add(&mut rows, "k8s.ingress_class", &spec["ingressClassName"]);
            add_joined(
                &mut rows,
                "k8s.routes",
                spec["rules"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .flat_map(|rule| {
                        rule["http"]["paths"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .map(move |path| {
                                format!(
                                    "{}{} → {}:{}",
                                    rule["host"].as_str().unwrap_or("*"),
                                    path["path"].as_str().unwrap_or("/"),
                                    path["backend"]["service"]["name"].as_str().unwrap_or("—"),
                                    text(&path["backend"]["service"]["port"]["number"])
                                        .or_else(|| text(
                                            &path["backend"]["service"]["port"]["name"]
                                        ))
                                        .unwrap_or_else(|| "—".into())
                                )
                            })
                    }),
            );
        }
        ResourceKind::PersistentVolumeClaim | ResourceKind::PersistentVolume => {
            for (key, field) in [
                ("k8s.phase", &status["phase"]),
                ("k8s.storage_class", &spec["storageClassName"]),
                ("k8s.volume", &spec["volumeName"]),
                (
                    "k8s.storage_requested",
                    &spec["resources"]["requests"]["storage"],
                ),
                (
                    "k8s.storage_capacity",
                    if kind == ResourceKind::PersistentVolume {
                        &spec["capacity"]["storage"]
                    } else {
                        &status["capacity"]["storage"]
                    },
                ),
                ("k8s.reclaim_policy", &spec["persistentVolumeReclaimPolicy"]),
                ("k8s.volume_mode", &spec["volumeMode"]),
            ] {
                add(&mut rows, key, field);
            }
            add_joined(
                &mut rows,
                "k8s.access_modes",
                spec["accessModes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(text),
            );
        }
        ResourceKind::ConfigMap => {
            add(&mut rows, "k8s.immutable", &value["immutable"]);
            add_joined(
                &mut rows,
                "k8s.config_keys",
                ["data", "binaryData"].into_iter().flat_map(|field| {
                    value[field]
                        .as_object()
                        .into_iter()
                        .flatten()
                        .map(|(key, _)| key.clone())
                }),
            );
        }
        ResourceKind::Job => {
            for (key, field) in [
                ("k8s.completions", &spec["completions"]),
                ("k8s.parallelism", &spec["parallelism"]),
                ("k8s.active_pods", &status["active"]),
                ("k8s.succeeded_pods", &status["succeeded"]),
                ("k8s.failed_pods", &status["failed"]),
                ("k8s.backoff_limit", &spec["backoffLimit"]),
                ("k8s.started", &status["startTime"]),
                ("k8s.completed", &status["completionTime"]),
            ] {
                add(&mut rows, key, field);
            }
        }
        ResourceKind::CronJob => {
            for (key, field) in [
                ("k8s.schedule", &spec["schedule"]),
                ("k8s.time_zone", &spec["timeZone"]),
                ("k8s.suspended", &spec["suspend"]),
                ("k8s.concurrency_policy", &spec["concurrencyPolicy"]),
                ("k8s.last_schedule", &status["lastScheduleTime"]),
            ] {
                add(&mut rows, key, field);
            }
        }
        ResourceKind::HorizontalPodAutoscaler => {
            for (key, field) in [
                ("k8s.min_replicas", &spec["minReplicas"]),
                ("k8s.max_replicas", &spec["maxReplicas"]),
                ("k8s.current_replicas", &status["currentReplicas"]),
                ("k8s.desired_replicas", &status["desiredReplicas"]),
            ] {
                add(&mut rows, key, field);
            }
            add_joined(
                &mut rows,
                "k8s.scale_target",
                std::iter::once(format!(
                    "{} / {}",
                    spec["scaleTargetRef"]["kind"].as_str().unwrap_or("—"),
                    spec["scaleTargetRef"]["name"].as_str().unwrap_or("—")
                )),
            );
        }
        _ => {}
    }
    if let Some(selector) = super::model::pod_selector(value, kind) {
        rows.push(("k8s.selector", selector));
    }
    add_joined(
        &mut rows,
        "k8s.conditions",
        status["conditions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|condition| {
                Some(format!(
                    "{}={}{}",
                    condition["type"].as_str()?,
                    condition["status"].as_str()?,
                    condition["message"]
                        .as_str()
                        .filter(|message| !message.is_empty())
                        .map(|message| format!(" · {message}"))
                        .unwrap_or_default()
                ))
            }),
    );
    rows
}

fn allocation(container: &Value, field: &str) -> Option<String> {
    let resources = container["resources"][field].as_object()?;
    let values = resources
        .iter()
        .filter_map(|(key, value)| {
            text(value).map(|value| {
                let value = if key == "memory" {
                    super::format::memory(&value)
                } else {
                    value
                };
                format!("{key}={value}")
            })
        })
        .collect::<Vec<_>>();
    (!values.is_empty()).then(|| {
        format!(
            "{}: {}",
            container["name"].as_str().unwrap_or("—"),
            values.join(", ")
        )
    })
}

pub(super) fn metadata(key: &str) -> bool {
    matches!(
        key,
        "k8s.name" | "k8s.namespace" | "k8s.created" | "k8s.labels"
    )
}
pub(super) fn priority(key: &str) -> usize {
    match key {
        "k8s.pod"
        | "k8s.ready_containers"
        | "k8s.restarts"
        | "k8s.desired_replicas"
        | "k8s.ready_replicas" => 0,
        "k8s.pod_ip" | "k8s.node" | "k8s.resource_requests" | "k8s.resource_limits" => 1,
        "k8s.images" | "k8s.ports" | "k8s.phase" | "k8s.update_strategy" => 2,
        "k8s.conditions" | "k8s.node_conditions" | "k8s.summary" => 9,
        _ => 3,
    }
}
