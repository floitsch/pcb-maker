// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Deterministic exact selection over an admitted route-family portfolio.
//!
//! This is the production port of the V3 branch-and-bound experiment. It is a
//! pure coordinator pass: it selects family IDs and never mutates candidates
//! or invokes the continuous engine.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::family_ir::{
    ConflictEvidence, FIXED_WITNESS_ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION, FamilyConflictKind,
    MULTI_RUN_ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION, ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION,
    RouteFamilyFeasibilityMode, RouteFamilyPortfolio, ValidationIssue,
};

const CAPACITY_EPSILON: f64 = 1.0e-12;
const MAX_BOUNDED_INFEASIBILITY_WITNESSES: usize = 64;
pub const BOUNDED_INFEASIBILITY_EVIDENCE_CONTRACT: &str =
    "layout-trace.family-assignment-bounded-infeasibility/v1";
const PORTFOLIO_CONTEXT_FINGERPRINT_CONTRACT: &str =
    "layout-trace.family-assignment-portfolio-context/v1";
const FIXED_WITNESS_PORTFOLIO_CONTEXT_FINGERPRINT_CONTRACT: &str =
    "layout-trace.family-assignment-portfolio-context/v2";
const FIXED_CONTEXT_PORTFOLIO_CONTEXT_FINGERPRINT_CONTRACT: &str =
    "layout-trace.family-assignment-portfolio-context/v3";
const FIXED_CONTEXT_DEPENDENCY_FINGERPRINT_CONTRACT: &str =
    "layout-trace.family-assignment-fixed-context-dependency/v1";

type PassageId = (String, String);

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FamilyAssignmentLimits {
    pub max_nodes: u64,
}

impl Default for FamilyAssignmentLimits {
    fn default() -> Self {
        Self { max_nodes: 100_000 }
    }
}

/// How the discrete selector treats a certified copper-clearance shortfall.
///
/// The predecessor contract hands topology-compatible shortfalls to a
/// continuous legalizer. Producers whose selected representation cannot yet
/// be legalized can instead require a conflict-free discrete combination.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FamilyClearancePolicy {
    #[default]
    RepairObligation,
    DiscreteSeparation,
}

