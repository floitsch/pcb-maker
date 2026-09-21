// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

use std::collections::BTreeSet;

use layout_trace_model::Problem;
use pcb_routing::{
    BranchRoutingStatus, DutGridRoutingConfig, DutGridRoutingEvidence, DutGridRoutingResult,
    RoutingBlockerKind, reroute_problem_branches_with_dut_grid_at_poses,
    route_problem_with_dut_grid_at_poses,
};
use pcb_validate::{CandidateArtifact, ExactValidationAssessment, SolvedComponent};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SelectiveRipupConfig {
    pub maximum_attempts: usize,
    pub maximum_reroute_branches: usize,
    pub stop_on_complete: bool,
}

impl SelectiveRipupConfig {
    pub fn check(&self) -> Result<(), String> {
        if self.maximum_attempts == 0 {
            return Err("selective rip-up maximum_attempts must be positive".into());
        }
        if self.maximum_reroute_branches < 2 {
            return Err("selective rip-up requires room for a failure and one blocker".into());
        }
        Ok(())
    }
}

impl Default for SelectiveRipupConfig {
    fn default() -> Self {
        Self {
            maximum_attempts: 8,
            maximum_reroute_branches: 4,
            stop_on_complete: true,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct SelectiveRipupAttempt {
    pub id: String,
    pub baseline_attempt: Option<usize>,
    pub trigger_branch: Option<String>,
    pub reroute_branches: Vec<String>,
    pub routing: DutGridRoutingResult,
}

#[derive(Clone, Debug, Serialize)]
pub struct SelectiveRipupEvidence {
    pub strategy: String,
    pub config: SelectiveRipupConfig,
    pub selected_attempt: usize,
    pub attempted_repairs: usize,
    pub total_expansions: u64,
    pub complete: bool,
    pub routing: DutGridRoutingEvidence,
}

#[derive(Clone, Debug, Serialize)]
pub struct SelectiveRipupResult {
    pub candidate: CandidateArtifact,
    pub validation: ExactValidationAssessment,
    pub evidence: SelectiveRipupEvidence,
    pub attempts: Vec<SelectiveRipupAttempt>,
}

/// Reroute one failed branch and only the routed branches found on its blocked
/// frontier. All other baseline copper remains fixed and every proposal is
/// independently exact-gated before selection.
pub fn repair_with_selective_ripup(
    problem: &Problem,
    components: &[SolvedComponent],
    routing_config: &DutGridRoutingConfig,
    repair_config: &SelectiveRipupConfig,
) -> Result<SelectiveRipupResult, String> {
    problem.check_schema()?;
    routing_config.check()?;
    repair_config.check()?;
    let initial = route_problem_with_dut_grid_at_poses(problem, components, routing_config)?;
    repair_routing_result_with_selective_ripup(
        problem,
        components,
        initial,
        routing_config,
        repair_config,
    )
}

/// Apply selective rip-up to an already evaluated placement without rerunning
/// its baseline. This is the composition seam used by placement-search
/// coordinators; attempt zero always preserves the supplied routed state.
pub fn repair_routing_result_with_selective_ripup(
    problem: &Problem,
    components: &[SolvedComponent],
    initial: DutGridRoutingResult,
    routing_config: &DutGridRoutingConfig,
    repair_config: &SelectiveRipupConfig,
) -> Result<SelectiveRipupResult, String> {
    problem.check_schema()?;
    routing_config.check()?;
    repair_config.check()?;
    let mut total_expansions = initial.evidence.expansions;
    let mut attempts = vec![SelectiveRipupAttempt {
        id: "initial".into(),
        baseline_attempt: None,
        trigger_branch: None,
        reroute_branches: Vec::new(),
        routing: initial.clone(),
    }];
    let mut selected = 0;
    let mut tried = BTreeSet::new();

    while attempts.len() - 1 < repair_config.maximum_attempts {
        let baseline_attempt = selected;
        let Some((trigger, priorities)) =
            ripup_proposals(&attempts[baseline_attempt].routing, repair_config)
                .into_iter()
                .find(|(_, priorities)| tried.insert((baseline_attempt, priorities.clone())))
        else {
            break;
        };
        let reroute_branches = priorities.iter().cloned().collect::<BTreeSet<_>>();
        let mut config = routing_config.clone();
        config.priority_branches = priorities.clone();
        let routing = reroute_problem_branches_with_dut_grid_at_poses(
            problem,
            components,
            &attempts[baseline_attempt].routing,
            &reroute_branches,
            &config,
        )?;
        total_expansions = total_expansions.saturating_add(routing.evidence.expansions);
        let index = attempts.len();
        let better = routing_rank(&routing) < routing_rank(&attempts[selected].routing);
        let complete = routing.complete();
        attempts.push(SelectiveRipupAttempt {
            id: format!("selective-ripup-{index:04}"),
            baseline_attempt: Some(baseline_attempt),
            trigger_branch: Some(trigger),
            reroute_branches: priorities,
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
    Ok(SelectiveRipupResult {
        candidate: selected_routing.candidate,
        validation: selected_routing.validation,
        evidence: SelectiveRipupEvidence {
            strategy: "failure-directed-selective-ripup-v1".into(),
            config: repair_config.clone(),
            selected_attempt: selected,
            attempted_repairs: attempts.len() - 1,
            total_expansions,
            complete,
            routing: selected_routing.evidence,
        },
        attempts,
    })
}

fn routing_rank(result: &DutGridRoutingResult) -> (usize, usize, usize) {
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
    )
}

fn ripup_proposals(
    result: &DutGridRoutingResult,
    config: &SelectiveRipupConfig,
) -> Vec<(String, Vec<String>)> {
    let mut proposals = Vec::new();
    let mut seen = BTreeSet::new();
    for failure in result
        .evidence
        .branches
        .iter()
        .filter(|branch| branch.status != BranchRoutingStatus::Found)
    {
        let mut blockers = failure
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
        let mut priorities = vec![failure.branch.clone()];
        priorities.extend(
            blockers
                .into_iter()
                .take(config.maximum_reroute_branches - 1)
                .map(|blocker| blocker.object.clone()),
        );
        priorities.dedup();
        if priorities.len() >= 2 && seen.insert(priorities.clone()) {
            proposals.push((failure.branch.clone(), priorities));
        }
    }
    proposals
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_candidate_needs_no_selective_ripup() {
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
        let result = repair_with_selective_ripup(
            &problem,
            &components,
            &DutGridRoutingConfig::default(),
            &SelectiveRipupConfig::default(),
        )
        .unwrap();
        assert!(result.evidence.complete);
        assert_eq!(result.evidence.attempted_repairs, 0);
        assert_eq!(result.attempts.len(), 1);
    }

    #[test]
    fn routed_baseline_composition_does_not_repeat_initial_search() {
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
        let routing_config = DutGridRoutingConfig::default();
        let initial =
            route_problem_with_dut_grid_at_poses(&problem, &components, &routing_config).unwrap();
        let initial_expansions = initial.evidence.expansions;

        let result = repair_routing_result_with_selective_ripup(
            &problem,
            &components,
            initial,
            &routing_config,
            &SelectiveRipupConfig::default(),
        )
        .unwrap();

        assert!(result.evidence.complete);
        assert_eq!(result.evidence.attempted_repairs, 0);
        assert_eq!(result.evidence.total_expansions, initial_expansions);
        assert_eq!(result.attempts.len(), 1);
    }
}
