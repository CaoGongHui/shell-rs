//! YAML editing and exact-resource mutations, prepared before confirmation.
use serde_json::{Value, json};

use super::{
    commands,
    model::{Resource, ResourceKind},
};
use crate::i18n::t;

pub(super) const EDIT_LIMIT: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceOperation {
    Edit,
    Restart,
    Delete,
}

impl ResourceOperation {
    pub(super) fn label(self) -> gpui_kit::SharedString {
        match self {
            Self::Edit => t!("k8s.edit_yaml"),
            Self::Restart => t!("k8s.restart"),
            Self::Delete => t!("common.delete"),
        }
    }
}

pub(super) fn editable(kind: ResourceKind) -> bool {
    !matches!(kind, ResourceKind::Secret)
}

pub(super) fn deletable(_: ResourceKind) -> bool {
    true
}

pub(super) fn restart_kind(kind: ResourceKind) -> bool {
    matches!(
        kind,
        ResourceKind::Deployment
            | ResourceKind::StatefulSet
            | ResourceKind::DaemonSet
            | ResourceKind::Pod
    )
}

pub(super) fn restart_blocker(kind: ResourceKind, value: &Value) -> Option<gpui_kit::SharedString> {
    if value["metadata"]["deletionTimestamp"]
        .as_str()
        .is_some_and(|timestamp| !timestamp.is_empty())
    {
        return Some(t!("k8s.restart_terminating"));
    }
    if kind == ResourceKind::Pod {
        let controlled = value["metadata"]["ownerReferences"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|owner| {
                owner["controller"] == true
                    && matches!(
                        owner["kind"].as_str(),
                        Some(
                            "ReplicaSet"
                                | "StatefulSet"
                                | "DaemonSet"
                                | "ReplicationController"
                                | "Job"
                        )
                    )
            });
        if !controlled
            || matches!(
                value["status"]["phase"].as_str(),
                Some("Succeeded" | "Failed")
            )
        {
            return Some(t!("k8s.pod_restart_unavailable"));
        }
    } else if !restart_kind(kind) {
        return Some(t!("k8s.operation_unsupported"));
    } else if value["spec"]["paused"] == true
        || value["spec"]["updateStrategy"]["type"] == "OnDelete"
    {
        return Some(t!("k8s.rollout_restart_unavailable"));
    }
    None
}

pub(super) fn manifest(value: &Value) -> Value {
    let mut value = value.clone();
    if let Some(object) = value.as_object_mut() {
        object.remove("status");
    }
    if let Some(metadata) = value["metadata"].as_object_mut() {
        metadata.remove("managedFields");
    }
    value
}

pub(super) fn yaml(value: &Value) -> Result<String, String> {
    serde_yaml_ng::to_string(value).map_err(|why| why.to_string())
}

pub(super) fn identity(value: &Value, resource: &Resource) -> Result<(), String> {
    if value["metadata"]["name"].as_str() != Some(&resource.name)
        || value["kind"].as_str().and_then(ResourceKind::from_kind) != Some(resource.kind)
        || value["metadata"]["namespace"].as_str().unwrap_or("") != resource.namespace
        || value["metadata"]["uid"].as_str().is_none_or(str::is_empty)
        || value["metadata"]["resourceVersion"]
            .as_str()
            .is_none_or(str::is_empty)
    {
        return Err(t!("k8s.mutation_identity_missing").to_string());
    }
    Ok(())
}

pub(super) fn edited(text: &str, original: &Value, resource: &Resource) -> Result<Value, String> {
    identity(original, resource)?;
    if !editable(resource.kind) {
        return Err(t!("k8s.operation_unsupported").to_string());
    }
    if text.len() > EDIT_LIMIT {
        return Err(t!("k8s.edit_too_large").to_string());
    }
    let yaml: serde_yaml_ng::Value = serde_yaml_ng::from_str(text)
        .map_err(|why| t!("k8s.yaml_invalid", error = why).to_string())?;
    let value: Value = serde_yaml_ng::from_value(yaml)
        .map_err(|why| t!("k8s.yaml_invalid", error = why).to_string())?;
    if !value.is_object() || value.get("status").is_some() {
        return Err(t!("k8s.edit_status_readonly").to_string());
    }
    for field in ["apiVersion", "kind"] {
        if value[field] != original[field] || value[field].as_str().is_none() {
            return Err(t!("k8s.edit_identity_fixed").to_string());
        }
    }
    for field in ["name", "namespace", "uid", "resourceVersion"] {
        if value["metadata"][field] != original["metadata"][field] {
            return Err(t!("k8s.edit_identity_fixed").to_string());
        }
    }
    let value = manifest(&value);
    if value == manifest(original) {
        return Err(t!("k8s.edit_unchanged").to_string());
    }
    Ok(value)
}

