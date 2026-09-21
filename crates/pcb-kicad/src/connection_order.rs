// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::KiCadConnectionProgressionResult;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadConnectionOrderProposalMode {
    /// Exchange the failed target with one routed connection whose removal
    /// made the target reachable. Every intervening ordinal is preserved.
    SwapPositions,
    /// Move the failed target immediately before the diagnosed blocker while
    /// preserving the relative order of all other connections.
    MoveTargetBeforeYielding,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadConnectionOrderFrontierPolicy {
    /// Evaluate every sibling before descendants at the next diagnosis depth.
    BreadthFirst,
    /// Follow the first stable causal lineage before returning to siblings.
    DepthFirst,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadConnectionOrderSearchConfig {
    /// Exact bound including the unmodified control trial.
    pub maximum_trials: usize,
    /// Bound on unique children retained from one failed trial.
    pub maximum_children_per_failure: usize,
    pub proposal_modes: Vec<KiCadConnectionOrderProposalMode>,
    pub frontier_policy: KiCadConnectionOrderFrontierPolicy,
    /// Stop once a native-complete order has been evaluated. All earlier
    /// controls and failures remain serialized.
    pub stop_after_first_complete: bool,
    pub via_penalty_mm: f64,
}

impl KiCadConnectionOrderSearchConfig {
    pub fn check(&self) -> Result<(), String> {
        if self.maximum_trials == 0 {
            return Err("connection-order search maximum_trials must be positive".into());
        }
        if self.maximum_children_per_failure == 0 {
            return Err(
                "connection-order search maximum_children_per_failure must be positive".into(),
            );
        }
        if self.proposal_modes.is_empty() {
            return Err("connection-order search requires at least one proposal mode".into());
        }
        if !self.via_penalty_mm.is_finite() || self.via_penalty_mm < 0.0 {
            return Err(
                "connection-order search via_penalty_mm must be finite and non-negative".into(),
            );
        }
        let mut modes = BTreeSet::new();
        for mode in &self.proposal_modes {
            if !modes.insert(format!("{mode:?}")) {
                return Err("connection-order search proposal modes must be unique".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadConnectionOrderProposalEvidence {
    pub mode: KiCadConnectionOrderProposalMode,
    pub failed_target: String,
    pub yielding_connections: Vec<String>,
    pub diagnosis_trial_ordinal: usize,
    pub interpretation: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadConnectionOrderProposal {
    pub connection_order: Vec<String>,
    pub evidence: KiCadConnectionOrderProposalEvidence,
}

pub fn connection_order_fingerprint(order: &[String]) -> String {
    order.join("\u{1f}")
}

/// Converts one successful yielding counterfactual into cold order children.
/// The proposal is causal guidance only; callers must reroute from zero copper.
pub fn connection_order_proposals_for_yielding(
    order: &[String],
    failed_target: &str,
    yielding_connections: &[String],
    diagnosis_trial_ordinal: usize,
    mode: KiCadConnectionOrderProposalMode,
) -> Vec<KiCadConnectionOrderProposal> {
    let Some(target_index) = order
        .iter()
        .position(|connection| connection == failed_target)
    else {
        return Vec::new();
    };
    let yielding = yielding_connections
        .iter()
        .filter_map(|connection| {
            order
                .iter()
                .position(|candidate| candidate == connection)
                .map(|index| (connection, index))
        })
        .filter(|(_, index)| *index < target_index)
        .collect::<Vec<_>>();
    if yielding.is_empty() {
        return Vec::new();
    }

    match mode {
        KiCadConnectionOrderProposalMode::SwapPositions => yielding
            .into_iter()
            .map(|(yielding_connection, yielding_index)| {
                let mut child = order.to_vec();
                child.swap(target_index, yielding_index);
                KiCadConnectionOrderProposal {
                    connection_order: child,
                    evidence: KiCadConnectionOrderProposalEvidence {
                        mode,
                        failed_target: failed_target.into(),
                        yielding_connections: vec![yielding_connection.clone()],
                        diagnosis_trial_ordinal,
                        interpretation: "swap the failed target with one earlier connection whose removal made the target reachable; reroute the complete prefix from zero copper".into(),
                    },
                }
            })
            .collect(),
        KiCadConnectionOrderProposalMode::MoveTargetBeforeYielding => {
            let earliest = yielding
                .iter()
                .min_by_key(|(_, index)| *index)
                .expect("nonempty yielding set");
            let mut child = order.to_vec();
            let target = child.remove(target_index);
            child.insert(earliest.1, target);
            vec![KiCadConnectionOrderProposal {
                connection_order: child,
                evidence: KiCadConnectionOrderProposalEvidence {
                    mode,
                    failed_target: failed_target.into(),
                    yielding_connections: yielding
                        .iter()
                        .map(|(connection, _)| (*connection).clone())
                        .collect(),
                    diagnosis_trial_ordinal,
                    interpretation: "move the failed target immediately before the earliest diagnosed yielding connection while preserving every other relative order; reroute the complete prefix from zero copper".into(),
                },
            }]
        }
    }
}

/// Converts the final failed insertion's successful yielding
/// counterfactuals into bounded order alternatives. The diagnosis is causal
/// guidance only: callers must regenerate and native-gate every returned
/// order independently.
pub fn diagnosed_connection_order_proposals(
    order: &[String],
    progression: &KiCadConnectionProgressionResult,
    config: &KiCadConnectionOrderSearchConfig,
) -> Vec<KiCadConnectionOrderProposal> {
    let Some(result) = progression
        .steps
        .last()
        .and_then(|step| step.result.as_ref())
    else {
        return Vec::new();
    };
    let diagnoses = result
        .evidence
        .attempts
        .iter()
        .filter_map(|attempt| attempt.single_connection_ripup.as_deref())
        .flat_map(|ripup| ripup.diagnosis.trials.iter())
        .filter(|trial| trial.route_found)
        .collect::<Vec<_>>();
    let mut proposals = Vec::new();
    let mut fingerprints = BTreeSet::new();
    // Cover distinct causal blockers with the preferred mode before spending
    // child slots on alternate encodings of the same precedence.
    for mode in &config.proposal_modes {
        for diagnosis in &diagnoses {
            for proposal in connection_order_proposals_for_yielding(
                order,
                &result.evidence.inserted_connection,
                &diagnosis.yielding_connections,
                diagnosis.ordinal,
                *mode,
            ) {
                let fingerprint = connection_order_fingerprint(&proposal.connection_order);
                if fingerprints.insert(fingerprint) {
                    proposals.push(proposal);
                }
                if proposals.len() == config.maximum_children_per_failure {
                    return proposals;
                }
            }
        }
    }
    proposals
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnosis_proposals_are_distinct_bounded_and_preserve_membership() {
        let order = ["A", "BLOCKER", "C", "TARGET"].map(str::to_string).to_vec();
        let mut proposals = Vec::new();
        let mut seen = BTreeSet::new();
        for mode in [
            KiCadConnectionOrderProposalMode::SwapPositions,
            KiCadConnectionOrderProposalMode::MoveTargetBeforeYielding,
        ] {
            for proposal in connection_order_proposals_for_yielding(
                &order,
                "TARGET",
                &["BLOCKER".into()],
                7,
                mode,
            ) {
                if seen.insert(connection_order_fingerprint(&proposal.connection_order)) {
                    proposals.push(proposal);
                }
            }
        }
        assert_eq!(proposals.len(), 2);
        assert_eq!(
            proposals[0].connection_order,
            ["A", "TARGET", "C", "BLOCKER"]
        );
        assert_eq!(
            proposals[1].connection_order,
            ["A", "TARGET", "BLOCKER", "C"]
        );
        for proposal in proposals {
            let mut members = proposal.connection_order;
            members.sort();
            assert_eq!(members, ["A", "BLOCKER", "C", "TARGET"]);
            assert_eq!(proposal.evidence.diagnosis_trial_ordinal, 7);
        }

        let adjacent = ["BLOCKER", "TARGET"].map(str::to_string).to_vec();
        let swap = connection_order_proposals_for_yielding(
            &adjacent,
            "TARGET",
            &["BLOCKER".into()],
            0,
            KiCadConnectionOrderProposalMode::SwapPositions,
        );
        let moved = connection_order_proposals_for_yielding(
            &adjacent,
            "TARGET",
            &["BLOCKER".into()],
            0,
            KiCadConnectionOrderProposalMode::MoveTargetBeforeYielding,
        );
        assert_eq!(swap[0].connection_order, moved[0].connection_order);
    }

    #[test]
    fn search_config_requires_explicit_finite_bounds() {
        let valid = KiCadConnectionOrderSearchConfig {
            maximum_trials: 3,
            maximum_children_per_failure: 2,
            proposal_modes: vec![
                KiCadConnectionOrderProposalMode::SwapPositions,
                KiCadConnectionOrderProposalMode::MoveTargetBeforeYielding,
            ],
            frontier_policy: KiCadConnectionOrderFrontierPolicy::DepthFirst,
            stop_after_first_complete: true,
            via_penalty_mm: 2.0,
        };
        valid.check().unwrap();
        let mut zero = valid.clone();
        zero.maximum_trials = 0;
        assert!(zero.check().is_err());
        let mut duplicate = valid;
        duplicate.proposal_modes = vec![
            KiCadConnectionOrderProposalMode::SwapPositions,
            KiCadConnectionOrderProposalMode::SwapPositions,
        ];
        assert!(duplicate.check().is_err());
    }
}
