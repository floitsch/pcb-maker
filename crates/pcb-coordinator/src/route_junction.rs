// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

use std::collections::{BTreeMap, BTreeSet};

use layout_trace_model::{
    Problem, Vec2,
    geometry::{length, sub},
    topology::TerminalSector,
};
use pcb_routing::{
    DutGridRoutingConfig, DutGridRoutingEvidence, route_problem_with_dut_grid_at_poses,
};
use pcb_validate::{
    CandidateArtifact, ExactValidationAssessment, SolvedComponent, SolvedRouteGraph,
    SolvedRouteNode, SolvedRouteNodeKind, SolvedTrace, validate_candidate,
};
use serde::{Deserialize, Serialize};

const LENGTH_EPSILON: f64 = 1.0e-9;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RouteJunctionConfig {
    pub maximum_proposals: usize,
    pub minimum_length_improvement_mm: f64,
    pub stop_on_first_improvement: bool,
}

impl RouteJunctionConfig {
    pub fn check(&self) -> Result<(), String> {
        if self.maximum_proposals == 0 {
            return Err("route junction maximum_proposals must be positive".into());
        }
        if !self.minimum_length_improvement_mm.is_finite()
            || self.minimum_length_improvement_mm < 0.0
        {
            return Err(
                "route junction minimum_length_improvement_mm must be finite and non-negative"
                    .into(),
            );
        }
        Ok(())
    }
}

