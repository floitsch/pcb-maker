// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

use std::collections::{BTreeMap, BTreeSet};

use layout_trace_model::{
    Problem, Vec2,
    geometry::{Obb, obbs_overlap},
    model::{CopperShape, Movement, PlacementAnchor, PlacementConstraint},
};
use pcb_routing::{
    DutGridRoutingConfig, DutGridRoutingEvidence, DutGridRoutingResult, RoutingBlockerKind,
    route_problem_with_dut_grid_at_poses,
};
use pcb_validate::{CandidateArtifact, ExactValidationAssessment, SolvedComponent};
use serde::{Deserialize, Serialize};

use crate::{
    SelectiveRipupConfig, SelectiveRipupResult, repair_routing_result_with_selective_ripup,
};

#[path = "passage_occupancy.rs"]
mod passage_occupancy;
pub use passage_occupancy::{PassageCopperDemand, PassageCopperOccupant};

const DIRECTION_EPSILON: f64 = 1.0e-9;
const PLACEMENT_CONTACT_TOLERANCE_MM: f64 = 1.0e-6;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PressureDirectionPolicy {
    /// Push from the mean blocked search frontier toward the component center.
    #[default]
    FrontierCentroid,
    /// Explore the same pressure-ordered axes and diagonals, followed by
    /// their opposites. Pressure remains ordering advice, not a claim that
    /// the local gradient identifies the only useful side.
    FrontierCentroidBidirectional,
    /// Open a trace-width passage between the blocker and a neighboring body
    /// along one of the component's legal translation axes.
    PassageCapacity,
    /// Include retained foreign copper at a passage cross-section when
    /// estimating the opening needed by a failed fixed-layer legacy branch.
    PassageCapacityWithCopper,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PressureComponentOrder {
    /// Historical predecessor order: the object receiving the most blocked
    /// frontier cells is tried first.
    #[default]
    FrontierHits,
    /// Prefer a blocker implicated in more distinct failed branches, then use
    /// frontier hits as spatial-strength evidence.
    TriggerBranchesThenHits,
    /// Try movable endpoints of failed branches before blockers, then retain
    /// the multi-branch and spatial-strength ordering.
    FailedTerminalsThenTriggerBranchesThenHits,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PressureSourcePolicy {
    #[default]
    BlockersOnly,
    /// Add a pull toward the opposite endpoint of a failed legacy branch.
    /// Explicit route-graph terminal identity is not yet carried by DUT
    /// routing evidence, so unsupported branch IDs remain blocker-only.
    BlockersAndFailedBranchTerminals,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PushPropagationPolicy {
    /// Move only the routing blocker. This preserves the original pressure
    /// experiment as a control.
    #[default]
    SingleComponent,
    /// Translate movable bodies contacted by the proposal by the same vector.
    CollisionChain,
    /// First pull neighbors whose maximum-distance relationship would be
    /// broken, then close the same-vector translation over body contacts.
    /// This is selectable because a maximum-distance relation is not a rigid
    /// group in the semantic model.
    RelationalAndCollisionChain,
}

fn default_maximum_components_per_push() -> usize {
    1
}

fn default_beam_width() -> usize {
    1
}

/// Bounded outer-loop policy ported from the predecessor's failure-pressure
/// experiment. Routing remains a replaceable evaluator; this policy only
/// proposes semantic component translations and commits an improved result.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PressureRepairConfig {
    #[serde(default)]
    pub direction_policy: PressureDirectionPolicy,
    #[serde(default)]
    pub component_order: PressureComponentOrder,
    #[serde(default)]
    pub pressure_sources: PressureSourcePolicy,
    #[serde(default)]
    pub push_propagation: PushPropagationPolicy,
    #[serde(default = "default_maximum_components_per_push")]
    pub maximum_components_per_push: usize,
    /// Number of distinct routed placement states expanded at the next
    /// iteration. Width one preserves the historical greedy policy.
    #[serde(default = "default_beam_width")]
    pub beam_width: usize,
    /// Permit a beam frontier to advance through children which do not improve
    /// the globally selected routing rank. Such states are exploration only;
    /// the returned candidate is still the best independently assessed state.
    #[serde(default)]
    pub retain_non_improving_states: bool,
    /// Optional route-topology repair evaluated transactionally at every
    /// routed placement state, including the initial state.
    #[serde(default)]
    pub selective_ripup: Option<SelectiveRipupConfig>,
    pub push_step_mm: f64,
    pub step_scales: Vec<f64>,
    pub maximum_iterations: usize,
    pub maximum_blocking_components: usize,
    pub maximum_candidates_per_iteration: usize,
    pub maximum_total_attempts: usize,
    pub stop_on_complete: bool,
}

impl PressureRepairConfig {
    pub fn check(&self) -> Result<(), String> {
        if !self.push_step_mm.is_finite() || self.push_step_mm <= 0.0 {
            return Err("pressure repair push_step_mm must be finite and positive".into());
        }
        if self.step_scales.is_empty()
            || self
                .step_scales
                .iter()
                .any(|scale| !scale.is_finite() || *scale <= 0.0)
        {
            return Err("pressure repair step_scales must be finite and positive".into());
        }
        if self.maximum_iterations == 0
            || self.maximum_blocking_components == 0
            || self.maximum_candidates_per_iteration == 0
            || self.maximum_total_attempts == 0
            || self.maximum_components_per_push == 0
            || self.beam_width == 0
        {
            return Err("pressure repair work budgets must be positive".into());
        }
        if let Some(selective_ripup) = &self.selective_ripup {
            selective_ripup.check()?;
        }
        Ok(())
    }
}

impl Default for PressureRepairConfig {
    fn default() -> Self {
        Self {
            direction_policy: PressureDirectionPolicy::FrontierCentroid,
            component_order: PressureComponentOrder::FrontierHits,
            pressure_sources: PressureSourcePolicy::BlockersOnly,
            push_propagation: PushPropagationPolicy::SingleComponent,
            maximum_components_per_push: 1,
            beam_width: 1,
            retain_non_improving_states: false,
            selective_ripup: None,
            push_step_mm: 0.5,
            step_scales: vec![1.0, 2.0],
            maximum_iterations: 2,
            maximum_blocking_components: 4,
            maximum_candidates_per_iteration: 16,
            maximum_total_attempts: 32,
            stop_on_complete: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PassageProposalEvidence {
    pub direction: Vec2,
    pub limiting_object: String,
    pub current_gap_mm: f64,
    pub required_gap_mm: f64,
    /// Continuous-space shortage before routing-grid discretization.
    pub physical_deficit_mm: f64,
    /// Discretization-aware move after projection into the legal region.
    pub target_distance_mm: f64,
    pub remaining_deficit_mm: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copper_demand: Option<PassageCopperDemand>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ComponentPressureEvidence {
    pub component: String,
    /// Mean vector from the blocked frontier toward the component center.
    pub vector: Vec2,
    pub frontier_hits: usize,
    pub trigger_branches: Vec<String>,
    /// Failed branches for which this component is a movable endpoint and the
    /// pressure vector includes a pull toward the opposite endpoint.
    pub terminal_for_failed_branches: Vec<String>,
    pub passage_proposals: Vec<PassageProposalEvidence>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PressureRepairIterationEvidence {
    pub iteration: usize,
    /// First parent retained for compatibility with greedy evidence readers.
    pub parent_attempt: usize,
    pub parent_attempts: Vec<usize>,
    pub pressures: Vec<ComponentPressureEvidence>,
    pub candidate_attempts: Vec<usize>,
    pub retained_attempts: Vec<usize>,
    pub selected_attempt: usize,
    pub improved: bool,
    pub advanced: bool,
    pub parents: Vec<PressureRepairParentEvidence>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PressureRepairParentEvidence {
    pub parent_attempt: usize,
    pub pressures: Vec<ComponentPressureEvidence>,
    pub candidate_attempts: Vec<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PushChainContactKind {
    BodyContact,
    CopperEnvelopeContact,
    MaximumDistance,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PushChainContactEvidence {
    pub pushing_component: String,
    pub yielding_component: String,
    pub kind: PushChainContactKind,
}

#[derive(Clone, Debug, Serialize)]
pub struct PressureRepairAttempt {
    pub id: String,
    pub iteration: usize,
    pub parent_attempt: usize,
    pub blocker: Option<String>,
    pub pressure: Option<Vec2>,
    pub direction: Option<Vec2>,
    pub displacement: Vec2,
    pub moved_components: Vec<String>,
    pub chain_contacts: Vec<PushChainContactEvidence>,
    pub routing: Option<DutGridRoutingResult>,
    pub selective_ripup: Option<SelectiveRipupResult>,
    pub rejected_before_routing: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PressureRepairEvidence {
    pub strategy: String,
    pub config: PressureRepairConfig,
    pub selected_attempt: usize,
    pub attempted_repairs: usize,
    pub attempted_topology_repairs: usize,
    pub rejected_placements: usize,
    pub completed_iterations: usize,
    pub total_expansions: u64,
    pub complete: bool,
    pub iterations: Vec<PressureRepairIterationEvidence>,
    pub routing: DutGridRoutingEvidence,
}

#[derive(Clone, Debug, Serialize)]
pub struct PressureRepairResult {
    pub candidate: CandidateArtifact,
    pub validation: ExactValidationAssessment,
    pub evidence: PressureRepairEvidence,
    pub attempts: Vec<PressureRepairAttempt>,
}

#[derive(Debug)]
struct PushChainProposal {
    components: Vec<SolvedComponent>,
    moved_components: Vec<String>,
    contacts: Vec<PushChainContactEvidence>,
}

#[derive(Debug)]
struct PushChainRejection {
    message: String,
    moved_components: Vec<String>,
    contacts: Vec<PushChainContactEvidence>,
}

#[derive(Default)]
struct PressureAccumulator {
    sum_x: f64,
    sum_y: f64,
    hits: usize,
    vector_samples: usize,
    branches: BTreeSet<String>,
    terminal_branches: BTreeSet<String>,
    required_gap_mm: f64,
}

/// Push movable objects away from the spatially classified blocked frontier,
/// rerouting after each bounded proposal. The retained candidate is always one
/// of the independently exact-assessed routing attempts.
pub fn repair_routing_with_pressure(
    problem: &Problem,
    initial_components: &[SolvedComponent],
    routing_config: &DutGridRoutingConfig,
    repair_config: &PressureRepairConfig,
) -> Result<PressureRepairResult, String> {
    problem.check_schema()?;
    routing_config.check()?;
    repair_config.check()?;
    validate_pressure_placement(problem, initial_components)?;
    let initial_routing =
        route_problem_with_dut_grid_at_poses(problem, initial_components, routing_config)?;
    let initial_positions = initial_components
        .iter()
        .map(|component| (component.id.as_str(), component.position))
        .collect::<BTreeMap<_, _>>();
    let (initial, initial_selective_ripup, mut total_expansions, mut attempted_topology_repairs) =
        if let Some(selective_config) = &repair_config.selective_ripup {
            let repaired = repair_routing_result_with_selective_ripup(
                problem,
                initial_components,
                initial_routing,
                routing_config,
                selective_config,
            )?;
            let selected_routing = repaired.attempts[repaired.evidence.selected_attempt]
                .routing
                .clone();
            let total_expansions = repaired.evidence.total_expansions;
            let attempted_repairs = repaired.evidence.attempted_repairs;
            (
                selected_routing,
                Some(repaired),
                total_expansions,
                attempted_repairs,
            )
        } else {
            let total_expansions = initial_routing.evidence.expansions;
            (initial_routing, None, total_expansions, 0)
        };
    let mut attempts = vec![PressureRepairAttempt {
        id: "initial".into(),
        iteration: 0,
        parent_attempt: 0,
        blocker: None,
        pressure: None,
        direction: None,
        displacement: Vec2::ZERO,
        moved_components: Vec::new(),
        chain_contacts: Vec::new(),
        routing: Some(initial),
        selective_ripup: initial_selective_ripup,
        rejected_before_routing: None,
    }];
    let mut selected = 0;
    let mut frontier = vec![0];
    let mut seen_placements = BTreeSet::from([component_pose_key(
        &attempts[0]
            .routing
            .as_ref()
            .expect("initial pressure attempt routed")
            .candidate
            .components,
    )]);
    let mut iterations = Vec::new();

    for iteration in 1..=repair_config.maximum_iterations {
        if attempts[selected]
            .routing
            .as_ref()
            .is_some_and(DutGridRoutingResult::complete)
        {
            break;
        }
        let prior_selected = selected;
        let parent_attempts = frontier.clone();
        let mut candidate_attempts = Vec::new();
        let mut parent_evidence = Vec::new();
        let mut compatibility_pressures = Vec::new();
        let mut stop_generation = false;

        for parent_attempt in parent_attempts.iter().copied() {
            let parent_routing = attempts[parent_attempt]
                .routing
                .as_ref()
                .expect("beam parent pressure attempt always routed");
            let parent_components = parent_routing.candidate.components.clone();
            let pressures = component_pressures(
                problem,
                &parent_components,
                parent_routing,
                routing_config.grid_mm,
                repair_config.component_order,
                repair_config.pressure_sources,
                repair_config.direction_policy
                    == PressureDirectionPolicy::PassageCapacityWithCopper,
            )?;
            if parent_evidence.is_empty() {
                compatibility_pressures = pressures.clone();
            }
            let mut parent_candidates = Vec::new();

            'pressures: for pressure in pressures
                .iter()
                .take(repair_config.maximum_blocking_components)
            {
                let declared = problem
                    .components
                    .iter()
                    .find(|component| component.id == pressure.component)
                    .ok_or_else(|| format!("unknown pressure component {}", pressure.component))?;
                let solved = parent_components
                    .iter()
                    .find(|component| component.id == pressure.component)
                    .ok_or_else(|| format!("missing pressure component {}", pressure.component))?;
                let proposals = match repair_config.direction_policy {
                    PressureDirectionPolicy::FrontierCentroid
                    | PressureDirectionPolicy::FrontierCentroidBidirectional => {
                        let mut directions =
                            pressure_directions(pressure.vector, declared.constraints.movement);
                        if repair_config.direction_policy
                            == PressureDirectionPolicy::FrontierCentroidBidirectional
                        {
                            let opposites = directions
                                .iter()
                                .map(|direction| Vec2::new(-direction.x, -direction.y))
                                .collect::<Vec<_>>();
                            for opposite in opposites {
                                if !directions.contains(&opposite) {
                                    directions.push(opposite);
                                }
                            }
                        }
                        directions
                            .into_iter()
                            .flat_map(|direction| {
                                repair_config.step_scales.iter().map(move |scale| {
                                    (direction, repair_config.push_step_mm * scale)
                                })
                            })
                            .collect::<Vec<_>>()
                    }
                    PressureDirectionPolicy::PassageCapacity
                    | PressureDirectionPolicy::PassageCapacityWithCopper => pressure
                        .passage_proposals
                        .iter()
                        .map(|proposal| (proposal.direction, proposal.target_distance_mm))
                        .collect(),
                };
                for (direction, distance) in proposals {
                    if candidate_attempts.len() >= repair_config.maximum_candidates_per_iteration
                        || attempts.len() > repair_config.maximum_total_attempts
                    {
                        stop_generation = true;
                        break 'pressures;
                    }
                    let desired = Vec2::new(
                        solved.position.x + direction.x * distance,
                        solved.position.y + direction.y * distance,
                    );
                    let proposed = project_position(problem, declared, solved, desired)?;
                    let displacement = Vec2::new(
                        proposed.x - solved.position.x,
                        proposed.y - solved.position.y,
                    );
                    if displacement.x.abs() <= DIRECTION_EPSILON
                        && displacement.y.abs() <= DIRECTION_EPSILON
                    {
                        continue;
                    }
                    let index = attempts.len();
                    let id = format!("pressure-{index:04}");
                    let proposal = match repair_config.push_propagation {
                        PushPropagationPolicy::SingleComponent => {
                            let mut components = parent_components.clone();
                            components
                                .iter_mut()
                                .find(|component| component.id == pressure.component)
                                .expect("pressure component exists")
                                .position = proposed;
                            Ok(PushChainProposal {
                                components,
                                moved_components: vec![pressure.component.clone()],
                                contacts: Vec::new(),
                            })
                        }
                        PushPropagationPolicy::CollisionChain
                        | PushPropagationPolicy::RelationalAndCollisionChain => {
                            push_chain_proposal(
                                problem,
                                &parent_components,
                                &pressure.component,
                                displacement,
                                repair_config.maximum_components_per_push,
                                repair_config.push_propagation
                                    == PushPropagationPolicy::RelationalAndCollisionChain,
                            )
                        }
                    };
                    let proposal = match proposal {
                        Ok(proposal) => proposal,
                        Err(rejection) => {
                            attempts.push(PressureRepairAttempt {
                                id,
                                iteration,
                                parent_attempt,
                                blocker: Some(pressure.component.clone()),
                                pressure: Some(pressure.vector),
                                direction: Some(direction),
                                displacement,
                                moved_components: rejection.moved_components,
                                chain_contacts: rejection.contacts,
                                routing: None,
                                selective_ripup: None,
                                rejected_before_routing: Some(rejection.message),
                            });
                            candidate_attempts.push(index);
                            parent_candidates.push(index);
                            continue;
                        }
                    };
                    if !seen_placements.insert(component_pose_key(&proposal.components)) {
                        continue;
                    }
                    if let Err(error) = validate_pressure_placement(problem, &proposal.components) {
                        attempts.push(PressureRepairAttempt {
                            id,
                            iteration,
                            parent_attempt,
                            blocker: Some(pressure.component.clone()),
                            pressure: Some(pressure.vector),
                            direction: Some(direction),
                            displacement,
                            moved_components: proposal.moved_components,
                            chain_contacts: proposal.contacts,
                            routing: None,
                            selective_ripup: None,
                            rejected_before_routing: Some(format!(
                                "placement exclusion rejected proposal: {error}"
                            )),
                        });
                        candidate_attempts.push(index);
                        parent_candidates.push(index);
                        continue;
                    }
                    match route_problem_with_dut_grid_at_poses(
                        problem,
                        &proposal.components,
                        routing_config,
                    ) {
                        Ok(mut routing) => {
                            total_expansions =
                                total_expansions.saturating_add(routing.evidence.expansions);
                            let mut selective_ripup = None;
                            if let Some(selective_config) = &repair_config.selective_ripup {
                                let baseline_expansions = routing.evidence.expansions;
                                let repaired = repair_routing_result_with_selective_ripup(
                                    problem,
                                    &proposal.components,
                                    routing,
                                    routing_config,
                                    selective_config,
                                )?;
                                total_expansions = total_expansions.saturating_add(
                                    repaired
                                        .evidence
                                        .total_expansions
                                        .saturating_sub(baseline_expansions),
                                );
                                attempted_topology_repairs = attempted_topology_repairs
                                    .saturating_add(repaired.evidence.attempted_repairs);
                                routing = repaired.attempts[repaired.evidence.selected_attempt]
                                    .routing
                                    .clone();
                                selective_ripup = Some(repaired);
                            }
                            let rank = routing_rank(&routing, &initial_positions);
                            let selected_rank = routing_rank(
                                attempts[selected]
                                    .routing
                                    .as_ref()
                                    .expect("selected pressure attempt always routed"),
                                &initial_positions,
                            );
                            let complete = routing.complete();
                            attempts.push(PressureRepairAttempt {
                                id,
                                iteration,
                                parent_attempt,
                                blocker: Some(pressure.component.clone()),
                                pressure: Some(pressure.vector),
                                direction: Some(direction),
                                displacement,
                                moved_components: proposal.moved_components,
                                chain_contacts: proposal.contacts,
                                routing: Some(routing),
                                selective_ripup,
                                rejected_before_routing: None,
                            });
                            candidate_attempts.push(index);
                            parent_candidates.push(index);
                            if rank < selected_rank {
                                selected = index;
                            }
                            if complete && repair_config.stop_on_complete {
                                stop_generation = true;
                                break 'pressures;
                            }
                        }
                        Err(error) => {
                            attempts.push(PressureRepairAttempt {
                                id,
                                iteration,
                                parent_attempt,
                                blocker: Some(pressure.component.clone()),
                                pressure: Some(pressure.vector),
                                direction: Some(direction),
                                displacement,
                                moved_components: proposal.moved_components,
                                chain_contacts: proposal.contacts,
                                routing: None,
                                selective_ripup: None,
                                rejected_before_routing: Some(error),
                            });
                            candidate_attempts.push(index);
                            parent_candidates.push(index);
                        }
                    }
                }
            }
            parent_evidence.push(PressureRepairParentEvidence {
                parent_attempt,
                pressures,
                candidate_attempts: parent_candidates,
            });
            if stop_generation {
                break;
            }
        }

        let mut retained_attempts = candidate_attempts
            .iter()
            .copied()
            .filter(|attempt| attempts[*attempt].routing.is_some())
            .filter(|attempt| {
                repair_config.retain_non_improving_states
                    || routing_rank(
                        attempts[*attempt]
                            .routing
                            .as_ref()
                            .expect("retained candidate routed"),
                        &initial_positions,
                    ) < routing_rank(
                        attempts[attempts[*attempt].parent_attempt]
                            .routing
                            .as_ref()
                            .expect("candidate parent routed"),
                        &initial_positions,
                    )
            })
            .collect::<Vec<_>>();
        retained_attempts.sort_by_key(|attempt| {
            let routing = attempts[*attempt]
                .routing
                .as_ref()
                .expect("retained candidate routed");
            (
                routing_rank(routing, &initial_positions),
                component_pose_key(&routing.candidate.components),
                *attempt,
            )
        });
        retained_attempts.truncate(repair_config.beam_width);
        let improved = selected != prior_selected;
        let advanced = !retained_attempts.is_empty();
        iterations.push(PressureRepairIterationEvidence {
            iteration,
            parent_attempt: parent_attempts[0],
            parent_attempts,
            pressures: compatibility_pressures,
            candidate_attempts,
            retained_attempts: retained_attempts.clone(),
            selected_attempt: selected,
            improved,
            advanced,
            parents: parent_evidence,
        });
        frontier = retained_attempts;
        if attempts[selected]
            .routing
            .as_ref()
            .is_some_and(DutGridRoutingResult::complete)
            || !advanced
        {
            break;
        }
    }

    let selected_routing = attempts[selected]
        .routing
        .as_ref()
        .expect("selected pressure attempt always routed")
        .clone();
    let rejected_placements = attempts
        .iter()
        .filter(|attempt| attempt.rejected_before_routing.is_some())
        .count();
    let complete = selected_routing.complete();
    Ok(PressureRepairResult {
        candidate: selected_routing.candidate,
        validation: selected_routing.validation,
        evidence: PressureRepairEvidence {
            strategy: format!(
                "{}{}-v1",
                match (repair_config.direction_policy, repair_config.beam_width > 1) {
                    (PressureDirectionPolicy::FrontierCentroid, false) => {
                        "blocked-frontier-pressure"
                    }
                    (PressureDirectionPolicy::FrontierCentroidBidirectional, false) => {
                        "bidirectional-blocked-frontier-pressure"
                    }
                    (PressureDirectionPolicy::PassageCapacity, false) =>
                        "passage-capacity-pressure",
                    (PressureDirectionPolicy::FrontierCentroid, true) => {
                        "blocked-frontier-pressure-beam"
                    }
                    (PressureDirectionPolicy::FrontierCentroidBidirectional, true) => {
                        "bidirectional-blocked-frontier-pressure-beam"
                    }
                    (PressureDirectionPolicy::PassageCapacity, true) => {
                        "passage-capacity-pressure-beam"
                    }
                    (PressureDirectionPolicy::PassageCapacityWithCopper, false) => {
                        "occupied-passage-capacity-pressure"
                    }
                    (PressureDirectionPolicy::PassageCapacityWithCopper, true) => {
                        "occupied-passage-capacity-pressure-beam"
                    }
                },
                if repair_config.selective_ripup.is_some() {
                    "-selective-ripup"
                } else {
                    ""
                }
            ),
            config: repair_config.clone(),
            selected_attempt: selected,
            attempted_repairs: attempts.len() - 1,
            attempted_topology_repairs,
            rejected_placements,
            completed_iterations: iterations.len(),
            total_expansions,
            complete,
            iterations,
            routing: selected_routing.evidence,
        },
        attempts,
    })
}

fn placement_envelope_obb(
    declared: &layout_trace_model::model::Component,
    solved: &SolvedComponent,
    displacement: Vec2,
    expansion: f64,
) -> Obb {
    let mut minimum = Vec2::new(-declared.size.x * 0.5, -declared.size.y * 0.5);
    let mut maximum = Vec2::new(declared.size.x * 0.5, declared.size.y * 0.5);
    for pin in &declared.pins {
        for pad in &pin.pads {
            let center = pin.pad_local_center(pad);
            let extent = match pad.shape {
                CopperShape::Circle { diameter } => Vec2::new(diameter * 0.5, diameter * 0.5),
                CopperShape::Rect {
                    size,
                    rotation_degrees,
                } => {
                    let radians = rotation_degrees.to_radians();
                    let cosine = radians.cos().abs();
                    let sine = radians.sin().abs();
                    Vec2::new(
                        0.5 * (size.x * cosine + size.y * sine),
                        0.5 * (size.x * sine + size.y * cosine),
                    )
                }
            };
            minimum.x = minimum.x.min(center.x - extent.x);
            minimum.y = minimum.y.min(center.y - extent.y);
            maximum.x = maximum.x.max(center.x + extent.x);
            maximum.y = maximum.y.max(center.y + extent.y);
        }
    }
    let local_center = Vec2::new((minimum.x + maximum.x) * 0.5, (minimum.y + maximum.y) * 0.5);
    let radians = solved.rotation_degrees.to_radians();
    let rotated_center = Vec2::new(
        local_center.x * radians.cos() - local_center.y * radians.sin(),
        local_center.x * radians.sin() + local_center.y * radians.cos(),
    );
    Obb {
        center: Vec2::new(
            solved.position.x + displacement.x + rotated_center.x,
            solved.position.y + displacement.y + rotated_center.y,
        ),
        half_size: Vec2::new(
            (maximum.x - minimum.x + expansion) * 0.5,
            (maximum.y - minimum.y + expansion) * 0.5,
        ),
        rotation_degrees: solved.rotation_degrees,
    }
}

fn component_pose_key(components: &[SolvedComponent]) -> Vec<(String, u64, u64, u64)> {
    let mut key = components
        .iter()
        .map(|component| {
            (
                component.id.clone(),
                component.position.x.to_bits(),
                component.position.y.to_bits(),
                component.rotation_degrees.to_bits(),
            )
        })
        .collect::<Vec<_>>();
    key.sort_by(|left, right| left.0.cmp(&right.0));
    key
}

fn validate_pressure_placement(
    problem: &Problem,
    components: &[SolvedComponent],
) -> Result<(), String> {
    let poses = components
        .iter()
        .map(|component| pcb_placement::PlacementPose {
            component: component.id.clone(),
            position: component.position,
            rotation_degrees: component.rotation_degrees,
        })
        .collect::<Vec<_>>();
    pcb_placement::validate_serialized_placement_poses(problem, &poses)
}

fn moved_component_ids(components: &[SolvedComponent], moving: &BTreeSet<usize>) -> Vec<String> {
    moving
        .iter()
        .map(|index| components[*index].id.clone())
        .collect()
}

fn reject_push_chain(
    message: String,
    components: &[SolvedComponent],
    moving: &BTreeSet<usize>,
    contacts: Vec<PushChainContactEvidence>,
) -> PushChainRejection {
    PushChainRejection {
        message,
        moved_components: moved_component_ids(components, moving),
        contacts,
    }
}

fn placement_anchor_position(
    problem: &Problem,
    components: &[SolvedComponent],
    anchor: &PlacementAnchor,
) -> Option<Vec2> {
    let declared = problem
        .components
        .iter()
        .find(|component| component.id == anchor.component)?;
    let solved = components
        .iter()
        .find(|component| component.id == anchor.component)?;
    let offset = anchor
        .pin
        .as_ref()
        .and_then(|pin| declared.pins.iter().find(|candidate| candidate.id == *pin))
        .map_or(Vec2::ZERO, |pin| pin.offset);
    let radians = solved.rotation_degrees.to_radians();
    let rotated = Vec2::new(
        offset.x * radians.cos() - offset.y * radians.sin(),
        offset.x * radians.sin() + offset.y * radians.cos(),
    );
    Some(Vec2::new(
        solved.position.x + rotated.x,
        solved.position.y + rotated.y,
    ))
}

/// Close a same-vector translation over newly contacted component bodies and,
/// when selected, maximum-distance relationships that the translation would
/// otherwise violate.
fn push_chain_proposal(
    problem: &Problem,
    components: &[SolvedComponent],
    initial_component: &str,
    displacement: Vec2,
    maximum_components: usize,
    propagate_relational_constraints: bool,
) -> Result<PushChainProposal, PushChainRejection> {
    let Some(initial_index) = components
        .iter()
        .position(|component| component.id == initial_component)
    else {
        return Err(PushChainRejection {
            message: format!("push chain starts from unknown component {initial_component}"),
            moved_components: Vec::new(),
            contacts: Vec::new(),
        });
    };
    let declared = problem
        .components
        .iter()
        .map(|component| (component.id.as_str(), component))
        .collect::<BTreeMap<_, _>>();
    let mut moving = BTreeSet::from([initial_index]);
    let mut contacts = Vec::new();

    loop {
        let mut additions = BTreeMap::<usize, (usize, PushChainContactKind)>::new();
        if propagate_relational_constraints {
            for constraint in &problem.placement_constraints {
                let PlacementConstraint::MaximumDistance {
                    first,
                    second,
                    maximum,
                } = constraint
                else {
                    continue;
                };
                let Some(first_index) = components
                    .iter()
                    .position(|component| component.id == first.component)
                else {
                    continue;
                };
                let Some(second_index) = components
                    .iter()
                    .position(|component| component.id == second.component)
                else {
                    continue;
                };
                let (moving_index, other_index, moving_anchor, other_anchor) = match (
                    moving.contains(&first_index),
                    moving.contains(&second_index),
                ) {
                    (true, false) => (first_index, second_index, first, second),
                    (false, true) => (second_index, first_index, second, first),
                    _ => continue,
                };
                let Some(moving_position) =
                    placement_anchor_position(problem, components, moving_anchor)
                else {
                    continue;
                };
                let Some(other_position) =
                    placement_anchor_position(problem, components, other_anchor)
                else {
                    continue;
                };
                let translated_distance = (moving_position.x + displacement.x - other_position.x)
                    .hypot(moving_position.y + displacement.y - other_position.y);
                if translated_distance > *maximum + DIRECTION_EPSILON {
                    additions
                        .entry(other_index)
                        .or_insert((moving_index, PushChainContactKind::MaximumDistance));
                }
            }
        }
        for moved_index in moving.iter().copied() {
            for other_index in 0..components.len() {
                if moving.contains(&other_index) || additions.contains_key(&other_index) {
                    continue;
                }
                let moved_declared = declared
                    .get(components[moved_index].id.as_str())
                    .expect("solved component is declared");
                let other_declared = declared
                    .get(components[other_index].id.as_str())
                    .expect("solved component is declared");
                let both_have_copper = moved_declared.pins.iter().any(|pin| !pin.pads.is_empty())
                    && other_declared.pins.iter().any(|pin| !pin.pads.is_empty());
                let expansion = if both_have_copper {
                    (problem.rules.clearance - 2.0 * PLACEMENT_CONTACT_TOLERANCE_MM).max(0.0)
                } else {
                    0.0
                };
                let old_overlap = obbs_overlap(
                    placement_envelope_obb(
                        moved_declared,
                        &components[moved_index],
                        Vec2::ZERO,
                        expansion,
                    ),
                    placement_envelope_obb(
                        other_declared,
                        &components[other_index],
                        Vec2::ZERO,
                        expansion,
                    ),
                );
                let new_overlap = obbs_overlap(
                    placement_envelope_obb(
                        moved_declared,
                        &components[moved_index],
                        displacement,
                        expansion,
                    ),
                    placement_envelope_obb(
                        other_declared,
                        &components[other_index],
                        Vec2::ZERO,
                        expansion,
                    ),
                );
                if new_overlap && !old_overlap {
                    let kind = if both_have_copper {
                        PushChainContactKind::CopperEnvelopeContact
                    } else {
                        PushChainContactKind::BodyContact
                    };
                    additions.entry(other_index).or_insert((moved_index, kind));
                }
            }
        }
        if additions.is_empty() {
            break;
        }
        for (other_index, (moved_index, kind)) in additions {
            let other = &components[other_index];
            contacts.push(PushChainContactEvidence {
                pushing_component: components[moved_index].id.clone(),
                yielding_component: other.id.clone(),
                kind,
            });
            let Some(other_declared) = declared.get(other.id.as_str()) else {
                return Err(reject_push_chain(
                    format!("push chain contacted undeclared component {}", other.id),
                    components,
                    &moving,
                    contacts,
                ));
            };
            if other_declared.constraints.movement == Movement::Fixed {
                return Err(reject_push_chain(
                    format!("push chain contacted fixed component {}", other.id),
                    components,
                    &moving,
                    contacts,
                ));
            }
            moving.insert(other_index);
            if moving.len() > maximum_components {
                return Err(reject_push_chain(
                    format!(
                        "push chain requires {} components, exceeding limit {maximum_components}",
                        moving.len()
                    ),
                    components,
                    &moving,
                    contacts,
                ));
            }
        }
    }

    let mut proposal = components.to_vec();
    for index in moving.iter().copied() {
        let component = &components[index];
        let component_declared = declared
            .get(component.id.as_str())
            .expect("solved component is declared");
        let desired = Vec2::new(
            component.position.x + displacement.x,
            component.position.y + displacement.y,
        );
        let projected = project_position(problem, component_declared, component, desired)
            .map_err(|message| reject_push_chain(message, components, &moving, contacts.clone()))?;
        if (projected.x - desired.x).abs() > DIRECTION_EPSILON
            || (projected.y - desired.y).abs() > DIRECTION_EPSILON
        {
            return Err(reject_push_chain(
                format!(
                    "push chain component {} cannot accept displacement ({:.6}, {:.6})",
                    component.id, displacement.x, displacement.y
                ),
                components,
                &moving,
                contacts,
            ));
        }
        proposal[index].position = desired;
    }

    Ok(PushChainProposal {
        components: proposal,
        moved_components: moved_component_ids(components, &moving),
        contacts,
    })
}

fn component_pressures(
    problem: &Problem,
    components: &[SolvedComponent],
    routing: &DutGridRoutingResult,
    routing_grid_mm: f64,
    order: PressureComponentOrder,
    sources: PressureSourcePolicy,
    include_copper: bool,
) -> Result<Vec<ComponentPressureEvidence>, String> {
    let movable = problem
        .components
        .iter()
        .filter(|component| component.constraints.movement != Movement::Fixed)
        .map(|component| component.id.as_str())
        .collect::<BTreeSet<_>>();
    let centers = components
        .iter()
        .map(|component| (component.id.as_str(), component.position))
        .collect::<BTreeMap<_, _>>();
    let mut accumulators = BTreeMap::<String, PressureAccumulator>::new();
    for branch in routing
        .evidence
        .branches
        .iter()
        .filter(|branch| branch.status != pcb_routing::BranchRoutingStatus::Found)
    {
        for blocker in &branch.blockers {
            if !matches!(
                blocker.kind,
                RoutingBlockerKind::ComponentBody
                    | RoutingBlockerKind::Keepout
                    | RoutingBlockerKind::Pad
            ) || !movable.contains(blocker.object.as_str())
            {
                continue;
            }
            let Some(center) = centers.get(blocker.object.as_str()) else {
                continue;
            };
            let accumulator = accumulators.entry(blocker.object.clone()).or_default();
            accumulator.sum_x +=
                (center.x - blocker.frontier_centroid.x) * blocker.frontier_hits as f64;
            accumulator.sum_y +=
                (center.y - blocker.frontier_centroid.y) * blocker.frontier_hits as f64;
            accumulator.hits += blocker.frontier_hits;
            accumulator.vector_samples += blocker.frontier_hits;
            accumulator.branches.insert(branch.branch.clone());
            accumulator.required_gap_mm = accumulator
                .required_gap_mm
                .max(branch.width + 2.0 * problem.rules.clearance);
        }
    }
    if sources == PressureSourcePolicy::BlockersAndFailedBranchTerminals {
        for branch in routing
            .evidence
            .branches
            .iter()
            .filter(|branch| branch.status != pcb_routing::BranchRoutingStatus::Found)
        {
            let Some(net) = problem.nets.iter().find(|net| net.id == branch.branch) else {
                continue;
            };
            for (endpoint, opposite) in [
                (&net.from.component, &net.to.component),
                (&net.to.component, &net.from.component),
            ] {
                if endpoint == opposite || !movable.contains(endpoint.as_str()) {
                    continue;
                }
                let Some(position) = centers.get(endpoint.as_str()) else {
                    continue;
                };
                let Some(opposite_position) = centers.get(opposite.as_str()) else {
                    continue;
                };
                let accumulator = accumulators.entry(endpoint.clone()).or_default();
                accumulator.sum_x += opposite_position.x - position.x;
                accumulator.sum_y += opposite_position.y - position.y;
                accumulator.vector_samples += 1;
                accumulator.branches.insert(branch.branch.clone());
                accumulator.terminal_branches.insert(branch.branch.clone());
                accumulator.required_gap_mm = accumulator
                    .required_gap_mm
                    .max(branch.width + 2.0 * problem.rules.clearance);
            }
        }
    }
    let mut pressures = accumulators
        .into_iter()
        .map(|(component, accumulator)| {
            let solved = components
                .iter()
                .find(|candidate| candidate.id == component)
                .expect("pressure accumulator has a solved component");
            let declared = problem
                .components
                .iter()
                .find(|candidate| candidate.id == component)
                .expect("pressure accumulator has a declared component");
            Ok(ComponentPressureEvidence {
                component,
                vector: Vec2::new(
                    accumulator.sum_x / accumulator.vector_samples as f64,
                    accumulator.sum_y / accumulator.vector_samples as f64,
                ),
                frontier_hits: accumulator.hits,
                trigger_branches: accumulator.branches.iter().cloned().collect(),
                terminal_for_failed_branches: accumulator.terminal_branches.into_iter().collect(),
                passage_proposals: passage_proposals(
                    problem,
                    components,
                    declared,
                    solved,
                    accumulator.required_gap_mm,
                    routing_grid_mm,
                    include_copper.then_some((&routing.candidate.traces, &accumulator.branches)),
                )?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    sort_component_pressures(&mut pressures, order);
    Ok(pressures)
}

fn sort_component_pressures(
    pressures: &mut [ComponentPressureEvidence],
    order: PressureComponentOrder,
) {
    pressures.sort_by(|left, right| {
        let branch_order = match order {
            PressureComponentOrder::FrontierHits => std::cmp::Ordering::Equal,
            PressureComponentOrder::TriggerBranchesThenHits => right
                .trigger_branches
                .len()
                .cmp(&left.trigger_branches.len()),
            PressureComponentOrder::FailedTerminalsThenTriggerBranchesThenHits => left
                .terminal_for_failed_branches
                .is_empty()
                .cmp(&right.terminal_for_failed_branches.is_empty())
                .then_with(|| {
                    right
                        .trigger_branches
                        .len()
                        .cmp(&left.trigger_branches.len())
                }),
        };
        branch_order
            .then_with(|| right.frontier_hits.cmp(&left.frontier_hits))
            .then_with(|| left.component.cmp(&right.component))
    });
}

#[derive(Clone, Copy)]
struct AxisAlignedBounds {
    min: Vec2,
    max: Vec2,
}

fn component_bounds(component: &SolvedComponent) -> AxisAlignedBounds {
    let radians = component.rotation_degrees.to_radians();
    let extent = Vec2::new(
        component.size.x * 0.5 * radians.cos().abs() + component.size.y * 0.5 * radians.sin().abs(),
        component.size.x * 0.5 * radians.sin().abs() + component.size.y * 0.5 * radians.cos().abs(),
    );
    AxisAlignedBounds {
        min: Vec2::new(
            component.position.x - extent.x,
            component.position.y - extent.y,
        ),
        max: Vec2::new(
            component.position.x + extent.x,
            component.position.y + extent.y,
        ),
    }
}

fn intervals_overlap(first: (f64, f64), second: (f64, f64)) -> bool {
    first.1 >= second.0 - DIRECTION_EPSILON && second.1 >= first.0 - DIRECTION_EPSILON
}

fn passage_gap(
    problem: &Problem,
    components: &[SolvedComponent],
    blocker: &str,
    mover: AxisAlignedBounds,
    direction: Vec2,
) -> (f64, String) {
    let mut limiting = if direction.x > 0.0 {
        (
            mover.min.x - problem.board.bounds.min.x,
            "board:left".to_string(),
        )
    } else if direction.x < 0.0 {
        (
            problem.board.bounds.max.x - mover.max.x,
            "board:right".to_string(),
        )
    } else if direction.y > 0.0 {
        (
            mover.min.y - problem.board.bounds.min.y,
            "board:top".to_string(),
        )
    } else {
        (
            problem.board.bounds.max.y - mover.max.y,
            "board:bottom".to_string(),
        )
    };
    for other in components
        .iter()
        .filter(|component| component.id != blocker)
    {
        let Some(declared) = problem
            .components
            .iter()
            .find(|candidate| candidate.id == other.id)
        else {
            continue;
        };
        if !declared.body_is_routing_keepout {
            continue;
        }
        let bounds = component_bounds(other);
        let gap = if direction.x > 0.0
            && intervals_overlap((mover.min.y, mover.max.y), (bounds.min.y, bounds.max.y))
            && bounds.max.x <= mover.min.x + DIRECTION_EPSILON
        {
            Some(mover.min.x - bounds.max.x)
        } else if direction.x < 0.0
            && intervals_overlap((mover.min.y, mover.max.y), (bounds.min.y, bounds.max.y))
            && bounds.min.x >= mover.max.x - DIRECTION_EPSILON
        {
            Some(bounds.min.x - mover.max.x)
        } else if direction.y > 0.0
            && intervals_overlap((mover.min.x, mover.max.x), (bounds.min.x, bounds.max.x))
            && bounds.max.y <= mover.min.y + DIRECTION_EPSILON
        {
            Some(mover.min.y - bounds.max.y)
        } else if direction.y < 0.0
            && intervals_overlap((mover.min.x, mover.max.x), (bounds.min.x, bounds.max.x))
            && bounds.min.y >= mover.max.y - DIRECTION_EPSILON
        {
            Some(bounds.min.y - mover.max.y)
        } else {
            None
        };
        if let Some(gap) = gap
            && gap < limiting.0
        {
            limiting = (gap, other.id.clone());
        }
    }
    limiting
}

fn passage_proposals(
    problem: &Problem,
    components: &[SolvedComponent],
    declared: &layout_trace_model::model::Component,
    solved: &SolvedComponent,
    required_gap_mm: f64,
    routing_grid_mm: f64,
    copper: Option<(&[pcb_validate::SolvedTrace], &BTreeSet<String>)>,
) -> Result<Vec<PassageProposalEvidence>, String> {
    let directions = pressure_directions(Vec2::ZERO, declared.constraints.movement);
    let mover = component_bounds(solved);
    let mut proposals = Vec::new();
    for direction in directions {
        let (current_gap_mm, limiting_object) =
            passage_gap(problem, components, &solved.id, mover, direction);
        let copper_demand = copper.and_then(|(traces, branches)| {
            passage_occupancy::demand(problem, traces, branches, mover, direction, current_gap_mm)
        });
        let required_gap_mm = copper_demand
            .as_ref()
            .map_or(required_gap_mm, |d| required_gap_mm.max(d.required_gap_mm));
        let deficit = (required_gap_mm - current_gap_mm).max(0.0);
        if deficit <= DIRECTION_EPSILON {
            continue;
        }
        // A physically sufficient sub-cell opening can remain absent from the
        // coarse raster. Round outward to the evaluator's first grid and let
        // legal-region projection clamp an unattainable target.
        let discretized =
            ((deficit - DIRECTION_EPSILON).max(0.0) / routing_grid_mm).ceil() * routing_grid_mm;
        let desired = Vec2::new(
            solved.position.x + direction.x * discretized,
            solved.position.y + direction.y * discretized,
        );
        let projected = project_position(problem, declared, solved, desired)?;
        let target_distance_mm =
            (projected.x - solved.position.x).hypot(projected.y - solved.position.y);
        if target_distance_mm <= DIRECTION_EPSILON {
            continue;
        }
        proposals.push(PassageProposalEvidence {
            direction,
            limiting_object,
            current_gap_mm,
            required_gap_mm,
            physical_deficit_mm: deficit,
            target_distance_mm,
            remaining_deficit_mm: (deficit - target_distance_mm).max(0.0),
            copper_demand,
        });
    }
    proposals.sort_by(|left, right| {
        left.remaining_deficit_mm
            .total_cmp(&right.remaining_deficit_mm)
            .then_with(|| {
                left.physical_deficit_mm
                    .total_cmp(&right.physical_deficit_mm)
            })
            .then_with(|| left.target_distance_mm.total_cmp(&right.target_distance_mm))
            .then_with(|| left.limiting_object.cmp(&right.limiting_object))
            .then_with(|| left.direction.x.total_cmp(&right.direction.x))
            .then_with(|| left.direction.y.total_cmp(&right.direction.y))
    });
    Ok(proposals)
}

fn pressure_directions(vector: Vec2, movement: Movement) -> Vec<Vec2> {
    let mut x = if vector.x < -DIRECTION_EPSILON {
        -1.0
    } else if vector.x > DIRECTION_EPSILON {
        1.0
    } else {
        0.0
    };
    let mut y = if vector.y < -DIRECTION_EPSILON {
        -1.0
    } else if vector.y > DIRECTION_EPSILON {
        1.0
    } else {
        0.0
    };
    match movement {
        Movement::Fixed => return Vec::new(),
        Movement::Horizontal => y = 0.0,
        Movement::Vertical => x = 0.0,
        Movement::Free => {}
    }
    let mut directions = Vec::new();
    if x != 0.0 && y != 0.0 {
        directions.push(Vec2::new(x, y));
    }
    if x != 0.0 {
        directions.push(Vec2::new(x, 0.0));
    }
    if y != 0.0 {
        directions.push(Vec2::new(0.0, y));
    }
    if directions.is_empty() {
        if matches!(movement, Movement::Horizontal | Movement::Free) {
            directions.extend([Vec2::new(1.0, 0.0), Vec2::new(-1.0, 0.0)]);
        }
        if matches!(movement, Movement::Vertical | Movement::Free) {
            directions.extend([Vec2::new(0.0, 1.0), Vec2::new(0.0, -1.0)]);
        }
    }
    directions
}

fn project_position(
    problem: &Problem,
    declared: &layout_trace_model::model::Component,
    solved: &SolvedComponent,
    desired: Vec2,
) -> Result<Vec2, String> {
    let region = declared.constraints.region.unwrap_or(problem.board.bounds);
    let bounds = layout_trace_model::Rect {
        min: Vec2::new(
            region.min.x.max(problem.board.bounds.min.x),
            region.min.y.max(problem.board.bounds.min.y),
        ),
        max: Vec2::new(
            region.max.x.min(problem.board.bounds.max.x),
            region.max.y.min(problem.board.bounds.max.y),
        ),
    };
    let radians = solved.rotation_degrees.to_radians();
    let extent = Vec2::new(
        solved.size.x * 0.5 * radians.cos().abs() + solved.size.y * 0.5 * radians.sin().abs(),
        solved.size.x * 0.5 * radians.sin().abs() + solved.size.y * 0.5 * radians.cos().abs(),
    );
    let mut minimum = Vec2::new(bounds.min.x + extent.x, bounds.min.y + extent.y);
    let mut maximum = Vec2::new(bounds.max.x - extent.x, bounds.max.y - extent.y);
    if minimum.x > maximum.x + DIRECTION_EPSILON || minimum.y > maximum.y + DIRECTION_EPSILON {
        return Err(format!(
            "pressure component {} has an empty movement region",
            declared.id
        ));
    }
    // A quarter-turn can make an exactly fitted body exceed its interval by
    // a few floating-point ulps (`cos(270°)` is not exactly zero). Collapse
    // only that tolerance-sized inversion; a materially empty region still
    // fails above.
    if minimum.x > maximum.x {
        let midpoint = (minimum.x + maximum.x) * 0.5;
        minimum.x = midpoint;
        maximum.x = midpoint;
    }
    if minimum.y > maximum.y {
        let midpoint = (minimum.y + maximum.y) * 0.5;
        minimum.y = midpoint;
        maximum.y = midpoint;
    }
    let mut projected = Vec2::new(
        desired.x.clamp(minimum.x, maximum.x),
        desired.y.clamp(minimum.y, maximum.y),
    );
    match declared.constraints.movement {
        Movement::Fixed => projected = declared.position,
        Movement::Horizontal => projected.y = declared.position.y,
        Movement::Vertical => projected.x = declared.position.x,
        Movement::Free => {}
    }
    Ok(projected)
}

fn routing_rank(
    result: &DutGridRoutingResult,
    initial_positions: &BTreeMap<&str, Vec2>,
) -> (usize, usize, u64, u64) {
    let displacement = result
        .candidate
        .components
        .iter()
        .filter_map(|component| {
            initial_positions.get(component.id.as_str()).map(|initial| {
                (component.position.x - initial.x).hypot(component.position.y - initial.y)
            })
        })
        .sum::<f64>();
    (
        result.evidence.failed_branches,
        result.validation.violations.len(),
        (displacement * 1_000_000.0).round() as u64,
        result.evidence.expansions,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wall_problem() -> Problem {
        serde_json::from_str(include_str!(
            "../../../benchmarks/imported/layout-trace/placement-perturbation-unblocks-routing.json"
        ))
        .unwrap()
    }

    fn asymmetric_wall_problem() -> Problem {
        serde_json::from_str(include_str!(
            "../../../benchmarks/small/passage-pressure-asymmetric.json"
        ))
        .unwrap()
    }

    #[test]
    fn exact_fit_quarter_turn_keeps_the_free_axis_projectable() {
        let problem: Problem = serde_json::from_str(include_str!(
            "../../../benchmarks/imported/layout-trace/dual-esp32-benchmark.json"
        ))
        .unwrap();
        let declared = problem
            .components
            .iter()
            .find(|component| component.id == "ESP_W")
            .unwrap();
        let solved = SolvedComponent {
            id: declared.id.clone(),
            position: Vec2::new(12.75, 28.33528683098293),
            size: declared.size,
            rotation_degrees: 270.0,
        };

        let projected = project_position(
            &problem,
            declared,
            &solved,
            Vec2::new(solved.position.x, solved.position.y + 1.0),
        )
        .unwrap();

        assert!((projected.x - 12.75).abs() <= DIRECTION_EPSILON);
        assert!((projected.y - (solved.position.y + 1.0)).abs() <= DIRECTION_EPSILON);
    }

    fn push_chain_problem() -> Problem {
        serde_json::from_str(include_str!(
            "../../../benchmarks/small/passage-pressure-push-chain.json"
        ))
        .unwrap()
    }

    fn declared_components(problem: &Problem) -> Vec<SolvedComponent> {
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

    fn routing_config() -> DutGridRoutingConfig {
        DutGridRoutingConfig {
            retry_grid_mm: vec![0.25, 0.1],
            max_expansions_per_search: 5_000_000,
            ..DutGridRoutingConfig::default()
        }
    }

    #[test]
    fn spatial_frontier_produces_semantic_component_pressure() {
        let problem = wall_problem();
        let components = declared_components(&problem);
        let routing =
            route_problem_with_dut_grid_at_poses(&problem, &components, &routing_config()).unwrap();
        let pressures = component_pressures(
            &problem,
            &components,
            &routing,
            routing_config().grid_mm,
            PressureComponentOrder::FrontierHits,
            PressureSourcePolicy::BlockersOnly,
            false,
        )
        .unwrap();
        assert_eq!(pressures.len(), 1);
        assert_eq!(pressures[0].component, "WALL");
        assert_eq!(pressures[0].trigger_branches, ["SIGNAL"]);
        assert_eq!(pressures[0].frontier_hits, 199);
        assert!(pressures[0].vector.x > 3.0);
        assert!(pressures[0].vector.y.abs() < 1.0e-9);
        assert_eq!(pressures[0].passage_proposals.len(), 2);
        assert!(
            pressures[0]
                .passage_proposals
                .iter()
                .all(|proposal| (proposal.target_distance_mm - 0.8).abs() < 1.0e-9)
        );
        assert_eq!(
            pressure_directions(pressures[0].vector, Movement::Vertical),
            [Vec2::new(0.0, 1.0), Vec2::new(0.0, -1.0)]
        );
    }

    #[test]
    fn multi_branch_pressure_order_is_explicit_and_stable() {
        let pressure =
            |component: &str, hits: usize, branches: &[&str]| ComponentPressureEvidence {
                component: component.into(),
                vector: Vec2::ZERO,
                frontier_hits: hits,
                trigger_branches: branches.iter().map(|branch| (*branch).into()).collect(),
                terminal_for_failed_branches: Vec::new(),
                passage_proposals: Vec::new(),
            };
        let original = vec![
            pressure("LARGE", 100, &["A"]),
            pressure("SHARED", 10, &["A", "B", "C"]),
            pressure("MEDIUM", 50, &["A", "B"]),
        ];
        let mut hits = original.clone();
        sort_component_pressures(&mut hits, PressureComponentOrder::FrontierHits);
        assert_eq!(
            hits.iter()
                .map(|item| item.component.as_str())
                .collect::<Vec<_>>(),
            ["LARGE", "MEDIUM", "SHARED"]
        );
        let mut branches = original;
        sort_component_pressures(
            &mut branches,
            PressureComponentOrder::TriggerBranchesThenHits,
        );
        assert_eq!(
            branches
                .iter()
                .map(|item| item.component.as_str())
                .collect::<Vec<_>>(),
            ["SHARED", "MEDIUM", "LARGE"]
        );
        branches[2].terminal_for_failed_branches = vec!["A".into()];
        sort_component_pressures(
            &mut branches,
            PressureComponentOrder::FailedTerminalsThenTriggerBranchesThenHits,
        );
        assert_eq!(branches[0].component, "LARGE");
    }

    #[test]
    fn pressure_feedback_solves_the_one_wall_control_transactionally() {
        let problem = wall_problem();
        let components = declared_components(&problem);
        let result = repair_routing_with_pressure(
            &problem,
            &components,
            &routing_config(),
            &PressureRepairConfig::default(),
        )
        .unwrap();
        assert!(result.evidence.complete);
        assert!(result.evidence.selected_attempt > 0);
        assert!(result.evidence.attempted_repairs <= 4);
        assert_eq!(result.evidence.completed_iterations, 1);
        assert_eq!(result.evidence.iterations[0].pressures[0].component, "WALL");
        let selected = &result.attempts[result.evidence.selected_attempt];
        assert_eq!(selected.blocker.as_deref(), Some("WALL"));
        assert_eq!(selected.direction, Some(Vec2::new(0.0, 1.0)));
        assert!(selected.displacement.y > 0.0);
        assert!(selected.displacement.y <= 0.8 + 1.0e-9);
    }

    #[test]
    fn asymmetric_passage_distinguishes_pressure_from_axis_sampling() {
        let problem = asymmetric_wall_problem();
        let components = declared_components(&problem);
        let axis = crate::repair_routing_blockers(
            &problem,
            &components,
            &routing_config(),
            &crate::BlockerRepairConfig::default(),
        )
        .unwrap();
        let frontier = repair_routing_with_pressure(
            &problem,
            &components,
            &routing_config(),
            &PressureRepairConfig::default(),
        )
        .unwrap();
        let passage = repair_routing_with_pressure(
            &problem,
            &components,
            &routing_config(),
            &PressureRepairConfig {
                direction_policy: PressureDirectionPolicy::PassageCapacity,
                ..PressureRepairConfig::default()
            },
        )
        .unwrap();

        assert!(axis.evidence.complete);
        assert!(frontier.evidence.complete);
        assert!(passage.evidence.complete);
        assert_eq!(axis.evidence.attempted_repairs, 3);
        assert_eq!(frontier.evidence.attempted_repairs, 1);
        assert_eq!(passage.evidence.attempted_repairs, 1);
        assert!(frontier.evidence.iterations[0].pressures[0].vector.y < 0.0);
        let proposal = &passage.evidence.iterations[0].pressures[0].passage_proposals[0];
        assert_eq!(proposal.direction, Vec2::new(0.0, -1.0));
        assert_eq!(proposal.limiting_object, "BOTTOM_WALL");
        assert!((proposal.current_gap_mm - 1.5).abs() < 1.0e-9);
        assert!((proposal.required_gap_mm - 1.6).abs() < 1.0e-9);
        assert!((proposal.target_distance_mm - 0.5).abs() < 1.0e-9);
        let axis_expansions = axis
            .attempts
            .iter()
            .filter_map(|attempt| attempt.routing.as_ref())
            .map(|routing| routing.evidence.expansions)
            .sum::<u64>();
        assert!(passage.evidence.total_expansions < axis_expansions);
    }

    #[test]
    fn collision_chain_adds_capability_on_a_two_body_control() {
        let problem = push_chain_problem();
        let components = declared_components(&problem);
        let axis = crate::repair_routing_blockers(
            &problem,
            &components,
            &routing_config(),
            &crate::BlockerRepairConfig::default(),
        )
        .unwrap();
        let single = repair_routing_with_pressure(
            &problem,
            &components,
            &routing_config(),
            &PressureRepairConfig {
                direction_policy: PressureDirectionPolicy::PassageCapacity,
                ..PressureRepairConfig::default()
            },
        )
        .unwrap();
        let chain = repair_routing_with_pressure(
            &problem,
            &components,
            &routing_config(),
            &PressureRepairConfig {
                direction_policy: PressureDirectionPolicy::PassageCapacity,
                push_propagation: PushPropagationPolicy::CollisionChain,
                maximum_components_per_push: 2,
                ..PressureRepairConfig::default()
            },
        )
        .unwrap();

        assert!(!axis.evidence.complete);
        assert!(!single.evidence.complete);
        assert_eq!(single.evidence.attempted_repairs, 1);
        assert_eq!(single.evidence.rejected_placements, 1);
        assert!(chain.evidence.complete);
        assert_eq!(chain.evidence.attempted_repairs, 1);
        assert_eq!(chain.evidence.rejected_placements, 0);

        let proposal = &chain.evidence.iterations[0].pressures[0].passage_proposals[0];
        assert_eq!(proposal.direction, Vec2::new(0.0, 1.0));
        assert_eq!(proposal.limiting_object, "TOP_WALL");
        assert!((proposal.physical_deficit_mm - 0.5).abs() < 1.0e-9);
        let selected = &chain.attempts[chain.evidence.selected_attempt];
        assert_eq!(
            selected
                .moved_components
                .iter()
                .map(String::as_str)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["FOLLOWER", "WALL"])
        );
        assert_eq!(
            selected.chain_contacts,
            [PushChainContactEvidence {
                pushing_component: "WALL".into(),
                yielding_component: "FOLLOWER".into(),
                kind: PushChainContactKind::BodyContact,
            }]
        );
        let bounded_rejection =
            match push_chain_proposal(&problem, &components, "WALL", Vec2::new(0.0, 0.5), 1, false)
            {
                Ok(_) => panic!("one-member chain unexpectedly accepted a contacted follower"),
                Err(rejection) => rejection,
            };
        assert!(bounded_rejection.message.contains("exceeding limit 1"));
        assert_eq!(bounded_rejection.contacts, selected.chain_contacts);
        for id in ["WALL", "FOLLOWER"] {
            let before = components
                .iter()
                .find(|component| component.id == id)
                .unwrap();
            let after = chain
                .candidate
                .components
                .iter()
                .find(|component| component.id == id)
                .unwrap();
            assert!((after.position.y - before.position.y - 0.5).abs() < 1.0e-9);
        }
    }

    #[test]
    fn relational_chain_pulls_only_a_neighbor_that_would_break_its_limit() {
        let mut problem = wall_problem();
        let bottom = problem
            .components
            .iter_mut()
            .find(|component| component.id == "BOTTOM_WALL")
            .unwrap();
        bottom.constraints.movement = Movement::Vertical;
        bottom.constraints.region = None;
        problem
            .placement_constraints
            .push(PlacementConstraint::MaximumDistance {
                first: PlacementAnchor {
                    component: "WALL".into(),
                    pin: None,
                },
                second: PlacementAnchor {
                    component: "BOTTOM_WALL".into(),
                    pin: None,
                },
                maximum: 10.5,
            });
        let components = declared_components(&problem);

        let proposal =
            push_chain_proposal(&problem, &components, "WALL", Vec2::new(0.0, -0.5), 2, true)
                .unwrap();

        assert_eq!(
            proposal
                .moved_components
                .iter()
                .map(String::as_str)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["BOTTOM_WALL", "WALL"])
        );
        assert_eq!(
            proposal.contacts,
            [PushChainContactEvidence {
                pushing_component: "WALL".into(),
                yielding_component: "BOTTOM_WALL".into(),
                kind: PushChainContactKind::MaximumDistance,
            }]
        );
    }

    #[test]
    fn push_chain_propagates_pad_extended_clearance_before_body_contact() {
        let mut problem = wall_problem();
        let pad_pin = |id: &str, y: f64| {
            serde_json::from_value(serde_json::json!({
                "id": id,
                "offset": {"x": 0.0, "y": y},
                "pads": [{
                    "id": "pad",
                    "layer": "top",
                    "shape": {"kind": "circle", "diameter": 0.2}
                }]
            }))
            .unwrap()
        };
        problem
            .components
            .iter_mut()
            .find(|component| component.id == "WALL")
            .unwrap()
            .pins
            .push(pad_pin("edge", 5.5));
        let bottom = problem
            .components
            .iter_mut()
            .find(|component| component.id == "BOTTOM_WALL")
            .unwrap();
        bottom.constraints.movement = Movement::Vertical;
        bottom.pins.push(pad_pin("edge", -4.0));
        let components = declared_components(&problem);

        let proposal =
            push_chain_proposal(&problem, &components, "WALL", Vec2::new(0.0, 0.5), 2, false)
                .unwrap();

        assert_eq!(
            proposal.contacts,
            [PushChainContactEvidence {
                pushing_component: "WALL".into(),
                yielding_component: "BOTTOM_WALL".into(),
                kind: PushChainContactKind::CopperEnvelopeContact,
            }]
        );
        assert_eq!(
            proposal
                .moved_components
                .iter()
                .map(String::as_str)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["BOTTOM_WALL", "WALL"])
        );
    }
}
