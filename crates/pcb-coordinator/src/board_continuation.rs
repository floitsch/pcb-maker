// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

use std::collections::{BTreeMap, BTreeSet};

use layout_trace_model::{
    Problem, Rect, Vec2,
    geometry::{add, rotate_degrees},
    model::Movement,
};
use pcb_routing::{
    BranchRoutingStatus, ContinuousCandidateRepairConfig, ContinuousCandidateRepairResult,
    DutGridRoutingConfig, DutGridRoutingResult, RoutingBlockerKind, repair_candidate_continuously,
    reroute_problem_branches_with_dut_grid_at_poses, route_problem_with_dut_grid,
};
use pcb_validate::{
    CandidateArtifact, ExactValidationAssessment, SolvedComponent, SolvedRouteNodeKind,
    validate_candidate, validate_geometry,
};
use serde::{Deserialize, Serialize};

use crate::{ExactRoutePostProcessConfig, ExactRoutePostProcessEvidence, post_process_route_exact};

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BoardContinuationStageStrategy {
    #[default]
    FullGridReroute,
    RetainedAffineLocalRepair,
}

fn default_maximum_local_repair_rounds() -> usize {
    4
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BoardContinuationConfig {
    pub initial_scale: f64,
    pub shrink_steps: usize,
    #[serde(default)]
    pub stage_strategy: BoardContinuationStageStrategy,
    #[serde(default = "default_maximum_local_repair_rounds")]
    pub maximum_local_repair_rounds: usize,
    #[serde(default)]
    pub compact_collinear_after_deformation: bool,
    #[serde(default)]
    pub continuous_candidate_repair: ContinuousCandidateRepairConfig,
    #[serde(default)]
    pub continuous_post_process: ExactRoutePostProcessConfig,
    pub routing: DutGridRoutingConfig,
}

impl BoardContinuationConfig {
    pub fn check(&self) -> Result<(), String> {
        if !self.initial_scale.is_finite() || self.initial_scale <= 1.0 {
            return Err(
                "board continuation initial_scale must be finite and greater than 1".into(),
            );
        }
        if !(1..=10_000).contains(&self.shrink_steps) {
            return Err("board continuation shrink_steps must be between 1 and 10000".into());
        }
        if !(1..=1_000).contains(&self.maximum_local_repair_rounds) {
            return Err(
                "board continuation maximum_local_repair_rounds must be between 1 and 1000".into(),
            );
        }
        self.continuous_candidate_repair.check()?;
        self.continuous_post_process.check()?;
        if !matches!(
            self.continuous_post_process,
            ExactRoutePostProcessConfig::None
        ) && (!self.continuous_candidate_repair.enabled
            || self.stage_strategy != BoardContinuationStageStrategy::RetainedAffineLocalRepair)
        {
            return Err(
                "board continuation continuous_post_process requires retained affine strategy and enabled continuous candidate repair"
                    .into(),
            );
        }
        self.routing.check()
    }
}

impl Default for BoardContinuationConfig {
    fn default() -> Self {
        Self {
            initial_scale: 1.5,
            shrink_steps: 5,
            stage_strategy: BoardContinuationStageStrategy::FullGridReroute,
            maximum_local_repair_rounds: default_maximum_local_repair_rounds(),
            compact_collinear_after_deformation: false,
            continuous_candidate_repair: ContinuousCandidateRepairConfig::default(),
            continuous_post_process: ExactRoutePostProcessConfig::None,
            routing: DutGridRoutingConfig {
                grid_mm: 0.25,
                allow_vias: false,
                ..DutGridRoutingConfig::default()
            },
        }
    }
}

#[derive(Clone, Debug)]
pub struct BoardContinuationProposal {
    pub routing: DutGridRoutingResult,
    pub reused_previous_candidate: bool,
    pub deformation_validation: Option<ExactValidationAssessment>,
    pub local_repair_rounds: usize,
    pub rerouted_branches: Vec<String>,
    pub retained_branch_count: usize,
    pub deformation_removed_points: usize,
    pub motion_processor: String,
    pub continuous_motion: Option<ContinuousCandidateRepairResult>,
    pub continuous_post_process: Option<ExactRoutePostProcessEvidence>,
}

#[derive(Clone, Copy, Debug)]
pub struct PreviousBoardContinuationStage<'a> {
    pub problem: &'a Problem,
    pub routing: &'a DutGridRoutingResult,
}

/// Replaceable boundary for each shrink transaction. The full-reroute control
/// and retained/local-repair producer share the same exact-gated coordinator,
/// schedule, and evidence boundary.
pub trait BoardContinuationStageProducer {
    fn name(&self) -> &'static str;

    fn propose(
        &self,
        stage_problem: &Problem,
        previous: Option<PreviousBoardContinuationStage<'_>>,
        routing: &DutGridRoutingConfig,
    ) -> Result<BoardContinuationProposal, String>;

    fn limitation(&self) -> &'static str;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct FullGridRerouteStageProducer;

impl BoardContinuationStageProducer for FullGridRerouteStageProducer {
    fn name(&self) -> &'static str {
        "full-grid-reroute-v1"
    }

    fn propose(
        &self,
        stage_problem: &Problem,
        _previous: Option<PreviousBoardContinuationStage<'_>>,
        routing: &DutGridRoutingConfig,
    ) -> Result<BoardContinuationProposal, String> {
        Ok(BoardContinuationProposal {
            routing: route_problem_with_dut_grid(stage_problem, routing)?,
            reused_previous_candidate: false,
            deformation_validation: None,
            local_repair_rounds: 0,
            rerouted_branches: Vec::new(),
            retained_branch_count: 0,
            deformation_removed_points: 0,
            motion_processor: "none".into(),
            continuous_motion: None,
            continuous_post_process: None,
        })
    }

    fn limitation(&self) -> &'static str {
        "component declarations and seed points follow an affine schedule; this control reroutes every stage from scratch"
    }
}

pub trait BoardContinuationMotionProcessor {
    fn name(&self) -> &'static str;

    fn process(
        &self,
        problem: &Problem,
        candidate: &CandidateArtifact,
    ) -> Result<Option<ContinuousCandidateRepairResult>, String>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NoBoardContinuationMotion;

impl BoardContinuationMotionProcessor for NoBoardContinuationMotion {
    fn name(&self) -> &'static str {
        "none"
    }

