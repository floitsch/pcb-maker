// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Immutable-parent transaction for adding one legacy two-terminal branch.
//!
//! Local trials reserve all certified parent geometry. The bounded global
//! fallback is allowed to reroute and move legal geometry through the pressure
//! coordinator. Only an independently exact-complete target candidate commits;
//! otherwise the exact parent is returned byte-for-byte.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use layout_trace_model::{Problem, model::Net};
use pcb_routing::{
    BranchRoutingStatus, DutGridRoutingConfig, DutGridRoutingResult,
    add_problem_branches_with_dut_grid_at_poses,
};
use pcb_validate::{CandidateArtifact, ExactValidationAssessment, validate_candidate};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{PressureRepairConfig, repair_routing_with_pressure};

pub const CONNECTION_INSERTION_CONTRACT: &str = "pcb-maker.connection-insertion-transaction/v1";
const RESULT_SCHEMA_VERSION: u32 = 1;
const PROBLEM_HASH_CONTRACT: &str = "pcb-maker.connection-insertion-problem/v1";
const NET_HASH_CONTRACT: &str = "pcb-maker.connection-insertion-net/v1";
const CONFIG_HASH_CONTRACT: &str = "pcb-maker.connection-insertion-config/v1";
const CANDIDATE_HASH_CONTRACT: &str = "pcb-maker.connection-insertion-candidate/v1";
const EVIDENCE_HASH_CONTRACT: &str = "pcb-maker.connection-insertion-evidence/v1";

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionInsertionConfig {
    /// Maximum number of local policies inspected from the ordered portfolio.
    pub local_candidate_cap: usize,
    /// Independent local A* policies. Every trial starts from the exact parent.
    pub local_routing_portfolio: Vec<DutGridRoutingConfig>,
    /// Router used by the unrestricted coupled fallback.
    pub global_routing: DutGridRoutingConfig,
    /// Bounded router-to-placer pressure policy used by the fallback.
    pub global_pressure: PressureRepairConfig,
}

impl ConnectionInsertionConfig {
    pub fn check(&self) -> Result<(), ConnectionInsertionRejection> {
        if self.local_candidate_cap == 0 {
            return Err(reject(
                ConnectionInsertionRejectionCode::InvalidConfig,
                "$.config.local_candidate_cap",
                "local_candidate_cap must be positive",
            ));
        }
        if self.local_routing_portfolio.is_empty() {
            return Err(reject(
                ConnectionInsertionRejectionCode::InvalidConfig,
                "$.config.local_routing_portfolio",
                "at least one local routing policy is required",
            ));
        }
        for (index, routing) in self.local_routing_portfolio.iter().enumerate() {
            routing.check().map_err(|detail| {
                reject(
                    ConnectionInsertionRejectionCode::InvalidConfig,
                    format!("$.config.local_routing_portfolio[{index}]"),
                    detail,
                )
            })?;
        }
        self.global_routing.check().map_err(|detail| {
            reject(
                ConnectionInsertionRejectionCode::InvalidConfig,
                "$.config.global_routing",
                detail,
            )
        })?;
        self.global_pressure.check().map_err(|detail| {
            reject(
                ConnectionInsertionRejectionCode::InvalidConfig,
                "$.config.global_pressure",
                detail,
            )
        })
    }
}