impl Default for RouteJunctionConfig {
    fn default() -> Self {
        Self {
            maximum_proposals: 32,
            minimum_length_improvement_mm: 0.01,
            stop_on_first_improvement: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RouteJunctionProposalEvidence {
    pub electrical_net: String,
    pub terminal_node: String,
    pub terminal: String,
    pub split_branch: String,
    pub attached_branches: Vec<String>,
    pub split_point_index: usize,
    pub junction: String,
    pub junction_position: Vec2,
    pub layer: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct RouteJunctionAttempt {
    pub id: String,
    pub parent_attempt: usize,
    pub proposal: Option<RouteJunctionProposalEvidence>,
    pub candidate: Option<CandidateArtifact>,
    pub validation: Option<ExactValidationAssessment>,
    pub trace_length_mm: Option<f64>,
    pub rejected_before_validation: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct RouteJunctionEvidence {
    pub strategy: String,
    pub config: RouteJunctionConfig,
    pub selected_attempt: usize,
    pub attempted_proposals: usize,
    pub accepted: bool,
    pub complete: bool,
    pub baseline_trace_length_mm: f64,
    pub selected_trace_length_mm: f64,
    pub trace_length_improvement_mm: f64,
    pub routing: Option<DutGridRoutingEvidence>,
}

#[derive(Clone, Debug, Serialize)]
pub struct RouteJunctionResult {
    pub candidate: CandidateArtifact,
    pub validation: ExactValidationAssessment,
    pub evidence: RouteJunctionEvidence,
    pub attempts: Vec<RouteJunctionAttempt>,
}

#[derive(Clone)]
struct RouteJunctionProposal {
    evidence: RouteJunctionProposalEvidence,
    split_trace: usize,
    attached_traces: Vec<usize>,
}

/// Try bounded explicit-junction topology changes after grid routing.
///
/// This ports the useful transaction from layout-trace: split one branch next
/// to a high-degree terminal, reattach its sibling branches to the new
/// junction, rebuild the durable graph, and retain only an exact-valid length
/// improvement. It deliberately does not infer connectivity from geometric
/// same-net contact.
pub fn optimize_route_junctions(
    problem: &Problem,
    components: &[SolvedComponent],
    routing_config: &DutGridRoutingConfig,
    junction_config: &RouteJunctionConfig,
) -> Result<RouteJunctionResult, String> {
    problem.check_schema()?;
    routing_config.check()?;
    junction_config.check()?;
    let initial = route_problem_with_dut_grid_at_poses(problem, components, routing_config)?;
    let routing_complete = initial.evidence.failed_branches == 0;
    let routing_evidence = initial.evidence;
    let mut result =
        optimize_candidate_route_junctions(problem, &initial.candidate, junction_config)?;
    result.evidence.complete &= routing_complete;
    result.evidence.routing = Some(routing_evidence);
    Ok(result)
}

/// Optimize an already materialized durable candidate independently of the
/// router that produced it.
pub fn optimize_candidate_route_junctions(
    problem: &Problem,
    initial_candidate: &CandidateArtifact,
    junction_config: &RouteJunctionConfig,
) -> Result<RouteJunctionResult, String> {
    problem.check_schema()?;
    junction_config.check()?;
    let initial_validation = validate_candidate(problem, initial_candidate)?;
    let baseline_length = candidate_trace_length(initial_candidate);
    let mut attempts = vec![RouteJunctionAttempt {
        id: "initial".into(),
        parent_attempt: 0,
        proposal: None,
        candidate: Some(initial_candidate.clone()),
        validation: Some(initial_validation.clone()),
        trace_length_mm: Some(baseline_length),
        rejected_before_validation: None,
    }];
    let mut selected = 0;
    let mut selected_length = baseline_length;

    if initial_validation.complete {
        let proposals = junction_proposals(initial_candidate, junction_config.maximum_proposals)?;
        for proposal in proposals {
            let attempt_index = attempts.len();
            let id = format!("junction-{attempt_index:04}");
            let mut candidate = initial_candidate.clone();
            match apply_junction_proposal(&mut candidate, &proposal) {
                Ok(()) => {
                    let validation = validate_candidate(problem, &candidate)?;
                    let trace_length = candidate_trace_length(&candidate);
                    let improvement = selected_length - trace_length;
                    let improved = validation.complete
                        && improvement > LENGTH_EPSILON
                        && improvement + LENGTH_EPSILON
                            >= junction_config.minimum_length_improvement_mm;
                    attempts.push(RouteJunctionAttempt {
                        id,
                        parent_attempt: 0,
                        proposal: Some(proposal.evidence),
                        candidate: Some(candidate),
                        validation: Some(validation),
                        trace_length_mm: Some(trace_length),
                        rejected_before_validation: None,
                    });
                    if improved {
                        selected = attempt_index;
                        selected_length = trace_length;
                        if junction_config.stop_on_first_improvement {
                            break;
                        }
                    }
                }
                Err(error) => attempts.push(RouteJunctionAttempt {
                    id,
                    parent_attempt: 0,
                    proposal: Some(proposal.evidence),
                    candidate: None,
                    validation: None,
                    trace_length_mm: None,
                    rejected_before_validation: Some(error),
                }),
            }
        }
    }

    let selected_candidate = attempts[selected]
        .candidate
        .as_ref()
        .expect("selected junction attempt has a candidate")
        .clone();
    let selected_validation = attempts[selected]
        .validation
        .as_ref()
        .expect("selected junction attempt was validated")
        .clone();
    let complete = selected_validation.complete;
    Ok(RouteJunctionResult {
        candidate: selected_candidate,
        validation: selected_validation,
        evidence: RouteJunctionEvidence {
            strategy: "layout-trace-adjacent-explicit-junction-v1".into(),
            config: junction_config.clone(),
            selected_attempt: selected,
            attempted_proposals: attempts.len() - 1,
            accepted: selected != 0,
            complete,
            baseline_trace_length_mm: baseline_length,
            selected_trace_length_mm: selected_length,
            trace_length_improvement_mm: (baseline_length - selected_length).max(0.0),
            routing: None,
        },
        attempts,
    })
}

fn candidate_trace_length(candidate: &CandidateArtifact) -> f64 {
    candidate
        .traces
        .iter()
        .flat_map(|trace| trace.points.windows(2))
        .map(|segment| length(sub(segment[1], segment[0])))
        .sum()
}

fn junction_sector(junction: &str) -> TerminalSector {
    TerminalSector {
        component: "@junction".into(),
        sector: junction.into(),
    }
}

fn endpoint_layer<'a>(trace: &'a SolvedTrace, node: &str) -> Option<&'a str> {
    if trace.from_node == node {
        trace.segment_layers.first().map(String::as_str)
    } else if trace.to_node == node {
        trace.segment_layers.last().map(String::as_str)
    } else {
        None
    }
}

fn endpoint_has_via(trace: &SolvedTrace, node: &str) -> bool {
    if trace.from_node == node {
        trace.vias.iter().any(|via| via.point_index == 0)
    } else if trace.to_node == node {
        trace
            .vias
            .iter()
            .any(|via| via.point_index + 1 == trace.points.len())
    } else {
        true
    }
}

fn junction_proposals(
    candidate: &CandidateArtifact,
    limit: usize,
) -> Result<Vec<RouteJunctionProposal>, String> {
    let trace_indexes = candidate
        .traces
        .iter()
        .enumerate()
        .map(|(index, trace)| (trace.branch.as_str(), index))
        .collect::<BTreeMap<_, _>>();
    let mut proposals = Vec::new();
    for graph in &candidate.route_graphs {
        for node in &graph.nodes {
            let SolvedRouteNodeKind::Terminal { component, pin } = &node.kind else {
                continue;
            };
            if node.incident_branches.len() < 2 {
                continue;
            }
            let incident =
                node.incident_branches
                    .iter()
                    .map(|branch| {
                        trace_indexes.get(branch.as_str()).copied().ok_or_else(|| {
                            format!("route graph references missing branch {branch}")
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
            for &split_trace in &incident {
                let split = &candidate.traces[split_trace];
                if split.points.len() < 3 || split.electrical_net != graph.electrical_net {
                    continue;
                }
                let split_point = if split.from_node == node.id {
                    1
                } else if split.to_node == node.id {
                    split.points.len() - 2
                } else {
                    continue;
                };
                if split.vias.iter().any(|via| via.point_index == split_point)
                    || split.segment_layers[split_point - 1] != split.segment_layers[split_point]
                {
                    continue;
                }
                let layer = split.segment_layers[split_point].clone();
                let attached_traces = incident
                    .iter()
                    .copied()
                    .filter(|index| *index != split_trace)
                    .collect::<Vec<_>>();
                if attached_traces.iter().any(|index| {
                    let trace = &candidate.traces[*index];
                    trace.electrical_net != split.electrical_net
                        || endpoint_layer(trace, &node.id) != Some(layer.as_str())
                        || endpoint_has_via(trace, &node.id)
                }) {
                    continue;
                }
                let proposal_number = proposals.len() + 1;
                let junction = format!("junction:{}:{proposal_number:04}", split.electrical_net);
                proposals.push(RouteJunctionProposal {
                    evidence: RouteJunctionProposalEvidence {
                        electrical_net: split.electrical_net.clone(),
                        terminal_node: node.id.clone(),
                        terminal: format!("{component}.{pin}"),
                        split_branch: split.branch.clone(),
                        attached_branches: attached_traces
                            .iter()
                            .map(|index| candidate.traces[*index].branch.clone())
                            .collect(),
                        split_point_index: split_point,
                        junction,
                        junction_position: split.points[split_point],
                        layer,
                    },
                    split_trace,
                    attached_traces,
                });
                if proposals.len() >= limit {
                    return Ok(proposals);
                }
            }
        }
    }
    Ok(proposals)
}

fn split_trace_at_junction(
    candidate: &mut CandidateArtifact,
    proposal: &RouteJunctionProposal,
) -> Result<(), String> {
    let trace = candidate
        .traces
        .get(proposal.split_trace)
        .ok_or_else(|| format!("cannot split missing trace {}", proposal.split_trace))?
        .clone();
    let point_index = proposal.evidence.split_point_index;
    if point_index == 0 || point_index + 1 >= trace.points.len() {
        return Err("junction split must use an interior trace point".into());
    }
    let tail_id = format!("{}#split:{}", trace.branch, proposal.evidence.junction);
    if candidate.traces.iter().any(|trace| trace.branch == tail_id) {
        return Err(format!("junction split would duplicate branch {tail_id}"));
    }

    let mut tail = trace.clone();
    tail.branch = tail_id.clone();
    tail.net = tail_id;
    tail.points = tail.points[point_index..].to_vec();
    tail.segment_layers = tail.segment_layers[point_index..].to_vec();
    tail.vias = tail
        .vias
        .into_iter()
        .filter(|via| via.point_index > point_index)
        .map(|mut via| {
            via.point_index -= point_index;
            via
        })
        .collect();
    tail.from_node = proposal.evidence.junction.clone();
    tail.route_class.from = junction_sector(&proposal.evidence.junction);
    tail.route_basis_fingerprint = None;
    if let Some(layer) = tail.segment_layers.first() {
        tail.layer = layer.clone();
    }

    let head = &mut candidate.traces[proposal.split_trace];
    head.points.truncate(point_index + 1);
    head.segment_layers.truncate(point_index);
    head.vias.retain(|via| via.point_index < point_index);
    head.to_node = proposal.evidence.junction.clone();
    head.route_class.to = junction_sector(&proposal.evidence.junction);
    head.route_basis_fingerprint = None;
    candidate.traces.push(tail);
    Ok(())
}

fn reattach_trace_endpoint(
    trace: &mut SolvedTrace,
    terminal_node: &str,
    junction: &str,
    junction_position: Vec2,
) -> Result<(), String> {
    if trace.from_node == terminal_node {
        trace.from_node = junction.into();
        trace.points[0] = junction_position;
        trace.route_class.from = junction_sector(junction);
    } else if trace.to_node == terminal_node {
        trace.to_node = junction.into();
        *trace
            .points
            .last_mut()
            .ok_or_else(|| format!("trace {} has no endpoint", trace.branch))? = junction_position;
        trace.route_class.to = junction_sector(junction);
    } else {
        return Err(format!(
            "trace {} is not incident to terminal {terminal_node}",
            trace.branch
        ));
    }
    trace.route_basis_fingerprint = None;
    Ok(())
}

fn apply_junction_proposal(
    candidate: &mut CandidateArtifact,
    proposal: &RouteJunctionProposal,
) -> Result<(), String> {
    if candidate
        .route_graphs
        .iter()
        .flat_map(|graph| &graph.nodes)
        .any(|node| node.id == proposal.evidence.junction)
    {
        return Err(format!(
            "junction identity {} already exists",
            proposal.evidence.junction
        ));
    }
    let graph = candidate
        .route_graphs
        .iter_mut()
        .find(|graph| graph.electrical_net == proposal.evidence.electrical_net)
        .ok_or_else(|| {
            format!(
                "missing route graph for {}",
                proposal.evidence.electrical_net
            )
        })?;
    graph.nodes.push(SolvedRouteNode {
        id: proposal.evidence.junction.clone(),
        electrical_net: proposal.evidence.electrical_net.clone(),
        position: proposal.evidence.junction_position,
        incident_branches: Vec::new(),
        kind: SolvedRouteNodeKind::Junction {
            layer: proposal.evidence.layer.clone(),
            movable: true,
        },
    });
    split_trace_at_junction(candidate, proposal)?;
    for &trace_index in &proposal.attached_traces {
        reattach_trace_endpoint(
            &mut candidate.traces[trace_index],
            &proposal.evidence.terminal_node,
            &proposal.evidence.junction,
            proposal.evidence.junction_position,
        )?;
    }
    rebuild_route_graphs(candidate)
}

fn rebuild_route_graphs(candidate: &mut CandidateArtifact) -> Result<(), String> {
    let mut nodes = BTreeMap::<String, BTreeMap<String, SolvedRouteNode>>::new();
    for graph in &candidate.route_graphs {
        let graph_nodes = nodes.entry(graph.electrical_net.clone()).or_default();
        for node in &graph.nodes {
            if matches!(node.kind, SolvedRouteNodeKind::Via { .. }) {
                continue;
            }
            let mut node = node.clone();
            node.incident_branches.clear();
            if graph_nodes.insert(node.id.clone(), node).is_some() {
                return Err(format!(
                    "duplicate connectivity node in graph {}",
                    graph.electrical_net
                ));
            }
        }
    }
    let mut branches = BTreeMap::<String, BTreeSet<String>>::new();
    for trace in &candidate.traces {
        let graph_nodes = nodes.entry(trace.electrical_net.clone()).or_default();
        for endpoint in [&trace.from_node, &trace.to_node] {
            let node = graph_nodes.get_mut(endpoint).ok_or_else(|| {
                format!(
                    "trace {} references missing endpoint node {endpoint}",
                    trace.branch
                )
            })?;
            node.incident_branches.push(trace.branch.clone());
        }
        if !branches
            .entry(trace.electrical_net.clone())
            .or_default()
            .insert(trace.branch.clone())
        {
            return Err(format!("duplicate trace branch {}", trace.branch));
        }
        for via in &trace.vias {
            let id = format!("{}::via::{}", trace.branch, via.point_index);
            if graph_nodes
                .insert(
                    id.clone(),
                    SolvedRouteNode {
                        id: id.clone(),
                        electrical_net: trace.electrical_net.clone(),
                        position: via.position,
                        incident_branches: vec![trace.branch.clone()],
                        kind: SolvedRouteNodeKind::Via {
                            point_index: via.point_index,
                            from_layer: via.from_layer.clone(),
                            to_layer: via.to_layer.clone(),
                            diameter: via.diameter,
                            drill: via.drill,
                        },
                    },
                )
                .is_some()
            {
                return Err(format!("duplicate via node {id}"));
            }
        }
    }
    candidate.route_graphs = nodes
        .into_iter()
        .map(|(electrical_net, nodes)| {
            let mut nodes = nodes.into_values().collect::<Vec<_>>();
            for node in &mut nodes {
                node.incident_branches.sort();
                node.incident_branches.dedup();
            }
            nodes.sort_by(|left, right| left.id.cmp(&right.id));
            SolvedRouteGraph {
                branches: branches
                    .remove(&electrical_net)
                    .unwrap_or_default()
                    .into_iter()
                    .collect(),
                electrical_net,
                nodes,
            }
        })
        .collect();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn problem() -> Problem {
        serde_json::from_str(include_str!(
            "../../../benchmarks/imported/layout-trace/multi-terminal-tree.json"
        ))
        .unwrap()
    }

    fn components(problem: &Problem) -> Vec<SolvedComponent> {
        problem
            .components
            .iter()
            .map(|component| SolvedComponent {
                id: component.id.clone(),
                position: component.position,
                size: component.size,
                rotation_degrees: component.rotation_degrees,
            })
            .collect()
    }

    #[test]
    fn adjacent_junction_transaction_shortens_an_explicit_three_terminal_tree() {
        let problem = problem();
        let result = optimize_route_junctions(
            &problem,
            &components(&problem),
            &DutGridRoutingConfig::default(),
            &RouteJunctionConfig::default(),
        )
        .unwrap();

        assert!(result.evidence.complete);
        assert!(result.evidence.accepted);
        assert_eq!(result.evidence.selected_attempt, 1);
        assert_eq!(result.evidence.attempted_proposals, 1);
        assert!(result.evidence.trace_length_improvement_mm > 0.2);
        assert_eq!(result.candidate.traces.len(), 3);
        let graph = &result.candidate.route_graphs[0];
        let junction = graph
            .nodes
            .iter()
            .find(|node| matches!(node.kind, SolvedRouteNodeKind::Junction { .. }))
            .expect("accepted explicit junction");
        assert_eq!(junction.incident_branches.len(), 3);
        let terminal_c = graph
            .nodes
            .iter()
            .find(|node| node.id == "terminal:BUS/C.1")
            .unwrap();
        assert_eq!(terminal_c.incident_branches.len(), 1);
        assert!(
            result.validation.complete,
            "{:?}",
            result.validation.violations
        );
    }

    #[test]
    fn junction_trials_do_not_commit_without_the_configured_improvement() {
        let problem = problem();
        let result = optimize_route_junctions(
            &problem,
            &components(&problem),
            &DutGridRoutingConfig::default(),
            &RouteJunctionConfig {
                maximum_proposals: 8,
                minimum_length_improvement_mm: 1.0,
                stop_on_first_improvement: false,
            },
        )
        .unwrap();

        assert!(result.evidence.complete);
        assert!(!result.evidence.accepted);
        assert_eq!(result.evidence.selected_attempt, 0);
        assert_eq!(result.evidence.attempted_proposals, 2);
        assert_eq!(result.candidate.traces.len(), 2);
        assert_eq!(
            result.evidence.selected_trace_length_mm,
            result.evidence.baseline_trace_length_mm
        );
    }
}
