//! Build relationships from object identity and explicit Kubernetes references.
//! Shared nodes (namespace, node, configuration) never pull unrelated workloads
//! into a workload's graph. They are still usable as the graph's starting point.
use super::model::{self, Resource, ResourceKind};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub(super) const NODE_LIMIT: usize = 160;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Relation {
    Owns,
    Selects,
    Routes,
    Runs,
    Contains,
    Uses,
    Mounts,
    Binds,
    Scales,
    Protects,
}

impl Relation {
    pub fn label(self) -> &'static str {
        match self {
            Self::Owns => "k8s.relation.owns",
            Self::Selects => "k8s.relation.selects",
            Self::Routes => "k8s.relation.routes",
            Self::Runs => "k8s.relation.runs",
            Self::Contains => "k8s.relation.contains",
            Self::Uses => "k8s.relation.uses",
            Self::Mounts => "k8s.relation.mounts",
            Self::Binds => "k8s.relation.binds",
            Self::Scales => "k8s.relation.scales",
            Self::Protects => "k8s.relation.protects",
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct GraphNode {
    pub resource: Resource,
    pub container: Option<String>,
    pub uid: Option<String>,
    pub fetched: bool,
}

impl GraphNode {
    pub fn key(&self) -> String {
        let key = self.resource.key();
        self.container
            .as_ref()
            .map_or(key.clone(), |name| format!("{key}:container:{name}"))
    }
    pub fn rank(&self) -> usize {
        if self.container.is_some() {
            return 7;
        }
        match self.resource.kind {
            ResourceKind::Namespace => 0,
            ResourceKind::Ingress
            | ResourceKind::HorizontalPodAutoscaler
            | ResourceKind::PodDisruptionBudget => 1,
            ResourceKind::Deployment
            | ResourceKind::StatefulSet
            | ResourceKind::DaemonSet
            | ResourceKind::CronJob => 2,
            ResourceKind::ReplicaSet | ResourceKind::Job | ResourceKind::Service => 3,
            ResourceKind::Node => 4,
            ResourceKind::Pod | ResourceKind::EndpointSlice => 5,
            _ => 6,
        }
    }
}

#[derive(Clone, Default)]
pub(super) struct Graph {
    pub nodes: BTreeMap<String, GraphNode>,
    pub edges: BTreeSet<(String, String, Relation)>,
    pub clipped: bool,
}

fn reference(kind: ResourceKind, namespace: &str, name: &str) -> GraphNode {
    GraphNode {
        resource: Resource {
            kind,
            namespace: if kind.namespaced() {
                namespace.into()
            } else {
                String::new()
            },
            name: name.into(),
            summary: String::new(),
            labels: String::new(),
            status: None,
        },
        container: None,
        uid: None,
        fetched: false,
    }
}

impl Graph {
    fn insert(&mut self, node: GraphNode) -> String {
        let key = node.key();
        if node.fetched || !self.nodes.contains_key(&key) {
            self.nodes.insert(key.clone(), node);
        }
        key
    }
    fn insert_reference(&mut self, mut node: GraphNode, uid: Option<&str>) -> Option<String> {
        if node.resource.name.is_empty() {
            return None;
        }
        node.uid = uid.map(str::to_owned);
        if self.nodes.get(&node.key()).is_some_and(|actual| {
            uid.is_some_and(|expected| {
                actual
                    .uid
                    .as_deref()
                    .is_some_and(|actual| actual != expected)
            })
        }) {
            return None;
        }
        Some(self.insert(node))
    }
    fn connect(&mut self, from: &str, to: GraphNode, relation: Relation) {
        if to.resource.name.is_empty() {
            return;
        }
        let to = self.insert(to);
        if from != to {
            self.edges.insert((from.into(), to, relation));
        }
    }
}

pub(super) fn build(root: &Resource, values: &[(ResourceKind, Value)]) -> Graph {
    let mut graph = Graph::default();
    graph.insert(GraphNode {
        resource: root.clone(),
        container: None,
        uid: None,
        fetched: false,
    });
    for (kind, value) in values {
        let name = value["metadata"]["name"].as_str().unwrap_or("");
        if name.is_empty() {
            continue;
        }
        let ns = value["metadata"]["namespace"].as_str().unwrap_or("");
        let mut node = reference(*kind, ns, name);
        node.uid = value["metadata"]["uid"].as_str().map(str::to_owned);
        node.fetched = true;
        node.resource.status = model::status(value, *kind);
        graph.insert(node);
    }
    for (kind, value) in values {
        let name = value["metadata"]["name"].as_str().unwrap_or("");
        let ns = value["metadata"]["namespace"].as_str().unwrap_or("");
        let key = reference(*kind, ns, name).key();
        if !graph.nodes.contains_key(&key) {
            continue;
        }
        for owner in value["metadata"]["ownerReferences"]
            .as_array()
            .into_iter()
            .flatten()
        {
            if let Some(kind) = reference_kind(owner) {
                if kind.namespaced() && ns.is_empty() {
                    continue;
                }
                if let Some(owner_key) = graph.insert_reference(
                    reference(kind, ns, owner["name"].as_str().unwrap_or("")),
                    owner["uid"].as_str(),
                ) {
                    graph.edges.insert((owner_key, key.clone(), Relation::Owns));
                }
            }
        }
        if !ns.is_empty() {
            let namespace = graph.insert(reference(ResourceKind::Namespace, "", ns));
            graph
                .edges
                .insert((namespace, key.clone(), Relation::Contains));
        }
        if *kind == ResourceKind::Pod {
            if let Some(node) = value["spec"]["nodeName"].as_str() {
                let node = graph.insert(reference(ResourceKind::Node, "", node));
                graph.edges.insert((node, key.clone(), Relation::Runs));
            }
            graph.connect(
                &key,
                reference(
                    ResourceKind::ServiceAccount,
                    ns,
                    value["spec"]["serviceAccountName"]
                        .as_str()
                        .unwrap_or("default"),
                ),
                Relation::Uses,
            );
            for field in ["containers", "initContainers", "ephemeralContainers"] {
                for container in value["spec"][field].as_array().into_iter().flatten() {
                    if let Some(name) = container["name"].as_str() {
                        let mut node = reference(ResourceKind::Pod, ns, name_for_pod(value));
                        node.container = Some(name.into());
                        node.uid = value["metadata"]["uid"].as_str().map(str::to_owned);
                        node.fetched = true;
                        node.resource.summary =
                            format!("{field} · {}", container["image"].as_str().unwrap_or("—"));
                        let container_key = node.key();
                        graph.connect(&key, node, Relation::Contains);
                        let volumes = value["spec"]["volumes"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter(|volume| {
                                ["volumeMounts", "volumeDevices"].iter().any(|field| {
                                    container[field]
                                        .as_array()
                                        .into_iter()
                                        .flatten()
                                        .any(|mount| mount["name"] == volume["name"])
                                })
                            })
                            .cloned()
                            .collect::<Vec<_>>();
                        let spec = serde_json::json!({"containers":[container], "volumes":volumes});
                        pod_dependencies(&mut graph, &container_key, ns, &spec, false);
                    }
                }
            }
        }
        let spec = match kind {
            ResourceKind::Pod => &value["spec"],
            ResourceKind::CronJob => &value["spec"]["jobTemplate"]["spec"]["template"]["spec"],
            _ => &value["spec"]["template"]["spec"],
        };
        pod_dependencies(&mut graph, &key, ns, spec, true);
        if *kind == ResourceKind::ServiceAccount {
            for field in ["imagePullSecrets", "secrets"] {
                for secret in value[field].as_array().into_iter().flatten() {
                    graph.connect(
                        &key,
                        reference(
                            ResourceKind::Secret,
                            ns,
                            secret["name"].as_str().unwrap_or(""),
                        ),
                        Relation::Uses,
                    );
                }
            }
        }
        if *kind == ResourceKind::PersistentVolumeClaim
            && let Some(name) = value["spec"]["volumeName"].as_str()
        {
            // A fetched volume's claimRef must still identify this claim.
            let compatible = values
                .iter()
                .find(|(kind, volume)| {
                    *kind == ResourceKind::PersistentVolume
                        && volume["metadata"]["name"].as_str() == Some(name)
                })
                .is_none_or(|(_, volume)| {
                    let claim = &volume["spec"]["claimRef"];
                    claim.is_null()
                        || (claim["name"].as_str()
                            == Some(value["metadata"]["name"].as_str().unwrap_or(""))
                            && claim["namespace"].as_str() == Some(ns)
                            && claim["uid"].as_str().is_none_or(|uid| {
                                value["metadata"]["uid"]
                                    .as_str()
                                    .is_none_or(|actual| actual == uid)
                            }))
                });
            if compatible {
                graph.connect(
                    &key,
                    reference(ResourceKind::PersistentVolume, "", name),
                    Relation::Binds,
                );
            }
        }
        if matches!(
            kind,
            ResourceKind::PersistentVolumeClaim | ResourceKind::PersistentVolume
        ) && let Some(name) = value["spec"]["storageClassName"].as_str()
        {
            graph.connect(
                &key,
                reference(ResourceKind::StorageClass, "", name),
                Relation::Uses,
            );
        }
        if *kind == ResourceKind::PersistentVolume {
            let claim = &value["spec"]["claimRef"];
            if let (Some(name), Some(ns)) = (claim["name"].as_str(), claim["namespace"].as_str())
                && let Some(claim) = graph.insert_reference(
                    reference(ResourceKind::PersistentVolumeClaim, ns, name),
                    value["spec"]["claimRef"]["uid"].as_str(),
                )
            {
                graph.edges.insert((claim, key.clone(), Relation::Binds));
            }
        }
        if *kind == ResourceKind::HorizontalPodAutoscaler {
            let target = &value["spec"]["scaleTargetRef"];
            if let Some(kind) = reference_kind(target) {
                graph.connect(
                    &key,
                    reference(kind, ns, target["name"].as_str().unwrap_or("")),
                    Relation::Scales,
                );
            }
        }
        if *kind == ResourceKind::Ingress {
            for backend in std::iter::once(&value["spec"]["defaultBackend"]).chain(
                value["spec"]["rules"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .flat_map(|rule| {
                        rule["http"]["paths"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .map(|path| &path["backend"])
                    }),
            ) {
                if let Some(name) = backend["service"]["name"]
                    .as_str()
                    .or_else(|| backend["serviceName"].as_str())
                {
                    graph.connect(
                        &key,
                        reference(ResourceKind::Service, ns, name),
                        Relation::Routes,
                    );
                }
            }
            for tls in value["spec"]["tls"].as_array().into_iter().flatten() {
                if let Some(name) = tls["secretName"].as_str() {
                    graph.connect(
                        &key,
                        reference(ResourceKind::Secret, ns, name),
                        Relation::Uses,
                    );
                }
            }
        }
        if *kind == ResourceKind::Service {
            for (pod_kind, pod) in values {
                if *pod_kind == ResourceKind::Pod
                    && pod["metadata"]["namespace"].as_str() == Some(ns)
                    && selector_matches(
                        &value["spec"]["selector"],
                        &pod["metadata"]["labels"],
                        false,
                    )
                {
                    graph.connect(
                        &key,
                        reference(ResourceKind::Pod, ns, name_for_pod(pod)),
                        Relation::Selects,
                    );
                }
            }
        }
        if *kind == ResourceKind::PodDisruptionBudget {
            for (pod_kind, pod) in values {
                if *pod_kind == ResourceKind::Pod
                    && pod["metadata"]["namespace"].as_str() == Some(ns)
                    && selector_matches(
                        &value["spec"]["selector"],
                        &pod["metadata"]["labels"],
                        true,
                    )
                {
                    graph.connect(
                        &key,
                        reference(ResourceKind::Pod, ns, name_for_pod(pod)),
                        Relation::Protects,
                    );
                }
            }
        }
        if *kind == ResourceKind::EndpointSlice {
            if let Some(name) = value["metadata"]["labels"]["kubernetes.io/service-name"].as_str() {
                let service = graph.insert(reference(ResourceKind::Service, ns, name));
                graph.edges.insert((service, key.clone(), Relation::Routes));
            }
            for endpoint in value["endpoints"].as_array().into_iter().flatten() {
                let target = &endpoint["targetRef"];
                if let Some(kind) = reference_kind(target)
                    && let Some(target_key) = graph.insert_reference(
                        reference(
                            kind,
                            target["namespace"].as_str().unwrap_or(ns),
                            target["name"].as_str().unwrap_or(""),
                        ),
                        target["uid"].as_str(),
                    )
                {
                    graph
                        .edges
                        .insert((key.clone(), target_key, Relation::Routes));
                }
            }
        }
    }
    graph.focused(&root.key())
}

/// The overview list is complete; the relationship diagram's node budget
/// must never decide which Pods belong to a workload.
pub(super) fn related_pods(
    root: &Resource,
    values: &[(ResourceKind, Value)],
) -> Result<Vec<Resource>, String> {
    let Some((_, snapshot)) = values.iter().find(|(kind, value)| {
        *kind == root.kind
            && value["metadata"]["name"].as_str() == Some(&root.name)
            && value["metadata"]["namespace"].as_str().unwrap_or("") == root.namespace
    }) else {
        return Ok(Vec::new());
    };
    let replica_sets = values
        .iter()
        .filter(|(kind, value)| {
            root.kind == ResourceKind::Deployment
                && *kind == ResourceKind::ReplicaSet
                && owned_by(value, root.kind, snapshot)
        })
        .filter_map(|(_, value)| Some((value["metadata"]["name"].as_str()?, value)))
        .collect::<BTreeMap<_, _>>();
    let items = values
        .iter()
        .filter(|(kind, value)| {
            if *kind != ResourceKind::Pod
                || value["metadata"]["namespace"].as_str() != Some(&root.namespace)
            {
                return false;
            }
            match root.kind {
                ResourceKind::Service | ResourceKind::PodDisruptionBudget => selector_matches(
                    &snapshot["spec"]["selector"],
                    &value["metadata"]["labels"],
                    root.kind == ResourceKind::PodDisruptionBudget,
                ),
                ResourceKind::Deployment => value["metadata"]["ownerReferences"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|owner| replica_sets.get(owner["name"].as_str()?))
                    .any(|owner| owned_by(value, ResourceKind::ReplicaSet, owner)),
                _ => owned_by(value, root.kind, snapshot),
            }
        })
        .map(|(_, value)| value.clone())
        .collect::<Vec<_>>();
    model::resource_items(&items, ResourceKind::Pod)
}

fn owned_by(value: &Value, kind: ResourceKind, owner: &Value) -> bool {
    value["metadata"]["namespace"] == owner["metadata"]["namespace"]
        && value["metadata"]["ownerReferences"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|reference| {
                reference_kind(reference) == Some(kind)
                    && reference["name"] == owner["metadata"]["name"]
                    && reference["uid"].as_str().is_none_or(|uid| {
                        owner["metadata"]["uid"]
                            .as_str()
                            .is_none_or(|actual| actual == uid)
                    })
            })
}

/// Kind names alone do not identify API types (a CRD may use the same Kind).
fn reference_kind(value: &Value) -> Option<ResourceKind> {
    let kind = value["kind"].as_str().and_then(ResourceKind::from_kind)?;
    let expected = match kind {
        ResourceKind::Deployment
        | ResourceKind::StatefulSet
        | ResourceKind::DaemonSet
        | ResourceKind::ReplicaSet => "apps",
        ResourceKind::Job | ResourceKind::CronJob => "batch",
        ResourceKind::Ingress => "networking.k8s.io",
        ResourceKind::StorageClass => "storage.k8s.io",
        ResourceKind::HorizontalPodAutoscaler => "autoscaling",
        ResourceKind::PodDisruptionBudget => "policy",
        ResourceKind::EndpointSlice => "discovery.k8s.io",
        _ => "",
    };
    if let Some(version) = value["apiVersion"].as_str() {
        let group = version.split_once('/').map_or("", |(group, _)| group);
        if group != expected {
            return None;
        }
    }
    Some(kind)
}

fn name_for_pod(value: &Value) -> &str {
    value["metadata"]["name"].as_str().unwrap_or("")
}

fn pod_dependencies(graph: &mut Graph, key: &str, ns: &str, spec: &Value, account: bool) {
    if account && spec.is_object() && spec["containers"].is_array() {
        graph.connect(
            key,
            reference(
                ResourceKind::ServiceAccount,
                ns,
                spec["serviceAccountName"].as_str().unwrap_or("default"),
            ),
            Relation::Uses,
        );
    }
    let mut add = |kind, value: &Value, field: &str, relation| {
        if let Some(name) = value[field].as_str() {
            graph.connect(key, reference(kind, ns, name), relation);
        }
    };
    for secret in spec["imagePullSecrets"].as_array().into_iter().flatten() {
        add(ResourceKind::Secret, secret, "name", Relation::Uses);
    }
    for volume in spec["volumes"].as_array().into_iter().flatten() {
        add(
            ResourceKind::Secret,
            &volume["csi"]["nodePublishSecretRef"],
            "name",
            Relation::Mounts,
        );
        add(
            ResourceKind::PersistentVolumeClaim,
            &volume["persistentVolumeClaim"],
            "claimName",
            Relation::Mounts,
        );
        add(
            ResourceKind::ConfigMap,
            &volume["configMap"],
            "name",
            Relation::Mounts,
        );
        add(
            ResourceKind::Secret,
            &volume["secret"],
            "secretName",
            Relation::Mounts,
        );
        for source in volume["projected"]["sources"]
            .as_array()
            .into_iter()
            .flatten()
        {
            add(
                ResourceKind::ConfigMap,
                &source["configMap"],
                "name",
                Relation::Mounts,
            );
            add(
                ResourceKind::Secret,
                &source["secret"],
                "name",
                Relation::Mounts,
            );
        }
    }
    for field in ["containers", "initContainers", "ephemeralContainers"] {
        for container in spec[field].as_array().into_iter().flatten() {
            for env in container["env"].as_array().into_iter().flatten() {
                add(
                    ResourceKind::ConfigMap,
                    &env["valueFrom"]["configMapKeyRef"],
                    "name",
                    Relation::Uses,
                );
                add(
                    ResourceKind::Secret,
                    &env["valueFrom"]["secretKeyRef"],
                    "name",
                    Relation::Uses,
                );
            }
            for env in container["envFrom"].as_array().into_iter().flatten() {
                add(
                    ResourceKind::ConfigMap,
                    &env["configMapRef"],
                    "name",
                    Relation::Uses,
                );
                add(
                    ResourceKind::Secret,
                    &env["secretRef"],
                    "name",
                    Relation::Uses,
                );
            }
        }
    }
}

fn selector_matches(selector: &Value, labels: &Value, structured: bool) -> bool {
    if !selector.is_object() {
        return false;
    }
    let matches = if structured {
        &selector["matchLabels"]
    } else {
        selector
    };
    let expressions = &selector["matchExpressions"];
    let has_terms = matches.as_object().is_some_and(|m| !m.is_empty())
        || expressions.as_array().is_some_and(|e| !e.is_empty());
    if !has_terms && !structured {
        return false;
    }
    if !matches
        .as_object()
        .into_iter()
        .flatten()
        .all(|(key, value)| labels.get(key) == Some(value))
    {
        return false;
    }
    expressions.as_array().into_iter().flatten().all(|expr| {
        let Some(key) = expr["key"].as_str() else {
            return false;
        };
        let value = labels.get(key);
        let contained = value.is_some_and(|value| {
            expr["values"]
                .as_array()
                .is_some_and(|values| values.contains(value))
        });
        match expr["operator"].as_str() {
            Some("In") => contained,
            Some("NotIn") => !contained,
            Some("Exists") => value.is_some(),
            Some("DoesNotExist") => value.is_none(),
            _ => false,
        }
    })
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Walk {
    Descendants,
    Ancestors,
    Dependencies,
    Network,
    Consumers,
}

impl Graph {
    fn focused(mut self, root: &str) -> Self {
        let mut visible = BTreeSet::from([root.to_owned()]);
        let mut visited = BTreeSet::new();
        let mut queue = VecDeque::from([(root.to_owned(), Walk::Descendants)]);
        if self.nodes.get(root).is_some_and(|node| {
            matches!(
                node.resource.kind,
                ResourceKind::ConfigMap
                    | ResourceKind::Secret
                    | ResourceKind::ServiceAccount
                    | ResourceKind::PersistentVolumeClaim
                    | ResourceKind::PersistentVolume
                    | ResourceKind::StorageClass
            )
        }) {
            queue.push_back((root.to_owned(), Walk::Consumers));
        }
        while let Some((key, walk)) = queue.pop_front() {
            if !visited.insert((key.clone(), walk)) {
                continue;
            }
            // A controller's descendants establish the scope. Upstream routes,
            // owners and shared dependencies never fan out into other consumers.
            if walk == Walk::Descendants {
                for next in [Walk::Ancestors, Walk::Dependencies, Walk::Network] {
                    queue.push_back((key.clone(), next));
                }
            }
            for (from, to, relation) in &self.edges {
                let forward = from == &key;
                let backward = to == &key;
                if !forward && !backward {
                    continue;
                }
                let next = if forward { to } else { from };
                let follow = match walk {
                    Walk::Descendants if forward => match relation {
                        Relation::Owns => Some(Walk::Descendants),
                        Relation::Contains
                            if self.nodes[&key].container.is_none()
                                && (self.nodes[&key].resource.kind == ResourceKind::Pod
                                    || key == root) =>
                        {
                            Some(Walk::Descendants)
                        }
                        Relation::Selects
                        | Relation::Routes
                        | Relation::Scales
                        | Relation::Protects => Some(Walk::Descendants),
                        Relation::Runs if key == root => Some(Walk::Descendants),
                        _ => None,
                    },
                    Walk::Ancestors if backward && *relation == Relation::Owns => {
                        Some(Walk::Ancestors)
                    }
                    Walk::Dependencies
                        if forward
                            && matches!(
                                relation,
                                Relation::Uses | Relation::Mounts | Relation::Binds
                            ) =>
                    {
                        Some(Walk::Dependencies)
                    }
                    Walk::Network
                        if backward
                            && matches!(
                                relation,
                                Relation::Routes
                                    | Relation::Selects
                                    | Relation::Scales
                                    | Relation::Protects
                            ) =>
                    {
                        Some(Walk::Network)
                    }
                    Walk::Consumers
                        if backward
                            && matches!(
                                relation,
                                Relation::Uses | Relation::Mounts | Relation::Binds
                            ) =>
                    {
                        if matches!(
                            self.nodes[next].resource.kind,
                            ResourceKind::PersistentVolumeClaim | ResourceKind::PersistentVolume
                        ) {
                            Some(Walk::Consumers)
                        } else {
                            Some(Walk::Descendants)
                        }
                    }
                    _ => None,
                };
                let context = backward
                    && matches!(walk, Walk::Ancestors | Walk::Network)
                    && matches!(relation, Relation::Runs | Relation::Contains);
                if follow.is_none() && !context {
                    continue;
                }
                if visible.len() >= NODE_LIMIT && !visible.contains(next) {
                    self.clipped = true;
                    continue;
                }
                visible.insert(next.clone());
                if let Some(mode) = follow {
                    queue.push_back((next.clone(), mode));
                }
            }
        }
        self.nodes.retain(|key, _| visible.contains(key));
        self.edges
            .retain(|(from, to, _)| visible.contains(from) && visible.contains(to));
        self
    }
}
