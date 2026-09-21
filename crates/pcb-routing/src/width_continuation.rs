// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

use std::collections::{BTreeMap, BTreeSet};

use layout_trace_model::Problem;
use pcb_validate::{
    CandidateArtifact, ExactValidationAssessment, SolvedComponent, validate_candidate,
};
use serde::{Deserialize, Serialize};

use crate::{
    ContinuousCandidateRepairConfig, ContinuousCandidateRepairResult,
    ContinuousCandidateRepairStatus, DutGridRoutingConfig, DutGridRoutingResult,
    repair_candidate_continuously, reroute_candidate_branches_with_dut_grid_at_poses,
    route_problem_with_dut_grid_at_poses,
};

fn default_initial_width_scale() -> f64 {
    0.2
}

fn default_width_stages() -> Vec<f64> {
    vec![0.4, 0.6, 0.8, 1.0]
}

fn default_maximum_repair_rounds_per_stage() -> usize {
    8
}

fn default_repair_config() -> ContinuousCandidateRepairConfig {
    ContinuousCandidateRepairConfig {
        enabled: true,
        steps: 16,
        projection_iterations: 64,
        trace_tension_strength: 1.0,
        ..ContinuousCandidateRepairConfig::default()
    }
}

fn default_maximum_discrete_attempts() -> usize {
    16
}

fn default_maximum_branches_per_discrete_attempt() -> usize {
    4
}

fn default_maximum_discrete_implicated_branches() -> usize {
    12
}

fn default_maximum_blocker_children_per_attempt() -> usize {
    4
}

/// Bounded outer-loop repair invoked only after continuous growth has stalled.
/// It enumerates small vertex covers of the trace-conflict graph, then asks A*
/// to reroute each cover while all other copper remains fixed.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WidthContinuationDiscreteRepairConfig {
    #[serde(default = "default_maximum_discrete_attempts")]
    pub maximum_attempts: usize,
    #[serde(default = "default_maximum_branches_per_discrete_attempt")]
    pub maximum_branches_per_attempt: usize,
    #[serde(default = "default_maximum_discrete_implicated_branches")]
    pub maximum_implicated_branches: usize,
    /// Highest-pressure routed-trace blockers used to extend a failed cover.
    /// Each child is still rerouted transactionally from the same parent.
    #[serde(default = "default_maximum_blocker_children_per_attempt")]
    pub maximum_blocker_children_per_attempt: usize,
    pub routing: DutGridRoutingConfig,
}

impl WidthContinuationDiscreteRepairConfig {
    pub fn check(&self) -> Result<(), String> {
        self.routing.check()?;
        if self.maximum_attempts == 0 {
            return Err(
                "width continuation discrete repair maximum_attempts must be positive".into(),
            );
        }
        if self.maximum_branches_per_attempt == 0 {
            return Err(
                "width continuation discrete repair maximum_branches_per_attempt must be positive"
                    .into(),
            );
        }
        if self.maximum_implicated_branches == 0 || self.maximum_implicated_branches > 24 {
            return Err(
                "width continuation discrete repair maximum_implicated_branches must be between 1 and 24"
                    .into(),
            );
        }
        if self.maximum_blocker_children_per_attempt == 0 {
            return Err(
                "width continuation discrete repair maximum_blocker_children_per_attempt must be positive"
                    .into(),
            );
        }
        Ok(())
    }
}

/// A topology-first experiment: route with thin copper, then preserve that
/// topology while the continuous engine grows it to the declared widths.
/// Clearance is never scaled down. The discrete router is deliberately not
/// called between width stages; a stalled stage is pressure for an outer loop.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WidthContinuationConfig {
    #[serde(default = "default_initial_width_scale")]
    pub initial_width_scale: f64,
    #[serde(default = "default_width_stages")]
    pub width_stages: Vec<f64>,
    #[serde(default = "default_maximum_repair_rounds_per_stage")]
    pub maximum_repair_rounds_per_stage: usize,
    pub routing: DutGridRoutingConfig,
    #[serde(default = "default_repair_config")]
    pub repair: ContinuousCandidateRepairConfig,
    /// Optional second continuous processor tried only when the primary
    /// processor cannot produce an exact or strictly improved proposal.
    #[serde(default)]
    pub fallback_repair: Option<ContinuousCandidateRepairConfig>,
    /// Optional discrete outer loop. This is deliberately downstream of all
    /// continuous rounds so topology changes are a response to a measured
    /// squeezing failure, not the first reaction to a clearance conflict.
    #[serde(default)]
    pub discrete_repair: Option<WidthContinuationDiscreteRepairConfig>,
}

