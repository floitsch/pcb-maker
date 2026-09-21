//! Exact final reachability and attachment checks for serialized route graphs.
//!
//! This gate proves only the explicit graph represented by `SolvedTrace` and
//! `SolvedRouteGraph`. It deliberately does not infer connectivity from
//! same-net copper touching or crossing away from an explicit route node.

use std::collections::{BTreeMap, BTreeSet, VecDeque, btree_map::Entry};

use serde::Serialize;

use crate::candidate::{
    SolvedComponent, SolvedRouteGraph, SolvedRouteNodeKind, SolvedTrace, Violation,
};
use layout_trace_model::geometry::{EPSILON, add, length, rotate_degrees, sub};
use layout_trace_model::{Problem, Vec2};

pub const ELECTRICAL_CONNECTIVITY_ASSESSMENT_CONTRACT: &str =
    "layout-trace.electrical-connectivity-assessment/v1";

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct ElectricalTerminalEvidence {
    pub component: String,
    pub pin: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ElectricalConnectivityComponentEvidence {
    pub node_ids: Vec<String>,
    pub branch_ids: Vec<String>,
    pub terminals: Vec<ElectricalTerminalEvidence>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ElectricalNetConnectivityEvidence {
    pub electrical_net: String,
    pub declared_terminals: Vec<ElectricalTerminalEvidence>,
    pub serialized_terminals: Vec<ElectricalTerminalEvidence>,
    pub branches: Vec<String>,
    pub checked_endpoint_incidences: usize,
    pub connected_components: Vec<ElectricalConnectivityComponentEvidence>,
    pub connected: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ElectricalConnectivityFindingCode {
    DuplicateDeclaredNet,
    DuplicateSolvedComponent,
    EmptyStableIdentity,
    DuplicateTraceBranch,
    TraceAliasMismatch,
    UndeclaredTraceNet,
    InvalidTraceContinuity,
    MissingRouteGraph,
    ExtraRouteGraph,
    DuplicateRouteGraph,
    DuplicateGraphBranch,
    BranchSetMismatch,
    DuplicateNodeId,
    NodeNetMismatch,
    DuplicateNodeIncidence,
    UnknownIncidentBranch,
    CrossNetIncidentBranch,
    MissingDeclaredTerminal,
    ExtraTerminal,
    DuplicateTerminalNode,
    TerminalPoseMismatch,
    MissingEndpointNode,
    EndpointNodeKind,
    SelfLoopBranch,
    BranchIncidenceMismatch,
    TraceEndpointDetached,
    EndpointLayerMismatch,
    ViaNodeMismatch,
    DisconnectedDeclaredTerminals,
    DisconnectedIsland,
}

impl ElectricalConnectivityFindingCode {
    pub fn violation_code(self) -> &'static str {
        match self {
            Self::DuplicateDeclaredNet => "electrical_connectivity_duplicate_declared_net",
            Self::DuplicateSolvedComponent => "electrical_connectivity_duplicate_solved_component",
            Self::EmptyStableIdentity => "electrical_connectivity_empty_stable_identity",
            Self::DuplicateTraceBranch => "electrical_connectivity_duplicate_trace_branch",
            Self::TraceAliasMismatch => "electrical_connectivity_trace_alias_mismatch",
            Self::UndeclaredTraceNet => "electrical_connectivity_undeclared_trace_net",
            Self::InvalidTraceContinuity => "electrical_connectivity_invalid_trace_continuity",
            Self::MissingRouteGraph => "electrical_connectivity_missing_route_graph",
            Self::ExtraRouteGraph => "electrical_connectivity_extra_route_graph",
            Self::DuplicateRouteGraph => "electrical_connectivity_duplicate_route_graph",
            Self::DuplicateGraphBranch => "electrical_connectivity_duplicate_graph_branch",
            Self::BranchSetMismatch => "electrical_connectivity_branch_set_mismatch",
            Self::DuplicateNodeId => "electrical_connectivity_duplicate_node_id",
            Self::NodeNetMismatch => "electrical_connectivity_node_net_mismatch",
            Self::DuplicateNodeIncidence => "electrical_connectivity_duplicate_node_incidence",
            Self::UnknownIncidentBranch => "electrical_connectivity_unknown_incident_branch",
            Self::CrossNetIncidentBranch => "electrical_connectivity_cross_net_incident_branch",
            Self::MissingDeclaredTerminal => "electrical_connectivity_missing_declared_terminal",
            Self::ExtraTerminal => "electrical_connectivity_extra_terminal",
            Self::DuplicateTerminalNode => "electrical_connectivity_duplicate_terminal_node",
            Self::TerminalPoseMismatch => "electrical_connectivity_terminal_pose_mismatch",
            Self::MissingEndpointNode => "electrical_connectivity_missing_endpoint_node",
            Self::EndpointNodeKind => "electrical_connectivity_endpoint_node_kind",
            Self::SelfLoopBranch => "electrical_connectivity_self_loop_branch",
            Self::BranchIncidenceMismatch => "electrical_connectivity_branch_incidence_mismatch",
            Self::TraceEndpointDetached => "electrical_connectivity_trace_endpoint_detached",
            Self::EndpointLayerMismatch => "electrical_connectivity_endpoint_layer_mismatch",
            Self::ViaNodeMismatch => "electrical_connectivity_via_node_mismatch",
            Self::DisconnectedDeclaredTerminals => {
                "electrical_connectivity_disconnected_declared_terminals"
            }
            Self::DisconnectedIsland => "electrical_connectivity_disconnected_island",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ElectricalConnectivityFinding {
    pub code: ElectricalConnectivityFindingCode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub electrical_net: Option<String>,
    pub objects: Vec<String>,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ElectricalConnectivityAssessment {
    pub contract: String,
    pub complete: bool,
    pub declared_net_count: usize,
    pub checked_branch_count: usize,
    pub checked_endpoint_incidences: usize,
    pub nets: Vec<ElectricalNetConnectivityEvidence>,
    pub findings: Vec<ElectricalConnectivityFinding>,
}

impl ElectricalConnectivityAssessment {
    pub fn violations(&self) -> Vec<Violation> {
        self.findings
            .iter()
            .map(|finding| Violation {
                code: finding.code.violation_code().into(),
                message: finding.detail.clone(),
                objects: finding.objects.clone(),
                required_distance: 0.0,
                actual_distance: 0.0,
            })
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ViaKey {
    branch: String,
    point_index: usize,
    x: u64,
    y: u64,
    from_layer: String,
    to_layer: String,
    diameter: u64,
    drill: u64,
}

fn terminal(component: &str, pin: &str) -> ElectricalTerminalEvidence {
    ElectricalTerminalEvidence {
        component: component.to_owned(),
        pin: pin.to_owned(),
    }
}

fn finding(
    findings: &mut Vec<ElectricalConnectivityFinding>,
    code: ElectricalConnectivityFindingCode,
    electrical_net: Option<&str>,
    mut objects: Vec<String>,
    detail: impl Into<String>,
) {
    objects.sort();
    objects.dedup();
    findings.push(ElectricalConnectivityFinding {
        code,
        electrical_net: electrical_net.map(str::to_owned),
        objects,
        detail: detail.into(),
    });
}

fn point_matches(first: Vec2, second: Vec2) -> bool {
    length(sub(first, second)) <= EPSILON
}

fn trace_continuity_error(problem: &Problem, trace: &SolvedTrace) -> Option<String> {
    if !trace.width.is_finite() || trace.width <= 0.0 {
        return Some("trace width must be finite and positive".into());
    }
    if trace.points.len() < 2 {
        return Some("trace needs at least two points".into());
    }
    if trace.segment_layers.len() != trace.points.len() - 1 {
        return Some(format!(
            "trace has {} segments but {} segment layers",
            trace.points.len() - 1,
            trace.segment_layers.len()
        ));
    }
    if trace
        .points
        .iter()
        .any(|point| !point.x.is_finite() || !point.y.is_finite())
    {
        return Some("trace contains a non-finite point".into());
    }
    let board_layers = problem
        .board
        .layers
        .iter()
        .map(|layer| layer.id.as_str())
        .collect::<BTreeSet<_>>();
    if trace
        .segment_layers
        .iter()
        .any(|layer| !board_layers.contains(layer.as_str()))
    {
        return Some("trace segment references an undeclared copper layer".into());
    }
    let mut vias = BTreeMap::new();
    for via in &trace.vias {
        if !via.position.x.is_finite()
            || !via.position.y.is_finite()
            || !via.diameter.is_finite()
            || !via.drill.is_finite()
            || via.diameter <= 0.0
            || via.drill <= 0.0
            || via.drill > via.diameter
        {
            return Some(format!(
                "via at point {} has invalid geometry",
                via.point_index
            ));
        }
        if !board_layers.contains(via.from_layer.as_str())
            || !board_layers.contains(via.to_layer.as_str())
        {
            return Some(format!(
                "via at point {} references an undeclared copper layer",
                via.point_index
            ));
        }
        if via.point_index >= trace.points.len() {
            return Some(format!("via references missing point {}", via.point_index));
        }
        if vias.insert(via.point_index, via).is_some() {
            return Some(format!("multiple vias occur at point {}", via.point_index));
        }
        if via.from_layer == via.to_layer {
            return Some(format!(
                "via at point {} does not change layer",
                via.point_index
            ));
        }
        if !point_matches(via.position, trace.points[via.point_index]) {
            return Some(format!("via at point {} is detached", via.point_index));
        }
    }
    for point_index in 1..trace.points.len() - 1 {
        let before = &trace.segment_layers[point_index - 1];
        let after = &trace.segment_layers[point_index];
        let via = vias.get(&point_index).copied();
        if before == after {
            if via.is_some() {
                return Some(format!(
                    "unmatched internal via occurs at point {point_index} on layer {before}"
                ));
            }
        } else {
            let Some(via) = via else {
                return Some(format!(
                    "layer changes {before} -> {after} at point {point_index} without a via"
                ));
            };
            if !((via.from_layer == *before && via.to_layer == *after)
                || (via.from_layer == *after && via.to_layer == *before))
            {
                return Some(format!(
                    "via at point {point_index} does not match layer change {before} -> {after}"
                ));
            }
        }
    }
    for point_index in [0, trace.points.len() - 1] {
        if let Some(via) = vias.get(&point_index) {
            let incident = if point_index == 0 {
                &trace.segment_layers[0]
            } else {
                trace.segment_layers.last().unwrap()
            };
            if via.from_layer != *incident && via.to_layer != *incident {
                return Some(format!(
                    "endpoint via at point {point_index} does not involve incident layer {incident}"
                ));
            }
        }
    }
    None
}

fn endpoint_layer_matches(
    problem: &Problem,
    trace: &SolvedTrace,
    node: &crate::candidate::SolvedRouteNode,
    from: bool,
) -> bool {
    let point_index = if from { 0 } else { trace.points.len() - 1 };
    let Some(route_layer) = (if from {
        trace.segment_layers.first()
    } else {
        trace.segment_layers.last()
    }) else {
        return false;
    };
    let endpoint_via = trace.vias.iter().find(|via| via.point_index == point_index);
    let accepts = |node_layer: &str| {
        node_layer == route_layer.as_str()
            || endpoint_via.is_some_and(|via| {
                (via.from_layer.as_str() == route_layer.as_str()
                    && via.to_layer.as_str() == node_layer)
                    || (via.to_layer.as_str() == route_layer.as_str()
                        && via.from_layer.as_str() == node_layer)
            })
    };
    match &node.kind {
        SolvedRouteNodeKind::Terminal { component, pin } => problem
            .components
            .iter()
            .find(|candidate| candidate.id == *component)
            .and_then(|component| component.pins.iter().find(|candidate| candidate.id == *pin))
            .is_some_and(|pin| {
                pin.pads.is_empty() || pin.pads.iter().any(|pad| accepts(&pad.layer))
            }),
        SolvedRouteNodeKind::Junction { layer, .. } => accepts(layer),
        SolvedRouteNodeKind::Via { .. } => false,
    }
}

fn declared_nets(
    problem: &Problem,
    findings: &mut Vec<ElectricalConnectivityFinding>,
) -> BTreeMap<String, BTreeSet<ElectricalTerminalEvidence>> {
    let mut declared = BTreeMap::<String, BTreeSet<_>>::new();
    let mut modern = BTreeSet::new();
    for net in &problem.electrical_nets {
        if !modern.insert(net.id.as_str()) || declared.contains_key(&net.id) {
            finding(
                findings,
                ElectricalConnectivityFindingCode::DuplicateDeclaredNet,
                Some(&net.id),
                vec![net.id.clone()],
                "electrical-net identity is declared more than once",
            );
        }
        let terminals = declared.entry(net.id.clone()).or_default();
        terminals.extend(
            net.terminals
                .iter()
                .map(|item| terminal(&item.component, &item.pin)),
        );
    }
    for branch in &problem.nets {
        let net = branch.electrical_net.as_deref().unwrap_or(&branch.id);
        if modern.contains(net) {
            finding(
                findings,
                ElectricalConnectivityFindingCode::DuplicateDeclaredNet,
                Some(net),
                vec![net.to_owned(), branch.id.clone()],
                "legacy branch collides with a modern electrical-net declaration",
            );
        }
        let terminals = declared.entry(net.to_owned()).or_default();
        terminals.insert(terminal(&branch.from.component, &branch.from.pin));
        terminals.insert(terminal(&branch.to.component, &branch.to.pin));
    }
    declared
}

fn via_key(
    branch: &str,
    point_index: usize,
    position: Vec2,
    from_layer: &str,
    to_layer: &str,
    diameter: f64,
    drill: f64,
) -> ViaKey {
    ViaKey {
        branch: branch.to_owned(),
        point_index,
        x: position.x.to_bits(),
        y: position.y.to_bits(),
        from_layer: from_layer.to_owned(),
        to_layer: to_layer.to_owned(),
        diameter: diameter.to_bits(),
        drill: drill.to_bits(),
    }
}

/// Validate the durable solved graph rather than trusting runtime-only node
/// indices. Input order never affects the returned evidence or findings.
pub fn assess_electrical_connectivity(
    problem: &Problem,
    components: &[SolvedComponent],
    traces: &[SolvedTrace],
    route_graphs: &[SolvedRouteGraph],
) -> ElectricalConnectivityAssessment {
    let mut findings = Vec::new();
    let declared = declared_nets(problem, &mut findings);

    let mut solved_components = BTreeMap::<&str, Option<&SolvedComponent>>::new();
    for component in components {
        match solved_components.entry(component.id.as_str()) {
            Entry::Vacant(entry) => {
                entry.insert(Some(component));
            }
            Entry::Occupied(mut entry) => {
                entry.insert(None);
                finding(
                    &mut findings,
                    ElectricalConnectivityFindingCode::DuplicateSolvedComponent,
                    None,
                    vec![component.id.clone()],
                    "solved component identity is duplicated",
                );
            }
        }
    }

    let mut trace_map = BTreeMap::<&str, Option<&SolvedTrace>>::new();
    let mut trace_valid = BTreeMap::<&str, bool>::new();
    for trace in traces {
        let identity_invalid =
            trace.branch.is_empty() || trace.from_node.is_empty() || trace.to_node.is_empty();
        if identity_invalid {
            finding(
                &mut findings,
                ElectricalConnectivityFindingCode::EmptyStableIdentity,
                Some(&trace.electrical_net),
                vec![
                    trace.branch.clone(),
                    trace.from_node.clone(),
                    trace.to_node.clone(),
                ],
                "trace branch and explicit endpoint-node identities must be non-empty",
            );
        }
        let duplicate = match trace_map.entry(trace.branch.as_str()) {
            Entry::Vacant(entry) => {
                entry.insert(Some(trace));
                false
            }
            Entry::Occupied(mut entry) => {
                entry.insert(None);
                finding(
                    &mut findings,
                    ElectricalConnectivityFindingCode::DuplicateTraceBranch,
                    Some(&trace.electrical_net),
                    vec![trace.branch.clone()],
                    "serialized trace branch identity is duplicated",
                );
                true
            }
        };
        if duplicate {
            trace_valid.insert(trace.branch.as_str(), false);
        }
        if trace.net != trace.branch {
            finding(
                &mut findings,
                ElectricalConnectivityFindingCode::TraceAliasMismatch,
                Some(&trace.electrical_net),
                vec![trace.branch.clone(), trace.net.clone()],
                "legacy trace alias differs from stable branch identity",
            );
        }
        if !declared.contains_key(&trace.electrical_net) {
            finding(
                &mut findings,
                ElectricalConnectivityFindingCode::UndeclaredTraceNet,
                Some(&trace.electrical_net),
                vec![trace.branch.clone()],
                "trace belongs to an undeclared electrical net",
            );
        }
        let error = trace_continuity_error(problem, trace);
        if let Some(detail) = &error {
            finding(
                &mut findings,
                ElectricalConnectivityFindingCode::InvalidTraceContinuity,
                Some(&trace.electrical_net),
                vec![trace.branch.clone()],
                detail.clone(),
            );
        }
        if !duplicate {
            trace_valid.insert(trace.branch.as_str(), error.is_none() && !identity_invalid);
        }
    }

    let mut graphs = BTreeMap::<&str, Option<&SolvedRouteGraph>>::new();
    for graph in route_graphs {
        match graphs.entry(graph.electrical_net.as_str()) {
            Entry::Vacant(entry) => {
                entry.insert(Some(graph));
            }
            Entry::Occupied(mut entry) => {
                entry.insert(None);
                finding(
                    &mut findings,
                    ElectricalConnectivityFindingCode::DuplicateRouteGraph,
                    Some(&graph.electrical_net),
                    vec![graph.electrical_net.clone()],
                    "serialized electrical net has more than one route graph",
                );
            }
        }
        if !declared.contains_key(&graph.electrical_net) {
            finding(
                &mut findings,
                ElectricalConnectivityFindingCode::ExtraRouteGraph,
                Some(&graph.electrical_net),
                vec![graph.electrical_net.clone()],
                "route graph belongs to an undeclared electrical net",
            );
        }
    }

    let problem_components = problem
        .components
        .iter()
        .map(|component| (component.id.as_str(), component))
        .collect::<BTreeMap<_, _>>();
    let mut net_evidence = Vec::new();
    let mut total_endpoint_incidences = 0;

    for (net, declared_terminals) in &declared {
        let expected_branches = traces
            .iter()
            .filter(|trace| trace.electrical_net == *net)
            .map(|trace| trace.branch.clone())
            .collect::<BTreeSet<_>>();
        let Some(graph) = graphs.get(net.as_str()).and_then(|graph| *graph) else {
            finding(
                &mut findings,
                ElectricalConnectivityFindingCode::MissingRouteGraph,
                Some(net),
                vec![net.clone()],
                "declared electrical net has no serialized route graph",
            );
            net_evidence.push(ElectricalNetConnectivityEvidence {
                electrical_net: net.clone(),
                declared_terminals: declared_terminals.iter().cloned().collect(),
                serialized_terminals: Vec::new(),
                branches: expected_branches.into_iter().collect(),
                checked_endpoint_incidences: 0,
                connected_components: Vec::new(),
                connected: false,
            });
            continue;
        };

        let mut graph_branches = BTreeSet::new();
        for branch in &graph.branches {
            if !graph_branches.insert(branch.clone()) {
                finding(
                    &mut findings,
                    ElectricalConnectivityFindingCode::DuplicateGraphBranch,
                    Some(net),
                    vec![branch.clone()],
                    "route graph branch list contains a duplicate",
                );
            }
        }
        if graph_branches != expected_branches {
            let missing = expected_branches
                .difference(&graph_branches)
                .cloned()
                .collect::<Vec<_>>();
            let extra = graph_branches
                .difference(&expected_branches)
                .cloned()
                .collect::<Vec<_>>();
            let mut objects = missing.clone();
            objects.extend(extra.clone());
            finding(
                &mut findings,
                ElectricalConnectivityFindingCode::BranchSetMismatch,
                Some(net),
                objects,
                format!("route graph branch set mismatch; missing={missing:?}, extra={extra:?}"),
            );
        }

        let mut nodes = BTreeMap::<&str, Option<&crate::candidate::SolvedRouteNode>>::new();
        let mut terminal_nodes = BTreeMap::<ElectricalTerminalEvidence, Vec<&str>>::new();
        let mut endpoint_incidence = BTreeMap::<&str, BTreeSet<&str>>::new();
        let mut actual_vias = BTreeMap::<ViaKey, usize>::new();
        for node in &graph.nodes {
            if node.id.is_empty() {
                finding(
                    &mut findings,
                    ElectricalConnectivityFindingCode::EmptyStableIdentity,
                    Some(net),
                    vec![node.id.clone()],
                    "route-node stable identity must be non-empty",
                );
            }
            match nodes.entry(node.id.as_str()) {
                Entry::Vacant(entry) => {
                    entry.insert(Some(node));
                }
                Entry::Occupied(mut entry) => {
                    entry.insert(None);
                    finding(
                        &mut findings,
                        ElectricalConnectivityFindingCode::DuplicateNodeId,
                        Some(net),
                        vec![node.id.clone()],
                        "route graph node identity is duplicated",
                    );
                }
            }
            if node.electrical_net != *net {
                finding(
                    &mut findings,
                    ElectricalConnectivityFindingCode::NodeNetMismatch,
                    Some(net),
                    vec![node.id.clone(), node.electrical_net.clone()],
                    "route node electrical identity differs from its graph",
                );
            }
            let mut incident = BTreeSet::new();
            for branch in &node.incident_branches {
                if !incident.insert(branch.as_str()) {
                    finding(
                        &mut findings,
                        ElectricalConnectivityFindingCode::DuplicateNodeIncidence,
                        Some(net),
                        vec![node.id.clone(), branch.clone()],
                        "node lists the same incident branch more than once",
                    );
                }
                match trace_map.get(branch.as_str()).and_then(|trace| *trace) {
                    None => finding(
                        &mut findings,
                        ElectricalConnectivityFindingCode::UnknownIncidentBranch,
                        Some(net),
                        vec![node.id.clone(), branch.clone()],
                        "node references a missing trace branch",
                    ),
                    Some(trace) if trace.electrical_net != *net => finding(
                        &mut findings,
                        ElectricalConnectivityFindingCode::CrossNetIncidentBranch,
                        Some(net),
                        vec![
                            node.id.clone(),
                            branch.clone(),
                            trace.electrical_net.clone(),
                        ],
                        "node references a trace from another electrical net",
                    ),
                    Some(_) => {}
                }
            }
            match &node.kind {
                SolvedRouteNodeKind::Terminal { component, pin } => {
                    let key = terminal(component, pin);
                    terminal_nodes
                        .entry(key.clone())
                        .or_default()
                        .push(&node.id);
                    let pose_matches = if let (Some(problem_component), Some(solved_component)) = (
                        problem_components.get(component.as_str()),
                        solved_components
                            .get(component.as_str())
                            .and_then(|component| *component),
                    ) {
                        if let Some(problem_pin) = problem_component
                            .pins
                            .iter()
                            .find(|candidate| candidate.id == *pin)
                        {
                            let local_centers = if problem_pin.pads.is_empty() {
                                vec![problem_pin.offset]
                            } else {
                                problem_pin
                                    .pads
                                    .iter()
                                    .map(|pad| problem_pin.pad_local_center(pad))
                                    .collect()
                            };
                            local_centers.into_iter().any(|local_center| {
                                let expected = add(
                                    solved_component.position,
                                    rotate_degrees(local_center, solved_component.rotation_degrees),
                                );
                                point_matches(expected, node.position)
                            })
                        } else {
                            false
                        }
                    } else {
                        false
                    };
                    if !pose_matches {
                        finding(
                            &mut findings,
                            ElectricalConnectivityFindingCode::TerminalPoseMismatch,
                            Some(net),
                            vec![node.id.clone(), component.clone(), pin.clone()],
                            "terminal node cannot be attached to the solved component pose",
                        );
                    }
                    for branch in incident {
                        endpoint_incidence
                            .entry(branch)
                            .or_default()
                            .insert(&node.id);
                    }
                }
                SolvedRouteNodeKind::Junction { .. } => {
                    for branch in incident {
                        endpoint_incidence
                            .entry(branch)
                            .or_default()
                            .insert(&node.id);
                    }
                }
                SolvedRouteNodeKind::Via {
                    point_index,
                    from_layer,
                    to_layer,
                    diameter,
                    drill,
                } => {
                    if incident.len() != 1 {
                        finding(
                            &mut findings,
                            ElectricalConnectivityFindingCode::ViaNodeMismatch,
                            Some(net),
                            vec![node.id.clone()],
                            "via decorator must name exactly one incident branch",
                        );
                    } else if let Some(branch) = incident.first() {
                        *actual_vias
                            .entry(via_key(
                                branch,
                                *point_index,
                                node.position,
                                from_layer,
                                to_layer,
                                *diameter,
                                *drill,
                            ))
                            .or_default() += 1;
                    }
                }
            }
        }

        for ids in terminal_nodes.values_mut() {
            ids.sort_unstable();
        }

        let serialized_terminals = terminal_nodes.keys().cloned().collect::<BTreeSet<_>>();
        for missing in declared_terminals.difference(&serialized_terminals) {
            finding(
                &mut findings,
                ElectricalConnectivityFindingCode::MissingDeclaredTerminal,
                Some(net),
                vec![missing.component.clone(), missing.pin.clone()],
                "declared terminal is absent from the serialized route graph",
            );
        }
        for extra in serialized_terminals.difference(declared_terminals) {
            finding(
                &mut findings,
                ElectricalConnectivityFindingCode::ExtraTerminal,
                Some(net),
                vec![extra.component.clone(), extra.pin.clone()],
                "route graph contains a terminal not declared for this electrical net",
            );
        }
        for (key, ids) in &terminal_nodes {
            if ids.len() > 1 {
                finding(
                    &mut findings,
                    ElectricalConnectivityFindingCode::DuplicateTerminalNode,
                    Some(net),
                    ids.iter().map(|id| (*id).to_owned()).collect(),
                    format!(
                        "terminal {}.{} is represented by more than one route node",
                        key.component, key.pin
                    ),
                );
            }
        }

        let expected_vias = traces
            .iter()
            .filter(|trace| trace.electrical_net == *net)
            .flat_map(|trace| {
                trace.vias.iter().map(move |via| {
                    via_key(
                        &trace.branch,
                        via.point_index,
                        via.position,
                        &via.from_layer,
                        &via.to_layer,
                        via.diameter,
                        via.drill,
                    )
                })
            })
            .fold(BTreeMap::<ViaKey, usize>::new(), |mut counts, key| {
                *counts.entry(key).or_default() += 1;
                counts
            });
        if actual_vias != expected_vias {
            finding(
                &mut findings,
                ElectricalConnectivityFindingCode::ViaNodeMismatch,
                Some(net),
                vec![net.clone()],
                "serialized via-node multiset differs from trace via transitions",
            );
        }

        let connectivity_nodes = graph
            .nodes
            .iter()
            .filter(|node| !matches!(node.kind, SolvedRouteNodeKind::Via { .. }))
            .map(|node| node.id.as_str())
            .collect::<BTreeSet<_>>();
        let mut adjacency = connectivity_nodes
            .iter()
            .map(|node| (*node, Vec::<(&str, &str)>::new()))
            .collect::<BTreeMap<_, _>>();
        let mut valid_edges = BTreeMap::<&str, (&str, &str)>::new();
        let mut checked = 0;
        for branch in &expected_branches {
            let Some(trace) = trace_map.get(branch.as_str()).and_then(|trace| *trace) else {
                continue;
            };
            checked += 2;
            let from = nodes.get(trace.from_node.as_str()).and_then(|node| *node);
            let to = nodes.get(trace.to_node.as_str()).and_then(|node| *node);
            if from.is_none() || to.is_none() {
                finding(
                    &mut findings,
                    ElectricalConnectivityFindingCode::MissingEndpointNode,
                    Some(net),
                    vec![
                        branch.clone(),
                        trace.from_node.clone(),
                        trace.to_node.clone(),
                    ],
                    "trace references a missing route-graph endpoint node",
                );
                continue;
            }
            let from = from.unwrap();
            let to = to.unwrap();
            if matches!(from.kind, SolvedRouteNodeKind::Via { .. })
                || matches!(to.kind, SolvedRouteNodeKind::Via { .. })
            {
                finding(
                    &mut findings,
                    ElectricalConnectivityFindingCode::EndpointNodeKind,
                    Some(net),
                    vec![branch.clone(), from.id.clone(), to.id.clone()],
                    "trace endpoint must be a terminal or junction, never a via decorator",
                );
                continue;
            }
            if from.id == to.id {
                finding(
                    &mut findings,
                    ElectricalConnectivityFindingCode::SelfLoopBranch,
                    Some(net),
                    vec![branch.clone(), from.id.clone()],
                    "self-loop branches are not credited by the V1 reachability proof",
                );
                continue;
            }
            let actual = endpoint_incidence
                .get(branch.as_str())
                .cloned()
                .unwrap_or_default();
            let expected = BTreeSet::from([from.id.as_str(), to.id.as_str()]);
            if actual != expected {
                finding(
                    &mut findings,
                    ElectricalConnectivityFindingCode::BranchIncidenceMismatch,
                    Some(net),
                    actual
                        .iter()
                        .map(|node| (*node).to_owned())
                        .chain([from.id.clone(), to.id.clone()])
                        .collect(),
                    "branch incidence does not exactly match explicit endpoint-node IDs",
                );
                continue;
            }
            if trace
                .points
                .first()
                .is_none_or(|point| !point_matches(*point, from.position))
                || trace
                    .points
                    .last()
                    .is_none_or(|point| !point_matches(*point, to.position))
            {
                finding(
                    &mut findings,
                    ElectricalConnectivityFindingCode::TraceEndpointDetached,
                    Some(net),
                    vec![branch.clone(), from.id.clone(), to.id.clone()],
                    "trace endpoint geometry is detached from its explicit route nodes",
                );
                continue;
            }
            if !endpoint_layer_matches(problem, trace, from, true)
                || !endpoint_layer_matches(problem, trace, to, false)
            {
                finding(
                    &mut findings,
                    ElectricalConnectivityFindingCode::EndpointLayerMismatch,
                    Some(net),
                    vec![branch.clone(), from.id.clone(), to.id.clone()],
                    "trace endpoint layer is not connected to its terminal pad or junction layer",
                );
                continue;
            }
            if !trace_valid.get(branch.as_str()).copied().unwrap_or(false) {
                continue;
            }
            adjacency
                .get_mut(from.id.as_str())
                .unwrap()
                .push((to.id.as_str(), branch.as_str()));
            adjacency
                .get_mut(to.id.as_str())
                .unwrap()
                .push((from.id.as_str(), branch.as_str()));
            valid_edges.insert(branch, (from.id.as_str(), to.id.as_str()));
        }
        total_endpoint_incidences += checked;

        let mut unseen = connectivity_nodes.clone();
        let mut components = Vec::new();
        while let Some(start) = unseen.first().copied() {
            let mut queue = VecDeque::from([start]);
            unseen.remove(start);
            let mut component_nodes = BTreeSet::new();
            while let Some(node) = queue.pop_front() {
                component_nodes.insert(node);
                for (other, _) in &adjacency[node] {
                    if unseen.remove(other) {
                        queue.push_back(other);
                    }
                }
            }
            let branch_ids = valid_edges
                .iter()
                .filter(|(_, (from, to))| {
                    component_nodes.contains(from) && component_nodes.contains(to)
                })
                .map(|(branch, _)| (*branch).to_owned())
                .collect::<Vec<_>>();
            let terminals = terminal_nodes
                .iter()
                .filter(|(_, ids)| ids.iter().any(|id| component_nodes.contains(id)))
                .map(|(terminal, _)| terminal.clone())
                .collect::<Vec<_>>();
            components.push(ElectricalConnectivityComponentEvidence {
                node_ids: component_nodes.into_iter().map(str::to_owned).collect(),
                branch_ids,
                terminals,
            });
        }
        components.sort_by(|left, right| left.node_ids.cmp(&right.node_ids));

        let root = declared_terminals
            .first()
            .and_then(|terminal| terminal_nodes.get(terminal))
            .and_then(|ids| ids.first())
            .copied();
        let root_component = root.and_then(|root| {
            components
                .iter()
                .position(|component| component.node_ids.iter().any(|node| node == root))
        });
        let reachable_terminals = root_component
            .map(|index| {
                components[index]
                    .terminals
                    .iter()
                    .cloned()
                    .collect::<BTreeSet<_>>()
            })
            .unwrap_or_default();
        let unreachable = declared_terminals
            .difference(&reachable_terminals)
            .cloned()
            .collect::<Vec<_>>();
        if !unreachable.is_empty() {
            finding(
                &mut findings,
                ElectricalConnectivityFindingCode::DisconnectedDeclaredTerminals,
                Some(net),
                unreachable
                    .iter()
                    .flat_map(|terminal| [terminal.component.clone(), terminal.pin.clone()])
                    .collect(),
                "not every declared terminal is reachable through explicit route-graph edges",
            );
        }
        for (index, component) in components.iter().enumerate() {
            if Some(index) == root_component {
                continue;
            }
            let mut objects = component.node_ids.clone();
            objects.extend(component.branch_ids.clone());
            finding(
                &mut findings,
                ElectricalConnectivityFindingCode::DisconnectedIsland,
                Some(net),
                objects,
                "route graph contains an island disconnected from the canonical terminal root",
            );
        }

        let connected = unreachable.is_empty()
            && components.len() == 1
            && graph_branches == expected_branches
            && valid_edges.len() == expected_branches.len();
        net_evidence.push(ElectricalNetConnectivityEvidence {
            electrical_net: net.clone(),
            declared_terminals: declared_terminals.iter().cloned().collect(),
            serialized_terminals: serialized_terminals.into_iter().collect(),
            branches: expected_branches.into_iter().collect(),
            checked_endpoint_incidences: checked,
            connected_components: components,
            connected,
        });
    }

    findings.sort_by(|left, right| {
        (
            left.electrical_net.as_deref(),
            left.code,
            left.objects.as_slice(),
            left.detail.as_str(),
        )
            .cmp(&(
                right.electrical_net.as_deref(),
                right.code,
                right.objects.as_slice(),
                right.detail.as_str(),
            ))
    });
    ElectricalConnectivityAssessment {
        contract: ELECTRICAL_CONNECTIVITY_ASSESSMENT_CONTRACT.into(),
        complete: findings.is_empty() && net_evidence.iter().all(|net| net.connected),
        declared_net_count: declared.len(),
        checked_branch_count: traces.len(),
        checked_endpoint_incidences: total_endpoint_incidences,
        nets: net_evidence,
        findings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candidate::{SolvedRouteNode, SolvedVia};
    use layout_trace_model::model::SCHEMA_VERSION;
    use layout_trace_model::topology::{RouteClass, TerminalSector};

    fn problem() -> Problem {
        serde_json::from_str(
            r#"{
                "schema_version": 1,
                "board": {
                    "bounds": {"min":{"x":0.0,"y":0.0},"max":{"x":12.0,"y":4.0}},
                    "layers": [{"id":"top"},{"id":"bottom"}]
                },
                "rules": {"clearance":0.2,"via_diameter":0.8,"via_drill":0.4},
                "components": [
                    {"id":"A","position":{"x":1.0,"y":2.0},"size":{"x":1.0,"y":1.0},
                     "body_is_routing_keepout":false,"pins":[{"id":"1","offset":{"x":0.0,"y":0.0},"pads":[{"layer":"top","shape":{"kind":"circle","diameter":0.8}}]}]},
                    {"id":"B","position":{"x":6.0,"y":2.0},"size":{"x":1.0,"y":1.0},
                     "body_is_routing_keepout":false,"pins":[{"id":"1","offset":{"x":0.0,"y":0.0},"pads":[{"layer":"top","shape":{"kind":"circle","diameter":0.8}}]}]},
                    {"id":"C","position":{"x":11.0,"y":2.0},"size":{"x":1.0,"y":1.0},
                     "body_is_routing_keepout":false,"pins":[{"id":"1","offset":{"x":0.0,"y":0.0},"pads":[{"layer":"top","shape":{"kind":"circle","diameter":0.8}}]}]}
                ],
                "electrical_nets":[{
                    "id":"SIGNAL","width":0.3,"layer":"top","allowed_layers":["top","bottom"],
                    "terminals":[{"component":"A","pin":"1"},{"component":"B","pin":"1"},{"component":"C","pin":"1"}]
                }]
            }"#,
        )
        .unwrap()
    }

    fn solved_component(id: &str, x: f64) -> SolvedComponent {
        SolvedComponent {
            id: id.into(),
            position: Vec2::new(x, 2.0),
            size: Vec2::new(1.0, 1.0),
            rotation_degrees: 0.0,
        }
    }

    fn terminal_node(id: &str, component: &str, x: f64, branches: &[&str]) -> SolvedRouteNode {
        SolvedRouteNode {
            id: id.into(),
            electrical_net: "SIGNAL".into(),
            position: Vec2::new(x, 2.0),
            incident_branches: branches.iter().map(|branch| (*branch).into()).collect(),
            kind: SolvedRouteNodeKind::Terminal {
                component: component.into(),
                pin: "1".into(),
            },
        }
    }

    fn trace(branch: &str, from_node: &str, to_node: &str, from_x: f64, to_x: f64) -> SolvedTrace {
        SolvedTrace {
            branch: branch.into(),
            electrical_net: "SIGNAL".into(),
            net: branch.into(),
            from_node: from_node.into(),
            to_node: to_node.into(),
            width: 0.3,
            tension_weight: 1.0,
            layer: "top".into(),
            segment_layers: vec!["top".into()],
            vias: Vec::new(),
            points: vec![Vec2::new(from_x, 2.0), Vec2::new(to_x, 2.0)],
            route_class: RouteClass::direct(
                "top",
                TerminalSector {
                    component: from_node.into(),
                    sector: "1".into(),
                },
                TerminalSector {
                    component: to_node.into(),
                    sector: "1".into(),
                },
            ),
            route_basis_fingerprint: None,
        }
    }

    fn valid_fixture() -> (
        Problem,
        Vec<SolvedComponent>,
        Vec<SolvedTrace>,
        Vec<SolvedRouteGraph>,
    ) {
        let problem = problem();
        let components = vec![
            solved_component("A", 1.0),
            solved_component("B", 6.0),
            solved_component("C", 11.0),
        ];
        let traces = vec![
            trace("AB", "a", "b", 1.0, 6.0),
            trace("BC", "b", "c", 6.0, 11.0),
        ];
        let graphs = vec![SolvedRouteGraph {
            electrical_net: "SIGNAL".into(),
            nodes: vec![
                terminal_node("a", "A", 1.0, &["AB"]),
                terminal_node("b", "B", 6.0, &["AB", "BC"]),
                terminal_node("c", "C", 11.0, &["BC"]),
            ],
            branches: vec!["AB".into(), "BC".into()],
        }];
        (problem, components, traces, graphs)
    }

    fn codes(
        assessment: &ElectricalConnectivityAssessment,
    ) -> BTreeSet<ElectricalConnectivityFindingCode> {
        assessment
            .findings
            .iter()
            .map(|finding| finding.code)
            .collect()
    }

    #[test]
    fn accepts_a_complete_explicit_three_terminal_tree() {
        let (problem, components, traces, graphs) = valid_fixture();
        let assessment = assess_electrical_connectivity(&problem, &components, &traces, &graphs);
        assert!(assessment.complete);
        assert!(assessment.findings.is_empty());
        assert_eq!(assessment.checked_endpoint_incidences, 4);
        assert_eq!(assessment.nets[0].connected_components.len(), 1);
    }

    #[test]
    fn rejects_incidence_endpoint_and_terminal_pose_tampering() {
        let (problem, components, mut traces, mut graphs) = valid_fixture();
        traces[0].from_node = "c".into();
        graphs[0].nodes[2].position.x = 10.5;
        let assessment = assess_electrical_connectivity(&problem, &components, &traces, &graphs);
        let codes = codes(&assessment);
        assert!(codes.contains(&ElectricalConnectivityFindingCode::BranchIncidenceMismatch));
        assert!(codes.contains(&ElectricalConnectivityFindingCode::TerminalPoseMismatch));
        assert!(codes.contains(&ElectricalConnectivityFindingCode::TraceEndpointDetached));
        assert!(codes.contains(&ElectricalConnectivityFindingCode::DisconnectedDeclaredTerminals));
        assert!(!assessment.complete);
    }

    #[test]
    fn rejects_missing_terminals_and_graph_branches() {
        let (problem, components, traces, mut graphs) = valid_fixture();
        graphs[0].nodes.retain(|node| node.id != "c");
        graphs[0].branches.retain(|branch| branch != "BC");
        let assessment = assess_electrical_connectivity(&problem, &components, &traces, &graphs);
        let codes = codes(&assessment);
        assert!(codes.contains(&ElectricalConnectivityFindingCode::MissingDeclaredTerminal));
        assert!(codes.contains(&ElectricalConnectivityFindingCode::BranchSetMismatch));
        assert!(codes.contains(&ElectricalConnectivityFindingCode::MissingEndpointNode));
        assert!(!assessment.complete);
    }

    #[test]
    fn duplicate_graph_identity_is_fail_closed_and_order_independent() {
        let (problem, components, traces, mut graphs) = valid_fixture();
        let mut duplicate = graphs[0].clone();
        duplicate.nodes.clear();
        graphs.push(duplicate);
        let first = assess_electrical_connectivity(&problem, &components, &traces, &graphs);
        graphs.reverse();
        let second = assess_electrical_connectivity(&problem, &components, &traces, &graphs);
        assert_eq!(first, second);
        assert!(codes(&first).contains(&ElectricalConnectivityFindingCode::DuplicateRouteGraph));
        assert!(codes(&first).contains(&ElectricalConnectivityFindingCode::MissingRouteGraph));
    }

    #[test]
    fn rejects_self_loops_and_never_credits_them_as_connectivity() {
        let (problem, components, mut traces, mut graphs) = valid_fixture();
        traces[1].from_node = "b".into();
        traces[1].to_node = "b".into();
        traces[1].points = vec![Vec2::new(6.0, 2.0), Vec2::new(6.0, 2.0)];
        graphs[0].nodes[2].incident_branches.clear();
        let assessment = assess_electrical_connectivity(&problem, &components, &traces, &graphs);
        let codes = codes(&assessment);
        assert!(codes.contains(&ElectricalConnectivityFindingCode::SelfLoopBranch));
        assert!(codes.contains(&ElectricalConnectivityFindingCode::DisconnectedDeclaredTerminals));
    }

    #[test]
    fn checks_layer_transitions_and_the_exact_via_decorator_multiset() {
        let (problem, components, mut traces, graphs) = valid_fixture();
        traces[0].points.insert(1, Vec2::new(3.0, 2.0));
        traces[0].segment_layers = vec!["top".into(), "bottom".into()];
        let assessment = assess_electrical_connectivity(&problem, &components, &traces, &graphs);
        assert!(
            codes(&assessment).contains(&ElectricalConnectivityFindingCode::InvalidTraceContinuity)
        );

        let via_position = traces[0].points[1];
        traces[0].vias.push(SolvedVia {
            position: via_position,
            from_layer: "top".into(),
            to_layer: "bottom".into(),
            diameter: 0.8,
            drill: 0.4,
            point_index: 1,
        });
        let assessment = assess_electrical_connectivity(&problem, &components, &traces, &graphs);
        let codes = codes(&assessment);
        assert!(!codes.contains(&ElectricalConnectivityFindingCode::InvalidTraceContinuity));
        assert!(codes.contains(&ElectricalConnectivityFindingCode::ViaNodeMismatch));
        assert!(codes.contains(&ElectricalConnectivityFindingCode::EndpointLayerMismatch));
    }

    #[test]
    fn via_decorators_cannot_connect_two_branches() {
        let (problem, components, mut traces, mut graphs) = valid_fixture();
        traces[0].to_node = "b1".into();
        traces[1].from_node = "b2".into();
        graphs[0].nodes.remove(1);
        graphs[0].nodes.push(terminal_node("b1", "B", 6.0, &["AB"]));
        graphs[0].nodes.push(terminal_node("b2", "B", 6.0, &["BC"]));
        graphs[0].nodes.push(SolvedRouteNode {
            id: "decorator".into(),
            electrical_net: "SIGNAL".into(),
            position: Vec2::new(6.0, 2.0),
            incident_branches: vec!["AB".into(), "BC".into()],
            kind: SolvedRouteNodeKind::Via {
                point_index: 1,
                from_layer: "top".into(),
                to_layer: "bottom".into(),
                diameter: 0.8,
                drill: 0.4,
            },
        });
        let assessment = assess_electrical_connectivity(&problem, &components, &traces, &graphs);
        let codes = codes(&assessment);
        assert!(codes.contains(&ElectricalConnectivityFindingCode::ViaNodeMismatch));
        assert!(codes.contains(&ElectricalConnectivityFindingCode::DisconnectedDeclaredTerminals));
        assert_eq!(assessment.nets[0].connected_components.len(), 2);
    }

    #[test]
    fn assessment_is_deterministic_under_serialized_input_permutation() {
        let (problem, mut components, mut traces, mut graphs) = valid_fixture();
        let first = assess_electrical_connectivity(&problem, &components, &traces, &graphs);
        components.reverse();
        traces.reverse();
        graphs[0].nodes.reverse();
        graphs[0].branches.reverse();
        graphs.reverse();
        let second = assess_electrical_connectivity(&problem, &components, &traces, &graphs);
        assert_eq!(first, second);
        assert_eq!(first.contract, ELECTRICAL_CONNECTIVITY_ASSESSMENT_CONTRACT);
        assert_eq!(problem.schema_version, SCHEMA_VERSION);
    }
}