    fn process(
        &self,
        _problem: &Problem,
        _candidate: &CandidateArtifact,
    ) -> Result<Option<ContinuousCandidateRepairResult>, String> {
        Ok(None)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ContinuousBoardContinuationMotion {
    pub config: ContinuousCandidateRepairConfig,
}

impl BoardContinuationMotionProcessor for ContinuousBoardContinuationMotion {
    fn name(&self) -> &'static str {
        "continuous-candidate-repair-v1"
    }

    fn process(
        &self,
        problem: &Problem,
        candidate: &CandidateArtifact,
    ) -> Result<Option<ContinuousCandidateRepairResult>, String> {
        Ok(Some(repair_candidate_continuously(
            problem,
            candidate,
            self.config,
        )?))
    }
}

#[derive(Clone, Debug)]
pub struct RetainedAffineLocalRepairStageProducer<M> {
    pub maximum_local_repair_rounds: usize,
    pub compact_collinear_after_deformation: bool,
    pub motion: M,
    pub continuous_post_process: ExactRoutePostProcessConfig,
}

impl<M: BoardContinuationMotionProcessor> BoardContinuationStageProducer
    for RetainedAffineLocalRepairStageProducer<M>
{
    fn name(&self) -> &'static str {
        "retained-affine-local-grid-repair-v1"
    }

    fn propose(
        &self,
        stage_problem: &Problem,
        previous: Option<PreviousBoardContinuationStage<'_>>,
        routing: &DutGridRoutingConfig,
    ) -> Result<BoardContinuationProposal, String> {
        let Some(previous) = previous else {
            return Ok(BoardContinuationProposal {
                routing: route_problem_with_dut_grid(stage_problem, routing)?,
                reused_previous_candidate: false,
                deformation_validation: None,
                local_repair_rounds: 0,
                rerouted_branches: Vec::new(),
                retained_branch_count: 0,
                deformation_removed_points: 0,
                motion_processor: self.motion.name().into(),
                continuous_motion: None,
                continuous_post_process: None,
            });
        };
        let (deformed, deformation_removed_points) = deform_routing_to_stage(
            previous,
            stage_problem,
            routing,
            self.compact_collinear_after_deformation,
        )?;
        let deformation_validation = deformed.validation.clone();
        let continuous_motion = if deformed.complete() {
            None
        } else {
            self.motion.process(stage_problem, &deformed.candidate)?
        };
        let mut repair_source = deformed;
        let mut continuous_post_process = None;
        if let Some(motion) = &continuous_motion
            && motion.accepted()
        {
            repair_source.candidate = motion.candidate.clone();
            repair_source.validation = motion.validation.clone();
            repair_source.evidence.strategy =
                "board-continuation-affine-continuous-motion-v1".into();
            if !matches!(
                self.continuous_post_process,
                ExactRoutePostProcessConfig::None
            ) {
                let processed = post_process_route_exact(
                    stage_problem,
                    &repair_source.candidate,
                    &self.continuous_post_process,
                )?;
                repair_source.candidate = processed.candidate;
                repair_source.validation =
                    validate_candidate(stage_problem, &repair_source.candidate)?;
                if !repair_source.validation.complete {
                    return Err(
                        "exact route post-processor returned an incomplete continuation candidate"
                            .into(),
                    );
                }
                repair_source.evidence.strategy =
                    "board-continuation-affine-continuous-motion-post-process-v1".into();
                continuous_post_process = Some(processed.evidence);
            }
        }
        let branch_ids = repair_source
            .evidence
            .branches
            .iter()
            .map(|branch| branch.branch.clone())
            .collect::<BTreeSet<_>>();
        let total_branch_count = branch_ids.len();
        if repair_source.complete() {
            return Ok(BoardContinuationProposal {
                routing: repair_source,
                reused_previous_candidate: true,
                deformation_validation: Some(deformation_validation),
                local_repair_rounds: 0,
                rerouted_branches: Vec::new(),
                retained_branch_count: total_branch_count,
                deformation_removed_points,
                motion_processor: self.motion.name().into(),
                continuous_motion,
                continuous_post_process,
            });
        }

        let mut rerouted =
            repair_set_for_fixed_geometry(stage_problem, &repair_source, &branch_ids)?;
        if rerouted.is_empty() {
            return Ok(BoardContinuationProposal {
                routing: repair_source,
                reused_previous_candidate: true,
                deformation_validation: Some(deformation_validation),
                local_repair_rounds: 0,
                rerouted_branches: Vec::new(),
                retained_branch_count: total_branch_count,
                deformation_removed_points,
                motion_processor: self.motion.name().into(),
                continuous_motion,
                continuous_post_process,
            });
        }

        let mut latest = repair_source.clone();
        let mut attempted_rerouted = BTreeSet::new();
        let mut rounds = 0;
        while rounds < self.maximum_local_repair_rounds {
            rounds += 1;
            attempted_rerouted = rerouted.clone();
            let mut repair_routing = routing.clone();
            repair_routing.priority_branches = attempted_rerouted.iter().cloned().collect();
            latest = reroute_problem_branches_with_dut_grid_at_poses(
                stage_problem,
                &repair_source.candidate.components,
                &repair_source,
                &attempted_rerouted,
                &repair_routing,
            )?;
            if latest.complete() {
                break;
            }
            let before = rerouted.len();
            extend_repair_set_from_result(&latest, &branch_ids, &mut rerouted);
            if rerouted.len() == before {
                break;
            }
            // The enlarged set must leave valid fixed geometry before it may
            // become a reservation for another repair round.
            if !extend_repair_set_until_fixed_geometry_is_valid(
                stage_problem,
                &repair_source,
                &branch_ids,
                &mut rerouted,
            )? {
                break;
            }
        }
        Ok(BoardContinuationProposal {
            routing: latest,
            reused_previous_candidate: true,
            deformation_validation: Some(deformation_validation),
            local_repair_rounds: rounds,
            rerouted_branches: attempted_rerouted.iter().cloned().collect(),
            retained_branch_count: total_branch_count.saturating_sub(attempted_rerouted.len()),
            deformation_removed_points,
            motion_processor: self.motion.name().into(),
            continuous_motion,
            continuous_post_process,
        })
    }