impl WidthContinuationConfig {
    pub fn check(&self) -> Result<(), String> {
        self.routing.check()?;
        self.repair.check()?;
        if !self.repair.enabled {
            return Err("width continuation requires repair.enabled=true".into());
        }
        if let Some(fallback) = self.fallback_repair {
            fallback.check()?;
            if !fallback.enabled {
                return Err("width continuation requires fallback_repair.enabled=true".into());
            }
        }
        if let Some(discrete) = &self.discrete_repair {
            discrete.check()?;
        }
        if !self.initial_width_scale.is_finite() || !(0.0..1.0).contains(&self.initial_width_scale)
        {
            return Err("initial_width_scale must be finite, positive, and below one".into());
        }
        if self.width_stages.is_empty() {
            return Err("width_stages must not be empty".into());
        }
        let mut previous = self.initial_width_scale;
        for &scale in &self.width_stages {
            if !scale.is_finite() || scale <= previous || scale > 1.0 {
                return Err(
                    "width_stages must be finite, strictly increasing, and at most one".into(),
                );
            }
            previous = scale;
        }
        if previous != 1.0 {
            return Err("width_stages must end at exactly one".into());
        }
        if self.maximum_repair_rounds_per_stage == 0 {
            return Err("maximum_repair_rounds_per_stage must be positive".into());
        }
        Ok(())
    }
}

