// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Placement-independent, read-only combinatorial routing regime boundary.
//!
//! This module owns discrete routing identity only. It deliberately contains
//! no placement coordinates, corridor IDs, capacity estimates, search policy,
//! realization logic, or engine integration.

use serde::{Deserialize, Serialize};

use crate::topology::HomotopyWord;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::error::Error;
use std::fmt::{self, Write};

pub const COMBINATORIAL_ROUTING_REGIME_SCHEMA_VERSION: &str =
    "layout-trace.combinatorial-routing-regime/v1";
pub const COMBINATORIAL_ROUTING_FINGERPRINT_PREFIX: &str = "sha256:";
const COMBINATORIAL_ROUTING_FINGERPRINT_SCHEMA: &str =
    "layout-trace.combinatorial-routing-fingerprint/v1";

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PhysicalPadRef {
    pub component: String,
    pub pin: String,
    pub pad: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BreakoutPortRef {
    pub component: String,
    pub interface: String,
    pub port: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ComponentPortRef {
    PhysicalPad { pad: PhysicalPadRef },
    BreakoutPort { port: BreakoutPortRef },
}

impl ComponentPortRef {
    pub fn component(&self) -> &str {
        match self {
            Self::PhysicalPad { pad } => &pad.component,
            Self::BreakoutPort { port } => &port.component,
        }
    }
}

/// One logical terminal selects immutable pad copper and the component-boundary
/// port exposed to the board-level map. They differ for an interior BGA ball.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalBinding {
    pub logical_terminal: String,
    pub physical_pad: PhysicalPadRef,
    pub exposed_port: ComponentPortRef,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentChirality {
    Normal,
    Mirrored,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentPortCycle {
    pub component: String,
    pub orientation_class: String,
    pub chirality: ComponentChirality,
    /// A cyclic order. Its first element is not semantically distinguished.
    pub clockwise_ports: Vec<ComponentPortRef>,
}

/// Selected local BGA/package escape order through one same-layer face.
/// Pairing is positional: `ordered_pads[i]` exits at `ordered_ports[i]`.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BreakoutFaceTransport {
    pub id: String,
    pub component: String,
    pub interface: String,
    pub layer: String,
    pub ordered_pads: Vec<PhysicalPadRef>,
    pub ordered_ports: Vec<BreakoutPortRef>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RouteNodeRef {
    pub electrical_net: String,
    pub node: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RegimeNodeKind {
    Terminal { logical_terminal: String },
    Split { movable: bool },
    Via,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RegimeNode {
    pub id: String,
    pub kind: RegimeNodeKind,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RegimeRun {
    pub id: String,
    pub layer: String,
    pub from_node: String,
    pub to_node: String,
    /// Coordinate-free obstacle scaffold and signed seam word for this run.
    /// `None` is valid only when the compiler certified that this layer has no
    /// unrelated obstacle capable of distinguishing route classes.
    #[serde(default)]
    pub obstacle_class: Option<RunObstacleClass>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunObstacleClass {
    pub scaffold_fingerprint: String,
    pub signed_seam_word: HomotopyWord,
}

#[derive(Clone, Copy)]
struct RunRecord<'a> {
    electrical_net: &'a str,
    branch: &'a str,
    run: &'a RegimeRun,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RegimeBranch {
    pub id: String,
    pub from_node: String,
    pub to_node: String,
    /// Directed source-to-target order; this order is semantic.
    pub ordered_runs: Vec<RegimeRun>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EmbeddedRouteGraph {
    pub electrical_net: String,
    pub nodes: Vec<RegimeNode>,
    pub branches: Vec<RegimeBranch>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DartDirection {
    Forward,
    Reverse,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LayerVertexRef {
    /// All selected terminal ports on one rigid component share this map
    /// vertex; `ComponentPortCycle` fixes their rotation around it.
    ComponentBoundary {
        component: String,
    },
    RouteNode {
        node: RouteNodeRef,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RouteDart {
    pub id: String,
    pub run: String,
    pub direction: DartDirection,
    pub origin: LayerVertexRef,
    pub inverse: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VertexRotation {
    pub vertex: LayerVertexRef,
    /// A cyclic order. Its first dart is not semantically distinguished.
    pub clockwise_darts: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RegimeFace {
    pub id: String,
    /// The derived face cycle. Its first dart is not distinguished.
    pub boundary_darts: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FaceTransport {
    pub id: String,
    pub face: String,
    pub entrance_arc: String,
    pub exit_arc: String,
    /// Authoritative lane order; unlike cycles, this is a linear order.
    pub ordered_runs: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LayerCombinatorialMap {
    pub layer: String,
    pub darts: Vec<RouteDart>,
    pub rotations: Vec<VertexRotation>,
    pub faces: Vec<RegimeFace>,
    pub outer_face: String,
    pub face_transports: Vec<FaceTransport>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LayerTransition {
    pub id: String,
    pub electrical_net: String,
    pub branch: String,
    pub via_node: String,
    pub from_run: String,
    pub to_run: String,
    pub from_layer: String,
    pub to_layer: String,
}

/// The schema intentionally has no same-layer pass-through variant.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CrossingResolution {
    SameNetJunction {
        id: String,
        electrical_net: String,
        node: String,
        runs: Vec<String>,
    },
    SeparatedLayers {
        id: String,
        first_run: String,
        second_run: String,
    },
}

impl CrossingResolution {
    pub fn id(&self) -> &str {
        match self {
            Self::SameNetJunction { id, .. } | Self::SeparatedLayers { id, .. } => id,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CombinatorialRoutingRegime {
    pub schema_version: String,
    /// Search/candidate identity. Deliberately excluded from the fingerprint.
    pub regime_id: String,
    pub intent_fingerprint: String,
    pub terminal_bindings: Vec<TerminalBinding>,
    pub component_port_cycles: Vec<ComponentPortCycle>,
    pub breakout_face_transports: Vec<BreakoutFaceTransport>,
    pub route_graphs: Vec<EmbeddedRouteGraph>,
    pub layer_maps: Vec<LayerCombinatorialMap>,
    pub layer_transitions: Vec<LayerTransition>,
    pub crossing_resolutions: Vec<CrossingResolution>,
    pub fingerprint: String,
}

fn normalize_cycle<T: Clone + Ord>(values: &mut Vec<T>) {
    let Some(minimum) = (0..values.len()).min_by(|left, right| values[*left].cmp(&values[*right]))
    else {
        return;
    };
    values.rotate_left(minimum);
}

impl CombinatorialRoutingRegime {
    /// Returns the deterministic, identity-independent form consumed by the
    /// projection and realization boundary in the solver integration crate.
    ///
    /// This is public only because those correctness layers intentionally
    /// remain outside the routing-kernel crate.
    #[doc(hidden)]
    pub fn canonicalized(&self) -> Self {
        let mut value = self.clone();
        value.regime_id.clear();
        value.fingerprint.clear();
        value
            .terminal_bindings
            .sort_by(|left, right| left.logical_terminal.cmp(&right.logical_terminal));
        for cycle in &mut value.component_port_cycles {
            normalize_cycle(&mut cycle.clockwise_ports);
        }
        value.component_port_cycles.sort_by(|left, right| {
            left.component
                .cmp(&right.component)
                .then_with(|| left.orientation_class.cmp(&right.orientation_class))
                .then_with(|| left.chirality.cmp(&right.chirality))
        });
        value
            .breakout_face_transports
            .sort_by(|left, right| left.id.cmp(&right.id));
        for graph in &mut value.route_graphs {
            graph.nodes.sort_by(|left, right| left.id.cmp(&right.id));
            graph.branches.sort_by(|left, right| left.id.cmp(&right.id));
        }
        value
            .route_graphs
            .sort_by(|left, right| left.electrical_net.cmp(&right.electrical_net));
        for layer in &mut value.layer_maps {
            layer.darts.sort_by(|left, right| left.id.cmp(&right.id));
            for rotation in &mut layer.rotations {
                normalize_cycle(&mut rotation.clockwise_darts);
            }
            layer
                .rotations
                .sort_by(|left, right| left.vertex.cmp(&right.vertex));
            for face in &mut layer.faces {
                normalize_cycle(&mut face.boundary_darts);
            }
            layer.faces.sort_by(|left, right| left.id.cmp(&right.id));
            layer
                .face_transports
                .sort_by(|left, right| left.id.cmp(&right.id));
        }
        value
            .layer_maps
            .sort_by(|left, right| left.layer.cmp(&right.layer));
        value
            .layer_transitions
            .sort_by(|left, right| left.id.cmp(&right.id));
        for resolution in &mut value.crossing_resolutions {
            match resolution {
                CrossingResolution::SameNetJunction { runs, .. } => runs.sort(),
                CrossingResolution::SeparatedLayers {
                    first_run,
                    second_run,
                    ..
                } if first_run.as_str() > second_run.as_str() => {
                    std::mem::swap(first_run, second_run)
                }
                CrossingResolution::SeparatedLayers { .. } => {}
            }
        }
        value
            .crossing_resolutions
            .sort_by(|left, right| left.id().cmp(right.id()));
        value
    }

    /// Fingerprint of discrete identity and order only. Placement, geometry,
    /// corridors, capacities, and `regime_id` are absent from this schema.
    pub fn computed_fingerprint(&self) -> String {
        #[derive(Serialize)]
        struct Record<'a> {
            fingerprint_schema: &'static str,
            regime: &'a CombinatorialRoutingRegime,
        }

        let canonical = self.canonicalized();
        let bytes = serde_json::to_vec(&Record {
            fingerprint_schema: COMBINATORIAL_ROUTING_FINGERPRINT_SCHEMA,
            regime: &canonical,
        })
        .expect("combinatorial routing regime contains no fallible JSON values");
        let digest = Sha256::digest(bytes);
        let mut result = String::with_capacity(COMBINATORIAL_ROUTING_FINGERPRINT_PREFIX.len() + 64);
        result.push_str(COMBINATORIAL_ROUTING_FINGERPRINT_PREFIX);
        for byte in digest {
            write!(&mut result, "{byte:02x}").expect("writing to a String cannot fail");
        }
        result
    }

    pub fn validate(&self) -> Result<(), Vec<CombinatorialRegimeIssue>> {
        let issues = self.validation_issues();
        if issues.is_empty() {
            Ok(())
        } else {
            Err(issues)
        }
    }

    pub fn validation_issues(&self) -> Vec<CombinatorialRegimeIssue> {
        let mut issues = Vec::new();
        if self.schema_version != COMBINATORIAL_ROUTING_REGIME_SCHEMA_VERSION {
            issues.push(CombinatorialRegimeIssue::new(
                "schema_version_unsupported",
                "$.schema_version",
                "unsupported combinatorial-routing-regime version",
            ));
        }
        require_nonempty(&self.regime_id, "$.regime_id", &mut issues);
        require_nonempty(
            &self.intent_fingerprint,
            "$.intent_fingerprint",
            &mut issues,
        );

        let mut bindings = BTreeMap::<&str, &TerminalBinding>::new();
        let mut physical_pads = BTreeSet::new();
        let mut exposed_ports = BTreeSet::new();
        for (index, binding) in self.terminal_bindings.iter().enumerate() {
            let path = format!("$.terminal_bindings[{index}]");
            require_nonempty(
                &binding.logical_terminal,
                format!("{path}.logical_terminal"),
                &mut issues,
            );
            validate_pad_ref(
                &binding.physical_pad,
                &format!("{path}.physical_pad"),
                &mut issues,
            );
            validate_port_ref(
                &binding.exposed_port,
                &format!("{path}.exposed_port"),
                &mut issues,
            );
            if binding.physical_pad.component != binding.exposed_port.component() {
                issues.push(CombinatorialRegimeIssue::new(
                    "binding_component_mismatch",
                    &path,
                    "physical pad and exposed port must belong to the same component frame",
                ));
            }
            if bindings
                .insert(binding.logical_terminal.as_str(), binding)
                .is_some()
            {
                issues.push(CombinatorialRegimeIssue::new(
                    "duplicate_terminal_binding",
                    format!("{path}.logical_terminal"),
                    "logical terminals must be bound exactly once",
                ));
            }
            if !physical_pads.insert(binding.physical_pad.clone()) {
                issues.push(CombinatorialRegimeIssue::new(
                    "duplicate_active_physical_pad",
                    format!("{path}.physical_pad"),
                    "one active physical pad cannot realize multiple logical terminals",
                ));
            }
            if !exposed_ports.insert(binding.exposed_port.clone()) {
                issues.push(CombinatorialRegimeIssue::new(
                    "duplicate_active_component_port",
                    format!("{path}.exposed_port"),
                    "one exposed component port cannot terminate multiple routes",
                ));
            }
        }

        let mut cycles = BTreeMap::<&str, &ComponentPortCycle>::new();
        for (index, cycle) in self.component_port_cycles.iter().enumerate() {
            let path = format!("$.component_port_cycles[{index}]");
            require_nonempty(&cycle.component, format!("{path}.component"), &mut issues);
            require_nonempty(
                &cycle.orientation_class,
                format!("{path}.orientation_class"),
                &mut issues,
            );
            if cycle.clockwise_ports.is_empty() {
                issues.push(CombinatorialRegimeIssue::new(
                    "empty_component_port_cycle",
                    format!("{path}.clockwise_ports"),
                    "a selected component cycle must expose at least one port",
                ));
            }
            let mut seen = BTreeSet::new();
            for (port_index, port) in cycle.clockwise_ports.iter().enumerate() {
                let port_path = format!("{path}.clockwise_ports[{port_index}]");
                validate_port_ref(port, &port_path, &mut issues);
                if port.component() != cycle.component {
                    issues.push(CombinatorialRegimeIssue::new(
                        "port_cycle_component_mismatch",
                        &port_path,
                        "every cyclic port must belong to the owning component",
                    ));
                }
                if !seen.insert(port) {
                    issues.push(CombinatorialRegimeIssue::new(
                        "duplicate_component_port",
                        &port_path,
                        "a component port may occur only once in its cycle",
                    ));
                }
            }
            if cycles.insert(cycle.component.as_str(), cycle).is_some() {
                issues.push(CombinatorialRegimeIssue::new(
                    "duplicate_component_port_cycle",
                    format!("{path}.component"),
                    "one regime selects exactly one orientation/chirality cycle per component",
                ));
            }
        }
        for (terminal, binding) in &bindings {
            let covered = cycles
                .get(binding.exposed_port.component())
                .is_some_and(|cycle| cycle.clockwise_ports.contains(&binding.exposed_port));
            if !covered {
                issues.push(CombinatorialRegimeIssue::new(
                    "terminal_port_not_in_component_cycle",
                    "$.component_port_cycles",
                    format!("bound terminal {terminal:?} is absent from its component cycle"),
                ));
            }
        }

        let mut breakout_ids = BTreeSet::new();
        let mut covered_breakout_bindings =
            BTreeSet::<(&str, PhysicalPadRef, BreakoutPortRef)>::new();
        for (index, transport) in self.breakout_face_transports.iter().enumerate() {
            let path = format!("$.breakout_face_transports[{index}]");
            require_nonempty(&transport.id, format!("{path}.id"), &mut issues);
            require_nonempty(
                &transport.component,
                format!("{path}.component"),
                &mut issues,
            );
            require_nonempty(
                &transport.interface,
                format!("{path}.interface"),
                &mut issues,
            );
            require_nonempty(&transport.layer, format!("{path}.layer"), &mut issues);
            if !breakout_ids.insert(transport.id.as_str()) {
                issues.push(CombinatorialRegimeIssue::new(
                    "duplicate_breakout_transport",
                    format!("{path}.id"),
                    "breakout transport IDs must be unique",
                ));
            }
            if transport.ordered_pads.is_empty()
                || transport.ordered_pads.len() != transport.ordered_ports.len()
            {
                issues.push(CombinatorialRegimeIssue::new(
                    "invalid_breakout_transport_arity",
                    &path,
                    "ordered pad and boundary-port lanes must have equal nonzero length",
                ));
            }
            let mut pads = BTreeSet::new();
            let mut ports = BTreeSet::new();
            for (lane, (pad, port)) in transport
                .ordered_pads
                .iter()
                .zip(&transport.ordered_ports)
                .enumerate()
            {
                validate_pad_ref(pad, &format!("{path}.ordered_pads[{lane}]"), &mut issues);
                validate_breakout_ref(port, &format!("{path}.ordered_ports[{lane}]"), &mut issues);
                if pad.component != transport.component
                    || port.component != transport.component
                    || port.interface != transport.interface
                {
                    issues.push(CombinatorialRegimeIssue::new(
                        "breakout_transport_owner_mismatch",
                        &path,
                        "all lanes must belong to the declared component and interface",
                    ));
                }
                if !pads.insert(pad) || !ports.insert(port) {
                    issues.push(CombinatorialRegimeIssue::new(
                        "duplicate_breakout_lane_endpoint",
                        &path,
                        "pads and boundary ports must each occur exactly once per transport",
                    ));
                }
                let matching: Vec<_> = bindings
                    .iter()
                    .filter(|(_, binding)| {
                        binding.physical_pad == *pad
                            && binding.exposed_port
                                == ComponentPortRef::BreakoutPort {
                                    port: (*port).clone(),
                                }
                    })
                    .collect();
                if matching.len() != 1 {
                    issues.push(CombinatorialRegimeIssue::new(
                        "breakout_lane_binding_mismatch",
                        &path,
                        "each ordered pad/port lane must match exactly one terminal binding",
                    ));
                } else {
                    covered_breakout_bindings.insert((
                        *matching[0].0,
                        (*pad).clone(),
                        (*port).clone(),
                    ));
                }
            }
        }
        for (terminal, binding) in &bindings {
            if let ComponentPortRef::BreakoutPort { port } = &binding.exposed_port
                && !covered_breakout_bindings.contains(&(
                    *terminal,
                    binding.physical_pad.clone(),
                    port.clone(),
                ))
            {
                issues.push(CombinatorialRegimeIssue::new(
                    "uncovered_breakout_binding",
                    "$.breakout_face_transports",
                    format!("breakout terminal {terminal:?} has no selected local face transport"),
                ));
            }
        }

        let mut nodes = BTreeMap::<RouteNodeRef, &RegimeNode>::new();
        let mut terminal_nodes = BTreeMap::<&str, RouteNodeRef>::new();
        let mut branches = BTreeMap::<&str, (&str, &RegimeBranch)>::new();
        let mut runs = BTreeMap::<&str, RunRecord<'_>>::new();
        let mut scaffold_by_layer = BTreeMap::<&str, &str>::new();
        let mut incidence = BTreeMap::<RouteNodeRef, Vec<&str>>::new();
        let mut expected_transitions =
            BTreeMap::<(String, String, String), (String, String, String, String)>::new();
        let mut graph_ids = BTreeSet::new();
        for (graph_index, graph) in self.route_graphs.iter().enumerate() {
            let path = format!("$.route_graphs[{graph_index}]");
            require_nonempty(
                &graph.electrical_net,
                format!("{path}.electrical_net"),
                &mut issues,
            );
            if !graph_ids.insert(graph.electrical_net.as_str()) {
                issues.push(CombinatorialRegimeIssue::new(
                    "duplicate_route_graph",
                    format!("{path}.electrical_net"),
                    "one embedded graph is required per electrical net",
                ));
            }
            let mut local_nodes = BTreeSet::new();
            for (node_index, node) in graph.nodes.iter().enumerate() {
                let node_path = format!("{path}.nodes[{node_index}]");
                require_nonempty(&node.id, format!("{node_path}.id"), &mut issues);
                if !local_nodes.insert(node.id.as_str()) {
                    issues.push(CombinatorialRegimeIssue::new(
                        "duplicate_route_node",
                        format!("{node_path}.id"),
                        "node IDs must be unique within an electrical net",
                    ));
                }
                let reference = RouteNodeRef {
                    electrical_net: graph.electrical_net.clone(),
                    node: node.id.clone(),
                };
                nodes.insert(reference.clone(), node);
                incidence.entry(reference.clone()).or_default();
                if let RegimeNodeKind::Terminal { logical_terminal } = &node.kind {
                    require_nonempty(
                        logical_terminal,
                        format!("{node_path}.logical_terminal"),
                        &mut issues,
                    );
                    if terminal_nodes
                        .insert(logical_terminal.as_str(), reference)
                        .is_some()
                    {
                        issues.push(CombinatorialRegimeIssue::new(
                            "duplicate_terminal_node",
                            format!("{node_path}.logical_terminal"),
                            "a logical terminal must occur once across all route graphs",
                        ));
                    }
                }
            }
            for (branch_index, branch) in graph.branches.iter().enumerate() {
                let branch_path = format!("{path}.branches[{branch_index}]");
                require_nonempty(&branch.id, format!("{branch_path}.id"), &mut issues);
                if branches
                    .insert(branch.id.as_str(), (&graph.electrical_net, branch))
                    .is_some()
                {
                    issues.push(CombinatorialRegimeIssue::new(
                        "duplicate_route_branch",
                        format!("{branch_path}.id"),
                        "branch IDs must be globally unique",
                    ));
                }
                for (field, node) in [
                    ("from_node", &branch.from_node),
                    ("to_node", &branch.to_node),
                ] {
                    if !local_nodes.contains(node.as_str()) {
                        issues.push(CombinatorialRegimeIssue::new(
                            "unknown_branch_node",
                            format!("{branch_path}.{field}"),
                            "branch endpoint references an unknown node in its electrical net",
                        ));
                    }
                }
                if branch.ordered_runs.is_empty() {
                    issues.push(CombinatorialRegimeIssue::new(
                        "empty_route_branch",
                        format!("{branch_path}.ordered_runs"),
                        "every branch must contain at least one layer run",
                    ));
                }
                for (run_index, run) in branch.ordered_runs.iter().enumerate() {
                    let run_path = format!("{branch_path}.ordered_runs[{run_index}]");
                    require_nonempty(&run.id, format!("{run_path}.id"), &mut issues);
                    require_nonempty(&run.layer, format!("{run_path}.layer"), &mut issues);
                    if let Some(obstacle_class) = &run.obstacle_class {
                        require_nonempty(
                            &obstacle_class.scaffold_fingerprint,
                            format!("{run_path}.obstacle_class.scaffold_fingerprint"),
                            &mut issues,
                        );
                        if !obstacle_class.signed_seam_word.is_reduced() {
                            issues.push(CombinatorialRegimeIssue::new(
                                "unreduced_run_seam_word",
                                format!("{run_path}.obstacle_class.signed_seam_word"),
                                "run seam words must be stored in reduced canonical form",
                            ));
                        }
                        if let Some(expected) = scaffold_by_layer.get(run.layer.as_str()) {
                            if *expected != obstacle_class.scaffold_fingerprint.as_str() {
                                issues.push(CombinatorialRegimeIssue::new(
                                    "inconsistent_layer_obstacle_scaffold",
                                    format!(
                                        "{run_path}.obstacle_class.scaffold_fingerprint"
                                    ),
                                    "all obstacle-relative runs on one layer must use the same coordinate-free scaffold",
                                ));
                            }
                        } else {
                            scaffold_by_layer.insert(
                                run.layer.as_str(),
                                obstacle_class.scaffold_fingerprint.as_str(),
                            );
                        }
                    }
                    for (field, node) in [("from_node", &run.from_node), ("to_node", &run.to_node)]
                    {
                        if !local_nodes.contains(node.as_str()) {
                            issues.push(CombinatorialRegimeIssue::new(
                                "unknown_run_node",
                                format!("{run_path}.{field}"),
                                "run endpoint references an unknown node in its electrical net",
                            ));
                        }
                        incidence
                            .entry(RouteNodeRef {
                                electrical_net: graph.electrical_net.clone(),
                                node: node.clone(),
                            })
                            .or_default()
                            .push(run.id.as_str());
                    }
                    if runs
                        .insert(
                            run.id.as_str(),
                            RunRecord {
                                electrical_net: &graph.electrical_net,
                                branch: &branch.id,
                                run,
                            },
                        )
                        .is_some()
                    {
                        issues.push(CombinatorialRegimeIssue::new(
                            "duplicate_route_run",
                            format!("{run_path}.id"),
                            "run IDs must be globally unique",
                        ));
                    }
                }
                if let Some(first) = branch.ordered_runs.first()
                    && first.from_node != branch.from_node
                {
                    issues.push(CombinatorialRegimeIssue::new(
                        "branch_run_endpoint_mismatch",
                        format!("{branch_path}.from_node"),
                        "first run must start at the branch source node",
                    ));
                }
                if let Some(last) = branch.ordered_runs.last()
                    && last.to_node != branch.to_node
                {
                    issues.push(CombinatorialRegimeIssue::new(
                        "branch_run_endpoint_mismatch",
                        format!("{branch_path}.to_node"),
                        "last run must end at the branch target node",
                    ));
                }
                for pair in branch.ordered_runs.windows(2) {
                    let (first, second) = (&pair[0], &pair[1]);
                    if first.to_node != second.from_node {
                        issues.push(CombinatorialRegimeIssue::new(
                            "discontinuous_route_runs",
                            format!("{branch_path}.ordered_runs"),
                            "adjacent directed runs must share one node",
                        ));
                    }
                    if first.layer == second.layer {
                        issues.push(CombinatorialRegimeIssue::new(
                            "nonmaximal_layer_run",
                            format!("{branch_path}.ordered_runs"),
                            "adjacent same-layer pieces must be one maximal run",
                        ));
                    } else {
                        expected_transitions.insert(
                            (branch.id.clone(), first.id.clone(), second.id.clone()),
                            (
                                graph.electrical_net.clone(),
                                first.to_node.clone(),
                                first.layer.clone(),
                                second.layer.clone(),
                            ),
                        );
                    }
                }
            }
        }

        for (terminal, node_ref) in &terminal_nodes {
            if !bindings.contains_key(terminal) {
                issues.push(CombinatorialRegimeIssue::new(
                    "unbound_terminal_node",
                    "$.terminal_bindings",
                    format!("terminal node {node_ref:?} has no selected pad/port binding"),
                ));
            }
        }
        for terminal in bindings.keys() {
            if !terminal_nodes.contains_key(terminal) {
                issues.push(CombinatorialRegimeIssue::new(
                    "binding_without_terminal_node",
                    "$.terminal_bindings",
                    format!("binding {terminal:?} is not represented by a route-graph terminal"),
                ));
            }
        }
        for (node_ref, node) in &nodes {
            let degree = incidence.get(node_ref).map_or(0, Vec::len);
            match &node.kind {
                RegimeNodeKind::Terminal { .. } if degree != 1 => {
                    issues.push(CombinatorialRegimeIssue::new(
                        "invalid_terminal_degree",
                        "$.route_graphs",
                        format!("terminal {node_ref:?} must have exactly one incident run"),
                    ))
                }
                RegimeNodeKind::Split { .. } if degree < 3 => {
                    issues.push(CombinatorialRegimeIssue::new(
                        "invalid_split_degree",
                        "$.route_graphs",
                        format!("movable split {node_ref:?} must have degree at least three"),
                    ))
                }
                RegimeNodeKind::Via if degree != 2 => issues.push(CombinatorialRegimeIssue::new(
                    "invalid_via_degree",
                    "$.route_graphs",
                    format!("via {node_ref:?} must join exactly two runs"),
                )),
                _ => {}
            }
        }

        let mut transition_ids = BTreeSet::new();
        let mut seen_transitions = BTreeSet::new();
        for (index, transition) in self.layer_transitions.iter().enumerate() {
            let path = format!("$.layer_transitions[{index}]");
            require_nonempty(&transition.id, format!("{path}.id"), &mut issues);
            if !transition_ids.insert(transition.id.as_str()) {
                issues.push(CombinatorialRegimeIssue::new(
                    "duplicate_layer_transition",
                    format!("{path}.id"),
                    "transition IDs must be unique",
                ));
            }
            let key = (
                transition.branch.clone(),
                transition.from_run.clone(),
                transition.to_run.clone(),
            );
            if !seen_transitions.insert(key.clone()) {
                issues.push(CombinatorialRegimeIssue::new(
                    "duplicate_layer_transition",
                    &path,
                    "one transition must certify each adjacent run pair exactly once",
                ));
            }
            match expected_transitions.get(&key) {
                Some((net, via, from_layer, to_layer))
                    if transition.electrical_net == net.as_str()
                        && transition.via_node == via.as_str()
                        && transition.from_layer == from_layer.as_str()
                        && transition.to_layer == to_layer.as_str() =>
                {
                    let node_ref = RouteNodeRef {
                        electrical_net: net.clone(),
                        node: via.clone(),
                    };
                    if !nodes
                        .get(&node_ref)
                        .is_some_and(|node| matches!(&node.kind, RegimeNodeKind::Via))
                    {
                        issues.push(CombinatorialRegimeIssue::new(
                            "transition_without_via_node",
                            format!("{path}.via_node"),
                            "a layer transition must occur at an explicit via node",
                        ));
                    }
                }
                _ => issues.push(CombinatorialRegimeIssue::new(
                    "invalid_layer_transition",
                    &path,
                    "transition must exactly match one adjacent directed run pair",
                )),
            }
        }
        for key in expected_transitions.keys() {
            if !seen_transitions.contains(key) {
                issues.push(CombinatorialRegimeIssue::new(
                    "missing_layer_transition",
                    "$.layer_transitions",
                    format!("adjacent layer runs {key:?} require an explicit via transition"),
                ));
            }
        }

        let mut layer_names = BTreeSet::new();
        let mut all_darts = BTreeMap::<&str, (&str, &RouteDart)>::new();
        let mut run_darts = BTreeMap::<&str, Vec<&RouteDart>>::new();
        for (layer_index, layer) in self.layer_maps.iter().enumerate() {
            let path = format!("$.layer_maps[{layer_index}]");
            require_nonempty(&layer.layer, format!("{path}.layer"), &mut issues);
            if !layer_names.insert(layer.layer.as_str()) {
                issues.push(CombinatorialRegimeIssue::new(
                    "duplicate_layer_map",
                    format!("{path}.layer"),
                    "one combinatorial map is required per selected layer",
                ));
            }
            for (dart_index, dart) in layer.darts.iter().enumerate() {
                let dart_path = format!("{path}.darts[{dart_index}]");
                require_nonempty(&dart.id, format!("{dart_path}.id"), &mut issues);
                require_nonempty(&dart.inverse, format!("{dart_path}.inverse"), &mut issues);
                if all_darts
                    .insert(dart.id.as_str(), (&layer.layer, dart))
                    .is_some()
                {
                    issues.push(CombinatorialRegimeIssue::new(
                        "duplicate_route_dart",
                        format!("{dart_path}.id"),
                        "dart IDs must be globally unique",
                    ));
                }
                let Some(record) = runs.get(dart.run.as_str()) else {
                    issues.push(CombinatorialRegimeIssue::new(
                        "unknown_dart_run",
                        format!("{dart_path}.run"),
                        "dart references an unknown layer run",
                    ));
                    continue;
                };
                if record.run.layer != layer.layer {
                    issues.push(CombinatorialRegimeIssue::new(
                        "dart_layer_mismatch",
                        &dart_path,
                        "dart and referenced run must occupy the same layer",
                    ));
                }
                run_darts.entry(dart.run.as_str()).or_default().push(dart);
                let endpoint = match dart.direction {
                    DartDirection::Forward => &record.run.from_node,
                    DartDirection::Reverse => &record.run.to_node,
                };
                let node_ref = RouteNodeRef {
                    electrical_net: record.electrical_net.into(),
                    node: endpoint.clone(),
                };
                let expected_origin = nodes.get(&node_ref).and_then(|node| match &node.kind {
                    RegimeNodeKind::Terminal { logical_terminal } => bindings
                        .get(logical_terminal.as_str())
                        .map(|binding| LayerVertexRef::ComponentBoundary {
                            component: binding.exposed_port.component().into(),
                        }),
                    _ => Some(LayerVertexRef::RouteNode {
                        node: node_ref.clone(),
                    }),
                });
                if expected_origin.as_ref() != Some(&dart.origin) {
                    issues.push(CombinatorialRegimeIssue::new(
                        "dart_origin_mismatch",
                        format!("{dart_path}.origin"),
                        "dart origin must be the run endpoint's route node or bound component boundary",
                    ));
                }
            }
        }
        for (id, (layer, dart)) in &all_darts {
            match all_darts.get(dart.inverse.as_str()) {
                Some((inverse_layer, inverse))
                    if inverse.inverse.as_str() == *id
                        && inverse.run == dart.run
                        && inverse.direction != dart.direction
                        && inverse_layer == layer => {}
                _ => issues.push(CombinatorialRegimeIssue::new(
                    "invalid_inverse_dart",
                    "$.layer_maps[*].darts",
                    format!("dart {id:?} does not have one mutual opposite-layer-run inverse"),
                )),
            }
        }
        for (run_id, record) in &runs {
            let darts = run_darts.get(run_id).map(Vec::as_slice).unwrap_or_default();
            if darts.len() != 2
                || !darts
                    .iter()
                    .any(|dart| dart.direction == DartDirection::Forward)
                || !darts
                    .iter()
                    .any(|dart| dart.direction == DartDirection::Reverse)
            {
                issues.push(CombinatorialRegimeIssue::new(
                    "incomplete_run_darts",
                    "$.layer_maps[*].darts",
                    format!(
                        "run {:?} on {:?}/{:?} needs exactly one forward and reverse dart",
                        run_id, record.electrical_net, record.branch
                    ),
                ));
            }
        }
        for transport in &self.breakout_face_transports {
            if !layer_names.contains(transport.layer.as_str()) {
                issues.push(CombinatorialRegimeIssue::new(
                    "unknown_breakout_layer",
                    "$.breakout_face_transports",
                    format!(
                        "breakout transport {:?} references an absent layer map",
                        transport.id
                    ),
                ));
            }
        }

        for (layer_index, layer) in self.layer_maps.iter().enumerate() {
            validate_layer_map(
                layer_index,
                layer,
                &all_darts,
                &runs,
                &nodes,
                &bindings,
                &cycles,
                &mut issues,
            );
        }

        let mut resolution_ids = BTreeSet::new();
        for (index, resolution) in self.crossing_resolutions.iter().enumerate() {
            let path = format!("$.crossing_resolutions[{index}]");
            require_nonempty(resolution.id(), format!("{path}.id"), &mut issues);
            if !resolution_ids.insert(resolution.id()) {
                issues.push(CombinatorialRegimeIssue::new(
                    "duplicate_crossing_resolution",
                    format!("{path}.id"),
                    "crossing-resolution IDs must be unique",
                ));
            }
            match resolution {
                CrossingResolution::SameNetJunction {
                    electrical_net,
                    node,
                    runs: joined,
                    ..
                } => {
                    let node_ref = RouteNodeRef {
                        electrical_net: electrical_net.clone(),
                        node: node.clone(),
                    };
                    if !nodes
                        .get(&node_ref)
                        .is_some_and(|node| matches!(&node.kind, RegimeNodeKind::Split { .. }))
                    {
                        issues.push(CombinatorialRegimeIssue::new(
                            "crossing_without_junction",
                            &path,
                            "same-layer contact is legal only at an explicit same-net split node",
                        ));
                    }
                    let mut unique = BTreeSet::new();
                    let mut selected_layer: Option<&str> = None;
                    if joined.len() < 2 {
                        issues.push(CombinatorialRegimeIssue::new(
                            "invalid_junction_resolution",
                            &path,
                            "a junction resolution must name at least two incident runs",
                        ));
                    }
                    for run_id in joined {
                        let valid = runs.get(run_id.as_str()).is_some_and(|record| {
                            let same_layer = selected_layer.map_or_else(
                                || {
                                    selected_layer = Some(record.run.layer.as_str());
                                    true
                                },
                                |layer| layer == record.run.layer,
                            );
                            record.electrical_net == electrical_net.as_str()
                                && (record.run.from_node == *node || record.run.to_node == *node)
                                && same_layer
                        });
                        if !unique.insert(run_id) || !valid {
                            issues.push(CombinatorialRegimeIssue::new(
                                "invalid_junction_resolution",
                                &path,
                                "junction runs must be unique, same-layer, same-net, and incident",
                            ));
                        }
                    }
                }
                CrossingResolution::SeparatedLayers {
                    first_run,
                    second_run,
                    ..
                } => match (runs.get(first_run.as_str()), runs.get(second_run.as_str())) {
                    (Some(first), Some(second))
                        if first_run != second_run && first.run.layer != second.run.layer => {}
                    _ => issues.push(CombinatorialRegimeIssue::new(
                        "invalid_layer_separation",
                        &path,
                        "separated runs must exist, be distinct, and occupy different layers",
                    )),
                },
            }
        }

        let expected_fingerprint = self.computed_fingerprint();
        if self.fingerprint != expected_fingerprint {
            issues.push(CombinatorialRegimeIssue::new(
                "fingerprint_mismatch",
                "$.fingerprint",
                "fingerprint must exactly match the canonical discrete regime",
            ));
        }
        issues
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CombinatorialRegimeIssue {
    pub code: &'static str,
    pub path: String,
    pub message: String,
}

impl CombinatorialRegimeIssue {
    fn new(code: &'static str, path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code,
            path: path.into(),
            message: message.into(),
        }
    }
}

#[derive(Debug)]
pub enum CombinatorialRegimeError {
    Json(serde_json::Error),
    Validation(Vec<CombinatorialRegimeIssue>),
}

impl fmt::Display for CombinatorialRegimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(error) => write!(formatter, "invalid combinatorial-regime JSON: {error}"),
            Self::Validation(issues) => write!(
                formatter,
                "invalid combinatorial routing regime ({} issues)",
                issues.len()
            ),
        }
    }
}

impl Error for CombinatorialRegimeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Json(error) => Some(error),
            Self::Validation(_) => None,
        }
    }
}

pub fn parse_and_validate_combinatorial_routing_regime(
    json: &str,
) -> Result<CombinatorialRoutingRegime, CombinatorialRegimeError> {
    let regime = serde_json::from_str::<CombinatorialRoutingRegime>(json)
        .map_err(CombinatorialRegimeError::Json)?;
    regime
        .validate()
        .map_err(CombinatorialRegimeError::Validation)?;
    Ok(regime)
}

fn require_nonempty(
    value: &str,
    path: impl Into<String>,
    issues: &mut Vec<CombinatorialRegimeIssue>,
) {
    if value.is_empty() {
        issues.push(CombinatorialRegimeIssue::new(
            "empty_identity",
            path,
            "stable identities must not be empty",
        ));
    }
}

fn validate_pad_ref(pad: &PhysicalPadRef, path: &str, issues: &mut Vec<CombinatorialRegimeIssue>) {
    require_nonempty(&pad.component, format!("{path}.component"), issues);
    require_nonempty(&pad.pin, format!("{path}.pin"), issues);
    require_nonempty(&pad.pad, format!("{path}.pad"), issues);
}

fn validate_breakout_ref(
    port: &BreakoutPortRef,
    path: &str,
    issues: &mut Vec<CombinatorialRegimeIssue>,
) {
    require_nonempty(&port.component, format!("{path}.component"), issues);
    require_nonempty(&port.interface, format!("{path}.interface"), issues);
    require_nonempty(&port.port, format!("{path}.port"), issues);
}

fn validate_port_ref(
    port: &ComponentPortRef,
    path: &str,
    issues: &mut Vec<CombinatorialRegimeIssue>,
) {
    match port {
        ComponentPortRef::PhysicalPad { pad } => validate_pad_ref(pad, path, issues),
        ComponentPortRef::BreakoutPort { port } => validate_breakout_ref(port, path, issues),
    }
}

fn normalized_cycle<T: Clone + Ord>(values: &[T]) -> Vec<T> {
    let mut result = values.to_vec();
    normalize_cycle(&mut result);
    result
}

fn validate_layer_map<'a>(
    layer_index: usize,
    layer: &'a LayerCombinatorialMap,
    all_darts: &BTreeMap<&'a str, (&'a str, &'a RouteDart)>,
    runs: &BTreeMap<&'a str, RunRecord<'a>>,
    nodes: &BTreeMap<RouteNodeRef, &'a RegimeNode>,
    bindings: &BTreeMap<&'a str, &'a TerminalBinding>,
    cycles: &BTreeMap<&'a str, &'a ComponentPortCycle>,
    issues: &mut Vec<CombinatorialRegimeIssue>,
) {
    let path = format!("$.layer_maps[{layer_index}]");
    let local_darts: BTreeMap<_, _> = layer
        .darts
        .iter()
        .map(|dart| (dart.id.as_str(), dart))
        .collect();
    let mut rotations = BTreeMap::<&LayerVertexRef, &VertexRotation>::new();
    let mut rotated_darts = BTreeSet::new();
    let mut sigma = BTreeMap::<&str, &str>::new();
    for (index, rotation) in layer.rotations.iter().enumerate() {
        let rotation_path = format!("{path}.rotations[{index}]");
        if rotation.clockwise_darts.is_empty() {
            issues.push(CombinatorialRegimeIssue::new(
                "empty_vertex_rotation",
                format!("{rotation_path}.clockwise_darts"),
                "every declared map vertex must own at least one dart",
            ));
        }
        if rotations.insert(&rotation.vertex, rotation).is_some() {
            issues.push(CombinatorialRegimeIssue::new(
                "duplicate_vertex_rotation",
                format!("{rotation_path}.vertex"),
                "one cyclic rotation is required per map vertex",
            ));
        }
        for (dart_index, dart_id) in rotation.clockwise_darts.iter().enumerate() {
            let Some(dart) = local_darts.get(dart_id.as_str()) else {
                issues.push(CombinatorialRegimeIssue::new(
                    "unknown_rotation_dart",
                    format!("{rotation_path}.clockwise_darts[{dart_index}]"),
                    "rotation references a dart outside this layer map",
                ));
                continue;
            };
            if dart.origin != rotation.vertex {
                issues.push(CombinatorialRegimeIssue::new(
                    "rotation_origin_mismatch",
                    format!("{rotation_path}.clockwise_darts[{dart_index}]"),
                    "every dart in a rotation must originate at that vertex",
                ));
            }
            if !rotated_darts.insert(dart_id.as_str()) {
                issues.push(CombinatorialRegimeIssue::new(
                    "duplicate_rotation_dart",
                    &rotation_path,
                    "each dart must occur in exactly one vertex rotation",
                ));
            }
            if !rotation.clockwise_darts.is_empty() {
                sigma.insert(
                    dart_id,
                    &rotation.clockwise_darts[(dart_index + 1) % rotation.clockwise_darts.len()],
                );
            }
        }
    }
    for dart in &layer.darts {
        if !rotated_darts.contains(dart.id.as_str()) {
            issues.push(CombinatorialRegimeIssue::new(
                "missing_rotation_dart",
                format!("{path}.rotations"),
                format!("dart {:?} is absent from all vertex rotations", dart.id),
            ));
        }
    }

    for rotation in &layer.rotations {
        let LayerVertexRef::ComponentBoundary { component } = &rotation.vertex else {
            continue;
        };
        let Some(cycle) = cycles.get(component.as_str()) else {
            issues.push(CombinatorialRegimeIssue::new(
                "missing_component_port_cycle",
                format!("{path}.rotations"),
                format!("component boundary {component:?} has no selected port cycle"),
            ));
            continue;
        };
        let actual_ports: Vec<_> = rotation
            .clockwise_darts
            .iter()
            .filter_map(|dart_id| {
                let dart = local_darts.get(dart_id.as_str())?;
                let run = runs.get(dart.run.as_str())?;
                let endpoint = match dart.direction {
                    DartDirection::Forward => &run.run.from_node,
                    DartDirection::Reverse => &run.run.to_node,
                };
                let node = nodes.get(&RouteNodeRef {
                    electrical_net: run.electrical_net.into(),
                    node: endpoint.clone(),
                })?;
                let RegimeNodeKind::Terminal { logical_terminal } = &node.kind else {
                    return None;
                };
                bindings
                    .get(logical_terminal.as_str())
                    .map(|binding| binding.exposed_port.clone())
            })
            .collect();
        let active: BTreeSet<_> = actual_ports.iter().cloned().collect();
        let expected_ports: Vec<_> = cycle
            .clockwise_ports
            .iter()
            .filter(|port| active.contains(*port))
            .cloned()
            .collect();
        if actual_ports.len() != rotation.clockwise_darts.len()
            || normalized_cycle(&actual_ports) != normalized_cycle(&expected_ports)
        {
            issues.push(CombinatorialRegimeIssue::new(
                "component_port_rotation_mismatch",
                format!("{path}.rotations"),
                "component-boundary dart rotation must preserve the selected clockwise port cycle",
            ));
        }
    }
    if rotated_darts.len() != layer.darts.len() || sigma.len() != layer.darts.len() {
        return;
    }

    let mut face_next = BTreeMap::<&str, &str>::new();
    for dart in &layer.darts {
        let Some((inverse_layer, inverse)) = all_darts.get(dart.inverse.as_str()) else {
            return;
        };
        if *inverse_layer != layer.layer {
            return;
        }
        let Some(next) = sigma.get(inverse.id.as_str()) else {
            return;
        };
        face_next.insert(dart.id.as_str(), *next);
    }
    let mut unseen: BTreeSet<_> = local_darts.keys().copied().collect();
    let mut derived_faces = Vec::<Vec<String>>::new();
    while let Some(start) = unseen.iter().next().copied() {
        let mut current = start;
        let mut face = Vec::new();
        loop {
            if !unseen.remove(current) {
                if current != start {
                    issues.push(CombinatorialRegimeIssue::new(
                        "invalid_face_permutation",
                        format!("{path}.rotations"),
                        "rotation/inverse face walk merged into another cycle",
                    ));
                }
                break;
            }
            face.push(current.to_owned());
            current = face_next[current];
        }
        normalize_cycle(&mut face);
        derived_faces.push(face);
    }
    derived_faces.sort();

    let mut face_ids = BTreeSet::new();
    let mut declared_faces = Vec::new();
    let mut faces_by_id = BTreeMap::new();
    for (index, face) in layer.faces.iter().enumerate() {
        let face_path = format!("{path}.faces[{index}]");
        require_nonempty(&face.id, format!("{face_path}.id"), issues);
        if !face_ids.insert(face.id.as_str()) {
            issues.push(CombinatorialRegimeIssue::new(
                "duplicate_face",
                format!("{face_path}.id"),
                "face IDs must be unique within a layer",
            ));
        }
        if face.boundary_darts.is_empty() {
            issues.push(CombinatorialRegimeIssue::new(
                "empty_face_boundary",
                format!("{face_path}.boundary_darts"),
                "declared faces must contain a derived dart cycle",
            ));
        }
        faces_by_id.insert(face.id.as_str(), face);
        declared_faces.push(normalized_cycle(&face.boundary_darts));
    }
    declared_faces.sort();
    if declared_faces != derived_faces {
        issues.push(CombinatorialRegimeIssue::new(
            "face_cycle_mismatch",
            format!("{path}.faces"),
            "declared faces must exactly equal cycles derived from inverse darts and rotations",
        ));
    }
    if !faces_by_id.contains_key(layer.outer_face.as_str()) {
        issues.push(CombinatorialRegimeIssue::new(
            "unknown_outer_face",
            format!("{path}.outer_face"),
            "the distinguished outer face must be one declared face",
        ));
    }

    // Euler characteristic is the explicit fail-closed same-layer crossing
    // check. A non-planar rotation system cannot be laundered by face labels.
    let vertices: BTreeSet<_> = layer.darts.iter().map(|dart| dart.origin.clone()).collect();
    let mut adjacency = BTreeMap::<LayerVertexRef, BTreeSet<LayerVertexRef>>::new();
    let mut undirected_edges = BTreeSet::new();
    for dart in &layer.darts {
        let inverse = local_darts[dart.inverse.as_str()];
        adjacency
            .entry(dart.origin.clone())
            .or_default()
            .insert(inverse.origin.clone());
        let mut pair = [dart.id.as_str(), inverse.id.as_str()];
        pair.sort();
        undirected_edges.insert((pair[0], pair[1]));
    }
    let mut remaining = vertices;
    while let Some(root) = remaining.iter().next().cloned() {
        let mut queue = VecDeque::from([root.clone()]);
        let mut component = BTreeSet::new();
        while let Some(vertex) = queue.pop_front() {
            if !component.insert(vertex.clone()) {
                continue;
            }
            remaining.remove(&vertex);
            for adjacent in adjacency.get(&vertex).into_iter().flatten() {
                if !component.contains(adjacent) {
                    queue.push_back(adjacent.clone());
                }
            }
        }
        let edge_count = undirected_edges
            .iter()
            .filter(|(dart, _)| component.contains(&local_darts[*dart].origin))
            .count();
        let face_count = derived_faces
            .iter()
            .filter(|face| {
                face.first()
                    .is_some_and(|dart| component.contains(&local_darts[dart.as_str()].origin))
            })
            .count();
        let characteristic = component.len() as isize - edge_count as isize + face_count as isize;
        if characteristic != 2 {
            issues.push(CombinatorialRegimeIssue::new(
                "nonplanar_same_layer_rotation",
                format!("{path}.rotations"),
                format!(
                    "same-layer rotation component has Euler characteristic {characteristic}, expected 2"
                ),
            ));
        }
    }

    let mut transport_ids = BTreeSet::new();
    for (index, transport) in layer.face_transports.iter().enumerate() {
        let transport_path = format!("{path}.face_transports[{index}]");
        require_nonempty(&transport.id, format!("{transport_path}.id"), issues);
        if !transport_ids.insert(transport.id.as_str()) {
            issues.push(CombinatorialRegimeIssue::new(
                "duplicate_face_transport",
                format!("{transport_path}.id"),
                "face transport IDs must be unique within a layer",
            ));
        }
        let Some(face) = faces_by_id.get(transport.face.as_str()) else {
            issues.push(CombinatorialRegimeIssue::new(
                "unknown_transport_face",
                format!("{transport_path}.face"),
                "transport references an unknown declared face",
            ));
            continue;
        };
        if transport.entrance_arc == transport.exit_arc
            || !face.boundary_darts.contains(&transport.entrance_arc)
            || !face.boundary_darts.contains(&transport.exit_arc)
        {
            issues.push(CombinatorialRegimeIssue::new(
                "invalid_transport_arcs",
                &transport_path,
                "distinct entrance and exit arcs must lie on the selected face boundary",
            ));
        }
        let mut ordered = BTreeSet::new();
        if transport.ordered_runs.len() < 2 {
            issues.push(CombinatorialRegimeIssue::new(
                "invalid_face_transport",
                format!("{transport_path}.ordered_runs"),
                "face transport is only meaningful for at least two ordered runs",
            ));
        }
        for run in &transport.ordered_runs {
            let touches_face = layer
                .darts
                .iter()
                .any(|dart| dart.run == *run && face.boundary_darts.contains(&dart.id));
            if !ordered.insert(run) || !touches_face {
                issues.push(CombinatorialRegimeIssue::new(
                    "invalid_face_transport_run",
                    &transport_path,
                    "transport runs must be unique and incident to the selected face",
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pad(component: &str, name: &str) -> PhysicalPadRef {
        PhysicalPadRef {
            component: component.into(),
            pin: name.into(),
            pad: format!("{name}:land"),
        }
    }

    fn pad_port(component: &str, name: &str) -> ComponentPortRef {
        ComponentPortRef::PhysicalPad {
            pad: pad(component, name),
        }
    }

    fn breakout(component: &str, name: &str) -> BreakoutPortRef {
        BreakoutPortRef {
            component: component.into(),
            interface: "west-escape".into(),
            port: name.into(),
        }
    }

    fn breakout_port(component: &str, name: &str) -> ComponentPortRef {
        ComponentPortRef::BreakoutPort {
            port: breakout(component, name),
        }
    }

    fn terminal(id: &str, logical_terminal: &str) -> RegimeNode {
        RegimeNode {
            id: id.into(),
            kind: RegimeNodeKind::Terminal {
                logical_terminal: logical_terminal.into(),
            },
        }
    }

    fn signed(mut regime: CombinatorialRoutingRegime) -> CombinatorialRoutingRegime {
        regime.fingerprint = regime.computed_fingerprint();
        regime
    }

    fn bga_regime() -> CombinatorialRoutingRegime {
        let route_graph = |suffix: &str, source: &str, target: &str| EmbeddedRouteGraph {
            electrical_net: format!("net-{suffix}"),
            nodes: vec![terminal("source", source), terminal("target", target)],
            branches: vec![RegimeBranch {
                id: format!("branch-{suffix}"),
                from_node: "source".into(),
                to_node: "target".into(),
                ordered_runs: vec![RegimeRun {
                    id: format!("run-{suffix}"),
                    layer: "top".into(),
                    from_node: "source".into(),
                    to_node: "target".into(),
                    obstacle_class: None,
                }],
            }],
        };
        let dart = |suffix: &str, direction: DartDirection, origin: LayerVertexRef| RouteDart {
            id: format!(
                "dart-{suffix}-{}",
                match direction {
                    DartDirection::Forward => "f",
                    DartDirection::Reverse => "r",
                }
            ),
            run: format!("run-{suffix}"),
            direction,
            origin,
            inverse: format!(
                "dart-{suffix}-{}",
                match direction {
                    DartDirection::Forward => "r",
                    DartDirection::Reverse => "f",
                }
            ),
        };
        let bga_vertex = LayerVertexRef::ComponentBoundary {
            component: "BGA".into(),
        };
        let leaf_b_vertex = LayerVertexRef::ComponentBoundary {
            component: "LEAF_B".into(),
        };
        let leaf_c_vertex = LayerVertexRef::ComponentBoundary {
            component: "LEAF_C".into(),
        };
        signed(CombinatorialRoutingRegime {
            schema_version: COMBINATORIAL_ROUTING_REGIME_SCHEMA_VERSION.into(),
            regime_id: "bga-top-lanes".into(),
            intent_fingerprint: "footprint:bga-stable-pads-v1".into(),
            terminal_bindings: vec![
                TerminalBinding {
                    logical_terminal: "B2".into(),
                    physical_pad: pad("BGA", "B2"),
                    exposed_port: breakout_port("BGA", "W_INNER"),
                },
                TerminalBinding {
                    logical_terminal: "C2".into(),
                    physical_pad: pad("BGA", "C2"),
                    exposed_port: breakout_port("BGA", "W_OUTER"),
                },
                TerminalBinding {
                    logical_terminal: "LEAF_B.P".into(),
                    physical_pad: pad("LEAF_B", "P"),
                    exposed_port: pad_port("LEAF_B", "P"),
                },
                TerminalBinding {
                    logical_terminal: "LEAF_C.P".into(),
                    physical_pad: pad("LEAF_C", "P"),
                    exposed_port: pad_port("LEAF_C", "P"),
                },
            ],
            component_port_cycles: vec![
                ComponentPortCycle {
                    component: "BGA".into(),
                    orientation_class: "normal".into(),
                    chirality: ComponentChirality::Normal,
                    clockwise_ports: vec![
                        breakout_port("BGA", "W_INNER"),
                        breakout_port("BGA", "W_OUTER"),
                    ],
                },
                ComponentPortCycle {
                    component: "LEAF_B".into(),
                    orientation_class: "normal".into(),
                    chirality: ComponentChirality::Normal,
                    clockwise_ports: vec![pad_port("LEAF_B", "P")],
                },
                ComponentPortCycle {
                    component: "LEAF_C".into(),
                    orientation_class: "normal".into(),
                    chirality: ComponentChirality::Normal,
                    clockwise_ports: vec![pad_port("LEAF_C", "P")],
                },
            ],
            breakout_face_transports: vec![BreakoutFaceTransport {
                id: "breakout:BGA:west:top".into(),
                component: "BGA".into(),
                interface: "west-escape".into(),
                layer: "top".into(),
                ordered_pads: vec![pad("BGA", "B2"), pad("BGA", "C2")],
                ordered_ports: vec![breakout("BGA", "W_INNER"), breakout("BGA", "W_OUTER")],
            }],
            route_graphs: vec![
                route_graph("b2", "B2", "LEAF_B.P"),
                route_graph("c2", "C2", "LEAF_C.P"),
            ],
            layer_maps: vec![LayerCombinatorialMap {
                layer: "top".into(),
                darts: vec![
                    dart("b2", DartDirection::Forward, bga_vertex.clone()),
                    dart("b2", DartDirection::Reverse, leaf_b_vertex.clone()),
                    dart("c2", DartDirection::Forward, bga_vertex.clone()),
                    dart("c2", DartDirection::Reverse, leaf_c_vertex.clone()),
                ],
                rotations: vec![
                    VertexRotation {
                        vertex: bga_vertex,
                        clockwise_darts: vec!["dart-b2-f".into(), "dart-c2-f".into()],
                    },
                    VertexRotation {
                        vertex: leaf_b_vertex,
                        clockwise_darts: vec!["dart-b2-r".into()],
                    },
                    VertexRotation {
                        vertex: leaf_c_vertex,
                        clockwise_darts: vec!["dart-c2-r".into()],
                    },
                ],
                faces: vec![RegimeFace {
                    id: "outside".into(),
                    boundary_darts: vec![
                        "dart-b2-f".into(),
                        "dart-b2-r".into(),
                        "dart-c2-f".into(),
                        "dart-c2-r".into(),
                    ],
                }],
                outer_face: "outside".into(),
                face_transports: vec![FaceTransport {
                    id: "west-lanes".into(),
                    face: "outside".into(),
                    entrance_arc: "dart-b2-f".into(),
                    exit_arc: "dart-c2-f".into(),
                    ordered_runs: vec!["run-b2".into(), "run-c2".into()],
                }],
            }],
            layer_transitions: Vec::new(),
            crossing_resolutions: Vec::new(),
            fingerprint: String::new(),
        })
    }

    fn via_regime() -> CombinatorialRoutingRegime {
        let a_pad = pad("A", "P");
        let b_pad = pad("B", "P");
        signed(CombinatorialRoutingRegime {
            schema_version: COMBINATORIAL_ROUTING_REGIME_SCHEMA_VERSION.into(),
            regime_id: "via".into(),
            intent_fingerprint: "intent-via".into(),
            terminal_bindings: vec![
                TerminalBinding {
                    logical_terminal: "A.P".into(),
                    physical_pad: a_pad.clone(),
                    exposed_port: ComponentPortRef::PhysicalPad { pad: a_pad },
                },
                TerminalBinding {
                    logical_terminal: "B.P".into(),
                    physical_pad: b_pad.clone(),
                    exposed_port: ComponentPortRef::PhysicalPad { pad: b_pad },
                },
            ],
            component_port_cycles: vec![
                ComponentPortCycle {
                    component: "A".into(),
                    orientation_class: "normal".into(),
                    chirality: ComponentChirality::Normal,
                    clockwise_ports: vec![pad_port("A", "P")],
                },
                ComponentPortCycle {
                    component: "B".into(),
                    orientation_class: "normal".into(),
                    chirality: ComponentChirality::Normal,
                    clockwise_ports: vec![pad_port("B", "P")],
                },
            ],
            breakout_face_transports: Vec::new(),
            route_graphs: vec![EmbeddedRouteGraph {
                electrical_net: "net".into(),
                nodes: vec![
                    terminal("a", "A.P"),
                    RegimeNode {
                        id: "v".into(),
                        kind: RegimeNodeKind::Via,
                    },
                    terminal("b", "B.P"),
                ],
                branches: vec![RegimeBranch {
                    id: "branch".into(),
                    from_node: "a".into(),
                    to_node: "b".into(),
                    ordered_runs: vec![
                        RegimeRun {
                            id: "top-run".into(),
                            layer: "top".into(),
                            from_node: "a".into(),
                            to_node: "v".into(),
                            obstacle_class: None,
                        },
                        RegimeRun {
                            id: "bottom-run".into(),
                            layer: "bottom".into(),
                            from_node: "v".into(),
                            to_node: "b".into(),
                            obstacle_class: None,
                        },
                    ],
                }],
            }],
            layer_maps: vec![
                one_run_layer(
                    "top",
                    "top-run",
                    LayerVertexRef::ComponentBoundary {
                        component: "A".into(),
                    },
                    LayerVertexRef::RouteNode {
                        node: RouteNodeRef {
                            electrical_net: "net".into(),
                            node: "v".into(),
                        },
                    },
                ),
                one_run_layer(
                    "bottom",
                    "bottom-run",
                    LayerVertexRef::RouteNode {
                        node: RouteNodeRef {
                            electrical_net: "net".into(),
                            node: "v".into(),
                        },
                    },
                    LayerVertexRef::ComponentBoundary {
                        component: "B".into(),
                    },
                ),
            ],
            layer_transitions: vec![LayerTransition {
                id: "transition:branch:0".into(),
                electrical_net: "net".into(),
                branch: "branch".into(),
                via_node: "v".into(),
                from_run: "top-run".into(),
                to_run: "bottom-run".into(),
                from_layer: "top".into(),
                to_layer: "bottom".into(),
            }],
            crossing_resolutions: vec![CrossingResolution::SeparatedLayers {
                id: "separation".into(),
                first_run: "top-run".into(),
                second_run: "bottom-run".into(),
            }],
            fingerprint: String::new(),
        })
    }

    fn one_run_layer(
        layer: &str,
        run: &str,
        from: LayerVertexRef,
        to: LayerVertexRef,
    ) -> LayerCombinatorialMap {
        let forward = format!("{run}:f");
        let reverse = format!("{run}:r");
        LayerCombinatorialMap {
            layer: layer.into(),
            darts: vec![
                RouteDart {
                    id: forward.clone(),
                    run: run.into(),
                    direction: DartDirection::Forward,
                    origin: from.clone(),
                    inverse: reverse.clone(),
                },
                RouteDart {
                    id: reverse.clone(),
                    run: run.into(),
                    direction: DartDirection::Reverse,
                    origin: to.clone(),
                    inverse: forward.clone(),
                },
            ],
            rotations: vec![
                VertexRotation {
                    vertex: from,
                    clockwise_darts: vec![forward.clone()],
                },
                VertexRotation {
                    vertex: to,
                    clockwise_darts: vec![reverse.clone()],
                },
            ],
            faces: vec![RegimeFace {
                id: format!("{layer}:outside"),
                boundary_darts: vec![forward, reverse],
            }],
            outer_face: format!("{layer}:outside"),
            face_transports: Vec::new(),
        }
    }

    fn twisted_three_route_regime() -> CombinatorialRoutingRegime {
        let names = ["a", "b", "c"];
        let mut bindings = Vec::new();
        let mut graphs = Vec::new();
        let mut darts = Vec::new();
        for name in names {
            let left_terminal = format!("LEFT.{name}");
            let right_terminal = format!("RIGHT.{name}");
            for (component, terminal_name) in [
                ("LEFT", left_terminal.as_str()),
                ("RIGHT", right_terminal.as_str()),
            ] {
                bindings.push(TerminalBinding {
                    logical_terminal: terminal_name.into(),
                    physical_pad: pad(component, name),
                    exposed_port: pad_port(component, name),
                });
            }
            graphs.push(EmbeddedRouteGraph {
                electrical_net: format!("net-{name}"),
                nodes: vec![
                    terminal("left", &left_terminal),
                    terminal("right", &right_terminal),
                ],
                branches: vec![RegimeBranch {
                    id: format!("branch-{name}"),
                    from_node: "left".into(),
                    to_node: "right".into(),
                    ordered_runs: vec![RegimeRun {
                        id: format!("run-{name}"),
                        layer: "top".into(),
                        from_node: "left".into(),
                        to_node: "right".into(),
                        obstacle_class: None,
                    }],
                }],
            });
            darts.push(RouteDart {
                id: format!("dart-{name}-f"),
                run: format!("run-{name}"),
                direction: DartDirection::Forward,
                origin: LayerVertexRef::ComponentBoundary {
                    component: "LEFT".into(),
                },
                inverse: format!("dart-{name}-r"),
            });
            darts.push(RouteDart {
                id: format!("dart-{name}-r"),
                run: format!("run-{name}"),
                direction: DartDirection::Reverse,
                origin: LayerVertexRef::ComponentBoundary {
                    component: "RIGHT".into(),
                },
                inverse: format!("dart-{name}-f"),
            });
        }
        signed(CombinatorialRoutingRegime {
            schema_version: COMBINATORIAL_ROUTING_REGIME_SCHEMA_VERSION.into(),
            regime_id: "three-route-twist".into(),
            intent_fingerprint: "parallel-terminals".into(),
            terminal_bindings: bindings,
            component_port_cycles: vec![
                ComponentPortCycle {
                    component: "LEFT".into(),
                    orientation_class: "normal".into(),
                    chirality: ComponentChirality::Normal,
                    clockwise_ports: names.iter().map(|name| pad_port("LEFT", name)).collect(),
                },
                ComponentPortCycle {
                    component: "RIGHT".into(),
                    orientation_class: "normal".into(),
                    chirality: ComponentChirality::Normal,
                    clockwise_ports: names.iter().map(|name| pad_port("RIGHT", name)).collect(),
                },
            ],
            breakout_face_transports: Vec::new(),
            route_graphs: graphs,
            layer_maps: vec![LayerCombinatorialMap {
                layer: "top".into(),
                darts,
                rotations: vec![
                    VertexRotation {
                        vertex: LayerVertexRef::ComponentBoundary {
                            component: "LEFT".into(),
                        },
                        clockwise_darts: names
                            .iter()
                            .map(|name| format!("dart-{name}-f"))
                            .collect(),
                    },
                    VertexRotation {
                        vertex: LayerVertexRef::ComponentBoundary {
                            component: "RIGHT".into(),
                        },
                        clockwise_darts: names
                            .iter()
                            .map(|name| format!("dart-{name}-r"))
                            .collect(),
                    },
                ],
                faces: vec![RegimeFace {
                    id: "twisted".into(),
                    boundary_darts: vec![
                        "dart-a-f".into(),
                        "dart-b-r".into(),
                        "dart-c-f".into(),
                        "dart-a-r".into(),
                        "dart-b-f".into(),
                        "dart-c-r".into(),
                    ],
                }],
                outer_face: "twisted".into(),
                face_transports: Vec::new(),
            }],
            layer_transitions: Vec::new(),
            crossing_resolutions: Vec::new(),
            fingerprint: String::new(),
        })
    }

    fn codes(regime: &CombinatorialRoutingRegime) -> BTreeSet<&'static str> {
        regime
            .validation_issues()
            .into_iter()
            .map(|issue| issue.code)
            .collect()
    }

    #[test]
    fn bga_breakout_keeps_physical_pads_and_boundary_lane_pairing_distinct() {
        let regime = bga_regime();
        regime.validate().unwrap();
        let original_pads: Vec<_> = regime
            .terminal_bindings
            .iter()
            .filter(|binding| binding.physical_pad.component == "BGA")
            .map(|binding| binding.physical_pad.clone())
            .collect();

        let mut swapped = regime.clone();
        let b2 = swapped
            .terminal_bindings
            .iter_mut()
            .find(|binding| binding.logical_terminal == "B2")
            .unwrap();
        b2.exposed_port = breakout_port("BGA", "W_OUTER");
        let c2 = swapped
            .terminal_bindings
            .iter_mut()
            .find(|binding| binding.logical_terminal == "C2")
            .unwrap();
        c2.exposed_port = breakout_port("BGA", "W_INNER");
        swapped.fingerprint = swapped.computed_fingerprint();
        let swapped_pads: Vec<_> = swapped
            .terminal_bindings
            .iter()
            .filter(|binding| binding.physical_pad.component == "BGA")
            .map(|binding| binding.physical_pad.clone())
            .collect();
        assert_eq!(original_pads, swapped_pads);
        assert_ne!(regime.fingerprint, swapped.fingerprint);
        assert!(codes(&swapped).contains("breakout_lane_binding_mismatch"));
    }

    #[test]
    fn one_layer_cannot_mix_obstacle_scaffolds() {
        let mut regime = bga_regime();
        for (graph, scaffold) in regime
            .route_graphs
            .iter_mut()
            .zip(["scaffold-a", "scaffold-b"])
        {
            graph.branches[0].ordered_runs[0].obstacle_class = Some(RunObstacleClass {
                scaffold_fingerprint: scaffold.into(),
                signed_seam_word: HomotopyWord::empty(),
            });
        }
        regime.fingerprint = regime.computed_fingerprint();
        assert!(
            codes(&regime).contains("inconsistent_layer_obstacle_scaffold"),
            "one layer needs one coordinate-free obstacle basis"
        );
    }

    #[test]
    fn canonical_fingerprint_ignores_record_permutations_and_cycle_origins() {
        let original = bga_regime();
        let mut permuted = original.clone();
        permuted.regime_id = "another-search-id".into();
        permuted.terminal_bindings.reverse();
        permuted.component_port_cycles.reverse();
        for graph in &mut permuted.route_graphs {
            graph.nodes.reverse();
        }
        permuted.route_graphs.reverse();
        permuted.layer_maps[0].darts.reverse();
        permuted.layer_maps[0].rotations.reverse();
        permuted.layer_maps[0].faces[0]
            .boundary_darts
            .rotate_left(2);
        permuted.layer_maps[0].rotations[2]
            .clockwise_darts
            .rotate_left(1);
        assert_eq!(original.fingerprint, permuted.computed_fingerprint());
        permuted.fingerprint = original.fingerprint.clone();
        permuted.validate().unwrap();
    }

    #[test]
    fn cyclic_and_linear_order_changes_have_distinct_semantic_behavior() {
        let original = bga_regime();
        let mut rotated_cycle = original.clone();
        rotated_cycle.component_port_cycles[0]
            .clockwise_ports
            .rotate_left(1);
        assert_eq!(original.fingerprint, rotated_cycle.computed_fingerprint());

        let mut reversed_transport = original.clone();
        reversed_transport.layer_maps[0].face_transports[0]
            .ordered_runs
            .reverse();
        assert_ne!(
            original.fingerprint,
            reversed_transport.computed_fingerprint()
        );
    }

    #[test]
    fn inverse_darts_and_terminal_coverage_fail_closed() {
        let mut stale_inverse = bga_regime();
        stale_inverse.layer_maps[0].darts[0].inverse = "missing".into();
        stale_inverse.fingerprint = stale_inverse.computed_fingerprint();
        assert!(codes(&stale_inverse).contains("invalid_inverse_dart"));

        let mut missing_terminal = bga_regime();
        missing_terminal
            .terminal_bindings
            .retain(|binding| binding.logical_terminal != "LEAF_C.P");
        missing_terminal.fingerprint = missing_terminal.computed_fingerprint();
        assert!(codes(&missing_terminal).contains("unbound_terminal_node"));
    }

    #[test]
    fn transition_continuity_and_explicit_via_are_validated() {
        let regime = via_regime();
        regime.validate().unwrap();

        let mut missing = regime.clone();
        missing.layer_transitions.clear();
        missing.fingerprint = missing.computed_fingerprint();
        assert!(codes(&missing).contains("missing_layer_transition"));

        let mut stale = regime;
        stale.layer_transitions[0].to_layer = "inner".into();
        stale.fingerprint = stale.computed_fingerprint();
        assert!(codes(&stale).contains("invalid_layer_transition"));
    }

    #[test]
    fn same_layer_twist_and_fake_layer_separation_are_rejected() {
        let twisted = twisted_three_route_regime();
        assert!(codes(&twisted).contains("nonplanar_same_layer_rotation"));

        let mut fake = bga_regime();
        fake.crossing_resolutions = vec![CrossingResolution::SeparatedLayers {
            id: "fake".into(),
            first_run: "run-b2".into(),
            second_run: "run-c2".into(),
        }];
        fake.fingerprint = fake.computed_fingerprint();
        assert!(codes(&fake).contains("invalid_layer_separation"));
    }

    #[test]
    fn parser_rejects_unknown_fields_and_stale_fingerprint() {
        let regime = bga_regime();
        let mut json = serde_json::to_value(&regime).unwrap();
        json["layer_maps"][0]["darts"][0]["coordinates"] = serde_json::json!([0, 0]);
        assert!(matches!(
            parse_and_validate_combinatorial_routing_regime(&json.to_string()).unwrap_err(),
            CombinatorialRegimeError::Json(_)
        ));

        let mut stale = regime;
        stale.intent_fingerprint.push_str("-changed");
        assert!(codes(&stale).contains("fingerprint_mismatch"));
    }
}
