use serde_json::Value;

/// Resource kinds offered by the Kubernetes browser.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ResourceKind {
    #[default]
    Pod,
    Deployment,
    StatefulSet,
    DaemonSet,
    ReplicaSet,
    Service,
    Ingress,
    Job,
    CronJob,
    PersistentVolumeClaim,
    Node,
    Namespace,
    ConfigMap,
    Secret,
    ServiceAccount,
    PersistentVolume,
    StorageClass,
    HorizontalPodAutoscaler,
    PodDisruptionBudget,
    EndpointSlice,
}

impl ResourceKind {
    pub const ALL: [Self; 19] = [
        Self::Pod,
        Self::Deployment,
        Self::StatefulSet,
        Self::DaemonSet,
        Self::ReplicaSet,
        Self::Service,
        Self::Ingress,
        Self::Job,
        Self::CronJob,
        Self::PersistentVolumeClaim,
        Self::Namespace,
        Self::ConfigMap,
        Self::Secret,
        Self::ServiceAccount,
        Self::PersistentVolume,
        Self::StorageClass,
        Self::HorizontalPodAutoscaler,
        Self::PodDisruptionBudget,
        Self::EndpointSlice,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Pod => "Pod",
            Self::Deployment => "Deployment",
            Self::StatefulSet => "StatefulSet",
            Self::DaemonSet => "DaemonSet",
            Self::ReplicaSet => "ReplicaSet",
            Self::Service => "Service",
            Self::Ingress => "Ingress",
            Self::Job => "Job",
            Self::CronJob => "CronJob",
            Self::PersistentVolumeClaim => "PVC",
            Self::Node => "Node",
            Self::Namespace => "Namespace",
            Self::ConfigMap => "ConfigMap",
            Self::Secret => "Secret",
            Self::ServiceAccount => "ServiceAccount",
            Self::PersistentVolume => "PV",
            Self::StorageClass => "StorageClass",
            Self::HorizontalPodAutoscaler => "HPA",
            Self::PodDisruptionBudget => "PDB",
            Self::EndpointSlice => "EndpointSlice",
        }
    }

    pub(super) fn resource(self) -> &'static str {
        match self {
            Self::Pod => "pods",
            Self::Deployment => "deployments",
            Self::StatefulSet => "statefulsets",
            Self::DaemonSet => "daemonsets",
            Self::ReplicaSet => "replicasets",
            Self::Service => "services",
            Self::Ingress => "ingresses",
            Self::Job => "jobs",
            Self::CronJob => "cronjobs",
            Self::PersistentVolumeClaim => "persistentvolumeclaims",
            Self::Node => "nodes",
            Self::Namespace => "namespaces",
            Self::ConfigMap => "configmaps",
            Self::Secret => "secrets",
            Self::ServiceAccount => "serviceaccounts",
            Self::PersistentVolume => "persistentvolumes",
            Self::StorageClass => "storageclasses",
            Self::HorizontalPodAutoscaler => "horizontalpodautoscalers",
            Self::PodDisruptionBudget => "poddisruptionbudgets",
            Self::EndpointSlice => "endpointslices",
        }
    }

    pub(super) fn namespaced(self) -> bool {
        !matches!(
            self,
            Self::Node | Self::Namespace | Self::PersistentVolume | Self::StorageClass
        )
    }

    pub(super) fn from_kind(kind: &str) -> Option<Self> {
        Self::ALL.into_iter().chain([Self::Node]).find(|value| {
            value.label() == kind
                || matches!(
                    (value, kind),
                    (Self::PersistentVolumeClaim, "PersistentVolumeClaim")
                        | (Self::PersistentVolume, "PersistentVolume")
                        | (Self::HorizontalPodAutoscaler, "HorizontalPodAutoscaler")
                        | (Self::PodDisruptionBudget, "PodDisruptionBudget")
                )
        })
    }
}

