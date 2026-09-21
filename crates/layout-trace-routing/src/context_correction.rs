// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Exact, bounded fixed-context correction for an admitted route-family portfolio.
//!
//! This pass does not select routes for application. It asks a narrower
//! question: if fixed-context branches were promoted into the next joint
//! route-family problem, what smallest branch set could make the *current*
//! admitted family portfolio hard-feasible? The coordinator must regenerate
//! that larger portfolio and run normal assignment before changing a route.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::family_ir::{
    MULTI_RUN_ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION, ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION,
    RouteFamilyFeasibilityMode, RouteFamilyPortfolio, ValidationIssue,
};

pub const CONTEXT_CORRECTION_EVIDENCE_CONTRACT: &str =
    "layout-trace.route-family-context-correction/v1";

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContextCorrectionLimits {
    pub max_nodes: u64,
    pub max_promoted_branches: usize,
}

impl Default for ContextCorrectionLimits {
    fn default() -> Self {
        Self {
            max_nodes: 50_000,
            max_promoted_branches: 2,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ContextCorrectionWork {
    pub nodes: u64,
    pub branches: u64,
    pub pruned_pair_conflict: u64,
    pub pruned_ineligible_context: u64,
    pub pruned_promotion_cap: u64,
    pub pruned_bound: u64,
    pub empty_domain_events: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FamilyContextBlockersEvidence {
    pub family_id: String,
    /// Complete, branch-grouped blocker set for this admitted family.
    pub context_branches: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UnaryBlockedDomainEvidence {
    pub branch: String,
    pub families: Vec<FamilyContextBlockersEvidence>,
    /// Branches present in every admitted option's complete blocker set.
    /// This is a mandatory core, not in general a sufficient correction.
    pub mandatory_context_core: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContextCorrectionSelection {
    /// Exact sufficient correction for the admitted portfolio under the
    /// supplied eligibility and cardinality bounds.
    pub promoted_context_branches: Vec<String>,
    /// Diagnostic witness only. These choices are invalidated by promotion
    /// and must never be applied without regenerating the enlarged portfolio.
    pub provisional_family_choices: BTreeMap<String, String>,
    pub provisional_route_cost: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum ContextCorrectionOutcome {
    Selected {
        selection: ContextCorrectionSelection,
        evidence: ContextCorrectionEvidence,
        work: ContextCorrectionWork,
    },
    NoCorrectionWithinBound {
        evidence: ContextCorrectionEvidence,
        work: ContextCorrectionWork,
    },
    NoPromotionNeeded {
        evidence: ContextCorrectionEvidence,
        work: ContextCorrectionWork,
    },
    BudgetExhausted {
        evidence: ContextCorrectionEvidence,
        work: ContextCorrectionWork,
        best_incumbent: Option<ContextCorrectionSelection>,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContextCorrectionEvidence {
    pub contract: String,
    pub candidate_set_fingerprint: String,
    pub fixed_context_fingerprint: String,
    pub eligible_context_branches: Vec<String>,
    pub max_promoted_branches: usize,
    pub exhaustive_within_admitted_portfolio: bool,
    pub unary_blocked_domains: Vec<UnaryBlockedDomainEvidence>,
    /// Commits to the inputs and, for a selected outcome, its exact correction.
    pub canonical_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContextCorrectionError {
    InvalidPortfolio(Vec<ValidationIssue>),
    UnsupportedPortfolio(String),
}

impl fmt::Display for ContextCorrectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPortfolio(issues) => write!(
                formatter,
                "context correction rejected {} portfolio validation issue{}",
                issues.len(),
                if issues.len() == 1 { "" } else { "s" }
            ),
            Self::UnsupportedPortfolio(detail) => formatter.write_str(detail),
        }
    }
}

impl Error for ContextCorrectionError {}

#[derive(Clone)]
struct CorrectionOption {
    family_id: String,
    cost: f64,
    required_promotions: BTreeSet<String>,
}

#[derive(Clone)]
struct Incumbent {
    promotions: BTreeSet<String>,
    choices: BTreeMap<String, String>,
    cost: f64,
}

impl Incumbent {
    fn objective_cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.promotions
            .len()
            .cmp(&other.promotions.len())
            .then_with(|| self.promotions.iter().cmp(other.promotions.iter()))
            .then_with(|| self.cost.total_cmp(&other.cost))
            .then_with(|| self.choices.iter().cmp(other.choices.iter()))
    }

    fn selection(&self) -> ContextCorrectionSelection {
        ContextCorrectionSelection {
            promoted_context_branches: self.promotions.iter().cloned().collect(),
            provisional_family_choices: self.choices.clone(),
            provisional_route_cost: self.cost,
        }
    }
}

struct CorrectionSearch {
    domains: BTreeMap<String, Vec<CorrectionOption>>,
    adjacency: BTreeMap<String, BTreeSet<String>>,
    eligible: BTreeSet<String>,
    limits: ContextCorrectionLimits,
    work: ContextCorrectionWork,
    exhausted: bool,
    best: Option<Incumbent>,
}

impl CorrectionSearch {
    fn visit(
        &mut self,
        remaining: &[String],
        promotions: &BTreeSet<String>,
        choices: &mut BTreeMap<String, String>,
        chosen_families: &mut BTreeSet<String>,
        cost: f64,
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
            let candidate = Incumbent {
                promotions: promotions.clone(),
                choices: choices.clone(),
                cost,
            };
            if self
                .best
                .as_ref()
                .is_none_or(|best| candidate.objective_cmp(best).is_lt())
            {
                self.best = Some(candidate);
            }
            return;
        }
        if self
            .best
            .as_ref()
            .is_some_and(|best| promotions.len() > best.promotions.len())
        {
            self.work.pruned_bound += 1;
            return;
        }

        // Minimum remaining values, then stable branch identity. Domains are
        // recomputed because pair choices and the promotion union are stateful.
        let mut feasible = BTreeMap::<String, Vec<usize>>::new();
        for branch in remaining {
            let mut indices = Vec::new();
            for (index, option) in self.domains[branch].iter().enumerate() {
                if option
                    .required_promotions
                    .iter()
                    .any(|required| !self.eligible.contains(required))
                {
                    self.work.pruned_ineligible_context += 1;
                    continue;
                }
                let promoted_count = promotions.union(&option.required_promotions).count();
                if promoted_count > self.limits.max_promoted_branches {
                    self.work.pruned_promotion_cap += 1;
                    continue;
                }
                if self
                    .adjacency
                    .get(&option.family_id)
                    .is_some_and(|neighbors| {
                        neighbors
                            .iter()
                            .any(|family| chosen_families.contains(family))
                    })
                {
                    self.work.pruned_pair_conflict += 1;
                    continue;
                }
                indices.push(index);
            }
            if indices.is_empty() {
                self.work.empty_domain_events += 1;
                return;
            }
            feasible.insert(branch.clone(), indices);
        }
        let branch = remaining
            .iter()
            .min_by(|left, right| {
                feasible[*left]
                    .len()
                    .cmp(&feasible[*right].len())
                    .then_with(|| left.cmp(right))
            })
            .expect("nonempty remaining variables have a minimum")
            .clone();
        let next_remaining = remaining
            .iter()
            .filter(|other| **other != branch)
            .cloned()
            .collect::<Vec<_>>();
        let domain = feasible.remove(&branch).unwrap();
        for option_index in domain {
            let option = self.domains[&branch][option_index].clone();
            let next_promotions = promotions
                .union(&option.required_promotions)
                .cloned()
                .collect::<BTreeSet<_>>();
            self.work.branches += 1;
            choices.insert(branch.clone(), option.family_id.clone());
            chosen_families.insert(option.family_id.clone());
            self.visit(
                &next_remaining,
                &next_promotions,
                choices,
                chosen_families,
                cost + option.cost,
            );
            chosen_families.remove(&option.family_id);
            choices.remove(&branch);
            if self.exhausted {
                return;
            }
        }
    }
}

/// Compute an exact minimum fixed-context correction for this finite admitted
/// portfolio. Pair conflicts remain hard. A context conflict is relaxed only
/// by adding its owning branch to the proposed promotion set.
pub fn select_context_correction(
    portfolio: &RouteFamilyPortfolio,
    eligible_context_branches: &BTreeSet<String>,
    limits: ContextCorrectionLimits,
) -> Result<ContextCorrectionOutcome, ContextCorrectionError> {
    if let Err(issues) = portfolio.validate() {
        return Err(ContextCorrectionError::InvalidPortfolio(issues));
    }
    if (portfolio.schema_version != ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION
        && portfolio.schema_version != MULTI_RUN_ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION)
        || portfolio.effective_feasibility_mode()
            != RouteFamilyFeasibilityMode::FixedFullWidthWitnessPairwise
    {
        return Err(ContextCorrectionError::UnsupportedPortfolio(
            "context correction requires a validated V4/V5 fixed-context pairwise portfolio".into(),
        ));
    }
    let context = portfolio.fixed_context.as_ref().ok_or_else(|| {
        ContextCorrectionError::UnsupportedPortfolio(
            "context correction requires complete fixed-context evidence".into(),
        )
    })?;
    if !context.search_complete || !portfolio.pair_analysis.search_complete {
        return Err(ContextCorrectionError::UnsupportedPortfolio(
            "context correction refuses incomplete fixed-context or pair evidence".into(),
        ));
    }
    let context_owners = context
        .runs
        .iter()
        .map(|run| (run.context_id.as_str(), run.branch.as_str()))
        .collect::<BTreeMap<_, _>>();
    let mut blockers = BTreeMap::<String, BTreeSet<String>>::new();
    for conflict in &portfolio.pair_analysis.context_conflicts {
        if !conflict.kind.requires_discrete_separation() {
            continue;
        }
        blockers
            .entry(conflict.family_id.clone())
            .or_default()
            .insert(context_owners[conflict.context_id.as_str()].to_owned());
    }

    let mut domains = BTreeMap::<String, Vec<CorrectionOption>>::new();
    let mut unary_blocked_domains = Vec::new();
    for variable in &portfolio.variables {
        let mut options = Vec::new();
        let mut family_evidence = Vec::new();
        let mut mandatory_core = None::<BTreeSet<String>>;
        let mut fully_unary_blocked = true;
        for family in &variable.families {
            let required_promotions = blockers.get(&family.family_id).cloned().unwrap_or_default();
            if required_promotions.is_empty() {
                fully_unary_blocked = false;
            }
            mandatory_core = Some(match mandatory_core {
                None => required_promotions.clone(),
                Some(core) => core.intersection(&required_promotions).cloned().collect(),
            });
            family_evidence.push(FamilyContextBlockersEvidence {
                family_id: family.family_id.clone(),
                context_branches: required_promotions.iter().cloned().collect(),
            });
            options.push(CorrectionOption {
                family_id: family.family_id.clone(),
                cost: family.cost,
                required_promotions,
            });
        }
        options.sort_by(|left, right| {
            left.required_promotions
                .len()
                .cmp(&right.required_promotions.len())
                .then_with(|| {
                    left.required_promotions
                        .iter()
                        .cmp(right.required_promotions.iter())
                })
                .then_with(|| left.cost.total_cmp(&right.cost))
                .then_with(|| left.family_id.cmp(&right.family_id))
        });
        family_evidence.sort_by(|left, right| left.family_id.cmp(&right.family_id));
        if fully_unary_blocked {
            unary_blocked_domains.push(UnaryBlockedDomainEvidence {
                branch: variable.branch.clone(),
                families: family_evidence,
                mandatory_context_core: mandatory_core.unwrap_or_default().into_iter().collect(),
            });
        }
        domains.insert(variable.branch.clone(), options);
    }
    unary_blocked_domains.sort_by(|left, right| left.branch.cmp(&right.branch));

    let mut adjacency = BTreeMap::<String, BTreeSet<String>>::new();
    for conflict in &portfolio.pair_analysis.conflicts {
        if !conflict.kind.requires_discrete_separation() {
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
    let remaining = domains.keys().cloned().collect::<Vec<_>>();
    let mut search = CorrectionSearch {
        domains,
        adjacency,
        eligible: eligible_context_branches.clone(),
        limits,
        work: ContextCorrectionWork::default(),
        exhausted: false,
        best: None,
    };
    search.visit(
        &remaining,
        &BTreeSet::new(),
        &mut BTreeMap::new(),
        &mut BTreeSet::new(),
        0.0,
    );
    let selected = search.best.as_ref().map(Incumbent::selection);
    let evidence = correction_evidence(
        portfolio,
        eligible_context_branches,
        limits,
        unary_blocked_domains,
        !search.exhausted,
        selected.as_ref(),
    );
    if search.exhausted {
        return Ok(ContextCorrectionOutcome::BudgetExhausted {
            evidence,
            work: search.work,
            best_incumbent: selected,
        });
    }
    match selected {
        None => Ok(ContextCorrectionOutcome::NoCorrectionWithinBound {
            evidence,
            work: search.work,
        }),
        Some(selection) if selection.promoted_context_branches.is_empty() => {
            Ok(ContextCorrectionOutcome::NoPromotionNeeded {
                evidence,
                work: search.work,
            })
        }
        Some(selection) => Ok(ContextCorrectionOutcome::Selected {
            selection,
            evidence,
            work: search.work,
        }),
    }
}

fn correction_evidence(
    portfolio: &RouteFamilyPortfolio,
    eligible: &BTreeSet<String>,
    limits: ContextCorrectionLimits,
    unary_blocked_domains: Vec<UnaryBlockedDomainEvidence>,
    exhaustive: bool,
    selection: Option<&ContextCorrectionSelection>,
) -> ContextCorrectionEvidence {
    let fixed_context_fingerprint = portfolio
        .pair_analysis
        .fixed_context_fingerprint
        .clone()
        .expect("validated V4 has a fixed-context fingerprint");
    let eligible_context_branches = eligible.iter().cloned().collect::<Vec<_>>();
    let mut hash = Sha256::new();
    hash_field(&mut hash, CONTEXT_CORRECTION_EVIDENCE_CONTRACT.as_bytes());
    hash_field(
        &mut hash,
        portfolio.pair_analysis.candidate_set_fingerprint.as_bytes(),
    );
    hash_field(&mut hash, fixed_context_fingerprint.as_bytes());
    hash.update((limits.max_promoted_branches as u64).to_le_bytes());
    hash.update(limits.max_nodes.to_le_bytes());
    hash.update([u8::from(exhaustive)]);
    hash_serialized(&mut hash, &eligible_context_branches);
    hash_serialized(&mut hash, &unary_blocked_domains);
    hash_serialized(&mut hash, &selection);
    ContextCorrectionEvidence {
        contract: CONTEXT_CORRECTION_EVIDENCE_CONTRACT.into(),
        candidate_set_fingerprint: portfolio.pair_analysis.candidate_set_fingerprint.clone(),
        fixed_context_fingerprint,
        eligible_context_branches,
        max_promoted_branches: limits.max_promoted_branches,
        exhaustive_within_admitted_portfolio: exhaustive,
        unary_blocked_domains,
        canonical_sha256: encode_sha256(hash.finalize()),
    }
}

fn hash_field(hash: &mut Sha256, value: &[u8]) {
    hash.update((value.len() as u64).to_le_bytes());
    hash.update(value);
}

fn hash_serialized(hash: &mut Sha256, value: &impl Serialize) {
    let encoded = serde_json::to_vec(value).expect("typed correction evidence serializes");
    hash_field(hash, &encoded);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn option(id: &str, cost: f64, required: &[&str]) -> CorrectionOption {
        CorrectionOption {
            family_id: id.into(),
            cost,
            required_promotions: required.iter().map(|branch| (*branch).into()).collect(),
        }
    }

    fn solve(
        domain_rows: &[(&str, Vec<CorrectionOption>)],
        conflicts: &[(&str, &str)],
        eligible: &[&str],
        limits: ContextCorrectionLimits,
    ) -> (
        Option<ContextCorrectionSelection>,
        ContextCorrectionWork,
        bool,
    ) {
        let domains = domain_rows
            .iter()
            .map(|(branch, options)| ((*branch).into(), options.clone()))
            .collect::<BTreeMap<_, _>>();
        let mut adjacency = BTreeMap::<String, BTreeSet<String>>::new();
        for (left, right) in conflicts {
            adjacency
                .entry((*left).into())
                .or_default()
                .insert((*right).into());
            adjacency
                .entry((*right).into())
                .or_default()
                .insert((*left).into());
        }
        let remaining = domains.keys().cloned().collect::<Vec<_>>();
        let mut search = CorrectionSearch {
            domains,
            adjacency,
            eligible: eligible.iter().map(|branch| (*branch).into()).collect(),
            limits,
            work: ContextCorrectionWork::default(),
            exhausted: false,
            best: None,
        };
        search.visit(
            &remaining,
            &BTreeSet::new(),
            &mut BTreeMap::new(),
            &mut BTreeSet::new(),
            0.0,
        );
        (
            search.best.as_ref().map(Incumbent::selection),
            search.work,
            search.exhausted,
        )
    }

    #[test]
    fn exact_multi_domain_correction_minimizes_promotion_union() {
        let (selection, _, exhausted) = solve(
            &[
                (
                    "a",
                    vec![option("a/x", 0.0, &["x"]), option("a/y", 5.0, &["y"])],
                ),
                (
                    "b",
                    vec![option("b/y", 0.0, &["y"]), option("b/z", 0.0, &["z"])],
                ),
            ],
            &[],
            &["x", "y", "z"],
            ContextCorrectionLimits::default(),
        );
        assert!(!exhausted);
        let selection = selection.unwrap();
        assert_eq!(selection.promoted_context_branches, ["y"]);
        assert_eq!(selection.provisional_family_choices["a"], "a/y");
        assert_eq!(selection.provisional_family_choices["b"], "b/y");
    }

    #[test]
    fn common_hitter_is_not_mislabeled_as_a_sufficient_correction() {
        let (selection, work, exhausted) = solve(
            &[(
                "a",
                vec![
                    option("a/xy", 0.0, &["x", "y"]),
                    option("a/xz", 0.0, &["x", "z"]),
                ],
            )],
            &[],
            &["x", "y", "z"],
            ContextCorrectionLimits {
                max_nodes: 100,
                max_promoted_branches: 1,
            },
        );
        assert!(!exhausted);
        assert!(selection.is_none());
        assert_eq!(work.pruned_promotion_cap, 2);
    }

    #[test]
    fn pair_conflicts_remain_hard_while_context_is_relaxed() {
        let (selection, _, exhausted) = solve(
            &[
                (
                    "a",
                    vec![option("a/x", 0.0, &["x"]), option("a/y", 0.0, &["y"])],
                ),
                ("b", vec![option("b/0", 0.0, &[]), option("b/1", 1.0, &[])]),
            ],
            &[("a/x", "b/0"), ("a/y", "b/1")],
            &["x", "y"],
            ContextCorrectionLimits::default(),
        );
        assert!(!exhausted);
        let selection = selection.unwrap();
        assert_eq!(selection.promoted_context_branches, ["x"]);
        assert_eq!(selection.provisional_family_choices["b"], "b/1");
    }

    #[test]
    fn ineligible_context_branch_cannot_be_promoted() {
        let (selection, work, exhausted) = solve(
            &[("a", vec![option("a/x", 0.0, &["x"])])],
            &[],
            &[],
            ContextCorrectionLimits::default(),
        );
        assert!(!exhausted);
        assert!(selection.is_none());
        assert_eq!(work.pruned_ineligible_context, 1);
    }

    #[test]
    fn node_exhaustion_never_returns_an_exact_selection() {
        let (incumbent, _, exhausted) = solve(
            &[("a", vec![option("a/x", 0.0, &["x"])])],
            &[],
            &["x"],
            ContextCorrectionLimits {
                max_nodes: 0,
                max_promoted_branches: 2,
            },
        );
        assert!(exhausted);
        assert!(incumbent.is_none());
    }

    #[test]
    fn input_permutations_have_the_same_exact_correction() {
        let forward = solve(
            &[
                (
                    "a",
                    vec![option("a/y", 2.0, &["y"]), option("a/x", 1.0, &["x"])],
                ),
                ("b", vec![option("b/x", 0.0, &["x"])]),
            ],
            &[],
            &["y", "x"],
            ContextCorrectionLimits::default(),
        )
        .0
        .unwrap();
        let reverse = solve(
            &[
                ("b", vec![option("b/x", 0.0, &["x"])]),
                (
                    "a",
                    vec![option("a/x", 1.0, &["x"]), option("a/y", 2.0, &["y"])],
                ),
            ],
            &[],
            &["x", "y"],
            ContextCorrectionLimits::default(),
        )
        .0
        .unwrap();
        assert_eq!(forward, reverse);
        assert_eq!(forward.promoted_context_branches, ["x"]);
    }
}
