// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

use std::collections::BTreeSet;

use layout_trace_model::Problem;
use pcb_routing::{
    BranchRoutingStatus, DutGridRoutingConfig, DutGridRoutingEvidence, DutGridRoutingResult,
    RoutingBlockerKind, route_problem_with_dut_grid_at_poses,
};
use pcb_validate::{CandidateArtifact, ExactValidationAssessment, SolvedComponent};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FailureDirectedOrderConfig {
    pub maximum_attempts: usize,
    pub maximum_routed_blockers_per_failure: usize,
    pub stop_on_complete: bool,
}

impl FailureDirectedOrderConfig {
    pub fn check(&self) -> Result<(), String> {
        if self.maximum_attempts == 0 {
            return Err("maximum_attempts must be positive".into());
        }
        Ok(())
    }
}

impl Default for FailureDirectedOrderConfig {
    fn default() -> Self {
        Self {
            maximum_attempts: 8,
            maximum_routed_blockers_per_failure: 4,
            stop_on_complete: true,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct FailureDirectedOrderAttempt {
    pub id: String,
    pub trigger_branch: Option<String>,
    pub priority_branches: Vec<String>,
    pub routing: DutGridRoutingResult,
}

#[derive(Clone, Debug, Serialize)]
pub struct FailureDirectedOrderEvidence {
    pub strategy: String,
    pub config: FailureDirectedOrderConfig,
    pub selected_attempt: usize,
    pub attempted_reorders: usize,
    pub complete: bool,
    pub routing: DutGridRoutingEvidence,
}

#[derive(Clone, Debug, Serialize)]
pub struct FailureDirectedOrderResult {
    pub candidate: CandidateArtifact,
    pub validation: ExactValidationAssessment,
    pub evidence: FailureDirectedOrderEvidence,
    pub attempts: Vec<FailureDirectedOrderAttempt>,
}

/// Bounded whole-candidate analogue of failure-directed rip-up. A failed
/// branch is promoted ahead of the routed traces found on its blocked frontier;
/// the board is then rerouted transactionally. Selective in-place rip-up can
/// later replace this policy without changing the evidence boundary.
pub fn repair_route_order(
    problem: &Problem,
    components: &[SolvedComponent],
    routing_config: &DutGridRoutingConfig,
    repair_config: &FailureDirectedOrderConfig,
) -> Result<FailureDirectedOrderResult, String> {
    problem.check_schema()?;
    routing_config.check()?;
    repair_config.check()?;
    let initial = route_problem_with_dut_grid_at_poses(problem, components, routing_config)?;
    let mut attempts = vec![FailureDirectedOrderAttempt {
        id: "initial".into(),
        trigger_branch: None,
        priority_branches: routing_config.priority_branches.clone(),
        routing: initial.clone(),
    }];
    let mut selected = 0;
    let proposals = reorder_proposals(&initial, repair_config);
    for (trigger, priorities) in proposals {
        if attempts.len() > repair_config.maximum_attempts {
            break;
        }
        let mut config = routing_config.clone();
        config.priority_branches = priorities.clone();
        let routing = route_problem_with_dut_grid_at_poses(problem, components, &config)?;
        let index = attempts.len();
        let better = routing_rank(&routing) < routing_rank(&attempts[selected].routing);
        let complete = routing.complete();
        attempts.push(FailureDirectedOrderAttempt {
            id: format!("reorder-{index:04}"),
            trigger_branch: Some(trigger),
            priority_branches: priorities,
            routing,
        });
        if better {
            selected = index;
        }
        if complete && repair_config.stop_on_complete {
            break;
        }
    }
    let selected_routing = attempts[selected].routing.clone();
    let complete = selected_routing.complete();
    Ok(FailureDirectedOrderResult {
        candidate: selected_routing.candidate,
        validation: selected_routing.validation,
        evidence: FailureDirectedOrderEvidence {
            strategy: "failure-directed-whole-candidate-reorder-v1".into(),
            config: repair_config.clone(),
            selected_attempt: selected,
            attempted_reorders: attempts.len() - 1,
            complete,
            routing: selected_routing.evidence,
        },
        attempts,
    })
}

fn routing_rank(result: &DutGridRoutingResult) -> (usize, usize, usize, u64) {
    let exact_rejected = result
        .evidence
        .branches
        .iter()
        .filter(|branch| branch.status == BranchRoutingStatus::ExactGeometryRejected)
        .count();
    (
        result.evidence.failed_branches,
        result.validation.violations.len(),
        exact_rejected,
        result.evidence.expansions,
    )
}

fn reorder_proposals(
    result: &DutGridRoutingResult,
    config: &FailureDirectedOrderConfig,
) -> Vec<(String, Vec<String>)> {
    let mut proposals = Vec::new();
    let mut seen = BTreeSet::new();
    for branch in result
        .evidence
        .branches
        .iter()
        .filter(|branch| branch.status != BranchRoutingStatus::Found)
    {
        let mut blockers = branch
            .blockers
            .iter()
            .filter(|blocker| blocker.kind == RoutingBlockerKind::RoutedTrace)
            .collect::<Vec<_>>();
        blockers.sort_by(|left, right| {
            right
                .frontier_hits
                .cmp(&left.frontier_hits)
                .then_with(|| left.object.cmp(&right.object))
        });
        let mut priorities = vec![branch.branch.clone()];
        priorities.extend(
            blockers
                .into_iter()
                .take(config.maximum_routed_blockers_per_failure)
                .map(|blocker| blocker.object.clone()),
        );
        priorities.dedup();
        if seen.insert(priorities.clone()) {
            proposals.push((branch.branch.clone(), priorities));
        }
    }
    proposals
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_candidate_needs_no_reorder() {
        let problem: Problem = serde_json::from_str(include_str!(
            "../../../benchmarks/imported/layout-trace/forced-crossing-two-layer.json"
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
        let result = repair_route_order(
            &problem,
            &components,
            &DutGridRoutingConfig::default(),
            &FailureDirectedOrderConfig::default(),
        )
        .unwrap();
        assert!(result.evidence.complete);
        assert_eq!(result.evidence.attempted_reorders, 0);
        assert_eq!(result.attempts.len(), 1);
    }
}
