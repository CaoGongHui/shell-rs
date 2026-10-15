//! Kubernetes resources and container logs, using kubectl on the SSH host.
//! No kubeconfig or cluster credentials are copied into ShellRS.

mod commands;
mod details;
mod format;
mod logs;
mod metrics;
mod model;
mod mutation;
mod nodes;
mod panel;
mod presentation;
mod properties;
mod query;
mod relation_scope;
mod relation_view;
mod relations;

pub use model::{KubernetesCommand, ResourceKind};
pub use mutation::ResourceOperation;
pub use panel::KubernetesPanel;

pub(crate) const YAML_KEY_CONTEXT: &str = "KubernetesYaml";

#[cfg(test)]
mod tests;
