//! The data-flow graph (spec §3.2 R3): each consumer's role, bound to the
//! providers its bindings select — `graph`.
//!
//! The graph is read from descriptors and interface tokens only, never
//! inferred from traffic (R3): an edge is a binding that matches a provider
//! present now, which is what [`zk2::presence::edges`] computes and what the
//! `edges` below are. A role whose bindings match nothing has no edge, and
//! stays visible on its node — whether that is a finding is `doctor`'s
//! question (binding-unsatisfied, FJ6), not this document's.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::report::InstanceRef;

/// `graph`: every service presence and descriptors show, and the bindings
/// between them.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BindingGraph {
    /// Whether the presence read completed before its timeout (§8.1).
    pub complete: bool,
    /// By address. Every instance holds an instance token, consumers
    /// included, so a pure consumer is a node too (§8.1).
    pub nodes: Vec<GraphNode>,
    /// Sorted by consumer, role, provider.
    pub edges: Vec<GraphEdge>,
    /// Instances whose descriptor did not read: their roles, and any
    /// tokenless interface they provide, are missing from the graph.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub undescribed: Vec<InstanceRef>,
}

/// One service.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GraphNode {
    /// `<system>/<service>`.
    pub address: String,
    /// How many instances presence shows.
    pub instances: usize,
    /// The interfaces it provides, by token or descriptor, `<name>.v<major>`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provides: Vec<String>,
    /// Its roles, by role name, with the bindings its descriptors declare.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub requires: BTreeMap<String, GraphRole>,
}

/// One role of a node.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GraphRole {
    /// The required interface, `<name>.v<major>`.
    pub interface: String,
    /// Service addresses, either chunk may be `*` (R1).
    pub bindings: Vec<String>,
}

/// One edge: a consumer's role bound to a provider present now.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct GraphEdge {
    /// The consumer's `<system>/<service>`.
    pub consumer: String,
    pub role: String,
    /// The interface the role requires.
    pub interface: String,
    /// The provider's `<system>/<service>`.
    pub provider: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The document `graph --format json` prints; a node with no roles and
    /// no interfaces is still a node.
    #[test]
    fn binding_graph_json_shape_is_pinned() {
        let graph = BindingGraph {
            complete: true,
            nodes: vec![
                GraphNode {
                    address: "vehicle-01/cam-front".into(),
                    instances: 1,
                    provides: vec!["camera.v1".into()],
                    requires: BTreeMap::new(),
                },
                GraphNode {
                    address: "vehicle-01/detector".into(),
                    instances: 1,
                    provides: vec!["detections.v1".into()],
                    requires: [(
                        "input".to_owned(),
                        GraphRole {
                            interface: "camera.v1".into(),
                            bindings: vec!["vehicle-01/cam-front".into()],
                        },
                    )]
                    .into(),
                },
                GraphNode {
                    address: "vehicle-01/logger".into(),
                    instances: 1,
                    provides: vec![],
                    requires: BTreeMap::new(),
                },
            ],
            edges: vec![GraphEdge {
                consumer: "vehicle-01/detector".into(),
                role: "input".into(),
                interface: "camera.v1".into(),
                provider: "vehicle-01/cam-front".into(),
            }],
            undescribed: vec![],
        };
        assert_eq!(
            serde_json::to_value(&graph).expect("serialize"),
            json!({
                "complete": true,
                "nodes": [
                    {"address": "vehicle-01/cam-front", "instances": 1, "provides": ["camera.v1"]},
                    {
                        "address": "vehicle-01/detector",
                        "instances": 1,
                        "provides": ["detections.v1"],
                        "requires": {"input": {"interface": "camera.v1", "bindings": ["vehicle-01/cam-front"]}},
                    },
                    {"address": "vehicle-01/logger", "instances": 1},
                ],
                "edges": [{
                    "consumer": "vehicle-01/detector",
                    "role": "input",
                    "interface": "camera.v1",
                    "provider": "vehicle-01/cam-front",
                }],
            })
        );
    }
}