const fn conflict_requires_discrete_separation(
    kind: FamilyConflictKind,
    clearance_policy: FamilyClearancePolicy,
) -> bool {
    kind.requires_discrete_separation()
        || matches!(clearance_policy, FamilyClearancePolicy::DiscreteSeparation)
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct FamilyAssignmentWork {
    pub capacity_enforcement: FamilyAssignmentCapacityEnforcement,
    pub nodes: u64,
    pub branches: u64,
    pub pruned_bound: u64,
    pub pruned_capacity: u64,
    pub pruned_conflict: u64,
    pub pruned_fixed_context: u64,
    pub pruned_empty_domain: u64,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FamilyAssignmentCapacityEnforcement {
    #[default]
    ScalarPassageWidth,
    FixedWitnessPairwiseNoScalar,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FamilyAssignmentChoiceEvidence {
    pub branch: String,
    pub family_id: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FamilyAssignmentOptionBlocker {
    FixedContext {
        option_family_id: String,
        option_branch: String,
        option_run_id: String,
        context_id: String,
        context_branch: String,
        context_layer: String,
        fixed_context_dependency_sha256: String,
        conflict_kind: FamilyConflictKind,
    },
    Conflict {
        option_family_id: String,
        chosen: FamilyAssignmentChoiceEvidence,
    },
    Capacity {
        option_family_id: String,
        layer: String,
        passage_key: String,
        used_width: f64,
        demand: f64,
        available_width: f64,
        contributors: Vec<FamilyAssignmentChoiceEvidence>,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FamilyAssignmentFixedContextRunEvidence {
    pub context_id: String,
    pub context_branch: String,
    pub context_layer: String,
    /// Exact current-epoch commitment to this context run's identity,
    /// polyline, layer, width, clearance, and placement/geometry revisions.
    pub fixed_context_dependency_sha256: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FamilyAssignmentDeadEndEvidence {
    pub blocked_branch: String,
    pub chosen: Vec<FamilyAssignmentChoiceEvidence>,
    /// Exactly one deterministic first blocker for every option in the empty
    /// domain. The full assignment search remains authoritative; this is a
    /// compact explanatory cover rather than a minimal unsatisfiable subset.
    pub option_blockers: Vec<FamilyAssignmentOptionBlocker>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FamilyAssignmentAdmittedDomainEvidence {
    pub branch: String,
    pub admitted_family_count: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FamilyAssignmentBoundedInfeasibilityEvidence {
    pub contract: String,
    pub max_families_per_variable: usize,
    pub admitted_domains: Vec<FamilyAssignmentAdmittedDomainEvidence>,
    /// The exact search exhausted every combination of the admitted domains,
    /// not every route family that a deeper generator might discover.
    pub exhaustive_within_admitted_portfolio: bool,
    pub candidate_set_fingerprint: String,
    /// Commits to variables, passage capacities, and pair analysis. The
    /// narrower candidate-set fingerprint intentionally does not.
    pub portfolio_context_sha256: String,
    /// The authoritative V4 commitment to every serialized fixed-context run,
    /// including runs unrelated to the observed dead ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub all_fixed_context_sha256: Option<String>,
    /// Commitment to the complete union of exact context runs observed in
    /// family-context blockers, including blockers in truncated witnesses.
    /// This is observational and is not a minimal or necessary dependency set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixed_context_dependency_sha256: Option<String>,
    /// Stable, complete union corresponding to `fixed_context_dependency_sha256`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conflict_context_runs: Vec<FamilyAssignmentFixedContextRunEvidence>,
    /// Complete union over every exhausted dead end, even when detailed
    /// witnesses below are truncated.
    pub implicated_variables: Vec<String>,
    pub empty_domain_events: u64,
    pub witnesses: Vec<FamilyAssignmentDeadEndEvidence>,
    pub witnesses_truncated: u64,
    /// SHA-256 over every canonical dead-end record plus the complete union
    /// and counts. Truncated records remain committed by this digest.
    pub canonical_sha256: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PassageUsage {
    pub layer: String,
    pub passage_key: String,
    pub used_width: f64,
    pub available_width: f64,
    pub remaining_width: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FamilySelection {
    /// Stable branch-variable ID to selected family ID.
    pub choices: BTreeMap<String, String>,
    pub total_cost: f64,
    pub passage_usage: Vec<PassageUsage>,
    /// Selected full-width witness conflicts whose centerlines are
    /// topologically compatible. These are explicit work for the continuous
    /// engine and remain subject to authoritative exact final validation.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub repair_obligations: Vec<FamilySelectionRepairObligation>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "scope", rename_all = "snake_case", deny_unknown_fields)]
pub enum FamilySelectionRepairObligation {
    FamilyPair {
        left_family_id: String,
        right_family_id: String,
        kind: FamilyConflictKind,
        evidence: ConflictEvidence,
    },
    FixedContext {
        family_id: String,
        context_id: String,
        kind: FamilyConflictKind,
        evidence: ConflictEvidence,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum FamilyAssignmentOutcome {
    Selected {
        selection: FamilySelection,
        work: FamilyAssignmentWork,
    },
    /// Exact exhaustion proves only that this finite admitted portfolio has no
    /// assignment. It is never a proof that the routing problem is globally
    /// infeasible or that a deeper family frontier cannot succeed.
    BoundedInfeasible {
        work: FamilyAssignmentWork,
        evidence: FamilyAssignmentBoundedInfeasibilityEvidence,
    },
    BudgetExhausted {
        work: FamilyAssignmentWork,
        /// Diagnostic only. Exhaustion never returns candidate choices.
        best_incumbent_cost: Option<f64>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FamilyAssignmentError {
    InvalidPortfolio(Vec<ValidationIssue>),
}

impl fmt::Display for FamilyAssignmentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPortfolio(issues) => write!(
                formatter,
                "route-family assignment rejected {} validation issue{}",
                issues.len(),
                if issues.len() == 1 { "" } else { "s" }
            ),
        }
    }
}

impl Error for FamilyAssignmentError {}

#[derive(Clone)]
struct AssignmentOption {
    net: String,
    family_id: String,
    cost: f64,
    demand: f64,
    passages: Vec<PassageId>,
    fixed_context_conflicts: Vec<FixedContextConflictDependency>,
}

#[derive(Clone)]
struct FixedContextConflictDependency {
    option_run_id: String,
    context_id: String,
    context_branch: String,
    context_layer: String,
    fixed_context_dependency_sha256: String,
    conflict_kind: FamilyConflictKind,
}

impl AssignmentOption {
    fn tie_key(&self) -> (&str, &str) {
        (&self.net, &self.family_id)
    }
}

struct Search {
    by_net: BTreeMap<String, Vec<AssignmentOption>>,
    capacities: BTreeMap<PassageId, f64>,
    adjacency: BTreeMap<String, BTreeSet<String>>,
    limits: FamilyAssignmentLimits,
    work: FamilyAssignmentWork,
    exhausted: bool,
    best: Option<(f64, BTreeMap<String, String>, BTreeMap<PassageId, f64>)>,
    max_families_per_variable: usize,
    admitted_domains: Vec<FamilyAssignmentAdmittedDomainEvidence>,
    candidate_set_fingerprint: String,
    portfolio_context_sha256: String,
    all_fixed_context_sha256: Option<String>,
    conflict_context_runs: BTreeMap<String, FamilyAssignmentFixedContextRunEvidence>,
    implicated_variables: BTreeSet<String>,
    empty_domain_events: u64,
    dead_end_witnesses: Vec<FamilyAssignmentDeadEndEvidence>,
    dead_end_witnesses_truncated: u64,
    dead_end_hash: Sha256,
}

impl Search {
    fn conflict_degree(&self, net: &str, domain: &[usize]) -> usize {
        domain
            .iter()
            .map(|option_index| {
                let option = &self.by_net[net][*option_index];
                self.adjacency
                    .get(&option.family_id)
                    .map_or(0, BTreeSet::len)
            })
            .sum()
    }

    fn fits(
        &self,
        option: &AssignmentOption,
        chosen: &BTreeSet<String>,
        usage: &BTreeMap<PassageId, f64>,
    ) -> Result<(), FitFailure> {
        if let Some(conflict) = option.fixed_context_conflicts.first() {
            return Err(FitFailure::FixedContext {
                option_run_id: conflict.option_run_id.clone(),
                context_id: conflict.context_id.clone(),
                context_branch: conflict.context_branch.clone(),
                context_layer: conflict.context_layer.clone(),
                fixed_context_dependency_sha256: conflict.fixed_context_dependency_sha256.clone(),
                conflict_kind: conflict.conflict_kind,
            });
        }
        if let Some(chosen_family_id) = self
            .adjacency
            .get(&option.family_id)
            .and_then(|neighbors| neighbors.iter().find(|family| chosen.contains(*family)))
        {
            return Err(FitFailure::Conflict {
                chosen_family_id: chosen_family_id.clone(),
            });
        }
        if let Some(passage) = option.passages.iter().find(|passage| {
            usage.get(*passage).copied().unwrap_or(0.0) + option.demand
                > self.capacities[*passage] + CAPACITY_EPSILON
        }) {
            return Err(FitFailure::Capacity {
                passage: passage.clone(),
                used_width: usage.get(passage).copied().unwrap_or(0.0),
                available_width: self.capacities[passage],
            });
        }
        Ok(())
    }

    fn record_empty_domain(
        &mut self,
        blocked_branch: &str,
        failures: &[(usize, FitFailure)],
        choices: &BTreeMap<String, String>,
        passage_users: &BTreeMap<PassageId, BTreeSet<String>>,
    ) {
        self.empty_domain_events += 1;
        self.implicated_variables.insert(blocked_branch.to_owned());
        let chosen = choices
            .iter()
            .map(|(branch, family_id)| FamilyAssignmentChoiceEvidence {
                branch: branch.clone(),
                family_id: family_id.clone(),
            })
            .collect::<Vec<_>>();
        let mut option_blockers = Vec::with_capacity(failures.len());
        for (option_index, failure) in failures {
            let option = &self.by_net[blocked_branch][*option_index];
            match failure {
                FitFailure::FixedContext {
                    option_run_id,
                    context_id,
                    context_branch,
                    context_layer,
                    fixed_context_dependency_sha256,
                    conflict_kind,
                } => {
                    self.conflict_context_runs.insert(
                        context_id.clone(),
                        FamilyAssignmentFixedContextRunEvidence {
                            context_id: context_id.clone(),
                            context_branch: context_branch.clone(),
                            context_layer: context_layer.clone(),
                            fixed_context_dependency_sha256: fixed_context_dependency_sha256
                                .clone(),
                        },
                    );
                    option_blockers.push(FamilyAssignmentOptionBlocker::FixedContext {
                        option_family_id: option.family_id.clone(),
                        option_branch: option.net.clone(),
                        option_run_id: option_run_id.clone(),
                        context_id: context_id.clone(),
                        context_branch: context_branch.clone(),
                        context_layer: context_layer.clone(),
                        fixed_context_dependency_sha256: fixed_context_dependency_sha256.clone(),
                        conflict_kind: *conflict_kind,
                    });
                }
                FitFailure::Conflict { chosen_family_id } => {
                    let (chosen_branch, chosen_family_id) = choices
                        .iter()
                        .find(|(_, family_id)| *family_id == chosen_family_id)
                        .expect("a conflicting chosen family has a selected branch");
                    self.implicated_variables.insert(chosen_branch.clone());
                    option_blockers.push(FamilyAssignmentOptionBlocker::Conflict {
                        option_family_id: option.family_id.clone(),
                        chosen: FamilyAssignmentChoiceEvidence {
                            branch: chosen_branch.clone(),
                            family_id: chosen_family_id.clone(),
                        },
                    });
                }
                FitFailure::Capacity {
                    passage,
                    used_width,
                    available_width,
                } => {
                    let contributors = passage_users
                        .get(passage)
                        .into_iter()
                        .flatten()
                        .map(|branch| {
                            self.implicated_variables.insert(branch.clone());
                            FamilyAssignmentChoiceEvidence {
                                branch: branch.clone(),
                                family_id: choices[branch].clone(),
                            }
                        })
                        .collect::<Vec<_>>();
                    option_blockers.push(FamilyAssignmentOptionBlocker::Capacity {
                        option_family_id: option.family_id.clone(),
                        layer: passage.0.clone(),
                        passage_key: passage.1.clone(),
                        used_width: *used_width,
                        demand: option.demand,
                        available_width: *available_width,
                        contributors,
                    });
                }
            }
        }
        let witness = FamilyAssignmentDeadEndEvidence {
            blocked_branch: blocked_branch.to_owned(),
            chosen,
            option_blockers,
        };
        hash_dead_end(&mut self.dead_end_hash, &witness);
        if self.dead_end_witnesses.len() < MAX_BOUNDED_INFEASIBILITY_WITNESSES {
            self.dead_end_witnesses.push(witness);
        } else {
            self.dead_end_witnesses_truncated += 1;
        }
    }

    fn bounded_infeasibility_evidence(&self) -> FamilyAssignmentBoundedInfeasibilityEvidence {
        let implicated_variables = self
            .implicated_variables
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        let mut hash = self.dead_end_hash.clone();
        let conflict_context_runs = self
            .conflict_context_runs
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let fixed_context_dependency_sha256 = (!conflict_context_runs.is_empty())
            .then(|| fixed_context_dependency_sha256(&conflict_context_runs));
        hash_field(
            &mut hash,
            BOUNDED_INFEASIBILITY_EVIDENCE_CONTRACT.as_bytes(),
        );
        hash_field(&mut hash, self.candidate_set_fingerprint.as_bytes());
        hash_field(&mut hash, self.portfolio_context_sha256.as_bytes());
        // Preserve the frozen V1/V2/V3 canonical digest. The added commitment
        // is a V4-only extension because older portfolios cannot contain
        // family-context blockers.
        if self.all_fixed_context_sha256.is_some() {
            hash_serialized(&mut hash, &self.all_fixed_context_sha256);
            hash_serialized(&mut hash, &fixed_context_dependency_sha256);
            hash_serialized(&mut hash, &conflict_context_runs);
        }
        hash.update((self.max_families_per_variable as u64).to_le_bytes());
        hash.update((self.admitted_domains.len() as u64).to_le_bytes());
        for domain in &self.admitted_domains {
            hash_field(&mut hash, domain.branch.as_bytes());
            hash.update((domain.admitted_family_count as u64).to_le_bytes());
        }
        hash.update(self.empty_domain_events.to_le_bytes());
        hash.update(self.dead_end_witnesses_truncated.to_le_bytes());
        for branch in &implicated_variables {
            hash_field(&mut hash, branch.as_bytes());
        }
        FamilyAssignmentBoundedInfeasibilityEvidence {
            contract: BOUNDED_INFEASIBILITY_EVIDENCE_CONTRACT.into(),
            max_families_per_variable: self.max_families_per_variable,
            admitted_domains: self.admitted_domains.clone(),
            exhaustive_within_admitted_portfolio: true,
            candidate_set_fingerprint: self.candidate_set_fingerprint.clone(),
            portfolio_context_sha256: self.portfolio_context_sha256.clone(),
            all_fixed_context_sha256: self.all_fixed_context_sha256.clone(),
            fixed_context_dependency_sha256,
            conflict_context_runs,
            implicated_variables,
            empty_domain_events: self.empty_domain_events,
            witnesses: self.dead_end_witnesses.clone(),
            witnesses_truncated: self.dead_end_witnesses_truncated,
            canonical_sha256: encode_sha256(hash.finalize()),
        }
    }

    fn visit(
        &mut self,
        remaining: &[String],
        cost: f64,
        choices: &mut BTreeMap<String, String>,
        chosen: &mut BTreeSet<String>,
        usage: &mut BTreeMap<PassageId, f64>,
        passage_users: &mut BTreeMap<PassageId, BTreeSet<String>>,
    ) {
        if self.exhausted {
            return;
        }
        self.work.nodes += 1;
        if self.work.nodes > self.limits.max_nodes {
            self.exhausted = true;
            return;
        }
        if remaining.is_empty() {
            let replace = match &self.best {
                None => true,
                Some((best_cost, _, _)) => cost.total_cmp(best_cost).is_lt(),
            };
            if replace {
                self.best = Some((cost, choices.clone(), usage.clone()));
            }
            return;
        }

        // Domains hold stable indices into `by_net`; copying every option's
        // passage keys at every search node makes large sparse portfolios pay
        // avoidable allocation costs.
        let mut domains = BTreeMap::<String, Vec<usize>>::new();
        let mut lower_bound = cost;
        for net in remaining {
            let mut domain = Vec::new();
            let mut conflict_prunes = 0_u64;
            let mut fixed_context_prunes = 0_u64;
            let mut capacity_prunes = 0_u64;
            let mut failures = Vec::new();
            for (option_index, option) in self.by_net[net].iter().enumerate() {
                match self.fits(option, chosen, usage) {
                    Ok(()) => domain.push(option_index),
                    Err(failure @ FitFailure::FixedContext { .. }) => {
                        fixed_context_prunes += 1;
                        failures.push((option_index, failure));
                    }
                    Err(failure @ FitFailure::Conflict { .. }) => {
                        conflict_prunes += 1;
                        failures.push((option_index, failure));
                    }
                    Err(failure @ FitFailure::Capacity { .. }) => {
                        capacity_prunes += 1;
                        failures.push((option_index, failure));
                    }
                }
            }
            self.work.pruned_conflict += conflict_prunes;
            self.work.pruned_fixed_context += fixed_context_prunes;
            self.work.pruned_capacity += capacity_prunes;
            if domain.is_empty() {
                self.work.pruned_empty_domain += 1;
                self.record_empty_domain(net, &failures, choices, passage_users);
                return;
            }
            lower_bound += self.by_net[net][domain[0]].cost;
            domains.insert(net.clone(), domain);
        }
        if self
            .best
            .as_ref()
            .is_some_and(|(best_cost, _, _)| !lower_bound.total_cmp(best_cost).is_lt())
        {
            // Options, nets, and MRV ties all have stable ordering. Therefore
            // the first optimum is also the deterministic tie-break winner,
            // and an equal-cost subtree cannot improve the selected result.
            self.work.pruned_bound += 1;
            return;
        }

        let net = remaining
            .iter()
            .min_by(|left, right| {
                let left_domain = &domains[*left];
                let right_domain = &domains[*right];
                left_domain
                    .len()
                    .cmp(&right_domain.len())
                    .then_with(|| {
                        self.conflict_degree(right.as_str(), right_domain)
                            .cmp(&self.conflict_degree(left.as_str(), left_domain))
                    })
                    .then_with(|| left.cmp(right))
            })
            .expect("a nonempty remaining set has a minimum")
            .clone();
        let next_remaining = remaining
            .iter()
            .filter(|candidate| **candidate != net)
            .cloned()
            .collect::<Vec<_>>();
        let domain = domains.remove(&net).unwrap();
        for option_index in domain {
            let option = self.by_net[&net][option_index].clone();
            self.work.branches += 1;
            choices.insert(net.clone(), option.family_id.clone());
            chosen.insert(option.family_id.clone());
            for passage in &option.passages {
                *usage.entry(passage.clone()).or_default() += option.demand;
                passage_users
                    .entry(passage.clone())
                    .or_default()
                    .insert(net.clone());
            }
            self.visit(
                &next_remaining,
                cost + option.cost,
                choices,
                chosen,
                usage,
                passage_users,
            );
            for passage in &option.passages {
                let remove = {
                    let used = usage.get_mut(passage).unwrap();
                    *used -= option.demand;
                    *used <= CAPACITY_EPSILON
                };
                if remove {
                    usage.remove(passage);
                }
                let remove_users = {
                    let users = passage_users.get_mut(passage).unwrap();
                    users.remove(&net);
                    users.is_empty()
                };
                if remove_users {
                    passage_users.remove(passage);
                }
            }
            chosen.remove(&option.family_id);
            choices.remove(&net);
            if self.exhausted {
                return;
            }
        }
    }
}

#[derive(Clone)]
enum FitFailure {
    FixedContext {
        option_run_id: String,
        context_id: String,
        context_branch: String,
        context_layer: String,
        fixed_context_dependency_sha256: String,
        conflict_kind: FamilyConflictKind,
    },
    Conflict {
        chosen_family_id: String,
    },
    Capacity {
        passage: PassageId,
        used_width: f64,
        available_width: f64,
    },
}

fn hash_dead_end(hash: &mut Sha256, witness: &FamilyAssignmentDeadEndEvidence) {
    hash_field(hash, witness.blocked_branch.as_bytes());
    hash.update((witness.chosen.len() as u64).to_le_bytes());
    for choice in &witness.chosen {
        hash_choice(hash, choice);
    }
    hash.update((witness.option_blockers.len() as u64).to_le_bytes());
    for blocker in &witness.option_blockers {
        match blocker {
            FamilyAssignmentOptionBlocker::FixedContext {
                option_family_id,
                option_branch,
                option_run_id,
                context_id,
                context_branch,
                context_layer,
                fixed_context_dependency_sha256,
                conflict_kind,
            } => {
                hash.update([3]);
                hash_field(hash, option_family_id.as_bytes());
                hash_field(hash, option_branch.as_bytes());
                hash_field(hash, option_run_id.as_bytes());
                hash_field(hash, context_id.as_bytes());
                hash_field(hash, context_branch.as_bytes());
                hash_field(hash, context_layer.as_bytes());
                hash_field(hash, fixed_context_dependency_sha256.as_bytes());
                hash_serialized(hash, conflict_kind);
            }
            FamilyAssignmentOptionBlocker::Conflict {
                option_family_id,
                chosen,
            } => {
                hash.update([1]);
                hash_field(hash, option_family_id.as_bytes());
                hash_choice(hash, chosen);
            }
            FamilyAssignmentOptionBlocker::Capacity {
                option_family_id,
                layer,
                passage_key,
                used_width,
                demand,
                available_width,
                contributors,
            } => {
                hash.update([2]);
                hash_field(hash, option_family_id.as_bytes());
                hash_field(hash, layer.as_bytes());
                hash_field(hash, passage_key.as_bytes());
                hash.update(used_width.to_bits().to_le_bytes());
                hash.update(demand.to_bits().to_le_bytes());
                hash.update(available_width.to_bits().to_le_bytes());
                hash.update((contributors.len() as u64).to_le_bytes());
                for contributor in contributors {
                    hash_choice(hash, contributor);
                }
            }
        }
    }
}

fn hash_choice(hash: &mut Sha256, choice: &FamilyAssignmentChoiceEvidence) {
    hash_field(hash, choice.branch.as_bytes());
    hash_field(hash, choice.family_id.as_bytes());
}

fn hash_field(hash: &mut Sha256, value: &[u8]) {
    hash.update((value.len() as u64).to_le_bytes());
    hash.update(value);
}

fn encode_sha256(digest: impl AsRef<[u8]>) -> String {
    let digest = digest.as_ref();
    let mut encoded = String::with_capacity(digest.len() * 2);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for &byte in digest {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn hash_serialized(hash: &mut Sha256, value: &impl Serialize) {
    let encoded = serde_json::to_vec(value).expect("validated route-family evidence serializes");
    hash_field(hash, &encoded);
}

fn fixed_context_run_dependency_sha256(
    context: &crate::family_ir::FixedContextCertificate,
    run: &crate::family_ir::FixedContextRun,
) -> String {
    let mut hash = Sha256::new();
    hash_field(
        &mut hash,
        FIXED_CONTEXT_DEPENDENCY_FINGERPRINT_CONTRACT.as_bytes(),
    );
    hash_field(&mut hash, context.method.as_bytes());
    hash.update(context.placement_revision.to_le_bytes());
    hash.update(context.geometry_revision.to_le_bytes());
    hash_field(&mut hash, run.context_id.as_bytes());
    hash_field(&mut hash, run.branch.as_bytes());
    hash_field(&mut hash, run.electrical_net.as_bytes());
    hash_field(&mut hash, run.layer.as_bytes());
    hash.update(run.required_width.to_bits().to_le_bytes());
    hash.update(run.required_clearance.to_bits().to_le_bytes());
    hash.update(run.placement_revision.to_le_bytes());
    hash.update(run.geometry_revision.to_le_bytes());
    hash.update((run.polyline.len() as u64).to_le_bytes());
    for point in &run.polyline {
        hash.update(point.x.to_bits().to_le_bytes());
        hash.update(point.y.to_bits().to_le_bytes());
    }
    encode_sha256(hash.finalize())
}

fn fixed_context_dependency_sha256(runs: &[FamilyAssignmentFixedContextRunEvidence]) -> String {
    let mut hash = Sha256::new();
    hash_field(
        &mut hash,
        FIXED_CONTEXT_DEPENDENCY_FINGERPRINT_CONTRACT.as_bytes(),
    );
    hash.update((runs.len() as u64).to_le_bytes());
    for run in runs {
        hash_field(&mut hash, run.context_id.as_bytes());
        hash_field(&mut hash, run.context_branch.as_bytes());
        hash_field(&mut hash, run.context_layer.as_bytes());
        hash_field(&mut hash, run.fixed_context_dependency_sha256.as_bytes());
    }
    encode_sha256(hash.finalize())
}

fn canonical_conflict_pair(conflict: &crate::family_ir::FamilyConflict) -> (&str, &str) {
    if conflict.left_family_id <= conflict.right_family_id {
        (&conflict.left_family_id, &conflict.right_family_id)
    } else {
        (&conflict.right_family_id, &conflict.left_family_id)
    }
}

/// A full bounded-search context commitment. This deliberately complements,
/// rather than changes, the narrower candidate geometry fingerprint.
fn portfolio_context_sha256(portfolio: &RouteFamilyPortfolio) -> String {
    let mut hash = Sha256::new();
    let v4 = portfolio.schema_version == ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION
        || portfolio.schema_version == MULTI_RUN_ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION;
    let v3 = portfolio.schema_version == FIXED_WITNESS_ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION;
    let fixed = v3 || v4;
    hash_field(
        &mut hash,
        if v4 {
            FIXED_CONTEXT_PORTFOLIO_CONTEXT_FINGERPRINT_CONTRACT.as_bytes()
        } else if v3 {
            FIXED_WITNESS_PORTFOLIO_CONTEXT_FINGERPRINT_CONTRACT.as_bytes()
        } else {
            PORTFOLIO_CONTEXT_FINGERPRINT_CONTRACT.as_bytes()
        },
    );
    hash_field(&mut hash, portfolio.schema_version.as_bytes());
    hash_field(&mut hash, portfolio.portfolio_id.as_bytes());
    hash.update(portfolio.placement_revision.to_le_bytes());
    if v4 {
        hash_serialized(&mut hash, &portfolio.geometry_revision);
    }
    if fixed {
        hash_serialized(&mut hash, &portfolio.feasibility_mode);
    }
    hash_serialized(&mut hash, &portfolio.family_generation);

    let mut variables = portfolio.variables.iter().collect::<Vec<_>>();
    variables.sort_by(|left, right| left.branch.cmp(&right.branch));
    hash.update((variables.len() as u64).to_le_bytes());
    for variable in variables {
        hash_field(&mut hash, variable.branch.as_bytes());
        hash_field(&mut hash, variable.electrical_net.as_bytes());
        hash.update(variable.required_width.to_bits().to_le_bytes());
        if fixed {
            hash_serialized(&mut hash, &variable.required_clearance);
        }
        let mut families = variable.families.iter().collect::<Vec<_>>();
        families.sort_by(|left, right| left.family_id.cmp(&right.family_id));
        hash.update((families.len() as u64).to_le_bytes());
        for family in families {
            hash_serialized(&mut hash, family);
        }
    }

    let mut passages = portfolio.passage_certificates.iter().collect::<Vec<_>>();
    passages.sort_by(|left, right| {
        left.layer
            .cmp(&right.layer)
            .then_with(|| left.key.cmp(&right.key))
    });
    hash.update((passages.len() as u64).to_le_bytes());
    for passage in passages {
        hash_serialized(&mut hash, passage);
    }

    let analysis = &portfolio.pair_analysis;
    hash_field(&mut hash, analysis.method.as_bytes());
    hash.update(analysis.placement_revision.to_le_bytes());
    hash_field(&mut hash, analysis.corridor_epoch_fingerprint.as_bytes());
    hash_field(&mut hash, analysis.candidate_set_fingerprint.as_bytes());
    hash.update([u8::from(analysis.search_complete)]);
    if fixed {
        hash_serialized(&mut hash, &analysis.analyzed_family_pairs);
    }
    let mut conflicts = analysis.conflicts.iter().collect::<Vec<_>>();
    conflicts
        .sort_by(|left, right| canonical_conflict_pair(left).cmp(&canonical_conflict_pair(right)));
    hash.update((conflicts.len() as u64).to_le_bytes());
    for conflict in conflicts {
        let (left, right) = canonical_conflict_pair(conflict);
        hash_field(&mut hash, left.as_bytes());
        hash_field(&mut hash, right.as_bytes());
        hash_serialized(&mut hash, &conflict.kind);
        hash.update([u8::from(conflict.certified)]);
        hash_serialized(&mut hash, &conflict.evidence);
    }
    if v4 {
        hash_serialized(&mut hash, &portfolio.fixed_context);
        hash_serialized(&mut hash, &analysis.fixed_context_fingerprint);
        hash_serialized(&mut hash, &analysis.analyzed_family_context_pairs);
        let mut context_conflicts = analysis.context_conflicts.iter().collect::<Vec<_>>();
        context_conflicts.sort_by(|left, right| {
            (&left.family_id, &left.context_id).cmp(&(&right.family_id, &right.context_id))
        });
        hash.update((context_conflicts.len() as u64).to_le_bytes());
        for conflict in context_conflicts {
            hash_field(&mut hash, conflict.family_id.as_bytes());
            hash_field(&mut hash, conflict.context_id.as_bytes());
            hash_serialized(&mut hash, &conflict.kind);
            hash.update([u8::from(conflict.certified)]);
            hash_serialized(&mut hash, &conflict.evidence);
        }
    }
    encode_sha256(hash.finalize())
}

/// Select exactly one certified family per branch variable and prove minimum
/// total cost.
///
/// Validation is repeated at the boundary. A typed portfolio assembled or
/// mutated without validation is rejected before any search node is visited.
/// V2 charges `required_width` once per unique passage in each branch family.
/// Same-electrical-net branches remain separate conservative demands until a
/// producer can supply a certified copper-union demand map.
/// Equal-cost optima use the stable search order: minimum remaining domain,
/// descending conflict degree, branch ID, then family cost and family ID.
pub fn select_route_families(
    portfolio: &RouteFamilyPortfolio,
    limits: FamilyAssignmentLimits,
) -> Result<FamilyAssignmentOutcome, FamilyAssignmentError> {
    select_route_families_with_clearance_policy(
        portfolio,
        limits,
        FamilyClearancePolicy::RepairObligation,
    )
}

/// Select one family per branch under an explicit copper-clearance policy.
///
/// This preserves the predecessor's repair-obligation behavior through
/// [`select_route_families`] while allowing experimental producers to require
/// an already clearance-compatible discrete assignment.
pub fn select_route_families_with_clearance_policy(
    portfolio: &RouteFamilyPortfolio,
    limits: FamilyAssignmentLimits,
    clearance_policy: FamilyClearancePolicy,
) -> Result<FamilyAssignmentOutcome, FamilyAssignmentError> {
    if let Err(issues) = portfolio.validate() {
        return Err(FamilyAssignmentError::InvalidPortfolio(issues));
    }

    let scalar_capacity = portfolio.effective_feasibility_mode()
        == RouteFamilyFeasibilityMode::ExclusivePassageCapacity;
    let capacities = portfolio
        .passage_certificates
        .iter()
        .filter_map(|passage| {
            scalar_capacity.then(|| {
                (
                    (passage.layer.clone(), passage.key.clone()),
                    passage
                        .available_width
                        .expect("validated scalar passage has an available width"),
                )
            })
        })
        .collect::<BTreeMap<_, _>>();
    // Validation above proves that V4 runs and pair analysis belong to the
    // current placement/geometry epoch. Only after that boundary may blocker
    // evidence commit to these exact serialized runs.
    let context_runs = portfolio
        .fixed_context
        .as_ref()
        .map(|context| {
            context
                .runs
                .iter()
                .map(|run| (run.context_id.as_str(), run))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let family_runs = portfolio
        .variables
        .iter()
        .flat_map(|variable| {
            variable
                .families
                .iter()
                .map(|family| (family.family_id.as_str(), family.runs[0].run_id.as_str()))
        })
        .collect::<BTreeMap<_, _>>();
    let mut fixed_context_conflicts =
        BTreeMap::<String, Vec<FixedContextConflictDependency>>::new();
    for conflict in &portfolio.pair_analysis.context_conflicts {
        if !conflict_requires_discrete_separation(conflict.kind, clearance_policy) {
            continue;
        }
        let context = portfolio
            .fixed_context
            .as_ref()
            .expect("validated context conflicts require a V4 context certificate");
        let context_run = context_runs[conflict.context_id.as_str()];
        fixed_context_conflicts
            .entry(conflict.family_id.clone())
            .or_default()
            .push(FixedContextConflictDependency {
                option_run_id: family_runs[conflict.family_id.as_str()].to_owned(),
                context_id: conflict.context_id.clone(),
                context_branch: context_run.branch.clone(),
                context_layer: context_run.layer.clone(),
                fixed_context_dependency_sha256: fixed_context_run_dependency_sha256(
                    context,
                    context_run,
                ),
                conflict_kind: conflict.kind,
            });
    }
    for conflicts in fixed_context_conflicts.values_mut() {
        conflicts.sort_by(|left, right| {
            (
                &left.context_id,
                &left.context_branch,
                &left.context_layer,
                &left.option_run_id,
                left.conflict_kind as u8,
            )
                .cmp(&(
                    &right.context_id,
                    &right.context_branch,
                    &right.context_layer,
                    &right.option_run_id,
                    right.conflict_kind as u8,
                ))
        });
    }
    let mut by_net = BTreeMap::<String, Vec<AssignmentOption>>::new();
    for variable in &portfolio.variables {
        let options = by_net.entry(variable.branch.clone()).or_default();
        for family in &variable.families {
            let passages = if scalar_capacity {
                family
                    .runs
                    .iter()
                    .flat_map(|run| {
                        run.semantic_passage_keys
                            .iter()
                            .map(move |key| (run.layer.clone(), key.clone()))
                    })
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect()
            } else {
                Vec::new()
            };
            options.push(AssignmentOption {
                net: variable.branch.clone(),
                family_id: family.family_id.clone(),
                cost: family.cost,
                demand: variable.required_width,
                passages,
                fixed_context_conflicts: fixed_context_conflicts
                    .get(&family.family_id)
                    .cloned()
                    .unwrap_or_default(),
            });
        }
        options.sort_by(|left, right| {
            left.cost
                .total_cmp(&right.cost)
                .then_with(|| left.tie_key().cmp(&right.tie_key()))
        });
    }
    let mut adjacency = BTreeMap::<String, BTreeSet<String>>::new();
    for conflict in &portfolio.pair_analysis.conflicts {
        if !conflict_requires_discrete_separation(conflict.kind, clearance_policy) {
            continue;
        }
        adjacency
            .entry(conflict.left_family_id.clone())
            .or_default()
            .insert(conflict.right_family_id.clone());
        adjacency
            .entry(conflict.right_family_id.clone())
            .or_default()
            .insert(conflict.left_family_id.clone());
    }

    let remaining = by_net.keys().cloned().collect::<Vec<_>>();
    let admitted_domains = by_net
        .iter()
        .map(|(branch, options)| FamilyAssignmentAdmittedDomainEvidence {
            branch: branch.clone(),
            admitted_family_count: options.len(),
        })
        .collect();
    let mut search = Search {
        by_net,
        capacities,
        adjacency,
        limits,
        work: FamilyAssignmentWork {
            capacity_enforcement: if scalar_capacity {
                FamilyAssignmentCapacityEnforcement::ScalarPassageWidth
            } else {
                FamilyAssignmentCapacityEnforcement::FixedWitnessPairwiseNoScalar
            },
            ..FamilyAssignmentWork::default()
        },
        exhausted: false,
        best: None,
        max_families_per_variable: portfolio.family_generation.max_families_per_variable,
        admitted_domains,
        candidate_set_fingerprint: portfolio.pair_analysis.candidate_set_fingerprint.clone(),
        portfolio_context_sha256: portfolio_context_sha256(portfolio),
        all_fixed_context_sha256: portfolio
            .fixed_context
            .as_ref()
            .map(|context| context.fixed_context_fingerprint.clone()),
        conflict_context_runs: BTreeMap::new(),
        implicated_variables: BTreeSet::new(),
        empty_domain_events: 0,
        dead_end_witnesses: Vec::new(),
        dead_end_witnesses_truncated: 0,
        dead_end_hash: Sha256::new(),
    };
    search.visit(
        &remaining,
        0.0,
        &mut BTreeMap::new(),
        &mut BTreeSet::new(),
        &mut BTreeMap::new(),
        &mut BTreeMap::new(),
    );
    if search.exhausted {
        return Ok(FamilyAssignmentOutcome::BudgetExhausted {
            work: search.work,
            best_incumbent_cost: search.best.as_ref().map(|(cost, _, _)| *cost),
        });
    }
    let Some((total_cost, choices, usage)) = search.best else {
        let evidence = search.bounded_infeasibility_evidence();
        return Ok(FamilyAssignmentOutcome::BoundedInfeasible {
            work: search.work,
            evidence,
        });
    };
    let passage_usage = usage
        .into_iter()
        .map(|((layer, passage_key), used_width)| {
            let available_width = search.capacities[&(layer.clone(), passage_key.clone())];
            PassageUsage {
                layer,
                passage_key,
                used_width,
                available_width,
                remaining_width: available_width - used_width,
            }
        })
        .collect();
    let selected_families = choices
        .values()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let mut repair_obligations = portfolio
        .pair_analysis
        .conflicts
        .iter()
        .filter(|conflict| {
            !conflict_requires_discrete_separation(conflict.kind, clearance_policy)
                && selected_families.contains(conflict.left_family_id.as_str())
                && selected_families.contains(conflict.right_family_id.as_str())
        })
        .map(|conflict| FamilySelectionRepairObligation::FamilyPair {
            left_family_id: conflict.left_family_id.clone(),
            right_family_id: conflict.right_family_id.clone(),
            kind: conflict.kind,
            evidence: conflict.evidence.clone(),
        })
        .collect::<Vec<_>>();
    repair_obligations.extend(
        portfolio
            .pair_analysis
            .context_conflicts
            .iter()
            .filter(|conflict| {
                !conflict_requires_discrete_separation(conflict.kind, clearance_policy)
                    && selected_families.contains(conflict.family_id.as_str())
            })
            .map(|conflict| FamilySelectionRepairObligation::FixedContext {
                family_id: conflict.family_id.clone(),
                context_id: conflict.context_id.clone(),
                kind: conflict.kind,
                evidence: conflict.evidence.clone(),
            }),
    );
    Ok(FamilyAssignmentOutcome::Selected {
        selection: FamilySelection {
            choices,
            total_cost,
            passage_usage,
            repair_obligations,
        },
        work: search.work,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::copper_pair::{
        FixedCopperPairClassification, FixedCopperRoute, classify_prepared_copper_pair,
        prepare_fixed_copper_route,
    };
    use crate::family_ir::{
        ConflictEvidence, CorridorEpoch, CutWordCertificate,
        FIXED_WITNESS_ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION, FamilyConflict, FamilyConflictKind,
        FamilyContextConflict, FamilyGenerationCertificate, FamilyPoint, FixedContextCertificate,
        FixedContextRun, LEGACY_ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION, PairAnalysisCertificate,
        PassageCapacityCertificate, ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION, RequiredNullable,
        RouteFamily, RouteFamilyRun, RouteFamilyVariable, RouteTransition, RouteTransitionKind,
        RouteWitness,
    };

    fn family(
        net: &str,
        ordinal: usize,
        cost: f64,
        width: f64,
        passages: &[String],
    ) -> RouteFamily {
        let family_id = format!("{net}/family-{ordinal}");
        RouteFamily {
            family_id: family_id.clone(),
            placement_revision: 7,
            complete_route: true,
            cost,
            runs: vec![RouteFamilyRun {
                run_id: format!("{family_id}/run-0"),
                run_index: 0,
                layer: "top".into(),
                start_point: 0,
                end_point: 1,
                corridor_revision: 11,
                transition_from_previous: RequiredNullable::Null,
                witness: RouteWitness {
                    polyline: vec![
                        FamilyPoint { x: 0.0, y: 0.0 },
                        FamilyPoint { x: 1.0, y: 0.0 },
                    ],
                    placement_revision: 7,
                    realizable: true,
                    clearance_certified: true,
                    clearance_model: "synthetic-exact-v1".into(),
                    minimum_clearance: 0.2,
                    required_width: width,
                },
                cut_word_certificate: CutWordCertificate {
                    cut_basis_fingerprint: "cuts-v1".into(),
                    basis_complete: true,
                    verified: true,
                    word: Vec::new(),
                },
                semantic_passage_keys: passages.to_vec(),
                passage_mapping_complete: true,
            }],
        }
    }

    fn portfolio(
        variables: Vec<RouteFamilyVariable>,
        capacities: Vec<(String, f64)>,
        conflict_ids: Vec<(String, String)>,
    ) -> RouteFamilyPortfolio {
        let max_families = variables
            .iter()
            .map(|variable| variable.families.len())
            .max()
            .unwrap_or(1);
        let mut portfolio = RouteFamilyPortfolio {
            schema_version: LEGACY_ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION.into(),
            feasibility_mode: None,
            portfolio_id: "assignment-test".into(),
            placement_revision: 7,
            geometry_revision: None,
            corridor_epochs: vec![CorridorEpoch {
                layer: "top".into(),
                corridor_revision: 11,
                semantic_fingerprint: "semantic-v1".into(),
                cut_basis_fingerprint: "cuts-v1".into(),
                cut_basis_complete: true,
            }],
            family_generation: FamilyGenerationCertificate {
                method: "bounded-test".into(),
                max_families_per_variable: max_families,
                search_complete: true,
                budget_exhausted: false,
            },
            passage_certificates: capacities
                .into_iter()
                .map(|(key, available_width)| PassageCapacityCertificate {
                    layer: "top".into(),
                    key,
                    placement_revision: 7,
                    corridor_revision: 11,
                    available_width: Some(available_width),
                    minimum_clearance: 0.2,
                    enforcement: None,
                    capacity_model: "certified-width-v1".into(),
                    certified: true,
                })
                .collect(),
            variables,
            fixed_context: None,
            pair_analysis: PairAnalysisCertificate {
                method: "complete-pair-test".into(),
                placement_revision: 7,
                corridor_epoch_fingerprint: "pending".into(),
                candidate_set_fingerprint: "pending".into(),
                search_complete: true,
                analyzed_family_pairs: None,
                fixed_context_fingerprint: None,
                analyzed_family_context_pairs: None,
                context_conflicts: Vec::new(),
                conflicts: conflict_ids
                    .into_iter()
                    .map(|(left_family_id, right_family_id)| FamilyConflict {
                        left_family_id,
                        right_family_id,
                        kind: FamilyConflictKind::CenterlineCrossing,
                        certified: true,
                        evidence: ConflictEvidence {
                            layer: RequiredNullable::Value("top".into()),
                            passage_key: RequiredNullable::Null,
                            detail: "certified test conflict".into(),
                        },
                    })
                    .collect(),
            },
        };
        portfolio.seal_pair_analysis_fingerprints().unwrap();
        portfolio.validate().unwrap();
        portfolio
    }

    fn variable(
        branch: &str,
        electrical_net: &str,
        width: f64,
        options: Vec<(f64, Vec<String>)>,
    ) -> RouteFamilyVariable {
        RouteFamilyVariable {
            branch: branch.into(),
            electrical_net: electrical_net.into(),
            required_width: width,
            required_clearance: None,
            families: options
                .into_iter()
                .enumerate()
                .map(|(ordinal, (cost, passages))| family(branch, ordinal, cost, width, &passages))
                .collect(),
        }
    }

    fn v4_unary_context_portfolio(second_y: f64) -> RouteFamilyPortfolio {
        let mut input = portfolio(
            vec![net("branch", 0.5, vec![(0.0, vec![]), (1.0, vec![])])],
            vec![],
            vec![],
        );
        input.schema_version = ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION.into();
        input.geometry_revision = Some(9);
        input.feasibility_mode = Some(RouteFamilyFeasibilityMode::FixedFullWidthWitnessPairwise);
        input.variables[0].required_clearance = Some(0.25);
        for (index, family) in input.variables[0].families.iter_mut().enumerate() {
            for point in &mut family.runs[0].witness.polyline {
                point.y = if index == 0 { 0.0 } else { second_y };
            }
            family.runs[0].witness.clearance_model =
                "exact_polyline_against_polygonal_obstacles".into();
            family.runs[0].witness.minimum_clearance = 1.0;
        }
        let context = FixedContextRun {
            context_id: "outside/run-0".into(),
            branch: "outside".into(),
            electrical_net: "outside-net".into(),
            layer: "top".into(),
            required_width: 0.5,
            required_clearance: 0.25,
            polyline: vec![
                FamilyPoint { x: 0.0, y: 0.0 },
                FamilyPoint { x: 1.0, y: 0.0 },
            ],
            placement_revision: 7,
            geometry_revision: 9,
        };
        input.fixed_context = Some(FixedContextCertificate {
            method: crate::family_ir::FIXED_CONTEXT_ANALYSIS_METHOD.into(),
            placement_revision: 7,
            geometry_revision: 9,
            fixed_context_fingerprint: String::new(),
            search_complete: true,
            runs: vec![context.clone()],
        });
        input.pair_analysis.method = crate::family_ir::FIXED_WITNESS_PAIR_ANALYSIS_METHOD.into();
        input.pair_analysis.analyzed_family_pairs = Some(0);
        rebuild_v4_context_analysis(&mut input);
        input
    }

    fn rebuild_v4_context_analysis(input: &mut RouteFamilyPortfolio) {
        let context = input.fixed_context.as_ref().unwrap();
        input.pair_analysis.context_conflicts.clear();
        let mut analyzed = 0_u64;
        for variable in &input.variables {
            for family in &variable.families {
                let family_run = &family.runs[0];
                let points = family_run
                    .witness
                    .polyline
                    .iter()
                    .map(|point| crate::model::Vec2::new(point.x, point.y))
                    .collect::<Vec<_>>();
                let prepared = prepare_fixed_copper_route(FixedCopperRoute {
                    polyline: &points,
                    width: variable.required_width,
                    clearance: variable.required_clearance.unwrap(),
                })
                .unwrap();
                for context_run in &context.runs {
                    if variable.electrical_net == context_run.electrical_net {
                        continue;
                    }
                    analyzed += 1;
                    if family_run.layer != context_run.layer {
                        continue;
                    }
                    let context_points = context_run
                        .polyline
                        .iter()
                        .map(|point| crate::model::Vec2::new(point.x, point.y))
                        .collect::<Vec<_>>();
                    let context_prepared = prepare_fixed_copper_route(FixedCopperRoute {
                        polyline: &context_points,
                        width: context_run.required_width,
                        clearance: context_run.required_clearance,
                    })
                    .unwrap();
                    let classification =
                        classify_prepared_copper_pair(&prepared, &context_prepared).unwrap();
                    let evidence = classification.evidence();
                    let kind = match classification {
                        FixedCopperPairClassification::Clear(_) => continue,
                        FixedCopperPairClassification::CenterlineIntersection(_) => {
                            FamilyConflictKind::CenterlineCrossing
                        }
                        FixedCopperPairClassification::ClearanceShortfall(_) => {
                            FamilyConflictKind::CopperClearance
                        }
                    };
                    input.pair_analysis.context_conflicts.push(FamilyContextConflict {
                        family_id: family.family_id.clone(),
                        context_id: context_run.context_id.clone(),
                        kind,
                        certified: true,
                        evidence: ConflictEvidence {
                            layer: RequiredNullable::Value(family_run.layer.clone()),
                            passage_key: RequiredNullable::Null,
                            detail: format!(
                                "fixed context copper: family={}, context={}, family_segment={}, context_segment={}, centerline_distance={:.17}, required_distance={:.17}, segment_pairs_analyzed={}",
                                family.family_id,
                                context_run.context_id,
                                evidence.first_segment_index,
                                evidence.second_segment_index,
                                evidence.minimum_centerline_distance,
                                evidence.required_centerline_distance,
                                evidence.segment_pairs_analyzed,
                            ),
                        },
                    });
                }
            }
        }
        input
            .pair_analysis
            .context_conflicts
            .sort_by(|left, right| {
                (&left.family_id, &left.context_id).cmp(&(&right.family_id, &right.context_id))
            });
        input.pair_analysis.analyzed_family_context_pairs = Some(analyzed);
        input.seal_pair_analysis_fingerprints().unwrap();
        input.validate().unwrap();
    }

    fn net(id: &str, width: f64, options: Vec<(f64, Vec<String>)>) -> RouteFamilyVariable {
        variable(id, id, width, options)
    }

    /// Frozen copy of the pre-V3 `/v1` portfolio-context implementation. This
    /// is intentionally separate from the version-dispatching production
    /// function so legacy compatibility is checked against the old algorithm,
    /// not merely against another call through current code.
    fn legacy_v1_portfolio_context_reference(portfolio: &RouteFamilyPortfolio) -> String {
        let mut hash = Sha256::new();
        hash_field(&mut hash, PORTFOLIO_CONTEXT_FINGERPRINT_CONTRACT.as_bytes());
        hash_field(&mut hash, portfolio.schema_version.as_bytes());
        hash_field(&mut hash, portfolio.portfolio_id.as_bytes());
        hash.update(portfolio.placement_revision.to_le_bytes());
        hash_serialized(&mut hash, &portfolio.family_generation);

        let mut variables = portfolio.variables.iter().collect::<Vec<_>>();
        variables.sort_by(|left, right| left.branch.cmp(&right.branch));
        hash.update((variables.len() as u64).to_le_bytes());
        for variable in variables {
            hash_field(&mut hash, variable.branch.as_bytes());
            hash_field(&mut hash, variable.electrical_net.as_bytes());
            hash.update(variable.required_width.to_bits().to_le_bytes());
            let mut families = variable.families.iter().collect::<Vec<_>>();
            families.sort_by(|left, right| left.family_id.cmp(&right.family_id));
            hash.update((families.len() as u64).to_le_bytes());
            for family in families {
                hash_serialized(&mut hash, family);
            }
        }

        let mut passages = portfolio.passage_certificates.iter().collect::<Vec<_>>();
        passages.sort_by(|left, right| {
            left.layer
                .cmp(&right.layer)
                .then_with(|| left.key.cmp(&right.key))
        });
        hash.update((passages.len() as u64).to_le_bytes());
        for passage in passages {
            hash_serialized(&mut hash, passage);
        }

        let analysis = &portfolio.pair_analysis;
        hash_field(&mut hash, analysis.method.as_bytes());
        hash.update(analysis.placement_revision.to_le_bytes());
        hash_field(&mut hash, analysis.corridor_epoch_fingerprint.as_bytes());
        hash_field(&mut hash, analysis.candidate_set_fingerprint.as_bytes());
        hash.update([u8::from(analysis.search_complete)]);
        let mut conflicts = analysis.conflicts.iter().collect::<Vec<_>>();
        conflicts.sort_by(|left, right| {
            canonical_conflict_pair(left).cmp(&canonical_conflict_pair(right))
        });
        hash.update((conflicts.len() as u64).to_le_bytes());
        for conflict in conflicts {
            let (left, right) = canonical_conflict_pair(conflict);
            hash_field(&mut hash, left.as_bytes());
            hash_field(&mut hash, right.as_bytes());
            hash_serialized(&mut hash, &conflict.kind);
            hash.update([u8::from(conflict.certified)]);
            hash_serialized(&mut hash, &conflict.evidence);
        }
        encode_sha256(hash.finalize())
    }

    #[test]
    fn certified_crossing_only_choices_are_bounded_infeasible_with_auditable_cover() {
        let input = portfolio(
            vec![
                net("horizontal", 0.2, vec![(0.0, vec![])]),
                net("vertical", 0.2, vec![(0.0, vec![])]),
            ],
            vec![],
            vec![("horizontal/family-0".into(), "vertical/family-0".into())],
        );
        let FamilyAssignmentOutcome::BoundedInfeasible { work, evidence } =
            select_route_families(&input, FamilyAssignmentLimits::default()).unwrap()
        else {
            panic!("the finite conflict-only portfolio must be exhausted");
        };
        assert_eq!(work.pruned_empty_domain, 1);
        assert_eq!(evidence.contract, BOUNDED_INFEASIBILITY_EVIDENCE_CONTRACT);
        assert_eq!(
            evidence.candidate_set_fingerprint,
            input.pair_analysis.candidate_set_fingerprint
        );
        assert_eq!(evidence.max_families_per_variable, 1);
        assert_eq!(
            evidence.admitted_domains,
            [
                FamilyAssignmentAdmittedDomainEvidence {
                    branch: "horizontal".into(),
                    admitted_family_count: 1,
                },
                FamilyAssignmentAdmittedDomainEvidence {
                    branch: "vertical".into(),
                    admitted_family_count: 1,
                },
            ]
        );
        assert!(evidence.exhaustive_within_admitted_portfolio);
        assert_eq!(evidence.portfolio_context_sha256.len(), 64);
        assert_eq!(
            evidence.implicated_variables,
            ["horizontal".to_owned(), "vertical".to_owned()]
        );
        assert_eq!(evidence.empty_domain_events, 1);
        assert_eq!(evidence.witnesses.len(), 1);
        assert_eq!(evidence.witnesses_truncated, 0);
        assert_eq!(evidence.canonical_sha256.len(), 64);
        assert!(
            evidence
                .canonical_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        );
        assert!(matches!(
            evidence.witnesses[0].option_blockers.as_slice(),
            [FamilyAssignmentOptionBlocker::Conflict { option_family_id, chosen }]
                if option_family_id == "vertical/family-0"
                    && chosen.branch == "horizontal"
                    && chosen.family_id == "horizontal/family-0"
        ));
    }

    #[test]
    fn ninth_family_changes_exact_raw8_infeasibility_into_a_selection() {
        let conflicts = (0..8)
            .map(|ordinal| (format!("a/family-{ordinal}"), "b/family-0".to_owned()))
            .collect::<Vec<_>>();
        let make = |a_count: usize| {
            portfolio(
                vec![
                    net(
                        "a",
                        0.2,
                        (0..a_count)
                            .map(|ordinal| (ordinal as f64, Vec::new()))
                            .collect(),
                    ),
                    net("b", 0.2, vec![(0.0, vec![])]),
                ],
                vec![],
                conflicts
                    .iter()
                    .filter(|(left, _)| {
                        left.rsplit_once('-')
                            .and_then(|(_, ordinal)| ordinal.parse::<usize>().ok())
                            .is_some_and(|ordinal| ordinal < a_count)
                    })
                    .cloned()
                    .collect(),
            )
        };

        let raw8 = make(8);
        let FamilyAssignmentOutcome::BoundedInfeasible { evidence, .. } =
            select_route_families(&raw8, FamilyAssignmentLimits::default()).unwrap()
        else {
            panic!("the first eight families are all exactly blocked");
        };
        assert!(evidence.exhaustive_within_admitted_portfolio);
        assert_eq!(evidence.max_families_per_variable, 8);
        assert_eq!(evidence.implicated_variables, ["a", "b"]);

        let raw9 = make(9);
        let FamilyAssignmentOutcome::Selected { selection, .. } =
            select_route_families(&raw9, FamilyAssignmentLimits::default()).unwrap()
        else {
            panic!("family nine must make the bounded fixture assignable");
        };
        assert_eq!(selection.choices["a"], "a/family-8");
        assert_eq!(selection.choices["b"], "b/family-0");
    }

    #[test]
    fn bounded_capacity_failure_names_the_passage_and_contributor() {
        let input = portfolio(
            vec![
                net("a", 0.6, vec![(0.0, vec!["shared".into()])]),
                net("b", 0.6, vec![(0.0, vec!["shared".into()])]),
            ],
            vec![("shared".into(), 1.0)],
            vec![],
        );
        let FamilyAssignmentOutcome::BoundedInfeasible { evidence, .. } =
            select_route_families(&input, FamilyAssignmentLimits::default()).unwrap()
        else {
            panic!("the finite over-capacity portfolio must be exhausted");
        };
        assert_eq!(evidence.implicated_variables, ["a", "b"]);
        assert!(matches!(
            evidence.witnesses[0].option_blockers.as_slice(),
            [FamilyAssignmentOptionBlocker::Capacity {
                option_family_id,
                layer,
                passage_key,
                used_width,
                demand,
                available_width,
                contributors,
            }] if option_family_id == "b/family-0"
                && layer == "top"
                && passage_key == "shared"
                && *used_width == 0.6
                && *demand == 0.6
                && *available_width == 1.0
                && contributors == &[FamilyAssignmentChoiceEvidence {
                    branch: "a".into(),
                    family_id: "a/family-0".into(),
                }]
        ));
    }

    #[test]
    fn bounded_infeasibility_evidence_is_input_order_independent() {
        let variables = vec![
            net("horizontal", 0.2, vec![(0.0, vec![])]),
            net("vertical", 0.2, vec![(0.0, vec![])]),
        ];
        let conflicts = vec![("horizontal/family-0".into(), "vertical/family-0".into())];
        let first = portfolio(variables.clone(), vec![], conflicts.clone());
        let second = portfolio(
            variables.into_iter().rev().collect(),
            vec![],
            conflicts
                .into_iter()
                .map(|(left, right)| (right, left))
                .collect(),
        );
        let evidence = |portfolio: &RouteFamilyPortfolio| {
            let FamilyAssignmentOutcome::BoundedInfeasible { evidence, .. } =
                select_route_families(portfolio, FamilyAssignmentLimits::default()).unwrap()
            else {
                panic!("fixture must remain bounded-infeasible");
            };
            evidence
        };
        assert_eq!(evidence(&first), evidence(&second));
    }

    #[test]
    fn portfolio_context_changes_with_capacity_while_candidate_fingerprint_does_not() {
        let variables = vec![
            net("a", 0.6, vec![(0.0, vec!["shared".into()])]),
            net("b", 0.6, vec![(0.0, vec!["shared".into()])]),
        ];
        let evidence = |capacity| {
            let input = portfolio(variables.clone(), vec![("shared".into(), capacity)], vec![]);
            let FamilyAssignmentOutcome::BoundedInfeasible { evidence, .. } =
                select_route_families(&input, FamilyAssignmentLimits::default()).unwrap()
            else {
                panic!("both capacity fixtures must remain bounded-infeasible");
            };
            evidence
        };
        let first = evidence(1.0);
        let second = evidence(1.1);
        assert_eq!(
            first.candidate_set_fingerprint,
            second.candidate_set_fingerprint
        );
        assert_ne!(
            first.portfolio_context_sha256,
            second.portfolio_context_sha256
        );
    }

    #[test]
    fn portfolio_context_commits_to_analyzed_family_pair_count() {
        let mut first = portfolio(
            vec![
                net("a", 0.2, vec![(0.0, vec![])]),
                net("b", 0.2, vec![(0.0, vec![])]),
            ],
            vec![],
            vec![],
        );
        first.schema_version = FIXED_WITNESS_ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION.into();
        first.feasibility_mode = Some(RouteFamilyFeasibilityMode::FixedFullWidthWitnessPairwise);
        first.pair_analysis.method = crate::family_ir::FIXED_WITNESS_PAIR_ANALYSIS_METHOD.into();
        for variable in &mut first.variables {
            variable.required_clearance = Some(0.2);
            variable.families[0].runs[0].witness.clearance_model =
                "exact_polyline_against_polygonal_obstacles".into();
            variable.families[0].runs[0].witness.minimum_clearance = 1.0;
        }
        for point in &mut first.variables[1].families[0].runs[0].witness.polyline {
            point.y = 1.0;
        }
        first.pair_analysis.analyzed_family_pairs = Some(1);
        first.seal_pair_analysis_fingerprints().unwrap();
        first.validate().unwrap();

        let mut second = first.clone();
        second.pair_analysis.analyzed_family_pairs = Some(0);

        // This field is completeness evidence, not candidate geometry. Even a
        // malformed post-validation mutation must therefore change the full
        // bounded-context commitment rather than silently aliasing the valid
        // exhaustive matrix.
        assert_eq!(
            first.pair_analysis.candidate_set_fingerprint,
            second.pair_analysis.candidate_set_fingerprint
        );
        assert_ne!(
            portfolio_context_sha256(&first),
            portfolio_context_sha256(&second)
        );
    }

    #[test]
    fn genuine_legacy_v2_portfolio_context_digest_is_immutable() {
        let mut input = portfolio(
            vec![
                net("horizontal", 0.2, vec![(0.0, vec![])]),
                net("vertical", 0.2, vec![(0.0, vec![])]),
            ],
            vec![],
            vec![("horizontal/family-0".into(), "vertical/family-0".into())],
        );
        input.schema_version = LEGACY_ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION.into();
        input.feasibility_mode = None;
        for variable in &mut input.variables {
            variable.required_clearance = None;
        }
        for passage in &mut input.passage_certificates {
            passage.enforcement = None;
        }
        input.pair_analysis.analyzed_family_pairs = None;
        input.seal_pair_analysis_fingerprints().unwrap();
        input.validate().unwrap();
        let legacy_reference = legacy_v1_portfolio_context_reference(&input);
        assert_eq!(
            legacy_reference,
            "c25a8cbcb1776936ddf3ef82ac0c022c91c95f941d9bb57a0dcf15bbd65677d4"
        );
        assert_eq!(portfolio_context_sha256(&input), legacy_reference);

        let FamilyAssignmentOutcome::BoundedInfeasible { evidence, .. } =
            select_route_families(&input, FamilyAssignmentLimits::default()).unwrap()
        else {
            panic!("legacy conflict fixture must remain bounded-infeasible");
        };
        assert_eq!(evidence.portfolio_context_sha256, legacy_reference);
        assert_eq!(
            evidence.canonical_sha256,
            "0671c2c3c2853d5ba3d727b0865e43c17e4b49294d37f0abe7b19a0c8dc9add6"
        );
    }

    #[test]
    fn bounded_evidence_truncates_details_but_hashes_the_complete_implicated_union() {
        let collect = || {
            let mut search = Search {
                by_net: BTreeMap::from([(
                    "blocked".into(),
                    vec![AssignmentOption {
                        net: "blocked".into(),
                        family_id: "blocked/family-0".into(),
                        cost: 0.0,
                        demand: 0.2,
                        passages: Vec::new(),
                        fixed_context_conflicts: Vec::new(),
                    }],
                )]),
                capacities: BTreeMap::new(),
                adjacency: BTreeMap::new(),
                limits: FamilyAssignmentLimits::default(),
                work: FamilyAssignmentWork::default(),
                exhausted: false,
                best: None,
                max_families_per_variable: 1,
                admitted_domains: vec![FamilyAssignmentAdmittedDomainEvidence {
                    branch: "blocked".into(),
                    admitted_family_count: 1,
                }],
                candidate_set_fingerprint: "candidate-set".into(),
                portfolio_context_sha256: "portfolio-context".into(),
                all_fixed_context_sha256: None,
                conflict_context_runs: BTreeMap::new(),
                implicated_variables: BTreeSet::new(),
                empty_domain_events: 0,
                dead_end_witnesses: Vec::new(),
                dead_end_witnesses_truncated: 0,
                dead_end_hash: Sha256::new(),
            };
            for ordinal in 0..70 {
                let branch = format!("chosen-{ordinal:02}");
                let family = format!("{branch}/family-0");
                search.record_empty_domain(
                    "blocked",
                    &[(
                        0,
                        FitFailure::Conflict {
                            chosen_family_id: family.clone(),
                        },
                    )],
                    &BTreeMap::from([(branch, family)]),
                    &BTreeMap::new(),
                );
            }
            search.bounded_infeasibility_evidence()
        };
        let first = collect();
        let second = collect();
        assert_eq!(first, second);
        assert_eq!(first.empty_domain_events, 70);
        assert_eq!(first.witnesses.len(), MAX_BOUNDED_INFEASIBILITY_WITNESSES);
        assert_eq!(first.witnesses_truncated, 6);
        assert_eq!(first.implicated_variables.len(), 71);
        assert!(first.implicated_variables.contains(&"blocked".to_owned()));
        assert!(first.implicated_variables.contains(&"chosen-69".to_owned()));
    }

    #[test]
    fn certified_crossing_diverts_to_a_nonconflicting_family() {
        let input = portfolio(
            vec![
                net("horizontal", 0.2, vec![(0.0, vec![]), (1.0, vec![])]),
                net("vertical", 0.2, vec![(0.0, vec![])]),
            ],
            vec![],
            vec![("horizontal/family-0".into(), "vertical/family-0".into())],
        );
        let FamilyAssignmentOutcome::Selected { selection, .. } =
            select_route_families(&input, FamilyAssignmentLimits::default()).unwrap()
        else {
            panic!("the noncrossing family must remain selectable");
        };
        assert_eq!(selection.total_cost, 1.0);
        assert_eq!(selection.choices["horizontal"], "horizontal/family-1");
        assert_eq!(selection.choices["vertical"], "vertical/family-0");
    }

    #[test]
    fn clearance_policy_can_choose_discrete_separation_instead_of_repair() {
        let mut input = portfolio(
            vec![
                net("horizontal", 0.2, vec![(0.0, vec![]), (1.0, vec![])]),
                net("vertical", 0.2, vec![(0.0, vec![])]),
            ],
            vec![],
            vec![("horizontal/family-0".into(), "vertical/family-0".into())],
        );
        input.schema_version = FIXED_WITNESS_ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION.into();
        input.feasibility_mode = Some(RouteFamilyFeasibilityMode::FixedFullWidthWitnessPairwise);
        input.pair_analysis.method = crate::family_ir::FIXED_WITNESS_PAIR_ANALYSIS_METHOD.into();
        input.pair_analysis.conflicts.clear();
        for variable in &mut input.variables {
            variable.required_clearance = Some(0.2);
            for family in &mut variable.families {
                family.runs[0].witness.clearance_model =
                    "exact_polyline_against_polygonal_obstacles".into();
                family.runs[0].witness.minimum_clearance = 1.0;
            }
        }
        for point in &mut input.variables[0].families[1].runs[0].witness.polyline {
            point.y = 1.0;
        }
        for point in &mut input.variables[1].families[0].runs[0].witness.polyline {
            point.y = 0.3;
        }
        input.rebuild_and_seal_fixed_witness_analysis().unwrap();
        input.validate().unwrap();
        assert_eq!(input.pair_analysis.conflicts.len(), 1);
        assert_eq!(
            input.pair_analysis.conflicts[0].kind,
            FamilyConflictKind::CopperClearance
        );

        let FamilyAssignmentOutcome::Selected {
            selection: repair_selection,
            ..
        } = select_route_families(&input, FamilyAssignmentLimits::default()).unwrap()
        else {
            panic!("the predecessor policy must retain a repairable selection");
        };
        assert_eq!(repair_selection.total_cost, 0.0);
        assert_eq!(repair_selection.repair_obligations.len(), 1);

        let FamilyAssignmentOutcome::Selected {
            selection: discrete_selection,
            work,
        } = select_route_families_with_clearance_policy(
            &input,
            FamilyAssignmentLimits::default(),
            FamilyClearancePolicy::DiscreteSeparation,
        )
        .unwrap()
        else {
            panic!("a clearance-compatible discrete combination exists");
        };
        assert_eq!(discrete_selection.total_cost, 1.0);
        assert_eq!(
            discrete_selection.choices["horizontal"],
            "horizontal/family-1"
        );
        assert!(discrete_selection.repair_obligations.is_empty());
        assert_eq!(work.pruned_conflict, 1);
    }

    #[test]
    fn capacity_diverts_the_cheapest_family_in_declared_width_units() {
        let input = portfolio(
            vec![
                net("a", 0.6, vec![(0.0, vec!["shared".into()]), (1.0, vec![])]),
                net("b", 0.6, vec![(0.0, vec!["shared".into()]), (2.0, vec![])]),
            ],
            vec![("shared".into(), 1.0)],
            vec![],
        );
        let FamilyAssignmentOutcome::Selected { selection, work } =
            select_route_families(&input, FamilyAssignmentLimits::default()).unwrap()
        else {
            panic!("capacity fixture must select a family per branch variable");
        };
        assert_eq!(selection.total_cost, 1.0);
        assert_eq!(selection.choices["a"], "a/family-1");
        assert_eq!(selection.choices["b"], "b/family-0");
        assert_eq!(selection.passage_usage[0].used_width, 0.6);
        assert_eq!(selection.passage_usage[0].remaining_width, 0.4);
        assert_eq!(
            work.capacity_enforcement,
            FamilyAssignmentCapacityEnforcement::ScalarPassageWidth
        );
    }

    #[test]
    fn fixed_witness_three_way_assignment_uses_complete_pairs_without_scalar_charging() {
        let mut input = portfolio(
            vec![
                net("a", 0.6, vec![(0.0, vec!["shared".into()])]),
                net("b", 0.6, vec![(0.0, vec!["shared".into()])]),
                net("c", 0.6, vec![(0.0, vec!["shared".into()])]),
            ],
            vec![("shared".into(), 0.7)],
            vec![],
        );
        input.schema_version = FIXED_WITNESS_ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION.into();
        input.feasibility_mode = Some(RouteFamilyFeasibilityMode::FixedFullWidthWitnessPairwise);
        for variable in &mut input.variables {
            variable.required_clearance = Some(0.2);
        }
        for passage in &mut input.passage_certificates {
            passage.available_width = None;
            passage.enforcement =
                Some(crate::family_ir::PassageEnforcementMode::FixedWitnessPairwise);
            passage.capacity_model =
                crate::family_ir::FIXED_WITNESS_NO_SCALAR_CAPACITY_MODEL.into();
        }
        input.pair_analysis.method = crate::family_ir::FIXED_WITNESS_PAIR_ANALYSIS_METHOD.into();
        for (index, variable) in input.variables.iter_mut().enumerate() {
            variable.families[0].runs[0].witness.clearance_model =
                "exact_polyline_against_polygonal_obstacles".into();
            variable.families[0].runs[0].witness.minimum_clearance = 1.0;
            for point in &mut variable.families[0].runs[0].witness.polyline {
                point.y = index as f64;
            }
        }
        input.pair_analysis.analyzed_family_pairs = Some(3);
        input.seal_pair_analysis_fingerprints().unwrap();
        input.validate().unwrap();

        let FamilyAssignmentOutcome::Selected { selection, work } =
            select_route_families(&input, FamilyAssignmentLimits::default()).unwrap()
        else {
            panic!("three pairwise-clear fixed witnesses must be jointly selectable");
        };
        assert_eq!(selection.choices.len(), 3);
        assert!(selection.passage_usage.is_empty());
        assert_eq!(work.pruned_capacity, 0);
        assert_eq!(
            work.capacity_enforcement,
            FamilyAssignmentCapacityEnforcement::FixedWitnessPairwiseNoScalar
        );

        input.pair_analysis.analyzed_family_pairs = Some(2);
        let FamilyAssignmentError::InvalidPortfolio(issues) =
            select_route_families(&input, FamilyAssignmentLimits::default()).unwrap_err();
        assert!(
            issues
                .iter()
                .any(|issue| issue.code == "fixed_witness_pair_matrix_incomplete")
        );
    }

    #[test]
    fn v4_unary_context_conflict_prunes_option_before_search_selection() {
        let input = v4_unary_context_portfolio(2.0);
        let FamilyAssignmentOutcome::Selected { selection, work } =
            select_route_families(&input, FamilyAssignmentLimits::default()).unwrap()
        else {
            panic!("one context-clear option must remain selectable");
        };
        assert_eq!(selection.choices["branch"], "branch/family-1");
        assert_eq!(work.pruned_fixed_context, 1);
        assert_eq!(work.pruned_conflict, 0);
        assert_eq!(work.pruned_capacity, 0);
        assert!(selection.repair_obligations.is_empty());
    }

    #[test]
    fn copper_only_context_shortfall_is_selected_as_an_engine_repair_obligation() {
        // The exact required centerline distance is 0.75 mm. Keep the second
        // witness strictly inside it without intersecting the context
        // centerline so it classifies as copper-only rather than clear.
        let input = v4_unary_context_portfolio(0.70);
        assert_eq!(
            input
                .pair_analysis
                .context_conflicts
                .iter()
                .map(|conflict| conflict.kind)
                .collect::<Vec<_>>(),
            [
                FamilyConflictKind::CenterlineCrossing,
                FamilyConflictKind::CopperClearance,
            ]
        );

        let FamilyAssignmentOutcome::Selected { selection, work } =
            select_route_families(&input, FamilyAssignmentLimits::default()).unwrap()
        else {
            panic!("the topology-compatible copper shortfall must reach continuous repair");
        };
        assert_eq!(selection.choices["branch"], "branch/family-1");
        assert_eq!(work.pruned_fixed_context, 1);
        assert_eq!(selection.repair_obligations.len(), 1);
        assert!(matches!(
            &selection.repair_obligations[0],
            FamilySelectionRepairObligation::FixedContext {
                family_id,
                context_id,
                kind: FamilyConflictKind::CopperClearance,
                ..
            } if family_id == "branch/family-1" && context_id == "outside/run-0"
        ));
    }

    #[test]
    fn v4_all_context_blocked_domain_has_typed_unary_evidence() {
        let input = v4_unary_context_portfolio(0.0);
        let FamilyAssignmentOutcome::BoundedInfeasible { work, evidence } =
            select_route_families(&input, FamilyAssignmentLimits::default()).unwrap()
        else {
            panic!("both unary-forbidden options must exhaust the admitted domain");
        };
        assert_eq!(work.pruned_fixed_context, 2);
        assert_eq!(work.pruned_empty_domain, 1);
        assert_eq!(evidence.witnesses.len(), 1);
        assert_eq!(
            evidence.all_fixed_context_sha256.as_deref(),
            input
                .fixed_context
                .as_ref()
                .map(|context| context.fixed_context_fingerprint.as_str())
        );
        assert_eq!(
            evidence
                .fixed_context_dependency_sha256
                .as_ref()
                .unwrap()
                .len(),
            64
        );
        assert_eq!(
            evidence.conflict_context_runs,
            [FamilyAssignmentFixedContextRunEvidence {
                context_id: "outside/run-0".into(),
                context_branch: "outside".into(),
                context_layer: "top".into(),
                fixed_context_dependency_sha256: evidence.conflict_context_runs[0]
                    .fixed_context_dependency_sha256
                    .clone(),
            }]
        );
        assert!(evidence.witnesses[0].option_blockers.iter().all(|blocker| {
            matches!(
                blocker,
                FamilyAssignmentOptionBlocker::FixedContext {
                    option_branch,
                    option_run_id,
                    context_id,
                    context_branch,
                    context_layer,
                    fixed_context_dependency_sha256,
                    conflict_kind: FamilyConflictKind::CenterlineCrossing,
                    ..
                } if option_branch == "branch"
                    && option_run_id.ends_with("/run-0")
                    && context_id == "outside/run-0"
                    && context_branch == "outside"
                    && context_layer == "top"
                    && fixed_context_dependency_sha256
                        == &evidence.conflict_context_runs[0].fixed_context_dependency_sha256
            )
        }));
        let serialized = serde_json::to_value(&evidence).unwrap();
        assert_eq!(
            serialized["witnesses"][0]["option_blockers"][0]["option_branch"],
            "branch"
        );
        assert!(
            serialized["witnesses"][0]["option_blockers"][0]["option_run_id"]
                .as_str()
                .unwrap()
                .ends_with("/run-0")
        );
        assert_eq!(
            serialized["conflict_context_runs"][0]["context_id"],
            "outside/run-0"
        );
    }

    fn bounded_evidence(
        input: &RouteFamilyPortfolio,
    ) -> FamilyAssignmentBoundedInfeasibilityEvidence {
        let FamilyAssignmentOutcome::BoundedInfeasible { evidence, .. } =
            select_route_families(input, FamilyAssignmentLimits::default()).unwrap()
        else {
            panic!("fixture must exhaust its admitted family domain");
        };
        evidence
    }

    #[test]
    fn fixed_context_dependency_changes_with_relevant_run_geometry() {
        let original = v4_unary_context_portfolio(0.0);
        let original_evidence = bounded_evidence(&original);
        let mut changed = original.clone();
        changed.fixed_context.as_mut().unwrap().runs[0].required_width = 0.75;
        rebuild_v4_context_analysis(&mut changed);
        let changed_evidence = bounded_evidence(&changed);

        assert_ne!(
            original_evidence.all_fixed_context_sha256,
            changed_evidence.all_fixed_context_sha256
        );
        assert_ne!(
            original_evidence.fixed_context_dependency_sha256,
            changed_evidence.fixed_context_dependency_sha256
        );
        assert_ne!(
            original_evidence.conflict_context_runs[0].fixed_context_dependency_sha256,
            changed_evidence.conflict_context_runs[0].fixed_context_dependency_sha256
        );
    }

    #[test]
    fn stale_fixed_context_cannot_produce_dependency_evidence() {
        let mut stale = v4_unary_context_portfolio(0.0);
        stale.fixed_context.as_mut().unwrap().runs[0].geometry_revision = 8;
        stale.seal_pair_analysis_fingerprints().unwrap();
        let FamilyAssignmentError::InvalidPortfolio(issues) =
            select_route_families(&stale, FamilyAssignmentLimits::default()).unwrap_err();
        assert!(
            issues
                .iter()
                .any(|issue| { issue.code == "stale_fixed_context_run_geometry_revision" })
        );
    }

    #[test]
    fn unrelated_fixed_context_changes_only_the_all_context_commitment() {
        let original = v4_unary_context_portfolio(0.0);
        let original_evidence = bounded_evidence(&original);
        let mut changed = original.clone();
        changed
            .fixed_context
            .as_mut()
            .unwrap()
            .runs
            .push(FixedContextRun {
                context_id: "unrelated/run-0".into(),
                branch: "unrelated".into(),
                electrical_net: "unrelated-net".into(),
                layer: "top".into(),
                required_width: 0.5,
                required_clearance: 0.25,
                polyline: vec![
                    FamilyPoint { x: 0.0, y: 20.0 },
                    FamilyPoint { x: 1.0, y: 20.0 },
                ],
                placement_revision: 7,
                geometry_revision: 9,
            });
        rebuild_v4_context_analysis(&mut changed);
        let changed_evidence = bounded_evidence(&changed);

        assert_ne!(
            original_evidence.all_fixed_context_sha256,
            changed_evidence.all_fixed_context_sha256
        );
        assert_eq!(
            original_evidence.fixed_context_dependency_sha256,
            changed_evidence.fixed_context_dependency_sha256
        );
        assert_eq!(
            original_evidence.conflict_context_runs,
            changed_evidence.conflict_context_runs
        );
    }

    #[test]
    fn fixed_context_dependency_order_is_deterministic() {
        let mut first = v4_unary_context_portfolio(2.0);
        let original_run = &mut first.fixed_context.as_mut().unwrap().runs[0];
        original_run.context_id = "z-context/run-0".into();
        original_run.branch = "z-context".into();
        first
            .fixed_context
            .as_mut()
            .unwrap()
            .runs
            .push(FixedContextRun {
                context_id: "a-context/run-0".into(),
                branch: "a-context".into(),
                electrical_net: "a-context-net".into(),
                layer: "top".into(),
                required_width: 0.5,
                required_clearance: 0.25,
                polyline: vec![
                    FamilyPoint { x: 0.0, y: 2.0 },
                    FamilyPoint { x: 1.0, y: 2.0 },
                ],
                placement_revision: 7,
                geometry_revision: 9,
            });
        rebuild_v4_context_analysis(&mut first);
        let first_evidence = bounded_evidence(&first);

        let mut reordered = first.clone();
        reordered.fixed_context.as_mut().unwrap().runs.reverse();
        reordered.pair_analysis.context_conflicts.reverse();
        rebuild_v4_context_analysis(&mut reordered);
        let reordered_evidence = bounded_evidence(&reordered);

        assert_eq!(
            first_evidence.all_fixed_context_sha256,
            reordered_evidence.all_fixed_context_sha256
        );
        assert_eq!(
            first_evidence.fixed_context_dependency_sha256,
            reordered_evidence.fixed_context_dependency_sha256
        );
        assert_eq!(
            first_evidence.conflict_context_runs,
            reordered_evidence.conflict_context_runs
        );
        assert_eq!(first_evidence.witnesses, reordered_evidence.witnesses);
        assert_eq!(
            first_evidence
                .conflict_context_runs
                .iter()
                .map(|run| run.context_id.as_str())
                .collect::<Vec<_>>(),
            ["a-context/run-0", "z-context/run-0"]
        );
    }

    #[test]
    fn same_electrical_net_branches_remain_separate_assignment_variables() {
        let input = portfolio(
            vec![
                variable("vcc-a", "VCC", 0.4, vec![(0.0, vec!["shared".into()])]),
                variable("vcc-b", "VCC", 0.4, vec![(0.0, vec!["shared".into()])]),
            ],
            vec![("shared".into(), 0.8)],
            vec![],
        );
        let FamilyAssignmentOutcome::Selected { selection, .. } =
            select_route_families(&input, FamilyAssignmentLimits::default()).unwrap()
        else {
            panic!("both same-net branch variables must be assigned");
        };
        assert_eq!(selection.choices.len(), 2);
        assert_eq!(selection.passage_usage[0].used_width, 0.8);
    }

    #[test]
    fn same_electrical_net_conflict_edge_fails_closed() {
        let mut input = portfolio(
            vec![
                variable("vcc-a", "VCC-A", 0.2, vec![(0.0, vec![])]),
                variable("vcc-b", "VCC-B", 0.2, vec![(0.0, vec![])]),
            ],
            vec![],
            vec![("vcc-a/family-0".into(), "vcc-b/family-0".into())],
        );
        input.variables[1].electrical_net = "VCC-A".into();
        input.seal_pair_analysis_fingerprints().unwrap();
        let FamilyAssignmentError::InvalidPortfolio(issues) =
            select_route_families(&input, FamilyAssignmentLimits::default()).unwrap_err();
        assert!(
            issues
                .iter()
                .any(|issue| issue.code == "conflict_same_electrical_net")
        );
    }

    #[test]
    fn a_family_charges_a_semantic_passage_only_once_across_runs() {
        let mut repeated = family("a", 0, 0.0, 0.6, &["shared".into()]);
        repeated.runs[0].witness.polyline[1] = FamilyPoint { x: 0.5, y: 0.0 };
        let mut continuation = repeated.runs[0].clone();
        continuation.run_id = "a/family-0/run-1".into();
        continuation.run_index = 1;
        continuation.start_point = 1;
        continuation.end_point = 2;
        continuation.witness.polyline = vec![
            FamilyPoint { x: 0.5, y: 0.0 },
            FamilyPoint { x: 1.0, y: 0.0 },
        ];
        continuation.transition_from_previous = RequiredNullable::Value(RouteTransition {
            kind: RouteTransitionKind::Continuous,
            position: FamilyPoint { x: 0.5, y: 0.0 },
            certified: true,
            via: None,
        });
        repeated.runs.push(continuation);
        let input = portfolio(
            vec![
                RouteFamilyVariable {
                    branch: "a".into(),
                    electrical_net: "a".into(),
                    required_width: 0.6,
                    required_clearance: None,
                    families: vec![repeated],
                },
                net("b", 0.4, vec![(0.0, vec!["shared".into()])]),
            ],
            vec![("shared".into(), 1.0)],
            vec![],
        );
        let FamilyAssignmentOutcome::Selected { selection, .. } =
            select_route_families(&input, FamilyAssignmentLimits::default()).unwrap()
        else {
            panic!("the repeated passage belongs to one physical width demand");
        };
        assert_eq!(selection.passage_usage[0].used_width, 1.0);
        assert_eq!(selection.passage_usage[0].remaining_width, 0.0);
    }

    #[test]
    fn node_exhaustion_with_an_incumbent_is_still_budget_exhausted() {
        let input = portfolio(
            vec![
                net("a", 0.2, vec![(0.0, vec![]), (1.0, vec![])]),
                net("b", 0.2, vec![(0.0, vec![]), (1.0, vec![])]),
                net("c", 0.2, vec![(0.0, vec![]), (1.0, vec![])]),
            ],
            vec![],
            vec![],
        );
        let outcome =
            select_route_families(&input, FamilyAssignmentLimits { max_nodes: 5 }).unwrap();
        let FamilyAssignmentOutcome::BudgetExhausted {
            best_incumbent_cost,
            ..
        } = outcome
        else {
            panic!("an interrupted proof must not return its incumbent as selected");
        };
        assert_eq!(best_incumbent_cost, Some(0.0));
    }

    #[test]
    fn invalid_typed_portfolio_fails_closed_before_search() {
        let mut input = portfolio(vec![net("a", 0.2, vec![(0.0, vec![])])], vec![], vec![]);
        input.pair_analysis.search_complete = false;
        let error = select_route_families(&input, FamilyAssignmentLimits::default()).unwrap_err();
        assert!(matches!(error, FamilyAssignmentError::InvalidPortfolio(_)));
    }

    #[test]
    fn input_reordering_preserves_the_deterministic_minimum() {
        let mut first = portfolio(
            vec![
                net("a", 0.2, vec![(1.0, vec![]), (1.0, vec![])]),
                net("b", 0.2, vec![(1.0, vec![]), (1.0, vec![])]),
            ],
            vec![],
            vec![],
        );
        let expected = select_route_families(&first, FamilyAssignmentLimits::default()).unwrap();
        first.variables.reverse();
        for variable in &mut first.variables {
            variable.families.reverse();
        }
        first.seal_pair_analysis_fingerprints().unwrap();
        let reordered = select_route_families(&first, FamilyAssignmentLimits::default()).unwrap();
        assert_eq!(expected, reordered);
        let FamilyAssignmentOutcome::Selected { selection, .. } = reordered else {
            panic!("tie fixture must be selected");
        };
        assert_eq!(selection.choices["a"], "a/family-0");
        assert_eq!(selection.choices["b"], "b/family-0");
    }

    #[test]
    fn sparse_twenty_by_four_v3_fixture_stays_bounded_and_optimal() {
        let mut nets = Vec::new();
        let mut capacities = Vec::new();
        for index in 0..20 {
            let id = format!("net-{index:02}");
            let group = index / 4;
            let main = format!("main-{group}");
            let bypass = format!("bypass-{group}");
            nets.push(net(
                &id,
                1.0,
                vec![
                    (0.0, vec![main.clone()]),
                    (1.0, vec![bypass.clone()]),
                    (2.25, vec![]),
                    (4.0, vec![]),
                ],
            ));
            if index % 4 == 0 {
                capacities.push((main, 2.0));
                capacities.push((bypass, 2.0));
            }
        }
        let mut conflicts = Vec::new();
        for index in 0..19 {
            for family in 0..=1 {
                conflicts.push((
                    format!("net-{index:02}/family-{family}"),
                    format!("net-{:02}/family-{family}", index + 1),
                ));
            }
        }
        let input = portfolio(nets, capacities, conflicts);
        let FamilyAssignmentOutcome::Selected { selection, work } =
            select_route_families(&input, FamilyAssignmentLimits::default()).unwrap()
        else {
            panic!("V3 scale fixture must be selected");
        };
        assert_eq!(selection.choices.len(), 20);
        assert_eq!(selection.total_cost, 10.0);
        assert!(work.nodes < 100_000, "{work:?}");
    }
}
