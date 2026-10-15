//! Optional, scoped reads. Metrics never block resource details or require
//! credentials beyond the kubectl identity already selected by the user.
use serde_json::Value;

use super::{
    commands,
    model::{Resource, ResourceKind},
};

#[derive(Clone, Debug, Default)]
pub(super) struct Usage {
    pub cpu: Option<f64>,
    pub memory: Option<u64>,
    pub cpu_total: Option<f64>,
    pub memory_total: Option<u64>,
}

#[derive(Clone, Debug)]
pub(super) struct Disk {
    pub used: u64,
    pub total: u64,
}

/// Kubernetes quantities, including cores, nano/millicores and SI/IEC bytes.
pub(super) fn quantity(text: &str) -> Option<f64> {
    let (number, scale) = [
        ("Ki", 1024.),
        ("Mi", 1024_f64.powi(2)),
        ("Gi", 1024_f64.powi(3)),
        ("Ti", 1024_f64.powi(4)),
        ("Pi", 1024_f64.powi(5)),
        ("Ei", 1024_f64.powi(6)),
        ("n", 1e-9),
        ("u", 1e-6),
        ("m", 1e-3),
        ("k", 1e3),
        ("M", 1e6),
        ("G", 1e9),
        ("T", 1e12),
        ("P", 1e15),
        ("E", 1e18),
    ]
    .into_iter()
    .find_map(|(suffix, scale)| text.strip_suffix(suffix).map(|number| (number, scale)))
    .unwrap_or((text, 1.));
    let value = number.parse::<f64>().ok()? * scale;
    (value.is_finite() && value >= 0.).then_some(value)
}

fn number(value: &Value) -> Option<f64> {
    quantity(value.as_str()?)
}

fn bytes(value: f64) -> Option<u64> {
    (value < u64::MAX as f64).then_some(value.round() as u64)
}

pub(super) fn command(context: &str, resource: &Resource, disk: bool) -> Option<String> {
    let name =
        percent_encoding::utf8_percent_encode(&resource.name, percent_encoding::NON_ALPHANUMERIC);
    let namespace = percent_encoding::utf8_percent_encode(
        &resource.namespace,
        percent_encoding::NON_ALPHANUMERIC,
    );
    let path = match (resource.kind, disk) {
        (ResourceKind::Node, true) => format!("/api/v1/nodes/{name}/proxy/stats/summary"),
        (ResourceKind::Node, false) => format!("/apis/metrics.k8s.io/v1beta1/nodes/{name}"),
        (ResourceKind::Pod, false) => {
            format!("/apis/metrics.k8s.io/v1beta1/namespaces/{namespace}/pods/{name}")
        }
        _ => return None,
    };
    Some(commands::remote_data(&format!(
        "{} get --raw={}",
        commands::kubectl(context),
        commands::quote(&path)
    )))
}

pub(super) fn usage(
    value: &Value,
    detail: &Value,
    resource: &Resource,
    selected: Option<&str>,
) -> Option<Usage> {
    if value["metadata"]["name"].as_str()? != resource.name
        || (resource.kind == ResourceKind::Pod
            && value["metadata"]["namespace"].as_str()? != resource.namespace)
    {
        return None;
    }
    if resource.kind == ResourceKind::Node {
        return Some(Usage {
            cpu: number(&value["usage"]["cpu"]),
            memory: number(&value["usage"]["memory"]).and_then(bytes),
            cpu_total: number(&detail["status"]["capacity"]["cpu"]).filter(|value| *value > 0.),
            memory_total: number(&detail["status"]["capacity"]["memory"])
                .and_then(bytes)
                .filter(|value| *value > 0),
        });
    }
    let containers = value["containers"]
        .as_array()?
        .iter()
        .filter(|container| selected.is_none_or(|name| container["name"].as_str() == Some(name)))
        .collect::<Vec<_>>();
    if containers.is_empty() {
        return None;
    }
    let sum = |field: &str, limits: bool| -> Option<f64> {
        containers
            .iter()
            .map(|container| {
                if !limits {
                    return number(&container["usage"][field]);
                }
                let name = container["name"].as_str()?;
                let spec = ["containers", "initContainers", "ephemeralContainers"]
                    .into_iter()
                    .flat_map(|key| detail["spec"][key].as_array().into_iter().flatten())
                    .find(|container| container["name"].as_str() == Some(name))?;
                number(&spec["resources"]["limits"][field]).filter(|value| *value > 0.)
            })
            .sum()
    };
    Some(Usage {
        cpu: sum("cpu", false),
        memory: sum("memory", false).and_then(bytes),
        cpu_total: sum("cpu", true),
        memory_total: sum("memory", true).and_then(bytes),
    })
}

pub(super) fn disk(value: &Value, name: &str) -> Option<Disk> {
    if value["node"]["nodeName"].as_str()? != name {
        return None;
    }
    let fs = &value["node"]["fs"];
    let total = fs["capacityBytes"].as_u64().filter(|value| *value > 0)?;
    Some(Disk {
        used: fs["usedBytes"].as_u64()?,
        total,
    })
}
