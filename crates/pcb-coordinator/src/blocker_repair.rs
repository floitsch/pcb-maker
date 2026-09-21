// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

use std::collections::{BTreeMap, BTreeSet};

use layout_trace_model::{Problem, Vec2, model::Movement};
use pcb_routing::{
    DutGridRoutingConfig, DutGridRoutingEvidence, DutGridRoutingResult, RoutingBlockerKind,
    route_problem_with_dut_grid_at_poses,
};
use pcb_validate::{CandidateArtifact, ExactValidationAssessment, SolvedComponent};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BlockerRepairConfig {
    /// Maximum distinct movable blockers examined from the initial failure.
    pub maximum_blocking_components: usize,
    /// Uniform samples across each permitted translation axis, including the
    /// allowed extremes. Axis moves are separate experiments for free bodies.
    pub samples_per_axis: usize,
    pub maximum_attempts: usize,
    pub stop_on_complete: bool,
}

impl BlockerRepairConfig {
    pub fn check(&self) -> Result<(), String> {
        if self.maximum_blocking_components == 0 {
            return Err("maximum_blocking_components must be positive".into());
        }
        if self.samples_per_axis < 2 {
            return Err("samples_per_axis must be at least two".into());
        }
        if self.maximum_attempts == 0 {
            return Err("maximum_attempts must be positive".into());
        }
        Ok(())
    }
}

