use layout_trace_model::{Vec2, topology::RouteClass};
use serde::{Deserialize, Serialize};

pub const CANDIDATE_SCHEMA_VERSION: u32 = 1;

/// Durable interchange boundary shared by all placer/router strategies.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateArtifact {
    pub schema_version: u32,
    pub components: Vec<SolvedComponent>,
    pub traces: Vec<SolvedTrace>,
    pub route_graphs: Vec<SolvedRouteGraph>,
}

/// A placed semantic component. Solver-internal particles and rigid bodies do
/// not cross this boundary.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SolvedComponent {
    pub id: String,
    pub position: Vec2,
    pub size: Vec2,
    pub rotation_degrees: f64,
}

/// One geometric branch of an electrical route graph.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SolvedTrace {
    pub branch: String,
    pub electrical_net: String,
    /// Compatibility alias for `branch` retained while importing historical
    /// layout-trace artifacts.
    pub net: String,
    pub from_node: String,
    pub to_node: String,
    pub width: f64,
    pub tension_weight: f64,
    /// Preferred layer. `segment_layers` is authoritative for actual copper.
    pub layer: String,
    pub segment_layers: Vec<String>,
    pub vias: Vec<SolvedVia>,
    pub points: Vec<Vec2>,
    pub route_class: RouteClass,
    pub route_basis_fingerprint: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SolvedRouteGraph {
    pub electrical_net: String,
    pub nodes: Vec<SolvedRouteNode>,
    pub branches: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SolvedRouteNode {
    pub id: String,
    pub electrical_net: String,
    pub position: Vec2,
    pub incident_branches: Vec<String>,
    #[serde(flatten)]
    pub kind: SolvedRouteNodeKind,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SolvedRouteNodeKind {
    Terminal {
        component: String,
        pin: String,
    },
    Junction {
        layer: String,
        movable: bool,
    },
    /// A via is a branch decorator, not an independent connectivity edge.
    Via {
        point_index: usize,
        from_layer: String,
        to_layer: String,
        diameter: f64,
        drill: f64,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SolvedVia {
    pub position: Vec2,
    pub from_layer: String,
    pub to_layer: String,
    pub diameter: f64,
    pub drill: f64,
    pub point_index: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Violation {
    pub code: String,
    pub objects: Vec<String>,
    pub required_distance: f64,
    pub actual_distance: f64,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flattened_route_nodes_round_trip_without_becoming_permissive() {
        let node = SolvedRouteNode {
            id: "terminal:N/U1.1".into(),
            electrical_net: "N".into(),
            position: Vec2::new(1.0, 2.0),
            incident_branches: vec!["N".into()],
            kind: SolvedRouteNodeKind::Terminal {
                component: "U1".into(),
                pin: "1".into(),
            },
        };
        let encoded = serde_json::to_value(&node).unwrap();
        let decoded: SolvedRouteNode = serde_json::from_value(encoded.clone()).unwrap();
        assert_eq!(decoded, node);

        let mut tampered = encoded;
        tampered["unexpected"] = serde_json::json!(true);
        assert!(serde_json::from_value::<SolvedRouteNode>(tampered).is_err());
    }
}