    fn limitation(&self) -> &'static str {
        "component poses and copper are affinely deformed; an optional exact-gated continuous trace/body processor runs before selective grid fallback and supports fixed rectangular explicit keepouts, but circular shapes, movable compound keepouts, body/body contacts, vias, and mixed-layer continuous motion remain unsupported"
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct BoardContinuationStage {
    pub id: String,
    pub state_id: String,
    pub parent_state_id: Option<String>,
    pub depth: usize,
    pub scale: f64,
    pub board_bounds: Rect,
    pub problem: Problem,
    pub accepted: bool,
    pub reused_previous_candidate: bool,
    pub deformation_validation: Option<ExactValidationAssessment>,
    pub local_repair_rounds: usize,
    pub rerouted_branches: Vec<String>,
    pub retained_branch_count: usize,
    pub deformation_removed_points: usize,
    pub motion_processor: String,
    pub continuous_motion: Option<ContinuousCandidateRepairResult>,
    pub continuous_post_process: Option<ExactRoutePostProcessEvidence>,
    pub routing: DutGridRoutingResult,
}

#[derive(Clone, Debug, Serialize)]
pub struct BoardContinuationEvidence {
    pub strategy: String,
    pub producer: String,
    pub config: BoardContinuationConfig,
    pub selected_attempt: Option<usize>,
    pub attempted_stages: usize,
    pub accepted_stages: usize,
    pub affine_reuse_stages: usize,
    pub local_repair_stages: usize,
    pub total_rerouted_branches: usize,
    pub total_searched_branches: usize,
    pub total_deformation_removed_points: usize,
    pub continuous_motion_stages: usize,
    pub continuous_motion_exact_stages: usize,
    pub total_motion_frames: usize,
    pub total_moved_components: usize,
    pub continuous_post_process_stages: usize,
    pub continuous_post_process_improved_stages: usize,
    pub total_post_process_operations: usize,
    pub total_post_process_exact_validation_calls: usize,
    pub total_post_process_saved_length_mm: f64,
    pub total_stage_expansions: u64,
    pub target_stage: usize,
    pub stopped_at_stage: Option<usize>,
    pub last_valid_scale: Option<f64>,
    pub target_complete: bool,
    pub limitation: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct BoardContinuationResult {
    pub candidate: CandidateArtifact,
    /// Assessment against the original target board, not merely the last
    /// oversized stage. A stuck run therefore cannot masquerade as complete.
    pub validation: ExactValidationAssessment,
    pub evidence: BoardContinuationEvidence,
    pub attempts: Vec<BoardContinuationStage>,
}

impl BoardContinuationResult {
    pub fn complete(&self) -> bool {
        self.evidence.target_complete && self.validation.complete
    }
}

pub fn continue_board_with<P: BoardContinuationStageProducer>(
    target: &Problem,
    config: &BoardContinuationConfig,
    producer: &P,
) -> Result<BoardContinuationResult, String> {
    target.check_schema()?;
    config.check()?;
    if !target.placement_constraints.is_empty() {
        return Err(
            "affine board continuation does not yet transform relational placement constraints"
                .into(),
        );
    }
    let mut attempts = Vec::<BoardContinuationStage>::new();
    let mut selected_attempt = None::<usize>;
    let mut stopped_at_stage = None;

    for stage in 0..=config.shrink_steps {
        let scale = continuation_scale(config, stage);
        let stage_problem = scaled_stage_problem(target, scale)?;
        let previous = selected_attempt.map(|index| PreviousBoardContinuationStage {
            problem: &attempts[index].problem,
            routing: &attempts[index].routing,
        });
        let proposal = producer.propose(&stage_problem, previous, &config.routing)?;
        let accepted = proposal.routing.complete();
        let state_id = format!("shrink-stage-{stage:04}");
        let parent_state_id = selected_attempt.map(|index| attempts[index].state_id.clone());
        attempts.push(BoardContinuationStage {
            id: state_id.clone(),
            state_id,
            parent_state_id,
            depth: stage,
            scale,
            board_bounds: stage_problem.board.bounds,
            problem: stage_problem,
            accepted,
            reused_previous_candidate: proposal.reused_previous_candidate,
            deformation_validation: proposal.deformation_validation,
            local_repair_rounds: proposal.local_repair_rounds,
            rerouted_branches: proposal.rerouted_branches,
            retained_branch_count: proposal.retained_branch_count,
            deformation_removed_points: proposal.deformation_removed_points,
            motion_processor: proposal.motion_processor,
            continuous_motion: proposal.continuous_motion,
            continuous_post_process: proposal.continuous_post_process,
            routing: proposal.routing,
        });
        if accepted {
            selected_attempt = Some(attempts.len() - 1);
        } else {
            stopped_at_stage = Some(stage);
            break;
        }
    }

    let selected = selected_attempt.unwrap_or(0);
    let candidate = attempts
        .get(selected)
        .ok_or_else(|| "board continuation produced no stage attempts".to_string())?
        .routing
        .candidate
        .clone();
    let validation = validate_candidate(target, &candidate)?;
    let target_stage = config.shrink_steps;
    let target_complete = selected_attempt.is_some_and(|index| {
        attempts[index].depth == target_stage && attempts[index].accepted && validation.complete
    });
    Ok(BoardContinuationResult {
        candidate,
        validation,
        evidence: BoardContinuationEvidence {
            strategy: "oversized-board-affine-continuation-v1".into(),
            producer: producer.name().into(),
            config: config.clone(),
            selected_attempt,
            attempted_stages: attempts.len(),
            accepted_stages: attempts.iter().filter(|attempt| attempt.accepted).count(),
            affine_reuse_stages: attempts
                .iter()
                .filter(|attempt| attempt.reused_previous_candidate)
                .count(),
            local_repair_stages: attempts
                .iter()
                .filter(|attempt| attempt.local_repair_rounds > 0)
                .count(),
            total_rerouted_branches: attempts
                .iter()
                .map(|attempt| attempt.rerouted_branches.len())
                .sum(),
            total_searched_branches: attempts
                .iter()
                .flat_map(|attempt| &attempt.routing.evidence.branches)
                .filter(|branch| branch.terminal_layer_searches > 0)
                .count(),
            total_deformation_removed_points: attempts
                .iter()
                .map(|attempt| attempt.deformation_removed_points)
                .sum(),
            continuous_motion_stages: attempts
                .iter()
                .filter(|attempt| attempt.continuous_motion.is_some())
                .count(),
            continuous_motion_exact_stages: attempts
                .iter()
                .filter(|attempt| {
                    attempt
                        .continuous_motion
                        .as_ref()
                        .is_some_and(ContinuousCandidateRepairResult::accepted)
                })
                .count(),
            total_motion_frames: attempts
                .iter()
                .filter_map(|attempt| attempt.continuous_motion.as_ref())
                .map(|motion| motion.frames.len())
                .sum(),
            total_moved_components: attempts
                .iter()
                .filter_map(|attempt| attempt.continuous_motion.as_ref())
                .map(|motion| motion.body_motion.len())
                .sum(),
            continuous_post_process_stages: attempts
                .iter()
                .filter(|attempt| attempt.continuous_post_process.is_some())
                .count(),
            continuous_post_process_improved_stages: attempts
                .iter()
                .filter(|attempt| {
                    attempt
                        .continuous_post_process
                        .as_ref()
                        .is_some_and(|post| post.status == "improved")
                })
                .count(),
            total_post_process_operations: attempts
                .iter()
                .filter_map(|attempt| attempt.continuous_post_process.as_ref())
                .map(|post| post.attempted_operations)
                .sum(),
            total_post_process_exact_validation_calls: attempts
                .iter()
                .filter_map(|attempt| attempt.continuous_post_process.as_ref())
                .map(|post| post.exact_validation_calls)
                .sum(),
            total_post_process_saved_length_mm: attempts
                .iter()
                .filter_map(|attempt| attempt.continuous_post_process.as_ref())
                .map(|post| post.input_trace_length_mm - post.output_trace_length_mm)
                .sum(),
            total_stage_expansions: attempts
                .iter()
                .map(|attempt| attempt.routing.evidence.expansions)
                .sum(),
            target_stage,
            stopped_at_stage,
            last_valid_scale: selected_attempt.map(|index| attempts[index].scale),
            target_complete,
            limitation: producer.limitation().into(),
        },
        attempts,
    })
}

pub fn continue_board_with_full_grid_reroute(
    target: &Problem,
    config: &BoardContinuationConfig,
) -> Result<BoardContinuationResult, String> {
    continue_board_with(target, config, &FullGridRerouteStageProducer)
}

pub fn continue_board_with_retained_affine_local_repair(
    target: &Problem,
    config: &BoardContinuationConfig,
) -> Result<BoardContinuationResult, String> {
    if config.continuous_candidate_repair.enabled {
        continue_board_with(
            target,
            config,
            &RetainedAffineLocalRepairStageProducer {
                maximum_local_repair_rounds: config.maximum_local_repair_rounds,
                compact_collinear_after_deformation: config.compact_collinear_after_deformation,
                motion: ContinuousBoardContinuationMotion {
                    config: config.continuous_candidate_repair,
                },
                continuous_post_process: config.continuous_post_process.clone(),
            },
        )
    } else {
        continue_board_with(
            target,
            config,
            &RetainedAffineLocalRepairStageProducer {
                maximum_local_repair_rounds: config.maximum_local_repair_rounds,
                compact_collinear_after_deformation: config.compact_collinear_after_deformation,
                motion: NoBoardContinuationMotion,
                continuous_post_process: ExactRoutePostProcessConfig::None,
            },
        )
    }
}

pub fn continue_board(
    target: &Problem,
    config: &BoardContinuationConfig,
) -> Result<BoardContinuationResult, String> {
    match config.stage_strategy {
        BoardContinuationStageStrategy::FullGridReroute => {
            continue_board_with_full_grid_reroute(target, config)
        }
        BoardContinuationStageStrategy::RetainedAffineLocalRepair => {
            continue_board_with_retained_affine_local_repair(target, config)
        }
    }
}

fn continuation_scale(config: &BoardContinuationConfig, stage: usize) -> f64 {
    if stage == config.shrink_steps {
        1.0
    } else {
        config.initial_scale
            - (config.initial_scale - 1.0) * stage as f64 / config.shrink_steps as f64
    }
}

fn scaled_stage_problem(target: &Problem, scale: f64) -> Result<Problem, String> {
    if !scale.is_finite() || scale < 1.0 {
        return Err("board continuation stage scale must be finite and at least 1".into());
    }
    let mut stage = target.clone();
    let center = rect_center(target.board.bounds);
    stage.board.bounds = scale_rect(target.board.bounds, center, scale);
    for component in &mut stage.components {
        component.position = scale_point(component.position, center, scale);
        if let Some(region) = component.constraints.region {
            component.constraints.region = Some(scale_rect(region, center, scale));
        }
    }
    for net in &mut stage.nets {
        for point in &mut net.seed_route {
            *point = scale_point(*point, center, scale);
        }
    }
    stage.check_schema()?;
    Ok(stage)
}

fn rect_center(rect: Rect) -> Vec2 {
    Vec2::new(
        (rect.min.x + rect.max.x) * 0.5,
        (rect.min.y + rect.max.y) * 0.5,
    )
}

fn scale_point(point: Vec2, center: Vec2, scale: f64) -> Vec2 {
    Vec2::new(
        center.x + (point.x - center.x) * scale,
        center.y + (point.y - center.y) * scale,
    )
}

fn scale_rect(rect: Rect, center: Vec2, scale: f64) -> Rect {
    Rect {
        min: scale_point(rect.min, center, scale),
        max: scale_point(rect.max, center, scale),
    }
}

fn deform_routing_to_stage(
    previous: PreviousBoardContinuationStage<'_>,
    stage_problem: &Problem,
    routing: &DutGridRoutingConfig,
    compact_collinear: bool,
) -> Result<(DutGridRoutingResult, usize), String> {
    let mut candidate = previous.routing.candidate.clone();
    let declarations = stage_problem
        .components
        .iter()
        .map(|component| (component.id.as_str(), component))
        .collect::<BTreeMap<_, _>>();
    for component in &mut candidate.components {
        let declared = declarations.get(component.id.as_str()).ok_or_else(|| {
            format!(
                "deformed candidate contains unknown component {}",
                component.id
            )
        })?;
        let mapped = map_between_rects(
            component.position,
            previous.problem.board.bounds,
            stage_problem.board.bounds,
        )?;
        component.position = match declared.constraints.movement {
            Movement::Fixed => declared.position,
            Movement::Horizontal => Vec2::new(mapped.x, declared.position.y),
            Movement::Vertical => Vec2::new(declared.position.x, mapped.y),
            Movement::Free => mapped,
        };
        component.size = declared.size;
        if declared.constraints.rotation == layout_trace_model::model::Rotation::Fixed {
            component.rotation_degrees = declared.rotation_degrees;
        }
    }

    let solved = candidate
        .components
        .iter()
        .map(|component| (component.id.as_str(), component))
        .collect::<BTreeMap<_, _>>();
    let mut node_positions = BTreeMap::<String, Vec2>::new();
    for graph in &mut candidate.route_graphs {
        for node in &mut graph.nodes {
            node.position = match &node.kind {
                SolvedRouteNodeKind::Terminal { component, pin } => {
                    solved_terminal_position(stage_problem, &solved, component, pin)?
                }
                SolvedRouteNodeKind::Junction { .. } | SolvedRouteNodeKind::Via { .. } => {
                    map_between_rects(
                        node.position,
                        previous.problem.board.bounds,
                        stage_problem.board.bounds,
                    )?
                }
            };
            if node_positions
                .insert(node.id.clone(), node.position)
                .is_some()
            {
                return Err(format!(
                    "deformed route graph contains duplicate node {}",
                    node.id
                ));
            }
        }
    }
    for trace in &mut candidate.traces {
        for point in &mut trace.points {
            *point = map_between_rects(
                *point,
                previous.problem.board.bounds,
                stage_problem.board.bounds,
            )?;
        }
        if let Some(first) = trace.points.first_mut() {
            *first = *node_positions.get(&trace.from_node).ok_or_else(|| {
                format!(
                    "deformed trace {} references unknown from-node {}",
                    trace.branch, trace.from_node
                )
            })?;
        }
        if let Some(last) = trace.points.last_mut() {
            *last = *node_positions.get(&trace.to_node).ok_or_else(|| {
                format!(
                    "deformed trace {} references unknown to-node {}",
                    trace.branch, trace.to_node
                )
            })?;
        }
        for via in &mut trace.vias {
            via.position = trace.points.get(via.point_index).copied().ok_or_else(|| {
                format!(
                    "deformed trace {} has via point index {} outside {} points",
                    trace.branch,
                    via.point_index,
                    trace.points.len()
                )
            })?;
        }
    }

    let deformation_removed_points = if compact_collinear {
        compact_candidate_collinear_points(&mut candidate)?
    } else {
        0
    };

    let validation = validate_candidate(stage_problem, &candidate)?;
    let mut evidence = previous.routing.evidence.clone();
    evidence.strategy = "board-continuation-affine-retained-v1".into();
    evidence.config = routing.clone();
    evidence.searches = 0;
    evidence.expansions = 0;
    evidence.failed_branches = 0;
    evidence.routed_branches = evidence.branch_count;
    for branch in &mut evidence.branches {
        branch.status = BranchRoutingStatus::Found;
        branch.terminal_layer_searches = 0;
        branch.expansions = 0;
        branch.exact_edge_retries = 0;
        branch.selected_cost = None;
        branch.selected_grid_mm = None;
        branch.grid_attempts.clear();
        branch.blockers.clear();
        branch.tree_attachment_attempts.clear();
        branch.tree_attachment_searches_truncated = false;
        branch.terminal_junction_attempts.clear();
        branch.terminal_junction_searches_truncated = false;
        if let Some(trace) = candidate
            .traces
            .iter()
            .find(|trace| trace.branch == branch.branch)
        {
            branch.point_count = trace.points.len();
            branch.via_count = trace.vias.len();
        }
    }
    Ok((
        DutGridRoutingResult {
            candidate,
            validation,
            evidence,
        },
        deformation_removed_points,
    ))
}

fn compact_candidate_collinear_points(candidate: &mut CandidateArtifact) -> Result<usize, String> {
    let mut removed = 0;
    for trace in &mut candidate.traces {
        if trace.points.len() < 3 {
            continue;
        }
        if trace.segment_layers.len() + 1 != trace.points.len() {
            return Err(format!(
                "cannot compact trace {} with inconsistent segment layers",
                trace.branch
            ));
        }
        let mandatory = trace
            .vias
            .iter()
            .map(|via| via.point_index)
            .collect::<BTreeSet<_>>();
        let mut kept = vec![0_usize];
        for index in 1..trace.points.len() - 1 {
            let incoming = trace.points[index];
            let previous = trace.points[index - 1];
            let next = trace.points[index + 1];
            let first = Vec2::new(incoming.x - previous.x, incoming.y - previous.y);
            let second = Vec2::new(next.x - incoming.x, next.y - incoming.y);
            let cross = first.x * second.y - first.y * second.x;
            let dot = first.x * second.x + first.y * second.y;
            let changes_layer = trace.segment_layers[index - 1] != trace.segment_layers[index];
            if mandatory.contains(&index) || changes_layer || cross.abs() > 1.0e-10 || dot <= 0.0 {
                kept.push(index);
            }
        }
        kept.push(trace.points.len() - 1);
        if kept.len() == trace.points.len() {
            continue;
        }
        let old_points = std::mem::take(&mut trace.points);
        let old_layers = std::mem::take(&mut trace.segment_layers);
        let index_map = kept
            .iter()
            .enumerate()
            .map(|(new, old)| (*old, new))
            .collect::<BTreeMap<_, _>>();
        trace.points = kept.iter().map(|index| old_points[*index]).collect();
        trace.segment_layers = kept
            .windows(2)
            .map(|span| {
                let layer = old_layers[span[0]].clone();
                if old_layers[span[0]..span[1]]
                    .iter()
                    .any(|candidate| candidate != &layer)
                {
                    return Err(format!(
                        "trace {} collinear compaction crossed a layer transition",
                        trace.branch
                    ));
                }
                Ok(layer)
            })
            .collect::<Result<Vec<_>, String>>()?;
        for via in &mut trace.vias {
            via.point_index = *index_map.get(&via.point_index).ok_or_else(|| {
                format!(
                    "trace {} compaction removed mandatory via point {}",
                    trace.branch, via.point_index
                )
            })?;
            via.position = trace.points[via.point_index];
        }
        for node in candidate
            .route_graphs
            .iter_mut()
            .flat_map(|graph| &mut graph.nodes)
            .filter(|node| node.incident_branches == [trace.branch.as_str()])
        {
            if let SolvedRouteNodeKind::Via { point_index, .. } = &mut node.kind {
                *point_index = *index_map.get(point_index).ok_or_else(|| {
                    format!(
                        "trace {} compaction removed serialized via-node point {}",
                        trace.branch, point_index
                    )
                })?;
                node.position = trace.points[*point_index];
            }
        }
        removed += old_points.len() - trace.points.len();
    }
    Ok(removed)
}

fn solved_terminal_position(
    problem: &Problem,
    solved: &BTreeMap<&str, &SolvedComponent>,
    component_id: &str,
    pin_id: &str,
) -> Result<Vec2, String> {
    let component = solved
        .get(component_id)
        .ok_or_else(|| format!("route terminal references unknown component {component_id}"))?;
    let declared = problem
        .components
        .iter()
        .find(|item| item.id == component_id)
        .ok_or_else(|| format!("route terminal references undeclared component {component_id}"))?;
    let pin = declared
        .pins
        .iter()
        .find(|item| item.id == pin_id)
        .ok_or_else(|| format!("route terminal references unknown pin {component_id}.{pin_id}"))?;
    Ok(add(
        component.position,
        rotate_degrees(pin.offset, component.rotation_degrees),
    ))
}

fn map_between_rects(point: Vec2, from: Rect, to: Rect) -> Result<Vec2, String> {
    let from_width = from.width();
    let from_height = from.height();
    if !from_width.is_finite()
        || !from_height.is_finite()
        || from_width <= 0.0
        || from_height <= 0.0
    {
        return Err("cannot deform from an invalid board rectangle".into());
    }
    Ok(Vec2::new(
        to.min.x + (point.x - from.min.x) * to.width() / from_width,
        to.min.y + (point.y - from.min.y) * to.height() / from_height,
    ))
}

fn repair_set_for_fixed_geometry(
    problem: &Problem,
    deformed: &DutGridRoutingResult,
    branch_ids: &BTreeSet<String>,
) -> Result<BTreeSet<String>, String> {
    let mut rerouted = BTreeSet::new();
    if extend_repair_set_until_fixed_geometry_is_valid(
        problem,
        deformed,
        branch_ids,
        &mut rerouted,
    )? {
        Ok(rerouted)
    } else {
        Ok(BTreeSet::new())
    }
}

fn extend_repair_set_until_fixed_geometry_is_valid(
    problem: &Problem,
    deformed: &DutGridRoutingResult,
    branch_ids: &BTreeSet<String>,
    rerouted: &mut BTreeSet<String>,
) -> Result<bool, String> {
    loop {
        let retained = deformed
            .candidate
            .traces
            .iter()
            .filter(|trace| !rerouted.contains(&trace.branch))
            .cloned()
            .collect::<Vec<_>>();
        let geometry = validate_geometry(
            problem,
            &deformed.candidate.components,
            &retained,
            &deformed.candidate.route_graphs,
        );
        if geometry.complete {
            return Ok(true);
        }
        let Some(branch) = geometry.violations.iter().find_map(|violation| {
            violation
                .objects
                .iter()
                .find(|object| branch_ids.contains(*object) && !rerouted.contains(*object))
                .cloned()
        }) else {
            return Ok(false);
        };
        rerouted.insert(branch);
        if rerouted.len() > branch_ids.len() {
            return Err("local repair selected more branches than the routing frontier".into());
        }
    }
}

fn extend_repair_set_from_result(
    result: &DutGridRoutingResult,
    branch_ids: &BTreeSet<String>,
    rerouted: &mut BTreeSet<String>,
) {
    for branch in &result.evidence.branches {
        if branch.status != BranchRoutingStatus::Found {
            rerouted.insert(branch.branch.clone());
            if let Some(blocker) = branch.blockers.iter().find(|blocker| {
                blocker.kind == RoutingBlockerKind::RoutedTrace
                    && branch_ids.contains(&blocker.object)
                    && !rerouted.contains(&blocker.object)
            }) {
                rerouted.insert(blocker.object.clone());
            }
        }
    }
    for violation in &result.validation.geometry.violations {
        if let Some(branch) = violation
            .objects
            .iter()
            .find(|object| branch_ids.contains(*object) && !rerouted.contains(*object))
        {
            rerouted.insert(branch.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pcb_routing::ContinuousCandidateRepairStatus;

    fn fixture(name: &str) -> Problem {
        let source = match name {
            "clear" => include_str!("../../../benchmarks/small/board-continuation-clear.json"),
            "bottleneck" => {
                include_str!("../../../benchmarks/small/board-continuation-bottleneck.json")
            }
            "local-repair" => {
                include_str!("../../../benchmarks/small/board-continuation-local-repair.json")
            }
            "force-motion" => {
                include_str!("../../../benchmarks/small/board-continuation-force-motion.json")
            }
            "explicit-keepout" => include_str!(
                "../../../benchmarks/small/board-continuation-explicit-keepout-fallback.json"
            ),
            "explicit-rect" => include_str!(
                "../../../benchmarks/small/board-continuation-explicit-rect-motion.json"
            ),
            _ => unreachable!(),
        };
        serde_json::from_str(source).unwrap()
    }

    fn continuous_motion_config() -> BoardContinuationConfig {
        BoardContinuationConfig {
            stage_strategy: BoardContinuationStageStrategy::RetainedAffineLocalRepair,
            compact_collinear_after_deformation: true,
            continuous_candidate_repair: ContinuousCandidateRepairConfig {
                enabled: true,
                ..ContinuousCandidateRepairConfig::default()
            },
            ..BoardContinuationConfig::default()
        }
    }

    fn projected_tension_config() -> BoardContinuationConfig {
        let mut config = continuous_motion_config();
        config.continuous_candidate_repair.steps = 8;
        config.continuous_candidate_repair.trace_tension_strength = 4.0;
        config
            .continuous_candidate_repair
            .maximum_trace_tension_step_mm = 0.2;
        config
    }

    fn candidate_length(candidate: &CandidateArtifact) -> f64 {
        candidate
            .traces
            .iter()
            .flat_map(|trace| trace.points.windows(2))
            .map(|edge| {
                layout_trace_model::geometry::length(layout_trace_model::geometry::sub(
                    edge[1], edge[0],
                ))
            })
            .sum()
    }

    #[test]
    fn clear_control_remains_exact_through_the_target_board() {
        let result = continue_board_with_full_grid_reroute(
            &fixture("clear"),
            &BoardContinuationConfig::default(),
        )
        .unwrap();
        assert!(result.complete());
        assert_eq!(result.attempts.len(), 6);
        assert!(result.attempts.iter().all(|stage| stage.accepted));
        assert_eq!(result.evidence.last_valid_scale, Some(1.0));
        assert!(result.validation.complete);
    }

    #[test]
    fn bottleneck_control_keeps_the_last_valid_large_board_and_reports_the_failed_shrink() {
        let result = continue_board_with_full_grid_reroute(
            &fixture("bottleneck"),
            &BoardContinuationConfig::default(),
        )
        .unwrap();
        assert!(!result.complete());
        assert!(result.evidence.accepted_stages > 0);
        assert!(result.evidence.stopped_at_stage.is_some());
        assert!(
            result
                .evidence
                .last_valid_scale
                .is_some_and(|scale| scale > 1.0)
        );
        assert!(!result.attempts.last().unwrap().accepted);
        assert!(!result.validation.complete);
    }

    #[test]
    fn retained_affine_control_repairs_only_when_deformation_becomes_invalid() {
        let config = BoardContinuationConfig {
            stage_strategy: BoardContinuationStageStrategy::RetainedAffineLocalRepair,
            ..BoardContinuationConfig::default()
        };
        let result = continue_board(&fixture("clear"), &config).unwrap();
        assert!(result.complete(), "{:?}", result.validation.violations);
        assert_eq!(result.attempts.len(), 6);
        assert_eq!(result.evidence.affine_reuse_stages, 5);
        assert!(result.evidence.local_repair_stages > 0);
        assert!(result.evidence.total_rerouted_branches > 0);
        assert!(result.attempts[1..].iter().all(|stage| {
            stage.reused_previous_candidate && stage.deformation_validation.is_some()
        }));
    }

    #[test]
    fn local_repair_preserves_the_unaffected_branch_and_reduces_search_work() {
        let problem = fixture("local-repair");
        let full =
            continue_board_with_full_grid_reroute(&problem, &BoardContinuationConfig::default())
                .unwrap();
        let retained_config = BoardContinuationConfig {
            stage_strategy: BoardContinuationStageStrategy::RetainedAffineLocalRepair,
            ..BoardContinuationConfig::default()
        };
        let retained = continue_board(&problem, &retained_config).unwrap();

        assert!(full.complete());
        assert!(retained.complete());
        assert_eq!(full.evidence.total_searched_branches, 12);
        assert_eq!(retained.evidence.total_searched_branches, 7);
        assert!(retained.evidence.total_stage_expansions < full.evidence.total_stage_expansions);
        for stage in &retained.attempts[1..] {
            assert_eq!(stage.rerouted_branches, ["SIGNAL"]);
            assert_eq!(stage.retained_branch_count, 1);
            assert_eq!(
                stage
                    .routing
                    .evidence
                    .branches
                    .iter()
                    .find(|branch| branch.branch == "UPPER")
                    .unwrap()
                    .expansions,
                0
            );
        }
    }

    #[test]
    fn optional_collinear_compaction_removes_sampling_debt_without_more_search() {
        let problem = fixture("local-repair");
        let raw_config = BoardContinuationConfig {
            stage_strategy: BoardContinuationStageStrategy::RetainedAffineLocalRepair,
            ..BoardContinuationConfig::default()
        };
        let compact_config = BoardContinuationConfig {
            compact_collinear_after_deformation: true,
            ..raw_config.clone()
        };
        let raw = continue_board(&problem, &raw_config).unwrap();
        let compact = continue_board(&problem, &compact_config).unwrap();

        assert!(compact.complete());
        assert_eq!(
            compact.evidence.total_stage_expansions,
            raw.evidence.total_stage_expansions
        );
        assert!(compact.evidence.total_deformation_removed_points > 0);
        let trace = compact
            .candidate
            .traces
            .iter()
            .find(|trace| trace.branch == "UPPER")
            .unwrap();
        assert_eq!(trace.points.len(), 2);
        assert_eq!(trace.segment_layers.len(), 1);
        assert!(trace.vias.is_empty());
    }

    #[test]
    fn continuous_motion_repairs_late_shrinks_by_moving_the_yielding_body() {
        let result = continue_board(&fixture("force-motion"), &continuous_motion_config()).unwrap();

        assert!(result.complete(), "{:?}", result.validation.violations);
        assert_eq!(result.evidence.total_searched_branches, 2);
        assert_eq!(result.evidence.local_repair_stages, 0);
        assert_eq!(result.evidence.continuous_motion_stages, 2);
        assert_eq!(result.evidence.continuous_motion_exact_stages, 2);
        assert_eq!(result.evidence.total_motion_frames, 10);
        assert_eq!(result.evidence.total_moved_components, 2);
        assert!(
            result.attempts[1..4]
                .iter()
                .all(|stage| stage.continuous_motion.is_none())
        );
        for stage in &result.attempts[4..] {
            let motion = stage.continuous_motion.as_ref().unwrap();
            assert_eq!(
                motion.status,
                ContinuousCandidateRepairStatus::ExactComplete
            );
            assert_eq!(motion.body_motion.len(), 1);
            assert_eq!(motion.body_motion[0].component, "YIELDING_BLOCKER");
            assert!(motion.trace_motion.is_empty());
            assert!(stage.rerouted_branches.is_empty());
        }
    }

    #[test]
    fn continuous_motion_bends_a_trace_around_a_fixed_body_without_new_searches() {
        let problem = fixture("local-repair");
        let no_motion = continue_board(
            &problem,
            &BoardContinuationConfig {
                stage_strategy: BoardContinuationStageStrategy::RetainedAffineLocalRepair,
                compact_collinear_after_deformation: true,
                ..BoardContinuationConfig::default()
            },
        )
        .unwrap();
        let motion = continue_board(&problem, &continuous_motion_config()).unwrap();

        assert!(motion.complete(), "{:?}", motion.validation.violations);
        assert_eq!(motion.evidence.total_searched_branches, 2);
        assert_eq!(motion.evidence.local_repair_stages, 0);
        assert_eq!(motion.evidence.continuous_motion_exact_stages, 5);
        assert!(motion.evidence.total_stage_expansions < no_motion.evidence.total_stage_expansions);
        for stage in &motion.attempts[1..] {
            let correction = stage.continuous_motion.as_ref().unwrap();
            assert_eq!(
                correction.status,
                ContinuousCandidateRepairStatus::ExactComplete
            );
            assert!(correction.body_motion.is_empty());
            assert!(
                correction
                    .trace_motion
                    .iter()
                    .any(|trace| trace.branch == "SIGNAL")
            );
            assert!(stage.rerouted_branches.is_empty());
        }
    }

    #[test]
    fn unsupported_explicit_keepout_motion_falls_back_to_selective_grid_repair() {
        let result =
            continue_board(&fixture("explicit-keepout"), &continuous_motion_config()).unwrap();

        assert!(result.complete(), "{:?}", result.validation.violations);
        assert_eq!(result.evidence.continuous_motion_stages, 5);
        assert_eq!(result.evidence.continuous_motion_exact_stages, 0);
        assert_eq!(result.evidence.local_repair_stages, 5);
        for stage in &result.attempts[1..] {
            let motion = stage.continuous_motion.as_ref().unwrap();
            assert_eq!(motion.status, ContinuousCandidateRepairStatus::Unsupported);
            assert!(motion.detail.contains("keepout:WALL:0"));
            assert_eq!(motion.selected_obstacles, ["keepout:WALL:0"]);
            assert_eq!(stage.rerouted_branches, ["SIGNAL"]);
            assert_eq!(stage.local_repair_rounds, 1);
        }
    }

    #[test]
    fn fixed_rectangular_explicit_keepout_is_lowered_without_discrete_reroute() {
        let problem = fixture("explicit-rect");
        let motion = continue_board(&problem, &continuous_motion_config()).unwrap();
        let discrete = continue_board(
            &problem,
            &BoardContinuationConfig {
                stage_strategy: BoardContinuationStageStrategy::RetainedAffineLocalRepair,
                compact_collinear_after_deformation: true,
                ..BoardContinuationConfig::default()
            },
        )
        .unwrap();

        assert!(motion.complete(), "{:?}", motion.validation.violations);
        assert_eq!(motion.evidence.total_searched_branches, 1);
        assert_eq!(motion.evidence.continuous_motion_exact_stages, 5);
        assert_eq!(motion.evidence.local_repair_stages, 0);
        assert_eq!(motion.evidence.total_rerouted_branches, 0);
        assert!(motion.evidence.total_stage_expansions < discrete.evidence.total_stage_expansions);
        for stage in &motion.attempts[1..] {
            let correction = stage.continuous_motion.as_ref().unwrap();
            assert_eq!(
                correction.status,
                ContinuousCandidateRepairStatus::ExactComplete
            );
            assert_eq!(correction.selected_obstacles, ["keepout:WALL:0"]);
            assert!(correction.selected_bodies.is_empty());
            assert!(correction.body_motion.is_empty());
            assert!(
                correction
                    .trace_motion
                    .iter()
                    .any(|trace| trace.branch == "SIGNAL")
            );
            assert!(stage.rerouted_branches.is_empty());
        }
    }

    #[test]
    fn generic_exact_vertex_pull_runs_after_motion_but_barely_improves_the_rectangle() {
        let problem = fixture("explicit-rect");
        let raw = continue_board(&problem, &continuous_motion_config()).unwrap();
        let mut post_config = continuous_motion_config();
        post_config.continuous_post_process = ExactRoutePostProcessConfig::ExactVertexPull {
            maximum_vertex_trials: 64,
            sweeps: 8,
            binary_steps: 24,
        };
        let post = continue_board(&problem, &post_config).unwrap();

        assert!(post.complete());
        assert_eq!(post.evidence.continuous_post_process_stages, 5);
        assert_eq!(post.evidence.continuous_post_process_improved_stages, 5);
        assert!(post.evidence.total_post_process_exact_validation_calls > 400);
        assert!(candidate_length(&post.candidate) < candidate_length(&raw.candidate));
        assert!(
            candidate_length(&raw.candidate) - candidate_length(&post.candidate) < 0.02,
            "raw={}, post={}",
            candidate_length(&raw.candidate),
            candidate_length(&post.candidate)
        );
    }

    #[test]
    fn projected_tension_slides_vertices_along_the_rectangle_clearance_boundary() {
        let problem = fixture("explicit-rect");
        let raw = continue_board(&problem, &continuous_motion_config()).unwrap();
        let tension = continue_board(&problem, &projected_tension_config()).unwrap();
        let discrete = continue_board(
            &problem,
            &BoardContinuationConfig {
                stage_strategy: BoardContinuationStageStrategy::RetainedAffineLocalRepair,
                compact_collinear_after_deformation: true,
                ..BoardContinuationConfig::default()
            },
        )
        .unwrap();

        assert!(tension.complete(), "{:?}", tension.validation.violations);
        assert_eq!(tension.evidence.total_searched_branches, 1);
        assert_eq!(tension.evidence.total_rerouted_branches, 0);
        assert_eq!(tension.evidence.total_motion_frames, 45);
        assert!(candidate_length(&tension.candidate) < candidate_length(&raw.candidate));
        assert!(candidate_length(&tension.candidate) < candidate_length(&discrete.candidate));
        assert!(
            tension
                .attempts
                .iter()
                .filter_map(|stage| stage.continuous_motion.as_ref())
                .all(|motion| motion.trace_tension_edge_integrations > 0)
        );
    }

    #[test]
    fn post_motion_optimizer_requires_enabled_retained_continuous_motion() {
        let config = BoardContinuationConfig {
            continuous_post_process: ExactRoutePostProcessConfig::ExactVertexPull {
                maximum_vertex_trials: 4,
                sweeps: 1,
                binary_steps: 4,
            },
            ..BoardContinuationConfig::default()
        };
        assert!(config.check().is_err());
    }

    #[test]
    fn collinear_compaction_preserves_via_points_and_layer_transitions() {
        let problem: Problem = serde_json::from_str(include_str!(
            "../../../benchmarks/imported/layout-trace/forced-crossing-two-layer.json"
        ))
        .unwrap();
        let mut candidate = route_problem_with_dut_grid(&problem, &DutGridRoutingConfig::default())
            .unwrap()
            .candidate;
        let via_count = candidate
            .traces
            .iter()
            .map(|trace| trace.vias.len())
            .sum::<usize>();

        let removed = compact_candidate_collinear_points(&mut candidate).unwrap();

        assert!(removed > 0);
        assert_eq!(
            candidate
                .traces
                .iter()
                .map(|trace| trace.vias.len())
                .sum::<usize>(),
            via_count
        );
        for trace in &candidate.traces {
            for via in &trace.vias {
                assert_eq!(via.position, trace.points[via.point_index]);
            }
        }
        let validation = validate_candidate(&problem, &candidate).unwrap();
        assert!(validation.complete, "{:?}", validation.violations);
    }
}