impl Default for BlockerRepairConfig {
    fn default() -> Self {
        Self {
            maximum_blocking_components: 4,
            samples_per_axis: 9,
            maximum_attempts: 32,
            stop_on_complete: true,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct BlockerRepairAttempt {
    pub id: String,
    pub trigger_branches: Vec<String>,
    pub blocker: Option<String>,
    pub displacement: Vec2,
    pub routing: Option<DutGridRoutingResult>,
    pub rejected_before_routing: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct BlockerRepairEvidence {
    pub strategy: String,
    pub config: BlockerRepairConfig,
    pub selected_attempt: usize,
    pub attempted_repairs: usize,
    pub rejected_placements: usize,
    pub complete: bool,
    pub routing: DutGridRoutingEvidence,
}

/// Result envelope intentionally follows the normal candidate/validation/
/// evidence shape, while retaining every transactional attempt for inspection.
#[derive(Clone, Debug, Serialize)]
pub struct BlockerRepairResult {
    pub candidate: CandidateArtifact,
    pub validation: ExactValidationAssessment,
    pub evidence: BlockerRepairEvidence,
    pub attempts: Vec<BlockerRepairAttempt>,
}

/// Try one feedback epoch: identify movable components on failed search
/// frontiers, move one blocker at a time along its declared axes, reroute, and
/// commit only the best exact-gated result. The proposal policy is deliberately
/// small and replaceable; it is not a general placement solver.
pub fn repair_routing_blockers(
    problem: &Problem,
    initial_components: &[SolvedComponent],
    routing_config: &DutGridRoutingConfig,
    repair_config: &BlockerRepairConfig,
) -> Result<BlockerRepairResult, String> {
    problem.check_schema()?;
    routing_config.check()?;
    repair_config.check()?;
    let initial =
        route_problem_with_dut_grid_at_poses(problem, initial_components, routing_config)?;
    let mut attempts = vec![BlockerRepairAttempt {
        id: "initial".into(),
        trigger_branches: failed_branches(&initial),
        blocker: None,
        displacement: Vec2::new(0.0, 0.0),
        routing: Some(initial.clone()),
        rejected_before_routing: None,
    }];
    let mut selected = 0;
    if !initial.complete() {
        let blockers = ranked_movable_blockers(problem, &initial);
        'blockers: for blocker in blockers
            .into_iter()
            .take(repair_config.maximum_blocking_components)
        {
            let trigger_branches = triggering_branches(&initial, &blocker);
            for proposed in component_axis_proposals(
                problem,
                initial_components,
                &blocker,
                repair_config.samples_per_axis,
            )? {
                if attempts.len() > repair_config.maximum_attempts {
                    break 'blockers;
                }
                let mut components = initial_components.to_vec();
                let component = components
                    .iter_mut()
                    .find(|component| component.id == blocker)
                    .ok_or_else(|| format!("missing solved blocker {blocker}"))?;
                let previous = component.position;
                component.position = proposed;
                let displacement = Vec2::new(proposed.x - previous.x, proposed.y - previous.y);
                let id = format!("repair-{:04}", attempts.len());
                match route_problem_with_dut_grid_at_poses(problem, &components, routing_config) {
                    Ok(routing) => {
                        let index = attempts.len();
                        let is_better = routing_rank(&routing, displacement)
                            < routing_rank(
                                attempts[selected]
                                    .routing
                                    .as_ref()
                                    .expect("selected routing"),
                                attempts[selected].displacement,
                            );
                        let complete = routing.complete();
                        attempts.push(BlockerRepairAttempt {
                            id,
                            trigger_branches: trigger_branches.clone(),
                            blocker: Some(blocker.clone()),
                            displacement,
                            routing: Some(routing),
                            rejected_before_routing: None,
                        });
                        if is_better {
                            selected = index;
                        }
                        if complete && repair_config.stop_on_complete {
                            break 'blockers;
                        }
                    }
                    Err(error) => attempts.push(BlockerRepairAttempt {
                        id,
                        trigger_branches: trigger_branches.clone(),
                        blocker: Some(blocker.clone()),
                        displacement,
                        routing: None,
                        rejected_before_routing: Some(error),
                    }),
                }
            }
        }
    }
    let selected_routing = attempts[selected]
        .routing
        .as_ref()
        .expect("selected attempt always routed")
        .clone();
    let rejected_placements = attempts
        .iter()
        .filter(|attempt| attempt.rejected_before_routing.is_some())
        .count();
    let complete = selected_routing.complete();
    Ok(BlockerRepairResult {
        candidate: selected_routing.candidate,
        validation: selected_routing.validation,
        evidence: BlockerRepairEvidence {
            strategy: "blocked-frontier-axis-sampling-v1".into(),
            config: repair_config.clone(),
            selected_attempt: selected,
            attempted_repairs: attempts.len() - 1,
            rejected_placements,
            complete,
            routing: selected_routing.evidence,
        },
        attempts,
    })
}

fn routing_rank(result: &DutGridRoutingResult, displacement: Vec2) -> (usize, usize, u64, u64) {
    (
        result.evidence.failed_branches,
        result.validation.violations.len(),
        (displacement.x * displacement.x + displacement.y * displacement.y)
            .sqrt()
            .mul_add(1_000_000.0, 0.0)
            .round() as u64,
        result.evidence.expansions,
    )
}

fn failed_branches(result: &DutGridRoutingResult) -> Vec<String> {
    result
        .evidence
        .branches
        .iter()
        .filter(|branch| branch.status != pcb_routing::BranchRoutingStatus::Found)
        .map(|branch| branch.branch.clone())
        .collect()
}

fn ranked_movable_blockers(problem: &Problem, result: &DutGridRoutingResult) -> Vec<String> {
    let movable = problem
        .components
        .iter()
        .filter(|component| component.constraints.movement != Movement::Fixed)
        .map(|component| component.id.as_str())
        .collect::<BTreeSet<_>>();
    let mut weights = BTreeMap::<String, usize>::new();
    for branch in &result.evidence.branches {
        for blocker in &branch.blockers {
            if matches!(
                blocker.kind,
                RoutingBlockerKind::ComponentBody
                    | RoutingBlockerKind::Keepout
                    | RoutingBlockerKind::Pad
            ) && movable.contains(blocker.object.as_str())
            {
                *weights.entry(blocker.object.clone()).or_default() += blocker.frontier_hits;
            }
        }
    }
    let mut ranked = weights.into_iter().collect::<Vec<_>>();
    ranked.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    ranked.into_iter().map(|(component, _)| component).collect()
}

fn triggering_branches(result: &DutGridRoutingResult, blocker: &str) -> Vec<String> {
    result
        .evidence
        .branches
        .iter()
        .filter(|branch| branch.blockers.iter().any(|item| item.object == blocker))
        .map(|branch| branch.branch.clone())
        .collect()
}

fn component_axis_proposals(
    problem: &Problem,
    components: &[SolvedComponent],
    blocker: &str,
    samples: usize,
) -> Result<Vec<Vec2>, String> {
    let declared = problem
        .components
        .iter()
        .find(|component| component.id == blocker)
        .ok_or_else(|| format!("unknown blocker component {blocker}"))?;
    let solved = components
        .iter()
        .find(|component| component.id == blocker)
        .ok_or_else(|| format!("missing solved blocker component {blocker}"))?;
    let bounds = declared
        .constraints
        .region
        .map_or(problem.board.bounds, |region| {
            region.intersection(problem.board.bounds)
        });
    let extent = rotated_extent(solved.size, solved.rotation_degrees);
    let minimum = Vec2::new(bounds.min.x + extent.x, bounds.min.y + extent.y);
    let maximum = Vec2::new(bounds.max.x - extent.x, bounds.max.y - extent.y);
    if minimum.x > maximum.x || minimum.y > maximum.y {
        return Err(format!("blocker {blocker} has an empty movement region"));
    }
    let mut result = Vec::new();
    if matches!(
        declared.constraints.movement,
        Movement::Horizontal | Movement::Free
    ) {
        for x in samples_between(minimum.x, maximum.x, samples) {
            result.push(Vec2::new(x, solved.position.y));
        }
    }
    if matches!(
        declared.constraints.movement,
        Movement::Vertical | Movement::Free
    ) {
        for y in samples_between(minimum.y, maximum.y, samples) {
            result.push(Vec2::new(solved.position.x, y));
        }
    }
    result.retain(|point| {
        (point.x - solved.position.x).abs() > 1.0e-9 || (point.y - solved.position.y).abs() > 1.0e-9
    });
    result.sort_by(|left, right| {
        let left_distance = squared_distance(*left, solved.position);
        let right_distance = squared_distance(*right, solved.position);
        right_distance
            .total_cmp(&left_distance)
            .then_with(|| left.x.total_cmp(&right.x))
            .then_with(|| left.y.total_cmp(&right.y))
    });
    result.dedup_by(|left, right| left.x == right.x && left.y == right.y);
    Ok(result)
}

fn samples_between(minimum: f64, maximum: f64, samples: usize) -> Vec<f64> {
    (0..samples)
        .map(|index| minimum + (maximum - minimum) * index as f64 / (samples - 1) as f64)
        .collect()
}

fn squared_distance(first: Vec2, second: Vec2) -> f64 {
    let dx = first.x - second.x;
    let dy = first.y - second.y;
    dx * dx + dy * dy
}

fn rotated_extent(size: Vec2, rotation_degrees: f64) -> Vec2 {
    let radians = rotation_degrees.to_radians();
    let cosine = radians.cos().abs();
    let sine = radians.sin().abs();
    Vec2::new(
        size.x * 0.5 * cosine + size.y * 0.5 * sine,
        size.x * 0.5 * sine + size.y * 0.5 * cosine,
    )
}

trait RectIntersection {
    fn intersection(self, other: Self) -> Self;
}

impl RectIntersection for layout_trace_model::model::Rect {
    fn intersection(self, other: Self) -> Self {
        Self {
            min: Vec2::new(self.min.x.max(other.min.x), self.min.y.max(other.min.y)),
            max: Vec2::new(self.max.x.min(other.max.x), self.max.y.min(other.max.y)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocker_feedback_moves_the_wall_and_keeps_failed_attempts() {
        let problem: Problem = serde_json::from_str(include_str!(
            "../../../benchmarks/imported/layout-trace/placement-perturbation-unblocks-routing.json"
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
        let result = repair_routing_blockers(
            &problem,
            &components,
            &DutGridRoutingConfig {
                retry_grid_mm: vec![0.25, 0.1],
                max_expansions_per_search: 5_000_000,
                ..DutGridRoutingConfig::default()
            },
            &BlockerRepairConfig::default(),
        )
        .unwrap();
        assert!(result.evidence.complete);
        assert!(result.evidence.selected_attempt > 0);
        assert!(
            result.attempts[0]
                .routing
                .as_ref()
                .is_some_and(|routing| !routing.complete())
        );
        assert_eq!(
            result.attempts[result.evidence.selected_attempt]
                .blocker
                .as_deref(),
            Some("WALL")
        );
        let wall = result
            .candidate
            .components
            .iter()
            .find(|component| component.id == "WALL")
            .unwrap();
        assert!((wall.position.y - 15.0).abs() > 0.1);
    }
}
