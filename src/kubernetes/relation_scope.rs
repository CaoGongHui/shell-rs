//! Only immediate, meaningful neighbors belong on screen.
use super::{
    model::ResourceKind,
    relations::{Graph, Relation},
};
pub(super) const PAGE_SIZE: usize = 3;

pub(super) fn neighbors(graph: &Graph, selected: &str, upstream: bool) -> Vec<String> {
    let Some(current) = graph.nodes.get(selected) else {
        return Vec::new();
    };
    let kind = current.resource.kind;
    let mut keys = graph
        .edges
        .iter()
        .filter_map(|(from, to, relation)| {
            if (upstream && to != selected) || (!upstream && from != selected) {
                return None;
            }
            let meaningful = match relation {
                Relation::Owns => true,
                Relation::Contains => {
                    kind == ResourceKind::Namespace
                        || (kind == ResourceKind::Pod && (upstream == current.container.is_some()))
                }
                Relation::Runs => !upstream && kind == ResourceKind::Node,
                Relation::Routes | Relation::Selects => matches!(
                    kind,
                    ResourceKind::Service | ResourceKind::Ingress | ResourceKind::EndpointSlice
                ),
                Relation::Binds => matches!(
                    kind,
                    ResourceKind::PersistentVolumeClaim
                        | ResourceKind::PersistentVolume
                        | ResourceKind::StorageClass
                ),
                Relation::Uses | Relation::Mounts => {
                    upstream
                        && graph.nodes[from].container.is_none()
                        && matches!(
                            kind,
                            ResourceKind::ConfigMap
                                | ResourceKind::Secret
                                | ResourceKind::ServiceAccount
                                | ResourceKind::PersistentVolumeClaim
                                | ResourceKind::PersistentVolume
                                | ResourceKind::StorageClass
                        )
                }
                Relation::Scales => !upstream && kind == ResourceKind::HorizontalPodAutoscaler,
                Relation::Protects => !upstream && kind == ResourceKind::PodDisruptionBudget,
            };
            meaningful.then(|| if upstream { from.clone() } else { to.clone() })
        })
        .collect::<Vec<_>>();
    keys.sort_by_key(|key| (graph.nodes[key].rank(), key.clone()));
    keys.dedup();
    keys
}
pub(super) fn query_kinds(kind: ResourceKind) -> &'static [ResourceKind] {
    use ResourceKind::*;
    match kind {
        Deployment => &[ReplicaSet],
        StatefulSet | DaemonSet | ReplicaSet | Job | Node | PodDisruptionBudget => &[Pod],
        Service => &[Pod, EndpointSlice],
        CronJob => &[Job],
        Namespace => &[
            Deployment,
            StatefulSet,
            DaemonSet,
            CronJob,
            Job,
            Service,
            Ingress,
            Pod,
        ],
        ConfigMap => &[Pod, Deployment, StatefulSet, DaemonSet, Job, CronJob],
        Secret => &[
            Pod,
            Deployment,
            StatefulSet,
            DaemonSet,
            Job,
            CronJob,
            Ingress,
            ServiceAccount,
        ],
        ServiceAccount => &[Pod, Deployment, StatefulSet, DaemonSet, Job, CronJob],
        PersistentVolume => &[PersistentVolumeClaim],
        StorageClass => &[PersistentVolume],
        _ => &[],
    }
}

/// Explicit dependencies are shown separately from the controller chain.
pub(super) fn dependencies(graph: &Graph, selected: &str) -> Vec<String> {
    let mut keys = graph
        .edges
        .iter()
        .filter(|(from, _, relation)| {
            from == selected && matches!(relation, Relation::Uses | Relation::Mounts)
        })
        .map(|(_, to, _)| to.clone())
        .collect::<Vec<_>>();
    keys.sort_by_key(|key| (graph.nodes[key].resource.kind.label(), key.clone()));
    keys.dedup();
    keys
}