impl Default for ConnectionInsertionConfig {
    fn default() -> Self {
        let base = DutGridRoutingConfig::default();
        Self {
            local_candidate_cap: 1,
            local_routing_portfolio: vec![base.clone()],
            global_routing: base,
            global_pressure: PressureRepairConfig::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionInsertionDisposition {
    Committed,
    RolledBack,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionInsertionAttemptScope {
    LocalInsertedBranchOnly,
    GlobalAllLegalGeometry,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionInsertionAttemptStatus {
    Committed,
    NoCandidate,
    BudgetExhausted,
    WholeTargetCertificationFailed,
    ExecutionFailed,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ConnectionInsertionAttemptWork {
    pub candidates_evaluated: u64,
    pub route_searches: u64,
    pub route_search_states: u64,
    pub coupled_generations: u64,
    pub exact_validation_runs: u64,
    pub connectivity_assessments: u64,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ConnectionInsertionRepairTelemetry {
    pub conflict_objects: Vec<String>,
    pub repair_scope_branches: Vec<String>,
    pub changed_branches: Vec<String>,
    pub changed_via_branches: Vec<String>,
    pub moved_components: Vec<String>,
}

impl ConnectionInsertionRepairTelemetry {
    fn canonicalize(&mut self) {
        for values in [
            &mut self.conflict_objects,
            &mut self.repair_scope_branches,
            &mut self.changed_branches,
            &mut self.changed_via_branches,
            &mut self.moved_components,
        ] {
            values.sort();
            values.dedup();
        }
    }

    fn is_canonical(&self) -> bool {
        [
            &self.conflict_objects,
            &self.repair_scope_branches,
            &self.changed_branches,
            &self.changed_via_branches,
            &self.moved_components,
        ]
        .into_iter()
        .all(|values| values.windows(2).all(|pair| pair[0] < pair[1]))
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionInsertionCertification {
    pub exhaustive_full_width_validation: bool,
    pub exact_violation_count: usize,
    pub electrical_connectivity_complete: bool,
    pub connectivity_finding_count: usize,
    pub strict_final_admission: bool,
    pub final_admission_blocker_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exact_child_sha256: Option<String>,
}

impl ConnectionInsertionCertification {
    pub fn complete(&self) -> bool {
        self.exhaustive_full_width_validation
            && self.exact_violation_count == 0
            && self.electrical_connectivity_complete
            && self.connectivity_finding_count == 0
            && self.strict_final_admission
            && self.final_admission_blocker_count == 0
            && self.exact_child_sha256.as_deref().is_some_and(is_sha256)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionInsertionLocalTrialEvidence {
    pub ordinal: usize,
    pub routing_config_sha256: String,
    pub status: ConnectionInsertionAttemptStatus,
    pub route_searches: u64,
    pub route_search_states: u64,
    pub exact_violation_count: Option<usize>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionInsertionAttemptEvidence {
    pub ordinal: usize,
    pub scope: ConnectionInsertionAttemptScope,
    pub status: ConnectionInsertionAttemptStatus,
    pub work: ConnectionInsertionAttemptWork,
    pub telemetry: ConnectionInsertionRepairTelemetry,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub local_trials: Vec<ConnectionInsertionLocalTrialEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub whole_target_certification: Option<ConnectionInsertionCertification>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyNetExtensionEvidence {
    pub branch: String,
    pub electrical_net: String,
    pub from_component: String,
    pub from_pin: String,
    pub to_component: String,
    pub to_pin: String,
    pub net_intent_sha256: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionInsertionEvidence {
    pub contract: String,
    pub disposition: ConnectionInsertionDisposition,
    pub config_sha256: String,
    pub source_problem_intent_sha256: String,
    pub target_problem_intent_sha256: String,
    pub parent_exact_sha256: String,
    pub extension: LegacyNetExtensionEvidence,
    pub attempts: Vec<ConnectionInsertionAttemptEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub committed_child_exact_sha256: Option<String>,
    pub canonical_sha256: String,
}

impl ConnectionInsertionEvidence {
    fn seal(mut self) -> Result<Self, ConnectionInsertionRejection> {
        self.contract = CONNECTION_INSERTION_CONTRACT.into();
        for attempt in &mut self.attempts {
            attempt.telemetry.canonicalize();
        }
        self.canonical_sha256.clear();
        self.check_state_machine()?;
        self.canonical_sha256 = canonical_hash(EVIDENCE_HASH_CONTRACT, &self);
        Ok(self)
    }

    pub fn check(&self) -> Result<(), ConnectionInsertionRejection> {
        self.check_state_machine()?;
        if !is_sha256(&self.canonical_sha256) {
            return Err(invalid_evidence("canonical_sha256 is not a sha256 value"));
        }
        let mut unhashed = self.clone();
        unhashed.canonical_sha256.clear();
        if canonical_hash(EVIDENCE_HASH_CONTRACT, &unhashed) != self.canonical_sha256 {
            return Err(invalid_evidence(
                "canonical_sha256 does not cover this evidence",
            ));
        }
        Ok(())
    }

    pub fn check_for_config(
        &self,
        config: &ConnectionInsertionConfig,
    ) -> Result<(), ConnectionInsertionRejection> {
        config.check()?;
        self.check()?;
        if self.config_sha256 != canonical_hash(CONFIG_HASH_CONTRACT, config) {
            return Err(invalid_evidence(
                "config_sha256 is bound to a different insertion config",
            ));
        }
        Ok(())
    }

    fn check_state_machine(&self) -> Result<(), ConnectionInsertionRejection> {
        if self.contract != CONNECTION_INSERTION_CONTRACT {
            return Err(invalid_evidence("unsupported evidence contract"));
        }
        for hash in [
            &self.config_sha256,
            &self.source_problem_intent_sha256,
            &self.target_problem_intent_sha256,
            &self.parent_exact_sha256,
            &self.extension.net_intent_sha256,
        ] {
            if !is_sha256(hash) {
                return Err(invalid_evidence("invalid provenance sha256 value"));
            }
        }
        if self.attempts.is_empty() || self.attempts.len() > 2 {
            return Err(invalid_evidence(
                "expected local attempt and at most one global attempt",
            ));
        }
        for (ordinal, attempt) in self.attempts.iter().enumerate() {
            let expected = if ordinal == 0 {
                ConnectionInsertionAttemptScope::LocalInsertedBranchOnly
            } else {
                ConnectionInsertionAttemptScope::GlobalAllLegalGeometry
            };
            if attempt.ordinal != ordinal || attempt.scope != expected {
                return Err(invalid_evidence("attempt order or scope is invalid"));
            }
            if !attempt.telemetry.is_canonical() {
                return Err(invalid_evidence("telemetry IDs must be sorted and unique"));
            }
            let certified = attempt
                .whole_target_certification
                .as_ref()
                .is_some_and(ConnectionInsertionCertification::complete);
            if certified != (attempt.status == ConnectionInsertionAttemptStatus::Committed) {
                return Err(invalid_evidence(
                    "committed attempt lacks complete certification",
                ));
            }
        }
        let committed = self
            .attempts
            .iter()
            .filter(|attempt| attempt.status == ConnectionInsertionAttemptStatus::Committed)
            .collect::<Vec<_>>();
        match self.disposition {
            ConnectionInsertionDisposition::Committed => {
                if committed.len() != 1 || self.attempts.last() != committed.first().copied() {
                    return Err(invalid_evidence(
                        "commit must be the final and only committed attempt",
                    ));
                }
                let child = committed[0]
                    .whole_target_certification
                    .as_ref()
                    .and_then(|certification| certification.exact_child_sha256.as_ref());
                if child != self.committed_child_exact_sha256.as_ref() {
                    return Err(invalid_evidence("committed child hashes disagree"));
                }
            }
            ConnectionInsertionDisposition::RolledBack => {
                if self.attempts.len() != 2
                    || !committed.is_empty()
                    || self.committed_child_exact_sha256.is_some()
                {
                    return Err(invalid_evidence("rollback requires two failed attempts"));
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ConnectionInsertionResult {
    pub schema_version: u32,
    pub disposition: ConnectionInsertionDisposition,
    /// Exact child on commit; exact unchanged parent on rollback.
    pub candidate: CandidateArtifact,
    pub evidence: ConnectionInsertionEvidence,
}

impl ConnectionInsertionResult {
    pub fn complete(&self) -> bool {
        self.disposition == ConnectionInsertionDisposition::Committed
    }

    /// Recheck an untrusted serialized result against all caller-held inputs.
    pub fn check(
        &self,
        source: &Problem,
        target: &Problem,
        parent: &CandidateArtifact,
        config: &ConnectionInsertionConfig,
    ) -> Result<(), ConnectionInsertionRejection> {
        if self.schema_version != RESULT_SCHEMA_VERSION {
            return Err(invalid_evidence("unsupported result schema version"));
        }
        self.evidence.check_for_config(config)?;
        if self.disposition != self.evidence.disposition {
            return Err(invalid_evidence(
                "result disposition differs from its evidence",
            ));
        }
        let extension = validate_one_legacy_net_extension(source, target)?;
        if self.evidence.source_problem_intent_sha256 != extension.source_problem_intent_sha256
            || self.evidence.target_problem_intent_sha256 != extension.target_problem_intent_sha256
            || self.evidence.extension != extension.evidence
        {
            return Err(invalid_evidence(
                "transaction evidence is bound to different source or target intent",
            ));
        }
        if self.evidence.parent_exact_sha256 != candidate_sha256(parent) {
            return Err(invalid_evidence(
                "transaction evidence is bound to a different exact parent",
            ));
        }
        match self.disposition {
            ConnectionInsertionDisposition::Committed => {
                let validation = validate_candidate(target, &self.candidate).map_err(|detail| {
                    invalid_evidence(format!("committed child cannot be validated: {detail}"))
                })?;
                let child_hash = candidate_sha256(&self.candidate);
                if !validation.complete
                    || self.evidence.committed_child_exact_sha256.as_deref()
                        != Some(child_hash.as_str())
                {
                    return Err(invalid_evidence(
                        "committed child is incomplete or has the wrong fingerprint",
                    ));
                }
            }
            ConnectionInsertionDisposition::RolledBack => {
                if &self.candidate != parent {
                    return Err(invalid_evidence(
                        "rolled-back result does not contain the exact parent",
                    ));
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionInsertionRejectionCode {
    InvalidConfig,
    InvalidSourceProblem,
    InvalidTargetProblem,
    NonLegacyNetChange,
    SourceNetChanged,
    NotExactlyOneLegacyNet,
    StaleOrIncompleteParent,
    InvalidEvidence,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionInsertionRejection {
    pub code: ConnectionInsertionRejectionCode,
    pub path: String,
    pub detail: String,
}

impl fmt::Display for ConnectionInsertionRejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "connection insertion rejected: {:?} at {}: {}",
            self.code, self.path, self.detail
        )
    }
}

impl std::error::Error for ConnectionInsertionRejection {}

#[derive(Clone, Debug)]
pub struct ValidatedLegacyNetExtension {
    pub added_net: Net,
    pub source_problem_intent_sha256: String,
    pub target_problem_intent_sha256: String,
    pub evidence: LegacyNetExtensionEvidence,
}

/// Require that the target differs from the source by exactly one legacy Net.
/// Collection order is not semantic, but every existing declaration is.
pub fn validate_one_legacy_net_extension(
    source: &Problem,
    target: &Problem,
) -> Result<ValidatedLegacyNetExtension, ConnectionInsertionRejection> {
    source.check_schema().map_err(|detail| {
        reject(
            ConnectionInsertionRejectionCode::InvalidSourceProblem,
            "$.source_problem",
            detail,
        )
    })?;
    target.check_schema().map_err(|detail| {
        reject(
            ConnectionInsertionRejectionCode::InvalidTargetProblem,
            "$.target_problem",
            detail,
        )
    })?;
    let mut source_without_nets = canonical_problem_value(source);
    let mut target_without_nets = canonical_problem_value(target);
    source_without_nets.as_object_mut().unwrap().remove("nets");
    target_without_nets.as_object_mut().unwrap().remove("nets");
    if source_without_nets != target_without_nets {
        return Err(reject(
            ConnectionInsertionRejectionCode::NonLegacyNetChange,
            "$.target_problem",
            "target may differ from source only by one legacy two-terminal Net",
        ));
    }
    let source_nets = net_values_by_id(source);
    let target_nets = net_values_by_id(target);
    for (id, net) in &source_nets {
        if target_nets.get(id) != Some(net) {
            return Err(reject(
                ConnectionInsertionRejectionCode::SourceNetChanged,
                format!("$.target_problem.nets[id={id:?}]"),
                "existing connection was removed or changed",
            ));
        }
    }
    let added = target
        .nets
        .iter()
        .filter(|net| !source_nets.contains_key(net.id.as_str()))
        .collect::<Vec<_>>();
    if target_nets.len() != source_nets.len() + 1 || added.len() != 1 {
        return Err(reject(
            ConnectionInsertionRejectionCode::NotExactlyOneLegacyNet,
            "$.target_problem.nets",
            "exactly one new legacy Net is required",
        ));
    }
    let added_net = added[0].clone();
    let electrical_net = added_net
        .electrical_net
        .clone()
        .unwrap_or_else(|| added_net.id.clone());
    Ok(ValidatedLegacyNetExtension {
        source_problem_intent_sha256: canonical_hash(PROBLEM_HASH_CONTRACT, source),
        target_problem_intent_sha256: canonical_hash(PROBLEM_HASH_CONTRACT, target),
        evidence: LegacyNetExtensionEvidence {
            branch: added_net.id.clone(),
            electrical_net,
            from_component: added_net.from.component.clone(),
            from_pin: added_net.from.pin.clone(),
            to_component: added_net.to.component.clone(),
            to_pin: added_net.to.pin.clone(),
            net_intent_sha256: canonical_hash(NET_HASH_CONTRACT, &added_net),
        },
        added_net,
    })
}

/// Execute one bounded immutable-parent transaction.
pub fn insert_connection(
    source: &Problem,
    target: &Problem,
    parent: &CandidateArtifact,
    config: &ConnectionInsertionConfig,
) -> Result<ConnectionInsertionResult, ConnectionInsertionRejection> {
    config.check()?;
    let extension = validate_one_legacy_net_extension(source, target)?;
    let parent_validation = validate_candidate(source, parent).map_err(|detail| {
        reject(
            ConnectionInsertionRejectionCode::StaleOrIncompleteParent,
            "$.parent",
            detail,
        )
    })?;
    if !parent_validation.complete {
        return Err(reject(
            ConnectionInsertionRejectionCode::StaleOrIncompleteParent,
            "$.parent",
            format!(
                "parent has {} exact finding(s)",
                parent_validation.violations.len()
            ),
        ));
    }

    let added = BTreeSet::from([extension.added_net.id.clone()]);
    let local = run_local_attempt(target, parent, &added, config);
    let mut attempts = vec![local.evidence];
    let mut committed = local.committed;
    if committed.is_none() {
        let global = run_global_attempt(target, parent, config);
        attempts.push(global.evidence);
        committed = global.committed;
    }
    let disposition = if committed.is_some() {
        ConnectionInsertionDisposition::Committed
    } else {
        ConnectionInsertionDisposition::RolledBack
    };
    let selected = committed.unwrap_or_else(|| parent.clone());
    let child_hash = (disposition == ConnectionInsertionDisposition::Committed)
        .then(|| candidate_sha256(&selected));
    let evidence = ConnectionInsertionEvidence {
        contract: String::new(),
        disposition,
        config_sha256: canonical_hash(CONFIG_HASH_CONTRACT, config),
        source_problem_intent_sha256: extension.source_problem_intent_sha256,
        target_problem_intent_sha256: extension.target_problem_intent_sha256,
        parent_exact_sha256: candidate_sha256(parent),
        extension: extension.evidence,
        attempts,
        committed_child_exact_sha256: child_hash,
        canonical_sha256: String::new(),
    }
    .seal()?;
    Ok(ConnectionInsertionResult {
        schema_version: RESULT_SCHEMA_VERSION,
        disposition,
        candidate: selected,
        evidence,
    })
}

struct AttemptRun {
    evidence: ConnectionInsertionAttemptEvidence,
    committed: Option<CandidateArtifact>,
}

fn run_local_attempt(
    target: &Problem,
    parent: &CandidateArtifact,
    added: &BTreeSet<String>,
    config: &ConnectionInsertionConfig,
) -> AttemptRun {
    let inserted = added.iter().next().expect("one added branch").clone();
    let mut work = ConnectionInsertionAttemptWork::default();
    let mut trials = Vec::new();
    let mut conflicts = Vec::new();
    let mut best_failed = None;
    let mut complete = Vec::<(
        usize,
        f64,
        usize,
        CandidateArtifact,
        ExactValidationAssessment,
    )>::new();
    let inspected = config
        .local_routing_portfolio
        .len()
        .min(config.local_candidate_cap);
    for (ordinal, routing) in config
        .local_routing_portfolio
        .iter()
        .take(config.local_candidate_cap)
        .enumerate()
    {
        work.candidates_evaluated += 1;
        match add_problem_branches_with_dut_grid_at_poses(target, parent, added, routing) {
            Ok(result) => {
                work.route_searches += result.evidence.searches as u64;
                work.route_search_states += result.evidence.expansions;
                work.exact_validation_runs += 1;
                work.connectivity_assessments += 1;
                conflicts.extend(validation_conflicts(&result.validation));
                conflicts.extend(routing_conflicts(&result));
                let status = routing_attempt_status(&result);
                trials.push(ConnectionInsertionLocalTrialEvidence {
                    ordinal,
                    routing_config_sha256: canonical_hash(CONFIG_HASH_CONTRACT, routing),
                    status,
                    route_searches: result.evidence.searches as u64,
                    route_search_states: result.evidence.expansions,
                    exact_violation_count: Some(result.validation.violations.len()),
                });
                if result.complete()
                    && local_parent_geometry_preserved(parent, &result.candidate, &inserted)
                {
                    let (length, vias) = candidate_quality(&result.candidate);
                    complete.push((ordinal, length, vias, result.candidate, result.validation));
                } else {
                    let certification = certification(&result.validation, None);
                    if best_failed
                        .as_ref()
                        .is_none_or(|best: &ConnectionInsertionCertification| {
                            certification_rank(&certification) < certification_rank(best)
                        })
                    {
                        best_failed = Some(certification);
                    }
                }
            }
            Err(detail) => {
                conflicts.push(format!("execution:{detail}"));
                trials.push(ConnectionInsertionLocalTrialEvidence {
                    ordinal,
                    routing_config_sha256: canonical_hash(CONFIG_HASH_CONTRACT, routing),
                    status: ConnectionInsertionAttemptStatus::ExecutionFailed,
                    route_searches: 0,
                    route_search_states: 0,
                    exact_violation_count: None,
                });
            }
        }
    }
    complete.sort_by(|left, right| {
        left.1
            .total_cmp(&right.1)
            .then(left.2.cmp(&right.2))
            .then(left.0.cmp(&right.0))
    });
    let selected = complete.into_iter().next();
    let (status, committed, certification) =
        if let Some((_, _, _, candidate, validation)) = selected {
            let hash = candidate_sha256(&candidate);
            (
                ConnectionInsertionAttemptStatus::Committed,
                Some(candidate),
                Some(certification(&validation, Some(hash))),
            )
        } else {
            let budget_limited = inspected < config.local_routing_portfolio.len()
                || trials
                    .iter()
                    .any(|trial| trial.status == ConnectionInsertionAttemptStatus::BudgetExhausted);
            let status = if budget_limited {
                ConnectionInsertionAttemptStatus::BudgetExhausted
            } else if trials
                .iter()
                .all(|trial| trial.status == ConnectionInsertionAttemptStatus::ExecutionFailed)
            {
                ConnectionInsertionAttemptStatus::ExecutionFailed
            } else if trials.is_empty() {
                ConnectionInsertionAttemptStatus::NoCandidate
            } else {
                ConnectionInsertionAttemptStatus::WholeTargetCertificationFailed
            };
            (status, None, best_failed)
        };
    AttemptRun {
        evidence: ConnectionInsertionAttemptEvidence {
            ordinal: 0,
            scope: ConnectionInsertionAttemptScope::LocalInsertedBranchOnly,
            status,
            work,
            telemetry: ConnectionInsertionRepairTelemetry {
                conflict_objects: conflicts,
                repair_scope_branches: vec![inserted.clone()],
                changed_branches: committed.as_ref().map_or_else(Vec::new, |_| vec![inserted]),
                changed_via_branches: committed.as_ref().map_or_else(Vec::new, |candidate| {
                    candidate
                        .traces
                        .iter()
                        .filter(|trace| added.contains(&trace.branch) && !trace.vias.is_empty())
                        .map(|trace| trace.branch.clone())
                        .collect()
                }),
                moved_components: Vec::new(),
            },
            local_trials: trials,
            whole_target_certification: certification,
        },
        committed,
    }
}

fn run_global_attempt(
    target: &Problem,
    parent: &CandidateArtifact,
    config: &ConnectionInsertionConfig,
) -> AttemptRun {
    let scope = parent
        .traces
        .iter()
        .map(|trace| trace.branch.clone())
        .chain(target.nets.iter().map(|net| net.id.clone()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    match repair_routing_with_pressure(
        target,
        &parent.components,
        &config.global_routing,
        &config.global_pressure,
    ) {
        Ok(result) => {
            let complete = result.validation.complete && result.evidence.complete;
            let child_hash = complete.then(|| candidate_sha256(&result.candidate));
            let mut conflicts = validation_conflicts(&result.validation);
            conflicts.extend(
                result
                    .attempts
                    .iter()
                    .filter_map(|attempt| attempt.routing.as_ref())
                    .flat_map(routing_conflicts),
            );
            let route_searches = result
                .attempts
                .iter()
                .filter_map(|attempt| attempt.routing.as_ref())
                .map(|routing| routing.evidence.searches as u64)
                .sum();
            let telemetry = geometry_telemetry(parent, &result.candidate, scope, conflicts);
            let evidence = ConnectionInsertionAttemptEvidence {
                ordinal: 1,
                scope: ConnectionInsertionAttemptScope::GlobalAllLegalGeometry,
                status: if complete {
                    ConnectionInsertionAttemptStatus::Committed
                } else {
                    ConnectionInsertionAttemptStatus::WholeTargetCertificationFailed
                },
                work: ConnectionInsertionAttemptWork {
                    candidates_evaluated: result.attempts.len() as u64,
                    route_searches,
                    route_search_states: result.evidence.total_expansions,
                    coupled_generations: result.evidence.attempted_repairs as u64,
                    exact_validation_runs: result
                        .attempts
                        .iter()
                        .filter(|attempt| attempt.routing.is_some())
                        .count() as u64,
                    connectivity_assessments: result
                        .attempts
                        .iter()
                        .filter(|attempt| attempt.routing.is_some())
                        .count() as u64,
                },
                telemetry,
                local_trials: Vec::new(),
                whole_target_certification: Some(certification(&result.validation, child_hash)),
            };
            AttemptRun {
                evidence,
                committed: complete.then_some(result.candidate),
            }
        }
        Err(detail) => AttemptRun {
            evidence: ConnectionInsertionAttemptEvidence {
                ordinal: 1,
                scope: ConnectionInsertionAttemptScope::GlobalAllLegalGeometry,
                status: ConnectionInsertionAttemptStatus::ExecutionFailed,
                work: ConnectionInsertionAttemptWork::default(),
                telemetry: ConnectionInsertionRepairTelemetry {
                    conflict_objects: vec![format!("execution:{detail}")],
                    repair_scope_branches: scope,
                    ..ConnectionInsertionRepairTelemetry::default()
                },
                local_trials: Vec::new(),
                whole_target_certification: None,
            },
            committed: None,
        },
    }
}

fn routing_attempt_status(result: &DutGridRoutingResult) -> ConnectionInsertionAttemptStatus {
    if result.complete() {
        ConnectionInsertionAttemptStatus::Committed
    } else if result
        .evidence
        .branches
        .iter()
        .any(|branch| branch.status == BranchRoutingStatus::BudgetExhausted)
    {
        ConnectionInsertionAttemptStatus::BudgetExhausted
    } else if result.evidence.routed_branches == 0 {
        ConnectionInsertionAttemptStatus::NoCandidate
    } else {
        ConnectionInsertionAttemptStatus::WholeTargetCertificationFailed
    }
}

fn local_parent_geometry_preserved(
    parent: &CandidateArtifact,
    child: &CandidateArtifact,
    added: &str,
) -> bool {
    if parent.components != child.components {
        return false;
    }
    let child_traces = child
        .traces
        .iter()
        .map(|trace| (trace.branch.as_str(), trace))
        .collect::<BTreeMap<_, _>>();
    parent
        .traces
        .iter()
        .all(|trace| child_traces.get(trace.branch.as_str()) == Some(&trace))
        && child
            .traces
            .iter()
            .filter(|trace| !parent.traces.iter().any(|old| old.branch == trace.branch))
            .all(|trace| trace.branch == added)
}

fn certification(
    validation: &ExactValidationAssessment,
    child_hash: Option<String>,
) -> ConnectionInsertionCertification {
    ConnectionInsertionCertification {
        exhaustive_full_width_validation: true,
        exact_violation_count: validation.geometry.violations.len(),
        electrical_connectivity_complete: validation.electrical.complete,
        connectivity_finding_count: validation.electrical.findings.len(),
        strict_final_admission: validation.complete,
        final_admission_blocker_count: validation.violations.len(),
        exact_child_sha256: child_hash,
    }
}

fn certification_rank(value: &ConnectionInsertionCertification) -> (usize, usize, usize) {
    (
        value.exact_violation_count,
        value.connectivity_finding_count,
        value.final_admission_blocker_count,
    )
}

fn geometry_telemetry(
    parent: &CandidateArtifact,
    child: &CandidateArtifact,
    repair_scope_branches: Vec<String>,
    conflict_objects: Vec<String>,
) -> ConnectionInsertionRepairTelemetry {
    let parent_components = parent
        .components
        .iter()
        .map(|component| (component.id.as_str(), component))
        .collect::<BTreeMap<_, _>>();
    let parent_traces = parent
        .traces
        .iter()
        .map(|trace| (trace.branch.as_str(), trace))
        .collect::<BTreeMap<_, _>>();
    ConnectionInsertionRepairTelemetry {
        conflict_objects,
        repair_scope_branches,
        changed_branches: child
            .traces
            .iter()
            .filter(|trace| parent_traces.get(trace.branch.as_str()) != Some(trace))
            .map(|trace| trace.branch.clone())
            .collect(),
        changed_via_branches: child
            .traces
            .iter()
            .filter(|trace| {
                parent_traces
                    .get(trace.branch.as_str())
                    .map_or(!trace.vias.is_empty(), |old| old.vias != trace.vias)
            })
            .map(|trace| trace.branch.clone())
            .collect(),
        moved_components: child
            .components
            .iter()
            .filter(|component| parent_components.get(component.id.as_str()) != Some(component))
            .map(|component| component.id.clone())
            .collect(),
    }
}

fn validation_conflicts(validation: &ExactValidationAssessment) -> Vec<String> {
    validation
        .violations
        .iter()
        .flat_map(|violation| violation.objects.iter().cloned())
        .collect()
}

fn routing_conflicts(result: &DutGridRoutingResult) -> Vec<String> {
    result
        .evidence
        .branches
        .iter()
        .flat_map(|branch| {
            branch
                .blockers
                .iter()
                .map(|blocker| format!("{:?}:{}", blocker.kind, blocker.object))
        })
        .collect()
}

fn candidate_quality(candidate: &CandidateArtifact) -> (f64, usize) {
    let length = candidate
        .traces
        .iter()
        .flat_map(|trace| trace.points.windows(2))
        .map(|points| (points[1].x - points[0].x).hypot(points[1].y - points[0].y))
        .sum();
    let vias = candidate.traces.iter().map(|trace| trace.vias.len()).sum();
    (length, vias)
}

fn candidate_sha256(candidate: &CandidateArtifact) -> String {
    canonical_hash(CANDIDATE_HASH_CONTRACT, candidate)
}

fn net_values_by_id(problem: &Problem) -> BTreeMap<&str, Value> {
    problem
        .nets
        .iter()
        .map(|net| {
            (
                net.id.as_str(),
                canonical_json_value(serde_json::to_value(net).expect("Net serializes")),
            )
        })
        .collect()
}

fn canonical_problem_value(problem: &Problem) -> Value {
    let mut value = serde_json::to_value(problem).expect("Problem serializes");
    let object = value.as_object_mut().expect("Problem serializes as object");
    for key in ["components", "nets", "electrical_nets"] {
        if let Some(items) = object.get_mut(key).and_then(Value::as_array_mut) {
            items.sort_by(|left, right| left["id"].as_str().cmp(&right["id"].as_str()));
        }
    }
    if let Some(items) = object
        .get_mut("placement_constraints")
        .and_then(Value::as_array_mut)
    {
        items.sort_by_key(|item| canonical_json_value(item.clone()).to_string());
    }
    canonical_json_value(value)
}

fn canonical_json_value(value: Value) -> Value {
    match value {
        Value::Array(values) => {
            Value::Array(values.into_iter().map(canonical_json_value).collect())
        }
        Value::Object(values) => Value::Object(
            values
                .into_iter()
                .map(|(key, value)| (key, canonical_json_value(value)))
                .collect::<BTreeMap<_, _>>()
                .into_iter()
                .collect(),
        ),
        value => value,
    }
}

fn canonical_hash(contract: &str, value: &impl Serialize) -> String {
    let value =
        canonical_json_value(serde_json::to_value(value).expect("transaction value serializes"));
    let envelope =
        canonical_json_value(serde_json::json!({ "contract": contract, "value": value }));
    let digest = Sha256::digest(serde_json::to_vec(&envelope).expect("canonical JSON serializes"));
    format!(
        "sha256:{}",
        digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

fn is_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn reject(
    code: ConnectionInsertionRejectionCode,
    path: impl Into<String>,
    detail: impl Into<String>,
) -> ConnectionInsertionRejection {
    ConnectionInsertionRejection {
        code,
        path: path.into(),
        detail: detail.into(),
    }
}

fn invalid_evidence(detail: impl Into<String>) -> ConnectionInsertionRejection {
    reject(
        ConnectionInsertionRejectionCode::InvalidEvidence,
        "$.evidence",
        detail,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use pcb_routing::route_problem_with_dut_grid;

    fn problem(json: &str) -> Problem {
        serde_json::from_str(json).unwrap()
    }

    fn extension_pair() -> (Problem, Problem) {
        let target = problem(include_str!(
            "../../../benchmarks/imported/layout-trace/forced-crossing-two-layer.json"
        ));
        let mut source = target.clone();
        source.nets.pop();
        (source, target)
    }

    fn progressive(stage: &str) -> Problem {
        let source = match stage {
            "00" => include_str!(
                "../../../benchmarks/small/connection-insertion-esp32-progressive/00-empty.json"
            ),
            "01" => include_str!(
                "../../../benchmarks/small/connection-insertion-esp32-progressive/01-a-west.json"
            ),
            "02" => include_str!(
                "../../../benchmarks/small/connection-insertion-esp32-progressive/02-a-complete.json"
            ),
            "03" => include_str!(
                "../../../benchmarks/small/connection-insertion-esp32-progressive/03-b-west.json"
            ),
            "04" => include_str!(
                "../../../benchmarks/small/connection-insertion-esp32-progressive/04-b-crossing.json"
            ),
            _ => panic!("unknown stage"),
        };
        problem(source)
    }

    #[test]
    fn exact_delta_is_order_independent_but_rejects_other_changes() {
        let (source, mut target) = extension_pair();
        target.nets.reverse();
        assert!(validate_one_legacy_net_extension(&source, &target).is_ok());
        target.rules.clearance += 0.01;
        assert_eq!(
            validate_one_legacy_net_extension(&source, &target)
                .unwrap_err()
                .code,
            ConnectionInsertionRejectionCode::NonLegacyNetChange
        );
    }

    #[test]
    fn exact_delta_rejects_changed_prefix_and_multiple_additions() {
        let (source, mut target) = extension_pair();
        target.nets[0].width += 0.01;
        assert_eq!(
            validate_one_legacy_net_extension(&source, &target)
                .unwrap_err()
                .code,
            ConnectionInsertionRejectionCode::SourceNetChanged
        );
        let (_, mut target) = extension_pair();
        let mut extra = target.nets.last().unwrap().clone();
        extra.id = "extra".into();
        target.nets.push(extra);
        assert_eq!(
            validate_one_legacy_net_extension(&source, &target)
                .unwrap_err()
                .code,
            ConnectionInsertionRejectionCode::NotExactlyOneLegacyNet
        );
    }

    #[test]
    fn progressive_esp32_additions_commit_locally_from_immutable_parents() {
        let config = ConnectionInsertionConfig::default();
        let mut source = progressive("00");
        let mut parent = route_problem_with_dut_grid(&source, &config.global_routing)
            .unwrap()
            .candidate;
        for next in ["01", "02", "03", "04"] {
            let target = progressive(next);
            let before = parent.clone();
            let result = insert_connection(&source, &target, &parent, &config).unwrap();
            assert_eq!(
                result.disposition,
                ConnectionInsertionDisposition::Committed
            );
            assert_eq!(result.evidence.attempts.len(), 1);
            assert!(result.evidence.check().is_ok());
            result.check(&source, &target, &before, &config).unwrap();
            assert!(local_parent_geometry_preserved(
                &before,
                &result.candidate,
                &result.evidence.extension.branch
            ));
            assert!(
                validate_candidate(&target, &result.candidate)
                    .unwrap()
                    .complete
            );
            source = target;
            parent = result.candidate;
        }
    }

    #[test]
    fn exhausted_transaction_returns_exact_parent_and_tampering_is_detected() {
        let source = progressive("00");
        let target = progressive("01");
        let mut config = ConnectionInsertionConfig::default();
        for routing in &mut config.local_routing_portfolio {
            routing.max_expansions_per_search = 1;
            routing.retry_grid_mm.clear();
        }
        config.global_routing.max_expansions_per_search = 1;
        config.global_routing.retry_grid_mm.clear();
        config.global_pressure.maximum_iterations = 1;
        let parent = route_problem_with_dut_grid(&source, &DutGridRoutingConfig::default())
            .unwrap()
            .candidate;
        let result = insert_connection(&source, &target, &parent, &config).unwrap();
        assert_eq!(
            result.disposition,
            ConnectionInsertionDisposition::RolledBack
        );
        assert_eq!(result.candidate, parent);
        result.check(&source, &target, &parent, &config).unwrap();
        assert_eq!(result.evidence.attempts.len(), 2);
        result.evidence.check().unwrap();

        let mut tampered = result.evidence;
        tampered.attempts[0].work.route_search_states += 1;
        assert_eq!(
            tampered.check().unwrap_err().code,
            ConnectionInsertionRejectionCode::InvalidEvidence
        );
    }

    #[test]
    fn incomplete_parent_is_rejected_before_any_attempt() {
        let source = progressive("01");
        let target = progressive("02");
        let empty =
            route_problem_with_dut_grid(&progressive("00"), &DutGridRoutingConfig::default())
                .unwrap()
                .candidate;
        assert_eq!(
            insert_connection(
                &source,
                &target,
                &empty,
                &ConnectionInsertionConfig::default()
            )
            .unwrap_err()
            .code,
            ConnectionInsertionRejectionCode::StaleOrIncompleteParent
        );
    }

    #[test]
    fn failed_local_insertion_can_commit_through_coupled_pressure() {
        let source = problem(include_str!(
            "../../../benchmarks/small/connection-insertion-pressure-source.json"
        ));
        let target = problem(include_str!(
            "../../../benchmarks/small/passage-pressure-asymmetric.json"
        ));
        let mut config = ConnectionInsertionConfig::default();
        config.local_routing_portfolio[0].retry_grid_mm = vec![0.25, 0.1];
        config.local_routing_portfolio[0].max_expansions_per_search = 5_000_000;
        config.global_routing.retry_grid_mm = vec![0.25, 0.1];
        config.global_routing.max_expansions_per_search = 5_000_000;
        let parent = route_problem_with_dut_grid(&source, &DutGridRoutingConfig::default())
            .unwrap()
            .candidate;
        let result = insert_connection(&source, &target, &parent, &config).unwrap();
        assert_eq!(
            result.disposition,
            ConnectionInsertionDisposition::Committed
        );
        assert_eq!(result.evidence.attempts.len(), 2);
        assert_eq!(
            result.evidence.attempts[0].scope,
            ConnectionInsertionAttemptScope::LocalInsertedBranchOnly
        );
        assert_ne!(
            result.evidence.attempts[0].status,
            ConnectionInsertionAttemptStatus::Committed
        );
        assert_eq!(
            result.evidence.attempts[1].status,
            ConnectionInsertionAttemptStatus::Committed
        );
        assert_eq!(
            result.evidence.attempts[1].telemetry.moved_components,
            ["WALL"]
        );
        result.check(&source, &target, &parent, &config).unwrap();
        assert!(
            validate_candidate(&target, &result.candidate)
                .unwrap()
                .complete
        );
    }
}