/// Commands routed through the workspace's single Kubernetes handler.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KubernetesCommand {
    Refresh,
    Context(String),
    Namespace(Option<String>),
    Kind(ResourceKind),
    Section(usize),
    Details {
        namespace: String,
        name: String,
    },
    Manage {
        kind: ResourceKind,
        namespace: String,
        name: String,
        operation: super::mutation::ResourceOperation,
    },
    RequestOperation(super::mutation::ResourceOperation),
    SaveYaml,
    CancelEdit,
    ConfirmMutation(String),
    CancelMutation(String),
    Changed {
        terminal: u64,
        context: String,
    },
    CloseDetails,
    DetailTab(usize),
    Container(String),
    Tail(u32),
    Since(String),
    Previous(bool),
    Regex(bool),
    ReadLogs,
    LiveLogs(bool),
    FollowLogs(bool),
    CopyValue(String),
    RefreshDetails,
    RefreshMetrics,
    ToggleProperties,
    RelatedPod(String),
    RelationNode(String),
    RelationBack,
    RefreshRelations,
    RelationPage {
        column: usize,
        page: usize,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Resource {
    pub kind: ResourceKind,
    pub name: String,
    pub namespace: String,
    pub summary: String,
    pub labels: String,
    pub status: Option<(String, Tone)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tone {
    Good,
    Warning,
    Danger,
    Quiet,
}

pub(super) fn status(value: &Value, kind: ResourceKind) -> Option<(String, Tone)> {
    if value["metadata"]["deletionTimestamp"]
        .as_str()
        .is_some_and(|timestamp| !timestamp.is_empty())
    {
        return Some((
            crate::i18n::t!("k8s.terminating").to_string(),
            Tone::Warning,
        ));
    }
    let status = &value["status"];
    let spec = &value["spec"];
    let count = |v: &Value| v.as_u64().unwrap_or(0);
    let readiness = |ready: u64, desired: u64| {
        (
            format!("{ready}/{desired}"),
            if ready >= desired {
                Tone::Good
            } else {
                Tone::Warning
            },
        )
    };
    Some(match kind {
        ResourceKind::Node => {
            let ready = status["conditions"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|condition| condition["type"] == "Ready");
            if ready.is_some_and(|condition| condition["status"] == "True") {
                if spec["unschedulable"] == true {
                    ("SchedulingDisabled".into(), Tone::Warning)
                } else {
                    ("Ready".into(), Tone::Good)
                }
            } else {
                ("NotReady".into(), Tone::Warning)
            }
        }
        ResourceKind::Pod => {
            let states = status["containerStatuses"].as_array();
            let reason = ["initContainerStatuses", "containerStatuses"]
                .into_iter()
                .flat_map(|field| status[field].as_array().into_iter().flatten())
                .find_map(|s| s["state"]["waiting"]["reason"].as_str());
            let phase = if value["metadata"]["deletionTimestamp"].is_string() {
                "Terminating"
            } else {
                reason.unwrap_or(status["phase"].as_str().unwrap_or("Unknown"))
            };
            let ready = states.is_some_and(|items| {
                !items.is_empty()
                    && spec["containers"]
                        .as_array()
                        .is_none_or(|containers| containers.len() == items.len())
                    && items.iter().all(|s| s["ready"] == true)
            });
            let tone = match phase {
                "Running" if ready => Tone::Good,
                "Succeeded" => Tone::Good,
                "Failed"
                | "CrashLoopBackOff"
                | "ImagePullBackOff"
                | "ErrImagePull"
                | "CreateContainerConfigError" => Tone::Danger,
                "Pending" | "Running" | "ContainerCreating" | "PodInitializing" | "Terminating" => {
                    Tone::Warning
                }
                _ => Tone::Quiet,
            };
            (phase.to_owned(), tone)
        }
        ResourceKind::Deployment | ResourceKind::StatefulSet | ResourceKind::ReplicaSet => {
            readiness(
                count(&status["readyReplicas"]),
                spec["replicas"].as_u64().unwrap_or(1),
            )
        }
        ResourceKind::DaemonSet => readiness(
            count(&status["numberReady"]),
            count(&status["desiredNumberScheduled"]),
        ),
        ResourceKind::PersistentVolumeClaim => {
            let phase = status["phase"].as_str().unwrap_or("Unknown");
            (
                phase.to_owned(),
                match phase {
                    "Bound" => Tone::Good,
                    "Lost" => Tone::Danger,
                    "Pending" => Tone::Warning,
                    _ => Tone::Quiet,
                },
            )
        }
        ResourceKind::Job => {
            let failed = status["conditions"].as_array().is_some_and(|conditions| {
                conditions
                    .iter()
                    .any(|c| c["type"] == "Failed" && c["status"] == "True")
            });
            if failed {
                ("Failed".into(), Tone::Danger)
            } else {
                readiness(
                    count(&status["succeeded"]),
                    spec["completions"].as_u64().unwrap_or(1),
                )
            }
        }
        _ => return None,
    })
}

impl Resource {
    pub fn key(&self) -> String {
        format!(
            "k8s-resource:{}:{}/{}",
            self.kind.resource(),
            self.namespace,
            self.name
        )
    }

    pub fn matches(&self, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        [&self.name, &self.namespace, &self.summary, &self.labels]
            .iter()
            .any(|text| text.to_lowercase().contains(&query))
            || self
                .status
                .as_ref()
                .is_some_and(|(status, _)| status.to_lowercase().contains(&query))
    }
}

#[cfg(test)]
pub(super) fn names(output: &str) -> Result<Vec<String>, String> {
    let value: Value = serde_json::from_str(output).map_err(|error| error.to_string())?;
    let items = value["items"]
        .as_array()
        .ok_or("Missing Kubernetes items")?;
    let mut names = items
        .iter()
        .map(|item| {
            item["metadata"]["name"]
                .as_str()
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
                .ok_or_else(|| "Missing resource name".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    names.sort();
    names.dedup();
    Ok(names)
}

pub(super) fn list_value(output: &str, kind: ResourceKind) -> Result<Value, String> {
    let mut value: Value = serde_json::from_str(output).map_err(|error| error.to_string())?;
    if matches!(kind, ResourceKind::Secret | ResourceKind::ConfigMap) && value.is_array() {
        value = serde_json::json!({"items":value.as_array().unwrap().iter().map(|metadata| serde_json::json!({"metadata":metadata})).collect::<Vec<_>>()});
    }
    if !value["items"].is_array() {
        return Err("Missing Kubernetes items".into());
    }
    Ok(value)
}

#[cfg(test)]
pub(super) fn resources(output: &str, kind: ResourceKind) -> Result<Vec<Resource>, String> {
    let value = list_value(output, kind)?;
    let items = value["items"]
        .as_array()
        .ok_or("Missing Kubernetes items")?;
    resource_items(items, kind)
}

pub(super) fn resource_items(items: &[Value], kind: ResourceKind) -> Result<Vec<Resource>, String> {
    let mut rows = items
        .iter()
        .map(|item| {
            let name = item["metadata"]["name"]
                .as_str()
                .filter(|name| !name.is_empty())
                .ok_or("Missing resource name")?
                .to_owned();
            let namespace = item["metadata"]["namespace"]
                .as_str()
                .filter(|ns| !ns.is_empty())
                .or_else(|| (!kind.namespaced()).then_some(""))
                .ok_or("Missing resource namespace")?
                .to_owned();
            let labels = item["metadata"]["labels"]
                .as_object()
                .map(|labels| {
                    labels
                        .iter()
                        .map(|(key, value)| format!("{key}={}", value.as_str().unwrap_or("")))
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            Ok(Resource {
                kind,
                name,
                namespace,
                summary: summary(item, kind),
                labels,
                status: status(item, kind),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    rows.sort_by(|a, b| (&a.namespace, &a.name).cmp(&(&b.namespace, &b.name)));
    Ok(rows)
}

fn summary(item: &Value, kind: ResourceKind) -> String {
    let status = &item["status"];
    let spec = &item["spec"];
    let text = |value: &Value| value.as_str().unwrap_or("—").to_owned();
    let count = |value: &Value| value.as_u64().unwrap_or(0);
    match kind {
        ResourceKind::Pod => {
            let statuses = status["containerStatuses"].as_array();
            let ready = statuses.map_or(0, |items| {
                items.iter().filter(|s| s["ready"] == true).count()
            });
            let total = spec["containers"].as_array().map_or(0, Vec::len);
            let restarts: u64 = statuses.map_or(0, |items| {
                items.iter().map(|s| count(&s["restartCount"])).sum()
            });
            let phase =
                self::status(item, kind).map_or_else(|| "Unknown".to_owned(), |(phase, _)| phase);
            format!(
                "{phase} · {ready}/{total} · ↻ {restarts} · {}",
                text(&spec["nodeName"])
            )
        }
        ResourceKind::Deployment | ResourceKind::StatefulSet | ResourceKind::ReplicaSet => format!(
            "{}/{}",
            count(&status["readyReplicas"]),
            spec["replicas"].as_u64().unwrap_or(1)
        ),
        ResourceKind::DaemonSet => format!(
            "{}/{}",
            count(&status["numberReady"]),
            count(&status["desiredNumberScheduled"])
        ),
        ResourceKind::Service => {
            let ports = spec["ports"]
                .as_array()
                .map(|ports| {
                    ports
                        .iter()
                        .map(|p| {
                            format!(
                                "{}/{}",
                                count(&p["port"]),
                                p["protocol"].as_str().unwrap_or("TCP")
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            format!(
                "{} · {} · {ports}",
                text(&spec["type"]),
                text(&spec["clusterIP"])
            )
        }
        ResourceKind::Ingress => spec["rules"]
            .as_array()
            .map(|rules| {
                rules
                    .iter()
                    .filter_map(|rule| rule["host"].as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default(),
        ResourceKind::Job => format!(
            "{}/{} · active {} · failed {}",
            count(&status["succeeded"]),
            spec["completions"].as_u64().unwrap_or(1),
            count(&status["active"]),
            count(&status["failed"])
        ),
        ResourceKind::CronJob => format!(
            "{} · suspend {}",
            text(&spec["schedule"]),
            spec["suspend"].as_bool().unwrap_or(false)
        ),
        ResourceKind::PersistentVolumeClaim => format!(
            "{} · {} · {}",
            text(&status["phase"]),
            text(&status["capacity"]["storage"]),
            text(&spec["storageClassName"])
        ),
        ResourceKind::Node => super::nodes::summary(item),
        _ => text(&status["phase"]),
    }
}

pub(super) fn containers(value: &Value) -> Vec<String> {
    ["containers", "initContainers", "ephemeralContainers"]
        .into_iter()
        .flat_map(|field| value["spec"][field].as_array().into_iter().flatten())
        .filter_map(|container| container["name"].as_str().map(str::to_owned))
        .collect()
}

/// kubectl's last-applied annotation can embed a Secret's complete data.
/// Exclude it from the metadata-only inspector, just as list reads do.
pub(super) fn secret_detail(value: &Value) -> Result<Value, String> {
    let mut metadata = value
        .as_array()
        .and_then(|values| values.first())
        .and_then(Value::as_object)
        .cloned()
        .ok_or_else(|| crate::i18n::t!("k8s.incomplete").to_string())?;
    metadata.remove("managedFields");
    if let Some(annotations) = metadata
        .get_mut("annotations")
        .and_then(Value::as_object_mut)
    {
        annotations.remove("kubectl.kubernetes.io/last-applied-configuration");
    }
    Ok(serde_json::json!({"kind":"Secret", "metadata":metadata}))
}

/// A selector for workloads and services, including set-based expressions.
pub(super) fn pod_selector(value: &Value, kind: ResourceKind) -> Option<String> {
    let selector = &value["spec"]["selector"];
    let labels = if kind == ResourceKind::Service {
        selector
    } else {
        &selector["matchLabels"]
    };
    let mut terms = labels
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(key, val)| val.as_str().map(|val| format!("{key}={val}")))
        .collect::<Vec<_>>();
    if let Some(expressions) = selector["matchExpressions"].as_array() {
        for expr in expressions {
            let key = expr["key"].as_str()?;
            let values = expr["values"]
                .as_array()
                .map(|vals| {
                    vals.iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .unwrap_or_default();
            terms.push(match expr["operator"].as_str()? {
                "In" => format!("{key} in ({values})"),
                "NotIn" => format!("{key} notin ({values})"),
                "Exists" => key.to_owned(),
                "DoesNotExist" => format!("!{key}"),
                _ => return None,
            });
        }
    }
    (!terms.is_empty()).then(|| terms.join(","))
}

pub(super) fn overview(value: &Value, kind: ResourceKind) -> Vec<(&'static str, String)> {
    if kind == ResourceKind::Node {
        return super::nodes::overview(value);
    }
    let labels = value["metadata"]["labels"]
        .as_object()
        .map(|labels| {
            labels
                .iter()
                .map(|(key, value)| format!("{key}={}", value.as_str().unwrap_or("")))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let spec = if kind == ResourceKind::Pod {
        &value["spec"]
    } else if kind == ResourceKind::CronJob {
        &value["spec"]["jobTemplate"]["spec"]["template"]["spec"]
    } else {
        &value["spec"]["template"]["spec"]
    };
    let images = ["containers", "initContainers", "ephemeralContainers"]
        .into_iter()
        .flat_map(|field| spec[field].as_array().into_iter().flatten())
        .map(|container| {
            format!(
                "{}: {}",
                container["name"].as_str().unwrap_or(""),
                container["image"].as_str().unwrap_or("")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut rows = vec![("k8s.summary", summary(value, kind))];
    for (key, field) in [
        ("k8s.name", &value["metadata"]["name"]),
        ("k8s.namespace", &value["metadata"]["namespace"]),
        ("k8s.pod_ip", &value["status"]["podIP"]),
        ("k8s.host_ip", &value["status"]["hostIP"]),
        ("k8s.cluster_ip", &value["spec"]["clusterIP"]),
        ("k8s.node", &value["spec"]["nodeName"]),
    ] {
        if let Some(text) = field.as_str().filter(|text| !text.is_empty()) {
            rows.push((key, text.to_owned()));
        }
    }
    if let Some(addresses) = value["status"]["loadBalancer"]["ingress"].as_array() {
        let addresses = addresses
            .iter()
            .filter_map(|a| a["ip"].as_str().or_else(|| a["hostname"].as_str()))
            .collect::<Vec<_>>()
            .join(", ");
        if !addresses.is_empty() {
            rows.push(("k8s.external_ip", addresses));
        }
    }
    if let Some(created) = value["metadata"]["creationTimestamp"].as_str() {
        rows.push(("k8s.created", super::format::time(created)));
    }
    if !labels.is_empty() {
        rows.push(("k8s.labels", labels));
    }
    if !images.is_empty() {
        rows.push(("k8s.images", images));
    }
    rows.extend(super::properties::resource(value, kind));
    rows
}