impl Default for WidthContinuationConfig {
    fn default() -> Self {
        Self {
            initial_width_scale: default_initial_width_scale(),
            width_stages: default_width_stages(),
            maximum_repair_rounds_per_stage: default_maximum_repair_rounds_per_stage(),
            routing: DutGridRoutingConfig::default(),
            repair: default_repair_config(),
            fallback_repair: None,
            discrete_repair: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WidthContinuationStatus {
    SeedRoutingIncomplete,
    StageStalled,
    FullWidthComplete,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WidthContinuationRepairRound {
    pub round: usize,
    pub processor: String,
    pub status: ContinuousCandidateRepairStatus,
    pub before_findings: usize,
    pub before_total_shortfall_mm: f64,
    pub proposal_findings: Option<usize>,
    pub proposal_total_shortfall_mm: Option<f64>,
    pub retained_for_next_round: bool,
    pub selected_branches: Vec<String>,
    pub selected_bodies: Vec<String>,
    pub selected_obstacles: Vec<String>,
    pub unsupported_findings: Vec<String>,
    pub body_motion_count: usize,
    pub trace_motion_count: usize,
    pub maximum_body_translation_mm: f64,
    pub maximum_body_rotation_degrees: f64,
    pub maximum_trace_point_motion_mm: Option<f64>,
    pub trust_region_trials: Vec<crate::ContinuousCandidateTrustRegionTrial>,
    pub segment_pairs: usize,
    pub segment_body_pairs: usize,
    pub engine_frames: usize,
    pub constraint_projections: u64,
    pub maximum_constraint_residual_mm: f64,
    pub detail: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WidthContinuationDiscreteAttemptStatus {
    Complete,
    Incomplete,
    Rejected,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WidthContinuationDiscreteAttempt {
    pub attempt: usize,
    pub branches: Vec<String>,
    pub status: WidthContinuationDiscreteAttemptStatus,
    pub routed_branches: Option<usize>,
    pub failed_branches: Option<usize>,
    pub searches: Option<usize>,
    pub expansions: Option<u64>,
    pub proposal_findings: Option<usize>,
    pub proposal_total_shortfall_mm: Option<f64>,
    pub branch_failures: Vec<crate::BranchRoutingEvidence>,
    pub selected: bool,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct WidthContinuationStage {
    pub width_scale: f64,
    pub exact_complete: bool,
    pub repair_rounds: Vec<WidthContinuationRepairRound>,
    pub discrete_attempts: Vec<WidthContinuationDiscreteAttempt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discrete_detail: Option<String>,
    /// The exact-complete stage candidate, or the last exact candidate from
    /// the preceding stage when this stage stalled.
    pub candidate: CandidateArtifact,
    pub validation: ExactValidationAssessment,
    /// Best incomplete geometry reached at this width. It is diagnostic only
    /// and is never silently promoted as the board result.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposed_candidate: Option<CandidateArtifact>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposed_validation: Option<ExactValidationAssessment>,
}

#[derive(Clone, Debug, Serialize)]
pub struct WidthContinuationResult {
    pub status: WidthContinuationStatus,
    pub initial_width_scale: f64,
    pub reached_width_scale: f64,
    pub seed: DutGridRoutingResult,
    pub stages: Vec<WidthContinuationStage>,
    /// Last exact candidate. It is a finished board only when status is
    /// `full_width_complete`.
    pub candidate: CandidateArtifact,
    /// Independent validation after forcing the returned geometry to the
    /// declared target widths. This prevents a thin stage from masquerading
    /// as a completed board.
    pub full_width_validation: ExactValidationAssessment,
}

impl WidthContinuationResult {
    pub fn complete(&self) -> bool {
        self.status == WidthContinuationStatus::FullWidthComplete
            && self.reached_width_scale == 1.0
            && self.full_width_validation.complete
    }
}

fn shortest_rotation_motion_degrees(before: f64, after: f64) -> f64 {
    ((after - before + 180.0).rem_euclid(360.0) - 180.0).abs()
}

fn repair_round_evidence(
    round: usize,
    processor: &str,
    before: &ExactValidationAssessment,
    correction: &ContinuousCandidateRepairResult,
) -> (WidthContinuationRepairRound, bool) {
    let current_score = validation_score(before);
    let retained_for_next_round = correction
        .proposed_validation
        .as_ref()
        .is_some_and(|validation| validation_score(validation) < current_score);
    (
        WidthContinuationRepairRound {
            round,
            processor: processor.to_owned(),
            status: correction.status,
            before_findings: before.violations.len(),
            before_total_shortfall_mm: total_clearance_shortfall_mm(before),
            proposal_findings: correction
                .proposed_validation
                .as_ref()
                .map(|validation| validation.violations.len()),
            proposal_total_shortfall_mm: correction
                .proposed_validation
                .as_ref()
                .map(total_clearance_shortfall_mm),
            retained_for_next_round,
            selected_branches: correction.selected_branches.clone(),
            selected_bodies: correction.selected_bodies.clone(),
            selected_obstacles: correction.selected_obstacles.clone(),
            unsupported_findings: correction.unsupported_findings.clone(),
            body_motion_count: correction.body_motion.len(),
            trace_motion_count: correction.trace_motion.len(),
            maximum_body_translation_mm: correction
                .body_motion
                .iter()
                .map(|motion| motion.distance_mm)
                .fold(0.0, f64::max),
            maximum_body_rotation_degrees: correction
                .body_motion
                .iter()
                .map(|motion| {
                    shortest_rotation_motion_degrees(
                        motion.rotation_before_degrees,
                        motion.rotation_after_degrees,
                    )
                })
                .fold(0.0, f64::max),
            maximum_trace_point_motion_mm: correction
                .trace_motion
                .iter()
                .filter_map(|motion| motion.maximum_point_motion_mm)
                .reduce(f64::max),
            trust_region_trials: correction.trust_region_trials.clone(),
            segment_pairs: correction.segment_pairs,
            segment_body_pairs: correction.segment_body_pairs,
            engine_frames: correction.frames.len(),
            constraint_projections: correction
                .frames
                .iter()
                .map(|frame| frame.metrics.constraint_projections)
                .sum(),
            maximum_constraint_residual_mm: correction
                .frames
                .iter()
                .map(|frame| f64::from(frame.metrics.max_constraint_residual))
                .fold(0.0, f64::max),
            detail: correction.detail.clone(),
        },
        retained_for_next_round,
    )
}

fn trace_conflict_edges(
    candidate: &CandidateArtifact,
    validation: &ExactValidationAssessment,
) -> Vec<(String, String)> {
    let branches = candidate
        .traces
        .iter()
        .map(|trace| trace.branch.as_str())
        .collect::<BTreeSet<_>>();
    let mut edges = BTreeSet::new();
    for finding in &validation.violations {
        if finding.code != "trace_trace_clearance" || finding.objects.len() != 2 {
            continue;
        }
        let left = finding.objects[0].as_str();
        let right = finding.objects[1].as_str();
        if left == right || !branches.contains(left) || !branches.contains(right) {
            continue;
        }
        let edge = if left < right {
            (left.to_owned(), right.to_owned())
        } else {
            (right.to_owned(), left.to_owned())
        };
        edges.insert(edge);
    }
    edges.into_iter().collect()
}

fn append_combinations(
    values: &[String],
    wanted: usize,
    start: usize,
    current: &mut Vec<String>,
    result: &mut Vec<Vec<String>>,
) {
    if current.len() == wanted {
        result.push(current.clone());
        return;
    }
    let remaining = wanted - current.len();
    for index in start..=values.len() - remaining {
        current.push(values[index].clone());
        append_combinations(values, wanted, index + 1, current, result);
        current.pop();
    }
}

fn bounded_conflict_covers(
    edges: &[(String, String)],
    config: &WidthContinuationDiscreteRepairConfig,
) -> Result<Vec<BTreeSet<String>>, String> {
    let implicated = edges
        .iter()
        .flat_map(|(left, right)| [left.clone(), right.clone()])
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if implicated.len() > config.maximum_implicated_branches {
        return Err(format!(
            "{} implicated branches exceed the configured discrete bound {}",
            implicated.len(),
            config.maximum_implicated_branches
        ));
    }
    let maximum_size = config.maximum_branches_per_attempt.min(implicated.len());
    let mut covers = Vec::new();
    for size in 1..=maximum_size {
        let mut combinations = Vec::new();
        append_combinations(&implicated, size, 0, &mut Vec::new(), &mut combinations);
        for combination in combinations {
            let cover = combination.into_iter().collect::<BTreeSet<_>>();
            if edges
                .iter()
                .all(|(left, right)| cover.contains(left) || cover.contains(right))
                && !covers
                    .iter()
                    .any(|existing: &BTreeSet<String>| existing.is_subset(&cover))
            {
                covers.push(cover);
                if covers.len() == config.maximum_attempts {
                    return Ok(covers);
                }
            }
        }
    }
    Ok(covers)
}

#[allow(clippy::type_complexity)]
fn repair_width_stage_discretely(
    problem: &Problem,
    stage_problem: &Problem,
    candidate: &CandidateArtifact,
    validation: &ExactValidationAssessment,
    config: &WidthContinuationDiscreteRepairConfig,
) -> Result<
    (
        Option<(CandidateArtifact, ExactValidationAssessment)>,
        Vec<WidthContinuationDiscreteAttempt>,
        String,
    ),
    String,
> {
    let edges = trace_conflict_edges(candidate, validation);
    if edges.is_empty() {
        return Ok((
            None,
            Vec::new(),
            "no trace/trace conflict graph was available for discrete repair".into(),
        ));
    }
    let covers = match bounded_conflict_covers(&edges, config) {
        Ok(covers) => covers,
        Err(detail) => return Ok((None, Vec::new(), detail)),
    };
    if covers.is_empty() {
        return Ok((
            None,
            Vec::new(),
            format!(
                "no conflict cover fits maximum_branches_per_attempt={}",
                config.maximum_branches_per_attempt
            ),
        ));
    }

    let known_branches = candidate
        .traces
        .iter()
        .map(|trace| trace.branch.clone())
        .collect::<BTreeSet<_>>();
    let mut portfolio = covers;
    let mut queued = portfolio.iter().cloned().collect::<BTreeSet<_>>();
    let mut attempts = Vec::new();
    let mut portfolio_index = 0;
    while portfolio_index < portfolio.len() && attempts.len() < config.maximum_attempts {
        let branches = portfolio[portfolio_index].clone();
        portfolio_index += 1;
        let attempt = attempts.len();
        let mut routing = config.routing.clone();
        let mut priorities = branches.iter().cloned().collect::<Vec<_>>();
        priorities.extend(routing.priority_branches);
        let mut priority_seen = BTreeSet::new();
        priorities.retain(|branch| priority_seen.insert(branch.clone()));
        routing.priority_branches = priorities;
        match reroute_candidate_branches_with_dut_grid_at_poses(
            stage_problem,
            candidate,
            &branches,
            &routing,
        ) {
            Ok(result) => {
                let proposal_validation = validate_candidate(problem, &result.candidate)?;
                let complete = result.evidence.failed_branches == 0 && proposal_validation.complete;
                let branch_failures = result
                    .evidence
                    .branches
                    .iter()
                    .filter(|branch| branch.status != crate::BranchRoutingStatus::Found)
                    .cloned()
                    .collect::<Vec<_>>();
                if !complete && branches.len() < config.maximum_branches_per_attempt {
                    let mut blocker_pressure = BTreeMap::<String, usize>::new();
                    for failure in &branch_failures {
                        for blocker in &failure.blockers {
                            if blocker.kind != crate::RoutingBlockerKind::RoutedTrace
                                || branches.contains(&blocker.object)
                                || !known_branches.contains(&blocker.object)
                            {
                                continue;
                            }
                            blocker_pressure
                                .entry(blocker.object.clone())
                                .and_modify(|hits| *hits = (*hits).max(blocker.frontier_hits))
                                .or_insert(blocker.frontier_hits);
                        }
                    }
                    let mut blockers = blocker_pressure.into_iter().collect::<Vec<_>>();
                    blockers.sort_by(|(left_id, left_hits), (right_id, right_hits)| {
                        right_hits
                            .cmp(left_hits)
                            .then_with(|| left_id.cmp(right_id))
                    });
                    for (blocker, _) in blockers
                        .into_iter()
                        .take(config.maximum_blocker_children_per_attempt)
                    {
                        let mut child = branches.clone();
                        child.insert(blocker);
                        if queued.insert(child.clone()) {
                            portfolio.push(child);
                        }
                    }
                }
                attempts.push(WidthContinuationDiscreteAttempt {
                    attempt,
                    branches: branches.into_iter().collect(),
                    status: if complete {
                        WidthContinuationDiscreteAttemptStatus::Complete
                    } else {
                        WidthContinuationDiscreteAttemptStatus::Incomplete
                    },
                    routed_branches: Some(result.evidence.routed_branches),
                    failed_branches: Some(result.evidence.failed_branches),
                    searches: Some(result.evidence.searches),
                    expansions: Some(result.evidence.expansions),
                    proposal_findings: Some(proposal_validation.violations.len()),
                    proposal_total_shortfall_mm: Some(total_clearance_shortfall_mm(
                        &proposal_validation,
                    )),
                    branch_failures,
                    selected: complete,
                    detail: if complete {
                        "selective reroute produced an exact stage candidate".into()
                    } else {
                        "selective reroute remained incomplete and was rolled back".into()
                    },
                });
                if complete {
                    return Ok((
                        Some((result.candidate, proposal_validation)),
                        attempts,
                        format!(
                            "selected attempt {attempt} from a bounded portfolio over {} conflict edges",
                            edges.len()
                        ),
                    ));
                }
            }
            Err(detail) => attempts.push(WidthContinuationDiscreteAttempt {
                attempt,
                branches: branches.into_iter().collect(),
                status: WidthContinuationDiscreteAttemptStatus::Rejected,
                routed_branches: None,
                failed_branches: None,
                searches: None,
                expansions: None,
                proposal_findings: None,
                proposal_total_shortfall_mm: None,
                branch_failures: Vec::new(),
                selected: false,
                detail,
            }),
        }
    }
    Ok((
        None,
        attempts,
        format!(
            "all bounded conflict-cover reroutes failed across {} conflict edges",
            edges.len()
        ),
    ))
}

pub fn route_with_width_continuation_at_poses(
    problem: &Problem,
    components: &[SolvedComponent],
    config: &WidthContinuationConfig,
) -> Result<WidthContinuationResult, String> {
    problem.check_schema()?;
    config.check()?;
    let target_widths = declared_branch_widths(problem)?;
    let mut thin_problem = problem.clone();
    scale_problem_widths(&mut thin_problem, config.initial_width_scale);
    let seed = route_problem_with_dut_grid_at_poses(&thin_problem, components, &config.routing)?;
    let mut candidate = seed.candidate.clone();
    let mut reached_width_scale = config.initial_width_scale;
    let mut stages = Vec::new();

    if seed.complete() {
        for &width_scale in &config.width_stages {
            let mut stage_problem = problem.clone();
            scale_problem_widths(&mut stage_problem, width_scale);
            let mut stage_input = candidate.clone();
            apply_width_scale(&mut stage_input, &target_widths, width_scale)?;
            let mut stage_validation = validate_candidate(problem, &stage_input)?;
            let mut working = stage_input;
            let mut rounds = Vec::new();
            let mut best_proposal = None::<CandidateArtifact>;
            let mut best_validation = None::<ExactValidationAssessment>;
            let mut discrete_attempts = Vec::new();
            let mut discrete_detail = None;

            for round in 0..config.maximum_repair_rounds_per_stage {
                if stage_validation.complete {
                    break;
                }
                let mut processor = "primary";
                let mut correction =
                    repair_candidate_continuously(problem, &working, config.repair)?;
                let (primary_evidence, primary_improves) =
                    repair_round_evidence(round, processor, &stage_validation, &correction);
                if !correction.accepted()
                    && !primary_improves
                    && let Some(fallback) = config.fallback_repair
                {
                    rounds.push(primary_evidence);
                    processor = "fallback";
                    correction = repair_candidate_continuously(problem, &working, fallback)?;
                }
                let (evidence, retained_for_next_round) =
                    repair_round_evidence(round, processor, &stage_validation, &correction);
                rounds.push(evidence);
                if correction.accepted() {
                    working = correction.candidate;
                    stage_validation = correction.validation;
                    break;
                }
                let (Some(proposal), Some(proposal_validation)) = (
                    correction.proposed_candidate,
                    correction.proposed_validation,
                ) else {
                    break;
                };
                if !retained_for_next_round {
                    if best_validation.as_ref().is_none_or(|best| {
                        validation_score(&proposal_validation) < validation_score(best)
                    }) {
                        best_proposal = Some(proposal);
                        best_validation = Some(proposal_validation);
                    }
                    break;
                }
                working = proposal.clone();
                stage_validation = proposal_validation.clone();
                best_proposal = Some(proposal);
                best_validation = Some(proposal_validation);
            }

            if !stage_validation.complete
                && let Some(discrete) = &config.discrete_repair
            {
                let (selected, attempts, detail) = repair_width_stage_discretely(
                    problem,
                    &stage_problem,
                    &working,
                    &stage_validation,
                    discrete,
                )?;
                discrete_attempts = attempts;
                discrete_detail = Some(detail);
                if let Some((selected_candidate, selected_validation)) = selected {
                    working = selected_candidate;
                    stage_validation = selected_validation;
                }
            }

            if stage_validation.complete {
                candidate = working.clone();
                reached_width_scale = width_scale;
                stages.push(WidthContinuationStage {
                    width_scale,
                    exact_complete: true,
                    repair_rounds: rounds,
                    discrete_attempts,
                    discrete_detail,
                    candidate: working,
                    validation: stage_validation,
                    proposed_candidate: None,
                    proposed_validation: None,
                });
            } else {
                stages.push(WidthContinuationStage {
                    width_scale,
                    exact_complete: false,
                    repair_rounds: rounds,
                    discrete_attempts,
                    discrete_detail,
                    candidate: candidate.clone(),
                    validation: validate_candidate(problem, &candidate)?,
                    proposed_candidate: best_proposal.or(Some(working)),
                    proposed_validation: best_validation.or(Some(stage_validation)),
                });
                break;
            }
        }
    }

    let mut full_width_candidate = candidate.clone();
    apply_width_scale(&mut full_width_candidate, &target_widths, 1.0)?;
    let full_width_validation = validate_candidate(problem, &full_width_candidate)?;
    let status = if !seed.complete() {
        WidthContinuationStatus::SeedRoutingIncomplete
    } else if reached_width_scale == 1.0 && full_width_validation.complete {
        WidthContinuationStatus::FullWidthComplete
    } else {
        WidthContinuationStatus::StageStalled
    };
    Ok(WidthContinuationResult {
        status,
        initial_width_scale: config.initial_width_scale,
        reached_width_scale,
        seed,
        stages,
        candidate,
        full_width_validation,
    })
}

fn scale_problem_widths(problem: &mut Problem, scale: f64) {
    for net in &mut problem.nets {
        net.width *= scale;
    }
    for net in &mut problem.electrical_nets {
        net.width *= scale;
    }
}

fn declared_branch_widths(problem: &Problem) -> Result<BTreeMap<String, f64>, String> {
    let mut result = BTreeMap::new();
    for net in &problem.nets {
        if result.insert(net.id.clone(), net.width).is_some() {
            return Err(format!("duplicate declared branch width for {}", net.id));
        }
    }
    for net in &problem.electrical_nets {
        if result.insert(net.id.clone(), net.width).is_some() {
            return Err(format!("duplicate electrical width for {}", net.id));
        }
    }
    Ok(result)
}

fn apply_width_scale(
    candidate: &mut CandidateArtifact,
    target_widths: &BTreeMap<String, f64>,
    scale: f64,
) -> Result<(), String> {
    for trace in &mut candidate.traces {
        let target = target_widths
            .get(&trace.branch)
            .or_else(|| target_widths.get(&trace.electrical_net))
            .ok_or_else(|| format!("no declared target width for branch {}", trace.branch))?;
        trace.width = target * scale;
    }
    Ok(())
}

fn total_clearance_shortfall_mm(validation: &ExactValidationAssessment) -> f64 {
    validation
        .violations
        .iter()
        .map(|finding| (finding.required_distance - finding.actual_distance).max(0.0))
        .sum()
}

fn validation_score(validation: &ExactValidationAssessment) -> (usize, u64) {
    (
        validation.violations.len(),
        total_clearance_shortfall_mm(validation).to_bits(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn discrete_config() -> WidthContinuationDiscreteRepairConfig {
        WidthContinuationDiscreteRepairConfig {
            maximum_attempts: 16,
            maximum_branches_per_attempt: 4,
            maximum_implicated_branches: 12,
            maximum_blocker_children_per_attempt: 4,
            routing: DutGridRoutingConfig::default(),
        }
    }

    #[test]
    fn config_requires_monotone_growth_to_full_width() {
        let mut config = WidthContinuationConfig::default();
        assert!(config.check().is_ok());
        config.width_stages = vec![0.8, 0.7, 1.0];
        assert!(config.check().is_err());
        config.width_stages = vec![0.8, 0.9];
        assert!(config.check().is_err());
        config.width_stages = vec![0.8, 1.0];
        config.fallback_repair = Some(ContinuousCandidateRepairConfig::default());
        assert!(config.check().is_err());
        config.fallback_repair.as_mut().unwrap().enabled = true;
        assert!(config.check().is_ok());
    }

    #[test]
    fn conflict_cover_portfolio_finds_both_minimum_k_two_two_choices() {
        let edges = vec![
            ("A".into(), "C".into()),
            ("A".into(), "D".into()),
            ("B".into(), "C".into()),
            ("B".into(), "D".into()),
        ];
        let covers = bounded_conflict_covers(&edges, &discrete_config()).unwrap();
        assert_eq!(
            &covers[..2],
            &[
                BTreeSet::from(["A".into(), "B".into()]),
                BTreeSet::from(["C".into(), "D".into()]),
            ]
        );
        assert!(covers.iter().all(|cover| {
            edges
                .iter()
                .all(|(left, right)| cover.contains(left) || cover.contains(right))
        }));
    }

    #[test]
    fn conflict_cover_portfolio_obeys_branch_and_attempt_bounds() {
        let edges = vec![("A".into(), "B".into()), ("C".into(), "D".into())];
        let mut config = discrete_config();
        config.maximum_branches_per_attempt = 1;
        assert!(bounded_conflict_covers(&edges, &config).unwrap().is_empty());
        config.maximum_branches_per_attempt = 4;
        config.maximum_attempts = 1;
        assert_eq!(bounded_conflict_covers(&edges, &config).unwrap().len(), 1);
    }

    #[test]
    fn thin_candidate_cannot_claim_full_width_completion() {
        let problem: Problem = serde_json::from_str(include_str!(
            "../../../benchmarks/small/board-continuation-force-motion.json"
        ))
        .unwrap();
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
        let config = WidthContinuationConfig {
            initial_width_scale: 0.2,
            width_stages: vec![1.0],
            maximum_repair_rounds_per_stage: 4,
            routing: DutGridRoutingConfig {
                allow_vias: false,
                grid_mm: 0.25,
                ..DutGridRoutingConfig::default()
            },
            repair: default_repair_config(),
            fallback_repair: None,
            discrete_repair: None,
        };
        let result =
            route_with_width_continuation_at_poses(&problem, &components, &config).unwrap();
        assert_eq!(result.status, WidthContinuationStatus::FullWidthComplete);
        assert!(result.complete());
        assert_eq!(result.reached_width_scale, 1.0);
        assert!(result.full_width_validation.complete);
    }
}
