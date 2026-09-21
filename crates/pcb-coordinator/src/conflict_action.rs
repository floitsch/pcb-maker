// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

use std::collections::{BTreeMap, BTreeSet};

use layout_trace_model::{
    Problem, Vec2,
    geometry::{add, length, rotate_degrees, sub},
    topology::{RouteClass, TerminalSector},
};
use pcb_validate::{
    CANDIDATE_SCHEMA_VERSION, CandidateArtifact, ExactValidationAssessment, SolvedComponent,
    SolvedRouteGraph, SolvedRouteNode, SolvedRouteNodeKind, SolvedTrace, SolvedVia, Violation,
    validate_candidate,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const AXIS_EPSILON: f64 = 1.0e-9;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConflictActionSearchConfig {
    pub maximum_actions: usize,
    pub detour_clearance_mm: f64,
    pub detour_span_mm: f64,
    pub via_offset_mm: f64,
    pub via_penalty_mm: f64,
    pub unresolved_conflict_penalty_mm: f64,
}

impl ConflictActionSearchConfig {
    pub fn check(&self) -> Result<(), String> {
        if self.maximum_actions == 0 || self.maximum_actions > 64 {
            return Err("conflict-action maximum_actions must be between 1 and 64".into());
        }
        for (name, value) in [
            ("detour_clearance_mm", self.detour_clearance_mm),
            ("detour_span_mm", self.detour_span_mm),
            ("via_offset_mm", self.via_offset_mm),
            ("via_penalty_mm", self.via_penalty_mm),
            (
                "unresolved_conflict_penalty_mm",
                self.unresolved_conflict_penalty_mm,
            ),
        ] {
            if !value.is_finite() || value <= 0.0 {
                return Err(format!(
                    "conflict-action {name} must be finite and positive"
                ));
            }
        }
        Ok(())
    }
}

impl Default for ConflictActionSearchConfig {
    fn default() -> Self {
        Self {
            maximum_actions: 6,
            detour_clearance_mm: 2.5,
            detour_span_mm: 2.0,
            via_offset_mm: 3.0,
            via_penalty_mm: 5.0,
            unresolved_conflict_penalty_mm: 100.0,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BestFirstConflictActionSearchConfig {
    pub action: ConflictActionSearchConfig,
    #[serde(default)]
    pub post_action: ConflictActionPostProcessConfig,
    pub maximum_expanded_states: usize,
    pub maximum_generated_states: usize,
    pub maximum_depth: usize,
    pub maximum_conflicts_per_state: usize,
}

impl BestFirstConflictActionSearchConfig {
    pub fn check(&self) -> Result<(), String> {
        self.action.check()?;
        self.post_action.check()?;
        for (name, value, maximum) in [
            (
                "maximum_expanded_states",
                self.maximum_expanded_states,
                100_000,
            ),
            (
                "maximum_generated_states",
                self.maximum_generated_states,
                1_000_000,
            ),
            ("maximum_depth", self.maximum_depth, 1_000),
            (
                "maximum_conflicts_per_state",
                self.maximum_conflicts_per_state,
                1_000,
            ),
        ] {
            if value == 0 || value > maximum {
                return Err(format!(
                    "best-first conflict-action {name} must be between 1 and {maximum}"
                ));
            }
        }
        Ok(())
    }
}

impl Default for BestFirstConflictActionSearchConfig {
    fn default() -> Self {
        Self {
            action: ConflictActionSearchConfig::default(),
            post_action: ConflictActionPostProcessConfig::default(),
            maximum_expanded_states: 128,
            maximum_generated_states: 256,
            maximum_depth: 2,
            maximum_conflicts_per_state: 2,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(tag = "strategy", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConflictActionPostProcessConfig {
    #[default]
    None,
    ExactLineOfSight {
        maximum_shortcut_attempts: usize,
    },
    ExactVertexPull {
        maximum_vertex_trials: usize,
        sweeps: usize,
        binary_steps: usize,
    },
}

impl ConflictActionPostProcessConfig {
    pub fn check(&self) -> Result<(), String> {
        match self {
            Self::None => Ok(()),
            Self::ExactLineOfSight {
                maximum_shortcut_attempts,
            } if (1..=100_000).contains(maximum_shortcut_attempts) => Ok(()),
            Self::ExactLineOfSight { .. } => Err(
                "exact line-of-sight maximum_shortcut_attempts must be between 1 and 100000".into(),
            ),
            Self::ExactVertexPull {
                maximum_vertex_trials,
                sweeps,
                binary_steps,
            } if (1..=100_000).contains(maximum_vertex_trials)
                && (1..=100).contains(sweeps)
                && (1..=64).contains(binary_steps) =>
            {
                Ok(())
            }
            Self::ExactVertexPull { .. } => Err(
                "exact vertex-pull requires 1..=100000 vertex trials, 1..=100 sweeps, and 1..=64 binary steps"
                    .into(),
            ),
        }
    }
}

/// Consumer-neutral names for the exact route optimizer. The implementation
/// originated as a conflict-action post-processor, but it operates only on a
/// semantic problem/candidate pair and is also suitable after continuous
/// motion or another routing strategy.
pub type ExactRoutePostProcessConfig = ConflictActionPostProcessConfig;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CardinalDetourSide {
    North,
    South,
    West,
    East,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConflictActionKind {
    PlanarDetour {
        moving_branch: String,
        blocking_branch: String,
        side: CardinalDetourSide,
    },
    LayerEscape {
        moving_branch: String,
        blocking_branch: String,
        escape_layer: String,
    },
}

#[derive(Clone, Debug, Serialize)]
pub struct ConflictAction {
    pub id: String,
    pub action: ConflictActionKind,
}

#[derive(Clone, Debug)]
pub struct ConflictActionProposal {
    pub action: ConflictAction,
    pub candidate: CandidateArtifact,
}

/// Replaceable producer boundary for discrete conflict-resolution actions.
/// Producers mutate complete candidate states; they do not own exact
/// validation, state ranking, or transactional selection.
pub trait ConflictActionProducer {
    fn name(&self) -> &'static str;

    fn propose(
        &self,
        problem: &Problem,
        source: &CandidateArtifact,
        conflict: &Violation,
        config: &ConflictActionSearchConfig,
    ) -> Result<Vec<ConflictActionProposal>, String>;
}

#[derive(Clone, Debug, Serialize)]
pub struct ConflictActionPostProcessEvidence {
    pub processor: String,
    pub status: String,
    pub attempted_operations: usize,
    pub accepted_operations: usize,
    pub exact_validation_calls: usize,
    pub input_trace_length_mm: f64,
    pub output_trace_length_mm: f64,
    pub input_exact_violations: usize,
    pub output_exact_violations: usize,
    pub detail: String,
}

pub type ExactRoutePostProcessEvidence = ConflictActionPostProcessEvidence;

#[derive(Clone, Debug)]
pub struct ConflictActionPostProcessResult {
    pub candidate: CandidateArtifact,
    pub evidence: ConflictActionPostProcessEvidence,
}

pub type ExactRoutePostProcessResult = ConflictActionPostProcessResult;

/// Replaceable optimization boundary applied after a semantic action and
/// before the resulting candidate is fingerprinted and queued.
pub trait ConflictActionPostProcessor {
    fn name(&self) -> &'static str;

    fn process(
        &self,
        problem: &Problem,
        candidate: &CandidateArtifact,
    ) -> Result<ConflictActionPostProcessResult, String>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NoConflictActionPostProcessor;

impl ConflictActionPostProcessor for NoConflictActionPostProcessor {
    fn name(&self) -> &'static str {
        "none"
    }

    fn process(
        &self,
        _problem: &Problem,
        candidate: &CandidateArtifact,
    ) -> Result<ConflictActionPostProcessResult, String> {
        Ok(ConflictActionPostProcessResult {
            candidate: candidate.clone(),
            evidence: ConflictActionPostProcessEvidence {
                processor: self.name().into(),
                status: "not_run".into(),
                attempted_operations: 0,
                accepted_operations: 0,
                exact_validation_calls: 0,
                input_trace_length_mm: candidate_trace_length(candidate),
                output_trace_length_mm: candidate_trace_length(candidate),
                input_exact_violations: 0,
                output_exact_violations: 0,
                detail: "post-action optimization disabled".into(),
            },
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ExactLineOfSightPostProcessor {
    pub maximum_shortcut_attempts: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct ExactVertexPullPostProcessor {
    pub maximum_vertex_trials: usize,
    pub sweeps: usize,
    pub binary_steps: usize,
}

pub fn post_process_route_exact(
    problem: &Problem,
    candidate: &CandidateArtifact,
    config: &ExactRoutePostProcessConfig,
) -> Result<ExactRoutePostProcessResult, String> {
    config.check()?;
    match config {
        ExactRoutePostProcessConfig::None => {
            NoConflictActionPostProcessor.process(problem, candidate)
        }
        ExactRoutePostProcessConfig::ExactLineOfSight {
            maximum_shortcut_attempts,
        } => ExactLineOfSightPostProcessor {
            maximum_shortcut_attempts: *maximum_shortcut_attempts,
        }
        .process(problem, candidate),
        ExactRoutePostProcessConfig::ExactVertexPull {
            maximum_vertex_trials,
            sweeps,
            binary_steps,
        } => ExactVertexPullPostProcessor {
            maximum_vertex_trials: *maximum_vertex_trials,
            sweeps: *sweeps,
            binary_steps: *binary_steps,
        }
        .process(problem, candidate),
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct AxisAlignedSixActionProducer;

#[derive(Clone, Debug, Serialize)]
pub struct ConflictActionScore {
    pub trace_length_mm: f64,
    pub via_count: usize,
    pub bend_count: usize,
    pub via_penalty_mm: f64,
    pub path_cost_mm: f64,
    pub remaining_exact_violations: usize,
    pub heuristic_cost_mm: f64,
    pub estimated_total_cost_mm: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct ConflictActionAttempt {
    pub state_id: String,
    pub parent_state_id: String,
    pub depth: usize,
    pub action: ConflictAction,
    pub score: ConflictActionScore,
    pub validation: ExactValidationAssessment,
    pub candidate: CandidateArtifact,
}

#[derive(Clone, Debug, Serialize)]
pub struct ConflictActionSearchEvidence {
    pub strategy: String,
    pub producer: String,
    pub config: ConflictActionSearchConfig,
    pub source_validation: ExactValidationAssessment,
    pub selected_attempt: Option<usize>,
    pub generated_actions: usize,
    pub exact_complete_actions: usize,
    pub maximum_depth: usize,
    pub complete: bool,
    pub limitation: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ConflictActionSearchResult {
    pub source_candidate: CandidateArtifact,
    pub candidate: CandidateArtifact,
    pub validation: ExactValidationAssessment,
    pub evidence: ConflictActionSearchEvidence,
    pub attempts: Vec<ConflictActionAttempt>,
}

#[derive(Clone, Debug, Serialize)]
pub struct BestFirstConflictActionState {
    pub id: String,
    pub state_id: String,
    pub parent_state_id: Option<String>,
    pub depth: usize,
    pub action: Option<ConflictAction>,
    pub action_history: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post_action: Option<ConflictActionPostProcessEvidence>,
    pub fingerprint: String,
    pub score: ConflictActionScore,
    pub validation: ExactValidationAssessment,
    pub candidate: CandidateArtifact,
    pub expansion_error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ConflictActionTransition {
    pub parent_state_id: String,
    pub action: ConflictAction,
    pub outcome: String,
    pub child_state_id: Option<String>,
    pub duplicate_of_state_id: Option<String>,
    pub fingerprint: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct BestFirstConflictActionSearchEvidence {
    pub strategy: String,
    pub producer: String,
    pub post_processor: String,
    pub config: BestFirstConflictActionSearchConfig,
    pub source_validation: ExactValidationAssessment,
    pub selected_attempt: Option<usize>,
    pub expanded_states: usize,
    pub generated_transitions: usize,
    pub unique_states: usize,
    pub duplicate_states: usize,
    pub exact_complete_states: usize,
    pub depth_rejections: usize,
    pub generated_state_budget_reached: bool,
    pub expanded_state_budget_reached: bool,
    pub search_exhausted: bool,
    pub complete: bool,
    pub limitation: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct BestFirstConflictActionSearchResult {
    pub source_candidate: CandidateArtifact,
    pub candidate: CandidateArtifact,
    pub validation: ExactValidationAssessment,
    pub evidence: BestFirstConflictActionSearchEvidence,
    pub attempts: Vec<BestFirstConflictActionState>,
    pub transitions: Vec<ConflictActionTransition>,
}

/// Compile the declared two-terminal seed routes into the durable candidate
/// boundary. This is the explicit "ratsnest/seed state" for conflict-action
/// experiments; it is allowed to be geometrically invalid but must remain
/// electrically complete.
pub fn candidate_from_declared_seed_routes(problem: &Problem) -> Result<CandidateArtifact, String> {
    problem.check_schema()?;
    if !problem.electrical_nets.is_empty() {
        return Err(
            "declared-seed candidate currently supports legacy two-terminal nets only".into(),
        );
    }
    let components = problem
        .components
        .iter()
        .map(|component| SolvedComponent {
            id: component.id.clone(),
            position: component.position,
            size: component.size,
            rotation_degrees: component.rotation_degrees,
        })
        .collect::<Vec<_>>();
    let mut traces = Vec::with_capacity(problem.nets.len());
    let mut graph_nodes = BTreeMap::<String, BTreeMap<String, SolvedRouteNode>>::new();
    let mut graph_branches = BTreeMap::<String, BTreeSet<String>>::new();
    for net in &problem.nets {
        let electrical_net = net.electrical_net.as_deref().unwrap_or(&net.id).to_string();
        let from = declared_pin_position(problem, &net.from.component, &net.from.pin)?;
        let to = declared_pin_position(problem, &net.to.component, &net.to.pin)?;
        let mut points = if net.seed_route.is_empty() {
            vec![from, to]
        } else {
            net.seed_route.clone()
        };
        if points.len() < 2 {
            return Err(format!("seed route {} needs at least two points", net.id));
        }
        points[0] = from;
        let last = points.len() - 1;
        points[last] = to;
        let from_node = terminal_node_id(&electrical_net, &net.from.component, &net.from.pin);
        let to_node = terminal_node_id(&electrical_net, &net.to.component, &net.to.pin);
        let trace = SolvedTrace {
            branch: net.id.clone(),
            electrical_net: electrical_net.clone(),
            net: net.id.clone(),
            from_node: from_node.clone(),
            to_node: to_node.clone(),
            width: net.width,
            tension_weight: net.tension_weight,
            layer: net.layer.clone(),
            segment_layers: vec![net.layer.clone(); points.len() - 1],
            vias: Vec::new(),
            points,
            route_class: RouteClass::direct(
                net.layer.clone(),
                TerminalSector {
                    component: net.from.component.clone(),
                    sector: net.from.pin.clone(),
                },
                TerminalSector {
                    component: net.to.component.clone(),
                    sector: net.to.pin.clone(),
                },
            ),
            route_basis_fingerprint: None,
        };
        for (id, component, pin, position) in [
            (&from_node, &net.from.component, &net.from.pin, from),
            (&to_node, &net.to.component, &net.to.pin, to),
        ] {
            let nodes = graph_nodes.entry(electrical_net.clone()).or_default();
            let node = nodes.entry(id.clone()).or_insert_with(|| SolvedRouteNode {
                id: id.clone(),
                electrical_net: electrical_net.clone(),
                position,
                incident_branches: Vec::new(),
                kind: SolvedRouteNodeKind::Terminal {
                    component: component.clone(),
                    pin: pin.clone(),
                },
            });
            node.incident_branches.push(net.id.clone());
            node.incident_branches.sort();
            node.incident_branches.dedup();
        }
        graph_branches
            .entry(electrical_net)
            .or_default()
            .insert(net.id.clone());
        traces.push(trace);
    }
    let route_graphs = graph_nodes
        .into_iter()
        .map(|(electrical_net, nodes)| SolvedRouteGraph {
            branches: graph_branches
                .remove(&electrical_net)
                .unwrap_or_default()
                .into_iter()
                .collect(),
            electrical_net,
            nodes: nodes.into_values().collect(),
        })
        .collect();
    Ok(CandidateArtifact {
        schema_version: CANDIDATE_SCHEMA_VERSION,
        components,
        traces,
        route_graphs,
    })
}

/// Evaluate one complete six-action frontier and select the cheapest exact
/// state. This is deliberately depth one: the state/evidence contract is ready
/// for best-first expansion, but no multi-conflict A* claim is made yet.
pub fn explore_one_conflict_with<P: ConflictActionProducer>(
    problem: &Problem,
    source: &CandidateArtifact,
    config: &ConflictActionSearchConfig,
    producer: &P,
) -> Result<ConflictActionSearchResult, String> {
    problem.check_schema()?;
    config.check()?;
    let source_validation = validate_candidate(problem, source)?;
    if !source_validation.electrical.complete {
        return Err("conflict-action source must be electrically complete".into());
    }
    let conflicts = source_validation
        .geometry
        .violations
        .iter()
        .filter(|violation| violation.code == "trace_trace_clearance")
        .collect::<Vec<_>>();
    if conflicts.len() != 1 || source_validation.geometry.violations.len() != 1 {
        return Err(format!(
            "depth-one conflict action requires exactly one trace/trace violation; found {} trace conflicts and {} total geometry violations",
            conflicts.len(),
            source_validation.geometry.violations.len()
        ));
    }
    let proposals = producer.propose(problem, source, conflicts[0], config)?;
    if proposals.is_empty() {
        return Err("conflict-action producer returned no alternatives".into());
    }
    let mut attempts = Vec::new();
    for (index, proposal) in proposals
        .into_iter()
        .take(config.maximum_actions)
        .enumerate()
    {
        let validation = validate_candidate(problem, &proposal.candidate)?;
        let score = score_candidate(&proposal.candidate, &validation, config);
        attempts.push(ConflictActionAttempt {
            state_id: format!("depth-0001-state-{index:04}"),
            parent_state_id: "seed".into(),
            depth: 1,
            action: proposal.action,
            score,
            validation,
            candidate: proposal.candidate,
        });
    }
    let selected_attempt = attempts
        .iter()
        .enumerate()
        .filter(|(_, attempt)| attempt.validation.complete)
        .min_by(|(_, left), (_, right)| {
            left.score
                .estimated_total_cost_mm
                .total_cmp(&right.score.estimated_total_cost_mm)
                .then_with(|| left.action.id.cmp(&right.action.id))
        })
        .map(|(index, _)| index);
    let (candidate, validation) = selected_attempt.map_or_else(
        || (source.clone(), source_validation.clone()),
        |index| {
            (
                attempts[index].candidate.clone(),
                attempts[index].validation.clone(),
            )
        },
    );
    let exact_complete_actions = attempts
        .iter()
        .filter(|attempt| attempt.validation.complete)
        .count();
    Ok(ConflictActionSearchResult {
        source_candidate: source.clone(),
        candidate,
        validation,
        evidence: ConflictActionSearchEvidence {
            strategy: "depth-one-discrete-conflict-frontier-v1".into(),
            producer: producer.name().into(),
            config: config.clone(),
            source_validation,
            selected_attempt,
            generated_actions: attempts.len(),
            exact_complete_actions,
            maximum_depth: 1,
            complete: selected_attempt.is_some(),
            limitation: "one orthogonal straight-trace conflict only; no recursive best-first expansion, component motion, or post-action push/pull".into(),
        },
        attempts,
    })
}

pub fn explore_one_axis_aligned_conflict(
    problem: &Problem,
    source: &CandidateArtifact,
    config: &ConflictActionSearchConfig,
) -> Result<ConflictActionSearchResult, String> {
    explore_one_conflict_with(problem, source, config, &AxisAlignedSixActionProducer)
}

/// Explore a bounded graph of complete candidates. Actions are proposed for
/// several unresolved conflicts, while validation, ranking, deduplication, and
/// selection remain strategy-independent.
pub fn search_conflict_actions_best_first_with<P: ConflictActionProducer>(
    problem: &Problem,
    source: &CandidateArtifact,
    config: &BestFirstConflictActionSearchConfig,
    producer: &P,
) -> Result<BestFirstConflictActionSearchResult, String> {
    search_conflict_actions_best_first_with_post_processor(
        problem,
        source,
        config,
        producer,
        &NoConflictActionPostProcessor,
    )
}

pub fn search_conflict_actions_best_first_with_post_processor<
    P: ConflictActionProducer,
    O: ConflictActionPostProcessor,
>(
    problem: &Problem,
    source: &CandidateArtifact,
    config: &BestFirstConflictActionSearchConfig,
    producer: &P,
    post_processor: &O,
) -> Result<BestFirstConflictActionSearchResult, String> {
    problem.check_schema()?;
    config.check()?;
    let source_validation = validate_candidate(problem, source)?;
    if !source_validation.electrical.complete {
        return Err("conflict-action source must be electrically complete".into());
    }

    let source_fingerprint = candidate_fingerprint(source)?;
    let mut attempts = vec![BestFirstConflictActionState {
        id: "seed".into(),
        state_id: "seed".into(),
        parent_state_id: None,
        depth: 0,
        action: None,
        action_history: Vec::new(),
        post_action: None,
        fingerprint: source_fingerprint.clone(),
        score: score_candidate(source, &source_validation, &config.action),
        validation: source_validation.clone(),
        candidate: source.clone(),
        expansion_error: None,
    }];
    let mut open = vec![0_usize];
    let mut seen = BTreeMap::from([(source_fingerprint, "seed".to_string())]);
    let mut transitions = Vec::new();
    let mut expanded_states = 0_usize;
    let mut duplicate_states = 0_usize;
    let mut depth_rejections = 0_usize;
    let mut generated_state_budget_reached = false;
    let mut expanded_state_budget_reached = false;

    while !open.is_empty() {
        let queue_position = best_open_position(&open, &attempts);
        let state_index = open.remove(queue_position);
        if attempts[state_index].validation.complete {
            continue;
        }
        if attempts[state_index].depth >= config.maximum_depth {
            depth_rejections += 1;
            continue;
        }
        if expanded_states >= config.maximum_expanded_states {
            expanded_state_budget_reached = true;
            break;
        }

        let unsupported = attempts[state_index]
            .validation
            .geometry
            .violations
            .iter()
            .filter(|violation| violation.code != "trace_trace_clearance")
            .map(|violation| violation.code.clone())
            .collect::<BTreeSet<_>>();
        if !unsupported.is_empty() {
            attempts[state_index].expansion_error = Some(format!(
                "unsupported exact geometry findings: {}",
                unsupported.into_iter().collect::<Vec<_>>().join(", ")
            ));
            continue;
        }
        let conflicts = attempts[state_index]
            .validation
            .geometry
            .violations
            .iter()
            .filter(|violation| violation.code == "trace_trace_clearance")
            .take(config.maximum_conflicts_per_state)
            .cloned()
            .collect::<Vec<_>>();
        if conflicts.is_empty() {
            attempts[state_index].expansion_error =
                Some("no supported trace/trace conflict remained to expand".into());
            continue;
        }
        expanded_states += 1;

        for conflict in conflicts {
            let proposals = match producer.propose(
                problem,
                &attempts[state_index].candidate,
                &conflict,
                &config.action,
            ) {
                Ok(proposals) => proposals,
                Err(error) => {
                    let detail = format!("{}: {error}", conflict.objects.join(" / "));
                    attempts[state_index].expansion_error.get_or_insert(detail);
                    continue;
                }
            };
            for proposal in proposals.into_iter().take(config.action.maximum_actions) {
                if transitions.len() >= config.maximum_generated_states {
                    generated_state_budget_reached = true;
                    break;
                }
                let raw_validation = validate_candidate(problem, &proposal.candidate)?;
                let processed = match post_processor.process(problem, &proposal.candidate) {
                    Ok(processed) => processed,
                    Err(error) => ConflictActionPostProcessResult {
                        candidate: proposal.candidate.clone(),
                        evidence: ConflictActionPostProcessEvidence {
                            processor: post_processor.name().into(),
                            status: "error".into(),
                            attempted_operations: 0,
                            accepted_operations: 0,
                            exact_validation_calls: 0,
                            input_trace_length_mm: candidate_trace_length(&proposal.candidate),
                            output_trace_length_mm: candidate_trace_length(&proposal.candidate),
                            input_exact_violations: raw_validation.violations.len(),
                            output_exact_violations: raw_validation.violations.len(),
                            detail: error,
                        },
                    },
                };
                let validation = validate_candidate(problem, &processed.candidate)?;
                let fingerprint = candidate_fingerprint(&processed.candidate)?;
                if let Some(existing) = seen.get(&fingerprint) {
                    duplicate_states += 1;
                    transitions.push(ConflictActionTransition {
                        parent_state_id: attempts[state_index].state_id.clone(),
                        action: proposal.action,
                        outcome: "duplicate".into(),
                        child_state_id: None,
                        duplicate_of_state_id: Some(existing.clone()),
                        fingerprint,
                    });
                    continue;
                }
                let depth = attempts[state_index].depth + 1;
                let child_index = attempts.len();
                let state_id = format!("depth-{depth:04}-state-{child_index:04}");
                let mut action_history = attempts[state_index].action_history.clone();
                action_history.push(proposal.action.id.clone());
                seen.insert(fingerprint.clone(), state_id.clone());
                transitions.push(ConflictActionTransition {
                    parent_state_id: attempts[state_index].state_id.clone(),
                    action: proposal.action.clone(),
                    outcome: "retained".into(),
                    child_state_id: Some(state_id.clone()),
                    duplicate_of_state_id: None,
                    fingerprint: fingerprint.clone(),
                });
                attempts.push(BestFirstConflictActionState {
                    id: state_id.clone(),
                    state_id,
                    parent_state_id: Some(attempts[state_index].state_id.clone()),
                    depth,
                    action: Some(proposal.action),
                    action_history,
                    post_action: (post_processor.name() != "none").then_some(processed.evidence),
                    fingerprint,
                    score: score_candidate(&processed.candidate, &validation, &config.action),
                    validation,
                    candidate: processed.candidate,
                    expansion_error: None,
                });
                open.push(child_index);
            }
            if generated_state_budget_reached {
                break;
            }
        }
        if generated_state_budget_reached {
            break;
        }
    }

    let selected_attempt = attempts
        .iter()
        .enumerate()
        .filter(|(_, state)| state.validation.complete)
        .min_by(|(_, left), (_, right)| compare_states(left, right))
        .map(|(index, _)| index)
        .or_else(|| {
            attempts
                .iter()
                .enumerate()
                .min_by(|(_, left), (_, right)| compare_states(left, right))
                .map(|(index, _)| index)
        });
    let selected = selected_attempt.unwrap_or(0);
    let exact_complete_states = attempts
        .iter()
        .filter(|state| state.validation.complete)
        .count();
    let search_exhausted = open.is_empty()
        && !generated_state_budget_reached
        && !expanded_state_budget_reached
        && depth_rejections == 0;
    Ok(BestFirstConflictActionSearchResult {
        source_candidate: source.clone(),
        candidate: attempts[selected].candidate.clone(),
        validation: attempts[selected].validation.clone(),
        evidence: BestFirstConflictActionSearchEvidence {
            strategy: "bounded-best-first-discrete-conflict-search-v1".into(),
            producer: producer.name().into(),
            post_processor: post_processor.name().into(),
            config: config.clone(),
            source_validation,
            selected_attempt,
            expanded_states,
            generated_transitions: transitions.len(),
            unique_states: attempts.len(),
            duplicate_states,
            exact_complete_states,
            depth_rejections,
            generated_state_budget_reached,
            expanded_state_budget_reached,
            search_exhausted,
            complete: attempts[selected].validation.complete,
            limitation: format!(
                "bounded search over producer-supplied actions; the current heuristic is not proven admissible, so this makes no global-optimality claim; post-processor {} does not provide general geometric rerouting or component motion",
                post_processor.name()
            ),
        },
        attempts,
        transitions,
    })
}

pub fn search_axis_aligned_conflicts_best_first(
    problem: &Problem,
    source: &CandidateArtifact,
    config: &BestFirstConflictActionSearchConfig,
) -> Result<BestFirstConflictActionSearchResult, String> {
    match config.post_action {
        ConflictActionPostProcessConfig::None => search_conflict_actions_best_first_with(
            problem,
            source,
            config,
            &AxisAlignedSixActionProducer,
        ),
        ConflictActionPostProcessConfig::ExactLineOfSight {
            maximum_shortcut_attempts,
        } => search_conflict_actions_best_first_with_post_processor(
            problem,
            source,
            config,
            &AxisAlignedSixActionProducer,
            &ExactLineOfSightPostProcessor {
                maximum_shortcut_attempts,
            },
        ),
        ConflictActionPostProcessConfig::ExactVertexPull {
            maximum_vertex_trials,
            sweeps,
            binary_steps,
        } => search_conflict_actions_best_first_with_post_processor(
            problem,
            source,
            config,
            &AxisAlignedSixActionProducer,
            &ExactVertexPullPostProcessor {
                maximum_vertex_trials,
                sweeps,
                binary_steps,
            },
        ),
    }
}

fn best_open_position(open: &[usize], states: &[BestFirstConflictActionState]) -> usize {
    open.iter()
        .enumerate()
        .min_by(|(_, left), (_, right)| compare_states(&states[**left], &states[**right]))
        .map(|(position, _)| position)
        .unwrap_or(0)
}

fn compare_states(
    left: &BestFirstConflictActionState,
    right: &BestFirstConflictActionState,
) -> std::cmp::Ordering {
    left.score
        .estimated_total_cost_mm
        .total_cmp(&right.score.estimated_total_cost_mm)
        .then_with(|| left.score.path_cost_mm.total_cmp(&right.score.path_cost_mm))
        .then_with(|| left.depth.cmp(&right.depth))
        .then_with(|| left.state_id.cmp(&right.state_id))
}

fn candidate_fingerprint(candidate: &CandidateArtifact) -> Result<String, String> {
    let encoded = serde_json::to_vec(candidate)
        .map_err(|error| format!("failed to fingerprint candidate: {error}"))?;
    Ok(format!("sha256:{:x}", Sha256::digest(encoded)))
}

impl ConflictActionPostProcessor for ExactLineOfSightPostProcessor {
    fn name(&self) -> &'static str {
        "exact-line-of-sight-v1"
    }

    fn process(
        &self,
        problem: &Problem,
        candidate: &CandidateArtifact,
    ) -> Result<ConflictActionPostProcessResult, String> {
        if !(1..=100_000).contains(&self.maximum_shortcut_attempts) {
            return Err(
                "exact line-of-sight maximum_shortcut_attempts must be between 1 and 100000".into(),
            );
        }
        let input_validation = validate_candidate(problem, candidate)?;
        if !input_validation.electrical.complete {
            return Err("post-action shortener requires electrically complete input".into());
        }
        let input_trace_length_mm = candidate_trace_length(candidate);
        let mut current = candidate.clone();
        let mut current_validation = input_validation.clone();
        let mut current_length = input_trace_length_mm;
        let mut attempted_shortcuts = 0_usize;
        let mut accepted_shortcuts = 0_usize;

        'search: loop {
            let mut accepted_one = false;
            for trace_index in 0..current.traces.len() {
                if !current.traces[trace_index].vias.is_empty()
                    || current.traces[trace_index].points.len() < 3
                {
                    continue;
                }
                let point_count = current.traces[trace_index].points.len();
                for span in (2..point_count).rev() {
                    for from in 0..point_count - span {
                        if attempted_shortcuts >= self.maximum_shortcut_attempts {
                            break 'search;
                        }
                        let to = from + span;
                        let segment_layers = &current.traces[trace_index].segment_layers[from..to];
                        if segment_layers
                            .iter()
                            .any(|layer| layer != &segment_layers[0])
                        {
                            continue;
                        }
                        attempted_shortcuts += 1;
                        let mut trial = current.clone();
                        let replacement_layer = segment_layers[0].clone();
                        trial.traces[trace_index].points.drain(from + 1..to);
                        trial.traces[trace_index]
                            .segment_layers
                            .splice(from..to, std::iter::once(replacement_layer));
                        trial.traces[trace_index].route_basis_fingerprint = None;
                        let trial_validation = validate_candidate(problem, &trial)?;
                        let trial_length = candidate_trace_length(&trial);
                        if post_action_improves(
                            &current_validation,
                            current_length,
                            &trial_validation,
                            trial_length,
                        ) {
                            current = trial;
                            current_validation = trial_validation;
                            current_length = trial_length;
                            accepted_shortcuts += 1;
                            accepted_one = true;
                            break;
                        }
                    }
                    if accepted_one {
                        break;
                    }
                }
                if accepted_one {
                    break;
                }
            }
            if !accepted_one {
                break;
            }
        }

        let status = if accepted_shortcuts == 0 {
            "no_change"
        } else {
            "improved"
        };
        Ok(ConflictActionPostProcessResult {
            candidate: current,
            evidence: ConflictActionPostProcessEvidence {
                processor: self.name().into(),
                status: status.into(),
                attempted_operations: attempted_shortcuts,
                accepted_operations: accepted_shortcuts,
                exact_validation_calls: attempted_shortcuts + 1,
                input_trace_length_mm,
                output_trace_length_mm: current_length,
                input_exact_violations: input_validation.violations.len(),
                output_exact_violations: current_validation.violations.len(),
                detail: if accepted_shortcuts == 0 {
                    "no exact-safe same-layer shortcut was found".into()
                } else {
                    format!(
                        "accepted {accepted_shortcuts} exact-safe shortcut(s), saving {:.6} mm",
                        input_trace_length_mm - current_length
                    )
                },
            },
        })
    }
}

impl ConflictActionPostProcessor for ExactVertexPullPostProcessor {
    fn name(&self) -> &'static str {
        "exact-vertex-pull-v1"
    }

    fn process(
        &self,
        problem: &Problem,
        candidate: &CandidateArtifact,
    ) -> Result<ConflictActionPostProcessResult, String> {
        ConflictActionPostProcessConfig::ExactVertexPull {
            maximum_vertex_trials: self.maximum_vertex_trials,
            sweeps: self.sweeps,
            binary_steps: self.binary_steps,
        }
        .check()?;
        let input_validation = validate_candidate(problem, candidate)?;
        if !input_validation.electrical.complete {
            return Err("post-action vertex pull requires electrically complete input".into());
        }
        let input_trace_length_mm = candidate_trace_length(candidate);
        let mut current = candidate.clone();
        let mut current_validation = input_validation.clone();
        let mut current_length = input_trace_length_mm;
        let mut attempted_vertices = 0_usize;
        let mut accepted_vertices = 0_usize;

        'sweeps: for _ in 0..self.sweeps {
            let mut accepted_in_sweep = false;
            for trace_index in 0..current.traces.len() {
                if !current.traces[trace_index].vias.is_empty()
                    || current.traces[trace_index].points.len() < 3
                {
                    continue;
                }
                for point_index in 1..current.traces[trace_index].points.len() - 1 {
                    if attempted_vertices >= self.maximum_vertex_trials {
                        break 'sweeps;
                    }
                    if current.traces[trace_index].segment_layers[point_index - 1]
                        != current.traces[trace_index].segment_layers[point_index]
                    {
                        continue;
                    }
                    attempted_vertices += 1;
                    let previous = current.traces[trace_index].points[point_index - 1];
                    let position = current.traces[trace_index].points[point_index];
                    let next = current.traces[trace_index].points[point_index + 1];
                    let target =
                        Vec2::new((previous.x + next.x) * 0.5, (previous.y + next.y) * 0.5);
                    let mut lower = 0.0_f64;
                    let mut upper = 1.0_f64;
                    let mut best = None;
                    for _ in 0..self.binary_steps {
                        let fraction = (lower + upper) * 0.5;
                        let mut trial = current.clone();
                        trial.traces[trace_index].points[point_index] = Vec2::new(
                            position.x + (target.x - position.x) * fraction,
                            position.y + (target.y - position.y) * fraction,
                        );
                        trial.traces[trace_index].route_basis_fingerprint = None;
                        let trial_validation = validate_candidate(problem, &trial)?;
                        let trial_length = candidate_trace_length(&trial);
                        if post_action_improves(
                            &current_validation,
                            current_length,
                            &trial_validation,
                            trial_length,
                        ) {
                            lower = fraction;
                            best = Some((trial, trial_validation, trial_length));
                        } else {
                            upper = fraction;
                        }
                    }
                    if let Some((trial, validation, length)) = best {
                        current = trial;
                        current_validation = validation;
                        current_length = length;
                        accepted_vertices += 1;
                        accepted_in_sweep = true;
                    }
                }
            }
            if !accepted_in_sweep {
                break;
            }
        }

        Ok(ConflictActionPostProcessResult {
            candidate: current,
            evidence: ConflictActionPostProcessEvidence {
                processor: self.name().into(),
                status: if accepted_vertices == 0 {
                    "no_change".into()
                } else {
                    "improved".into()
                },
                attempted_operations: attempted_vertices,
                accepted_operations: accepted_vertices,
                exact_validation_calls: attempted_vertices * self.binary_steps + 1,
                input_trace_length_mm,
                output_trace_length_mm: current_length,
                input_exact_violations: input_validation.violations.len(),
                output_exact_violations: current_validation.violations.len(),
                detail: if accepted_vertices == 0 {
                    "no exact-safe interior vertex pull was found".into()
                } else {
                    format!(
                        "accepted {accepted_vertices} exact-safe vertex pull(s), saving {:.6} mm",
                        input_trace_length_mm - current_length
                    )
                },
            },
        })
    }
}

fn post_action_improves(
    current: &ExactValidationAssessment,
    current_length: f64,
    proposed: &ExactValidationAssessment,
    proposed_length: f64,
) -> bool {
    if !proposed.electrical.complete || proposed_length >= current_length - 1.0e-9 {
        return false;
    }
    let current_findings = geometry_finding_keys(current);
    let proposed_findings = geometry_finding_keys(proposed);
    proposed_findings.is_subset(&current_findings)
}

fn geometry_finding_keys(
    validation: &ExactValidationAssessment,
) -> BTreeSet<(String, Vec<String>)> {
    validation
        .geometry
        .violations
        .iter()
        .map(|violation| {
            let mut objects = violation.objects.clone();
            objects.sort();
            (violation.code.clone(), objects)
        })
        .collect()
}

fn candidate_trace_length(candidate: &CandidateArtifact) -> f64 {
    candidate
        .traces
        .iter()
        .flat_map(|trace| trace.points.windows(2))
        .map(|segment| length(sub(segment[1], segment[0])))
        .sum()
}

impl ConflictActionProducer for AxisAlignedSixActionProducer {
    fn name(&self) -> &'static str {
        "axis-aligned-six-action-v1"
    }

    fn propose(
        &self,
        problem: &Problem,
        source: &CandidateArtifact,
        conflict: &Violation,
        config: &ConflictActionSearchConfig,
    ) -> Result<Vec<ConflictActionProposal>, String> {
        if conflict.objects.len() != 2 {
            return Err("trace/trace conflict must name exactly two branches".into());
        }
        let first = source
            .traces
            .iter()
            .find(|trace| trace.branch == conflict.objects[0])
            .ok_or_else(|| format!("missing conflicted branch {}", conflict.objects[0]))?;
        let second = source
            .traces
            .iter()
            .find(|trace| trace.branch == conflict.objects[1])
            .ok_or_else(|| format!("missing conflicted branch {}", conflict.objects[1]))?;
        let first_axis = straight_axis(first)?;
        let second_axis = straight_axis(second)?;
        if first_axis == second_axis || first.segment_layers[0] != second.segment_layers[0] {
            return Err(
                "axis-aligned producer requires orthogonal straight traces on one layer".into(),
            );
        }
        let (horizontal, vertical) = if first_axis == Axis::Horizontal {
            (first, second)
        } else {
            (second, first)
        };
        let crossing = Vec2::new(vertical.points[0].x, horizontal.points[0].y);
        if !strictly_between(crossing.x, horizontal.points[0].x, horizontal.points[1].x)
            || !strictly_between(crossing.y, vertical.points[0].y, vertical.points[1].y)
        {
            return Err("orthogonal traces do not cross in both segment interiors".into());
        }

        let mut proposals = Vec::with_capacity(6);
        for side in [CardinalDetourSide::North, CardinalDetourSide::South] {
            proposals.push(planar_detour_proposal(
                source, horizontal, vertical, crossing, side, config,
            )?);
        }
        for side in [CardinalDetourSide::West, CardinalDetourSide::East] {
            proposals.push(planar_detour_proposal(
                source, vertical, horizontal, crossing, side, config,
            )?);
        }
        proposals.push(layer_escape_proposal(
            problem, source, horizontal, vertical, crossing, config,
        )?);
        proposals.push(layer_escape_proposal(
            problem, source, vertical, horizontal, crossing, config,
        )?);
        Ok(proposals)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Axis {
    Horizontal,
    Vertical,
}

fn straight_axis(trace: &SolvedTrace) -> Result<Axis, String> {
    if trace.points.len() != 2
        || trace.segment_layers.len() != 1
        || !trace.vias.is_empty()
        || trace.points[0] == trace.points[1]
    {
        return Err(format!(
            "branch {} is not one straight via-free segment",
            trace.branch
        ));
    }
    let delta = sub(trace.points[1], trace.points[0]);
    if delta.y.abs() <= AXIS_EPSILON {
        Ok(Axis::Horizontal)
    } else if delta.x.abs() <= AXIS_EPSILON {
        Ok(Axis::Vertical)
    } else {
        Err(format!("branch {} is not axis-aligned", trace.branch))
    }
}

fn strictly_between(value: f64, first: f64, second: f64) -> bool {
    value > first.min(second) + AXIS_EPSILON && value < first.max(second) - AXIS_EPSILON
}

fn planar_detour_proposal(
    source: &CandidateArtifact,
    moving: &SolvedTrace,
    blocking: &SolvedTrace,
    crossing: Vec2,
    side: CardinalDetourSide,
    config: &ConflictActionSearchConfig,
) -> Result<ConflictActionProposal, String> {
    let axis = straight_axis(moving)?;
    let mut points = vec![moving.points[0]];
    match (axis, side) {
        (Axis::Horizontal, CardinalDetourSide::North | CardinalDetourSide::South) => {
            let y = match side {
                CardinalDetourSide::North => {
                    blocking.points[0].y.min(blocking.points[1].y) - config.detour_clearance_mm
                }
                CardinalDetourSide::South => {
                    blocking.points[0].y.max(blocking.points[1].y) + config.detour_clearance_mm
                }
                _ => unreachable!(),
            };
            let direction = (moving.points[1].x - moving.points[0].x).signum();
            points.push(Vec2::new(crossing.x - direction * config.detour_span_mm, y));
            points.push(Vec2::new(crossing.x + direction * config.detour_span_mm, y));
        }
        (Axis::Vertical, CardinalDetourSide::West | CardinalDetourSide::East) => {
            let x = match side {
                CardinalDetourSide::West => {
                    blocking.points[0].x.min(blocking.points[1].x) - config.detour_clearance_mm
                }
                CardinalDetourSide::East => {
                    blocking.points[0].x.max(blocking.points[1].x) + config.detour_clearance_mm
                }
                _ => unreachable!(),
            };
            let direction = (moving.points[1].y - moving.points[0].y).signum();
            points.push(Vec2::new(x, crossing.y - direction * config.detour_span_mm));
            points.push(Vec2::new(x, crossing.y + direction * config.detour_span_mm));
        }
        _ => return Err("detour side is incompatible with moving trace orientation".into()),
    }
    points.push(moving.points[1]);
    let mut replacement = moving.clone();
    replacement.points = points;
    replacement.segment_layers = vec![moving.segment_layers[0].clone(); 3];
    replacement.vias.clear();
    replacement.route_basis_fingerprint = None;
    let mut candidate = source.clone();
    replace_trace(&mut candidate, replacement)?;
    let side_name = match side {
        CardinalDetourSide::North => "north",
        CardinalDetourSide::South => "south",
        CardinalDetourSide::West => "west",
        CardinalDetourSide::East => "east",
    };
    Ok(ConflictActionProposal {
        action: ConflictAction {
            id: format!("{}_around_{}_{}", moving.branch, blocking.branch, side_name),
            action: ConflictActionKind::PlanarDetour {
                moving_branch: moving.branch.clone(),
                blocking_branch: blocking.branch.clone(),
                side,
            },
        },
        candidate,
    })
}

fn layer_escape_proposal(
    problem: &Problem,
    source: &CandidateArtifact,
    moving: &SolvedTrace,
    blocking: &SolvedTrace,
    crossing: Vec2,
    config: &ConflictActionSearchConfig,
) -> Result<ConflictActionProposal, String> {
    let axis = straight_axis(moving)?;
    let declared = problem
        .nets
        .iter()
        .find(|net| net.id == moving.branch)
        .ok_or_else(|| format!("missing declaration for branch {}", moving.branch))?;
    let primary = &moving.segment_layers[0];
    let escape_layer = declared
        .allowed_layers
        .iter()
        .find(|layer| *layer != primary)
        .cloned()
        .ok_or_else(|| format!("branch {} has no alternate copper layer", moving.branch))?;
    let direction = sub(moving.points[1], moving.points[0]);
    let magnitude = length(direction);
    let unit = Vec2::new(direction.x / magnitude, direction.y / magnitude);
    let before = Vec2::new(
        crossing.x - unit.x * config.via_offset_mm,
        crossing.y - unit.y * config.via_offset_mm,
    );
    let after = Vec2::new(
        crossing.x + unit.x * config.via_offset_mm,
        crossing.y + unit.y * config.via_offset_mm,
    );
    let reversed = match axis {
        Axis::Horizontal => direction.x < 0.0,
        Axis::Vertical => direction.y < 0.0,
    };
    let ordered = if reversed {
        [after, before]
    } else {
        [before, after]
    };
    let mut replacement = moving.clone();
    replacement.points = vec![moving.points[0], ordered[0], ordered[1], moving.points[1]];
    replacement.segment_layers = vec![primary.clone(), escape_layer.clone(), primary.clone()];
    replacement.vias = vec![
        SolvedVia {
            position: ordered[0],
            from_layer: primary.clone(),
            to_layer: escape_layer.clone(),
            diameter: problem.rules.via_diameter,
            drill: problem.rules.via_drill,
            point_index: 1,
        },
        SolvedVia {
            position: ordered[1],
            from_layer: escape_layer.clone(),
            to_layer: primary.clone(),
            diameter: problem.rules.via_diameter,
            drill: problem.rules.via_drill,
            point_index: 2,
        },
    ];
    replacement.route_basis_fingerprint = None;
    let mut candidate = source.clone();
    replace_trace(&mut candidate, replacement)?;
    Ok(ConflictActionProposal {
        action: ConflictAction {
            id: format!("{}_via_{}", moving.branch, escape_layer),
            action: ConflictActionKind::LayerEscape {
                moving_branch: moving.branch.clone(),
                blocking_branch: blocking.branch.clone(),
                escape_layer,
            },
        },
        candidate,
    })
}

fn replace_trace(
    candidate: &mut CandidateArtifact,
    replacement: SolvedTrace,
) -> Result<(), String> {
    let trace = candidate
        .traces
        .iter_mut()
        .find(|trace| trace.branch == replacement.branch)
        .ok_or_else(|| format!("missing replacement branch {}", replacement.branch))?;
    *trace = replacement.clone();
    let graph = candidate
        .route_graphs
        .iter_mut()
        .find(|graph| graph.electrical_net == replacement.electrical_net)
        .ok_or_else(|| {
            format!(
                "missing route graph for electrical net {}",
                replacement.electrical_net
            )
        })?;
    graph.nodes.retain(|node| {
        !matches!(node.kind, SolvedRouteNodeKind::Via { .. })
            || node.incident_branches.as_slice() != [replacement.branch.as_str()]
    });
    graph
        .nodes
        .extend(replacement.vias.iter().map(|via| SolvedRouteNode {
            id: format!(
                "{}::conflict-action-via::{:04}",
                replacement.branch, via.point_index
            ),
            electrical_net: replacement.electrical_net.clone(),
            position: via.position,
            incident_branches: vec![replacement.branch.clone()],
            kind: SolvedRouteNodeKind::Via {
                point_index: via.point_index,
                from_layer: via.from_layer.clone(),
                to_layer: via.to_layer.clone(),
                diameter: via.diameter,
                drill: via.drill,
            },
        }));
    graph.nodes.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(())
}

fn score_candidate(
    candidate: &CandidateArtifact,
    validation: &ExactValidationAssessment,
    config: &ConflictActionSearchConfig,
) -> ConflictActionScore {
    let trace_length_mm = candidate_trace_length(candidate);
    let via_count = candidate
        .traces
        .iter()
        .map(|trace| trace.vias.len())
        .sum::<usize>();
    let bend_count = candidate
        .traces
        .iter()
        .map(|trace| {
            trace
                .points
                .windows(3)
                .filter(|points| {
                    let first = sub(points[1], points[0]);
                    let second = sub(points[2], points[1]);
                    (first.x * second.y - first.y * second.x).abs() > AXIS_EPSILON
                        || first.x * second.x + first.y * second.y <= 0.0
                })
                .count()
        })
        .sum();
    let path_cost_mm = trace_length_mm + via_count as f64 * config.via_penalty_mm;
    let remaining_exact_violations = validation.violations.len();
    let heuristic_cost_mm =
        remaining_exact_violations as f64 * config.unresolved_conflict_penalty_mm;
    ConflictActionScore {
        trace_length_mm,
        via_count,
        bend_count,
        via_penalty_mm: config.via_penalty_mm,
        path_cost_mm,
        remaining_exact_violations,
        heuristic_cost_mm,
        estimated_total_cost_mm: path_cost_mm + heuristic_cost_mm,
    }
}

fn declared_pin_position(
    problem: &Problem,
    component_id: &str,
    pin_id: &str,
) -> Result<Vec2, String> {
    let component = problem
        .components
        .iter()
        .find(|component| component.id == component_id)
        .ok_or_else(|| format!("unknown component {component_id}"))?;
    let pin = component
        .pins
        .iter()
        .find(|pin| pin.id == pin_id)
        .ok_or_else(|| format!("unknown pin {component_id}.{pin_id}"))?;
    Ok(add(
        component.position,
        rotate_degrees(pin.offset, component.rotation_degrees),
    ))
}

fn terminal_node_id(net: &str, component: &str, pin: &str) -> String {
    format!("terminal:{net}/{component}.{pin}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Problem {
        serde_json::from_str(include_str!(
            "../../../benchmarks/small/conflict-action-orthogonal-crossing.json"
        ))
        .unwrap()
    }

    fn two_crossing_fixture() -> Problem {
        serde_json::from_str(include_str!(
            "../../../benchmarks/small/conflict-action-two-crossings.json"
        ))
        .unwrap()
    }

    fn shared_trace_fixture() -> Problem {
        serde_json::from_str(include_str!(
            "../../../benchmarks/small/conflict-action-shared-trace.json"
        ))
        .unwrap()
    }

    #[test]
    fn declared_seed_is_electrically_complete_but_has_one_exact_crossing() {
        let problem = fixture();
        let candidate = candidate_from_declared_seed_routes(&problem).unwrap();
        let validation = validate_candidate(&problem, &candidate).unwrap();
        assert!(validation.electrical.complete);
        assert!(!validation.geometry.complete);
        assert_eq!(validation.geometry.violations.len(), 1);
        assert_eq!(
            validation.geometry.violations[0].code,
            "trace_trace_clearance"
        );
    }

    #[test]
    fn six_requested_actions_are_retained_and_exact_gated() {
        let problem = fixture();
        let source = candidate_from_declared_seed_routes(&problem).unwrap();
        let result = explore_one_axis_aligned_conflict(
            &problem,
            &source,
            &ConflictActionSearchConfig::default(),
        )
        .unwrap();
        assert_eq!(result.evidence.generated_actions, 6);
        assert_eq!(result.attempts.len(), 6);
        assert_eq!(result.evidence.exact_complete_actions, 6);
        assert!(result.evidence.complete);
        assert!(result.validation.complete);
        assert_eq!(
            result
                .attempts
                .iter()
                .filter(|attempt| matches!(
                    attempt.action.action,
                    ConflictActionKind::PlanarDetour { .. }
                ))
                .count(),
            4
        );
        assert_eq!(
            result
                .attempts
                .iter()
                .filter(|attempt| matches!(
                    attempt.action.action,
                    ConflictActionKind::LayerEscape { .. }
                ))
                .count(),
            2
        );
    }

    #[test]
    fn via_penalty_changes_the_selected_action_without_changing_exactness() {
        let problem = fixture();
        let source = candidate_from_declared_seed_routes(&problem).unwrap();
        let low_via_penalty = explore_one_axis_aligned_conflict(
            &problem,
            &source,
            &ConflictActionSearchConfig {
                via_penalty_mm: 1.0,
                ..ConflictActionSearchConfig::default()
            },
        )
        .unwrap();
        let high_via_penalty = explore_one_axis_aligned_conflict(
            &problem,
            &source,
            &ConflictActionSearchConfig {
                via_penalty_mm: 20.0,
                ..ConflictActionSearchConfig::default()
            },
        )
        .unwrap();
        fn selected(result: &ConflictActionSearchResult) -> &ConflictActionAttempt {
            &result.attempts[result.evidence.selected_attempt.unwrap()]
        }
        assert!(matches!(
            selected(&low_via_penalty).action.action,
            ConflictActionKind::LayerEscape { .. }
        ));
        assert!(matches!(
            selected(&high_via_penalty).action.action,
            ConflictActionKind::PlanarDetour { .. }
        ));
        assert!(selected(&low_via_penalty).validation.complete);
        assert!(selected(&high_via_penalty).validation.complete);
    }

    #[test]
    fn bounded_best_first_search_resolves_two_conflicts_and_deduplicates_action_order() {
        let problem = two_crossing_fixture();
        let source = candidate_from_declared_seed_routes(&problem).unwrap();
        let source_validation = validate_candidate(&problem, &source).unwrap();
        assert!(source_validation.electrical.complete);
        assert_eq!(source_validation.geometry.violations.len(), 2);
        assert!(
            source_validation
                .geometry
                .violations
                .iter()
                .all(|violation| violation.code == "trace_trace_clearance")
        );

        let result = search_axis_aligned_conflicts_best_first(
            &problem,
            &source,
            &BestFirstConflictActionSearchConfig::default(),
        )
        .unwrap();
        assert!(result.evidence.complete);
        assert!(result.validation.complete);
        assert_eq!(result.attempts[0].state_id, "seed");
        assert_eq!(result.evidence.expanded_states, 13);
        assert_eq!(result.evidence.generated_transitions, 84);
        assert_eq!(result.evidence.unique_states, 49);
        assert_eq!(result.evidence.duplicate_states, 36);
        assert_eq!(result.evidence.exact_complete_states, 36);
        assert!(result.evidence.search_exhausted);
        assert_eq!(
            result.attempts[result.evidence.selected_attempt.unwrap()].depth,
            2
        );
        assert_eq!(
            result.attempts[result.evidence.selected_attempt.unwrap()]
                .action_history
                .len(),
            2
        );
        assert!(result.evidence.duplicate_states > 0);
        assert!(
            result
                .transitions
                .iter()
                .any(|transition| transition.outcome == "duplicate")
        );
    }

    #[test]
    fn depth_bound_is_reported_instead_of_claiming_search_exhaustion() {
        let problem = two_crossing_fixture();
        let source = candidate_from_declared_seed_routes(&problem).unwrap();
        let result = search_axis_aligned_conflicts_best_first(
            &problem,
            &source,
            &BestFirstConflictActionSearchConfig {
                maximum_depth: 1,
                ..BestFirstConflictActionSearchConfig::default()
            },
        )
        .unwrap();
        assert!(!result.evidence.complete);
        assert!(!result.evidence.search_exhausted);
        assert!(result.evidence.depth_rejections > 0);
        assert_eq!(result.evidence.exact_complete_states, 0);
    }

    #[test]
    fn exact_vertex_pull_shortens_retained_states_without_changing_exact_frontier() {
        let problem = two_crossing_fixture();
        let source = candidate_from_declared_seed_routes(&problem).unwrap();
        let baseline = search_axis_aligned_conflicts_best_first(
            &problem,
            &source,
            &BestFirstConflictActionSearchConfig::default(),
        )
        .unwrap();
        let pulled = search_axis_aligned_conflicts_best_first(
            &problem,
            &source,
            &BestFirstConflictActionSearchConfig {
                post_action: ConflictActionPostProcessConfig::ExactVertexPull {
                    maximum_vertex_trials: 32,
                    sweeps: 4,
                    binary_steps: 12,
                },
                ..BestFirstConflictActionSearchConfig::default()
            },
        )
        .unwrap();
        assert!(pulled.validation.complete);
        assert_eq!(pulled.evidence.exact_complete_states, 36);
        assert_eq!(pulled.evidence.unique_states, 49);
        assert!(
            pulled.attempts[pulled.evidence.selected_attempt.unwrap()]
                .score
                .path_cost_mm
                < baseline.attempts[baseline.evidence.selected_attempt.unwrap()]
                    .score
                    .path_cost_mm
        );
        assert!(pulled.attempts.iter().any(|state| {
            state
                .post_action
                .as_ref()
                .is_some_and(|evidence| evidence.status == "improved")
        }));
    }

    #[test]
    fn shared_trace_control_retains_unsupported_action_orders_and_finds_valid_ones() {
        let problem = shared_trace_fixture();
        let source = candidate_from_declared_seed_routes(&problem).unwrap();
        let source_validation = validate_candidate(&problem, &source).unwrap();
        assert_eq!(source_validation.geometry.violations.len(), 2);
        let high_via_action = ConflictActionSearchConfig {
            via_penalty_mm: 20.0,
            ..ConflictActionSearchConfig::default()
        };
        let baseline = search_axis_aligned_conflicts_best_first(
            &problem,
            &source,
            &BestFirstConflictActionSearchConfig {
                action: high_via_action.clone(),
                ..BestFirstConflictActionSearchConfig::default()
            },
        )
        .unwrap();
        let pulled = search_axis_aligned_conflicts_best_first(
            &problem,
            &source,
            &BestFirstConflictActionSearchConfig {
                action: high_via_action,
                post_action: ConflictActionPostProcessConfig::ExactVertexPull {
                    maximum_vertex_trials: 32,
                    sweeps: 4,
                    binary_steps: 12,
                },
                ..BestFirstConflictActionSearchConfig::default()
            },
        )
        .unwrap();
        assert!(baseline.validation.complete);
        assert!(pulled.validation.complete);
        assert_eq!(baseline.evidence.exact_complete_states, 12);
        assert_eq!(pulled.evidence.exact_complete_states, 12);
        assert!(baseline.attempts.iter().any(|state| {
            state
                .expansion_error
                .as_deref()
                .is_some_and(|error| error.contains("not one straight via-free segment"))
        }));
        assert!(
            pulled.attempts[pulled.evidence.selected_attempt.unwrap()]
                .score
                .path_cost_mm
                < baseline.attempts[baseline.evidence.selected_attempt.unwrap()]
                    .score
                    .path_cost_mm
        );
    }
}