fn path(resource: &Resource) -> String {
    let escape = |value: &str| {
        percent_encoding::utf8_percent_encode(value, percent_encoding::NON_ALPHANUMERIC).to_string()
    };
    let group = match resource.kind {
        ResourceKind::Deployment
        | ResourceKind::StatefulSet
        | ResourceKind::DaemonSet
        | ResourceKind::ReplicaSet => "/apis/apps/v1",
        ResourceKind::Ingress => "/apis/networking.k8s.io/v1",
        ResourceKind::Job | ResourceKind::CronJob => "/apis/batch/v1",
        ResourceKind::StorageClass => "/apis/storage.k8s.io/v1",
        ResourceKind::HorizontalPodAutoscaler => "/apis/autoscaling/v2",
        ResourceKind::PodDisruptionBudget => "/apis/policy/v1",
        ResourceKind::EndpointSlice => "/apis/discovery.k8s.io/v1",
        _ => "/api/v1",
    };
    let scope = if resource.kind.namespaced() {
        format!("/namespaces/{}", escape(&resource.namespace))
    } else {
        String::new()
    };
    format!(
        "{group}{scope}/{}/{}",
        resource.kind.resource(),
        escape(&resource.name)
    )
}

pub(super) fn replace(
    context: &str,
    resource: &Resource,
    original: &Value,
    edited: &Value,
) -> Result<String, String> {
    identity(original, resource)?;
    let command = commands::remote_stdin(
        &format!(
            "{} replace --raw={} -f -",
            commands::kubectl(context),
            commands::quote(&path(resource))
        ),
        edited,
    );
    if command.len() > commands::COMMAND_LIMIT {
        return Err(t!("k8s.edit_too_large").to_string());
    }
    Ok(command)
}

pub(super) fn delete(context: &str, resource: &Resource, value: &Value) -> Result<String, String> {
    identity(value, resource)?;
    if !deletable(resource.kind) {
        return Err(t!("k8s.operation_unsupported").to_string());
    }
    let options = json!({"apiVersion":"v1","kind":"DeleteOptions","propagationPolicy":"Background",
        "preconditions":{"uid":value["metadata"]["uid"],"resourceVersion":value["metadata"]["resourceVersion"]}});
    Ok(commands::remote_stdin(
        &format!(
            "{} delete --raw={} -f -",
            commands::kubectl(context),
            commands::quote(&path(resource))
        ),
        &options,
    ))
}

pub(super) fn restart(context: &str, resource: &Resource, value: &Value) -> Result<String, String> {
    identity(value, resource)?;
    if let Some(why) = restart_blocker(resource.kind, value) {
        return Err(why.to_string());
    }
    if resource.kind == ResourceKind::Pod {
        return delete(context, resource, value);
    }
    let annotations = json!({"kubectl.kubernetes.io/restartedAt":chrono::Utc::now().to_rfc3339()});
    let mut patch = vec![
        json!({"op":"test","path":"/metadata/uid","value":value["metadata"]["uid"]}),
        json!({"op":"test","path":"/metadata/resourceVersion","value":value["metadata"]["resourceVersion"]}),
    ];
    if !value["spec"]["template"]["metadata"].is_object() {
        patch.push(json!({"op":"add","path":"/spec/template/metadata","value":{"annotations":annotations}}));
    } else if !value["spec"]["template"]["metadata"]["annotations"].is_object() {
        patch.push(
            json!({"op":"add","path":"/spec/template/metadata/annotations","value":annotations}),
        );
    } else {
        patch.push(json!({"op":"add","path":"/spec/template/metadata/annotations/kubectl.kubernetes.io~1restartedAt","value":annotations["kubectl.kubernetes.io/restartedAt"]}));
    }
    Ok(commands::remote(&format!(
        "{} patch {} {} --namespace={} --type=json --patch={} -o name",
        commands::kubectl(context),
        resource.kind.resource(),
        commands::quote(&resource.name),
        commands::quote(&resource.namespace),
        commands::quote(&serde_json::to_string(&patch).expect("JSON patch serializes"))
    )))
}
