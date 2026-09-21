//! Versioned contracts for composing coarse placer/router stages.
//!
//! The contract is deliberately independent of the in-process coordinator.
//! The same document can guard a library call, a standalone process, or a
//! persisted experiment artifact.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

pub const STAGE_CONTRACT_SCHEMA_VERSION: &str = "layout-trace.stage-contract/v1";
pub const STAGE_FINGERPRINT_PREFIX: &str = "sha256:";

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StageKind {
    Normalize,
    SeedPlacement,
    Corridors,
    DiscretePlan,
    ParticleCompile,
    ParticleSolve,
    Recertify,
    ExactValidate,
    View,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PassClass {
    Preserving,
    Speculative,
    Repairing,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InvariantKind {
    NormalizedProblem,
    Placement,
    Corridors,
    DiscretePlan,
    ParticleBatch,
    ParticleResult,
    RecertifiedRouting,
    ExactValidation,
    ViewArtifact,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CertificateKind {
    Normalization,
    Placement,
    CorridorEpoch,
    DiscretePlan,
    ParticleTopology,
    ParticleSolve,
    RoutingRecertification,
    ExactValidation,
    ViewManifest,
}

impl InvariantKind {
    fn certificate_kind(self) -> CertificateKind {
        match self {
            Self::NormalizedProblem => CertificateKind::Normalization,
            Self::Placement => CertificateKind::Placement,
            Self::Corridors => CertificateKind::CorridorEpoch,
            Self::DiscretePlan => CertificateKind::DiscretePlan,
            Self::ParticleBatch => CertificateKind::ParticleTopology,
            Self::ParticleResult => CertificateKind::ParticleSolve,
            Self::RecertifiedRouting => CertificateKind::RoutingRecertification,
            Self::ExactValidation => CertificateKind::ExactValidation,
            Self::ViewArtifact => CertificateKind::ViewManifest,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CertificateReference {
    pub kind: CertificateKind,
    pub schema_version: String,
    pub artifact: String,
    /// Fingerprint of the invariant certified by this artifact.
    pub subject_fingerprint: String,
    /// Fingerprint of the complete certificate artifact itself.
    pub certificate_fingerprint: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InvariantReference {
    pub kind: InvariantKind,
    pub fingerprint: String,
    pub certificate: CertificateReference,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DebtKind {
    PlacementUncertified,
    TopologyUncertified,
    WidthUnrealized,
    RoutingUnrecertified,
    ExactValidationPending,
    ViewStale,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StageDebt {
    pub id: String,
    pub kind: DebtKind,
    pub introduced_by: String,
    pub detail: String,
    /// Invariant kinds which cannot be consumed as authoritative while this
    /// debt remains live.
    pub blocks: Vec<InvariantKind>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StagePassContract {
    pub id: String,
    pub stage: StageKind,
    pub class: PassClass,
    /// Explicit dependencies on prior pass IDs. Execution order alone is not
    /// accepted as dependency evidence.
    pub after: Vec<String>,
    pub requires: Vec<InvariantReference>,
    pub preserves: Vec<InvariantKind>,
    pub may_invalidate: Vec<InvariantKind>,
    pub establishes: Vec<InvariantReference>,
    pub requires_debts: Vec<String>,
    pub introduces_debts: Vec<StageDebt>,
    pub discharges_debts: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StageContract {
    pub schema_version: String,
    pub contract_id: String,
    pub initial_invariants: Vec<InvariantReference>,
    pub initial_debts: Vec<StageDebt>,
    pub passes: Vec<StagePassContract>,
    /// Exact expected live invariant set after the last pass.
    pub outputs: Vec<InvariantReference>,
    /// Exact expected live debt set after the last pass.
    pub remaining_debts: Vec<StageDebt>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StageContractIssue {
    pub path: String,
    pub message: String,
}

impl StageContract {
    pub fn validate(&self) -> Result<(), Vec<StageContractIssue>> {
        let mut issues = Vec::new();
        if self.schema_version != STAGE_CONTRACT_SCHEMA_VERSION {
            issue(
                &mut issues,
                "schema_version",
                format!("expected {STAGE_CONTRACT_SCHEMA_VERSION}"),
            );
        }
        require_identity(&mut issues, "contract_id", &self.contract_id);

        let mut live_invariants =
            collect_invariants(&mut issues, "initial_invariants", &self.initial_invariants);
        let mut live_debts = collect_debts(&mut issues, "initial_debts", &self.initial_debts);
        let mut prior_passes = BTreeSet::new();

        for (index, pass) in self.passes.iter().enumerate() {
            let path = format!("passes[{index}]");
            require_identity(&mut issues, &format!("{path}.id"), &pass.id);
            if !prior_passes.insert(pass.id.clone()) {
                issue(&mut issues, &format!("{path}.id"), "duplicate pass ID");
            }
            validate_sorted_unique_strings(&mut issues, &format!("{path}.after"), &pass.after);
            for dependency in &pass.after {
                if !prior_passes.contains(dependency) || dependency == &pass.id {
                    issue(
                        &mut issues,
                        &format!("{path}.after"),
                        format!("dependency {dependency:?} is not a prior pass"),
                    );
                }
            }

            let required =
                collect_invariants(&mut issues, &format!("{path}.requires"), &pass.requires);
            for (kind, reference) in &required {
                match live_invariants.get(kind) {
                    Some(live) if live == reference => {}
                    Some(_) => issue(
                        &mut issues,
                        &format!("{path}.requires"),
                        format!(
                            "{kind:?} fingerprint/certificate does not match the live invariant"
                        ),
                    ),
                    None => issue(
                        &mut issues,
                        &format!("{path}.requires"),
                        format!("{kind:?} is not live"),
                    ),
                }
                for debt in live_debts
                    .values()
                    .filter(|debt| debt.blocks.contains(kind))
                {
                    if !pass.requires_debts.contains(&debt.id) {
                        issue(
                            &mut issues,
                            &format!("{path}.requires"),
                            format!(
                                "{kind:?} is blocked by live debt {:?}, which the pass did not require",
                                debt.id
                            ),
                        );
                    }
                }
            }

            let preserves =
                collect_kinds(&mut issues, &format!("{path}.preserves"), &pass.preserves);
            let invalidates = collect_kinds(
                &mut issues,
                &format!("{path}.may_invalidate"),
                &pass.may_invalidate,
            );
            let establishes = collect_invariants(
                &mut issues,
                &format!("{path}.establishes"),
                &pass.establishes,
            );
            for kind in preserves.intersection(&invalidates) {
                issue(
                    &mut issues,
                    &path,
                    format!("{kind:?} is both preserved and invalidated"),
                );
            }
            let established_kinds = establishes.keys().copied().collect::<BTreeSet<_>>();
            for kind in preserves.intersection(&established_kinds) {
                issue(
                    &mut issues,
                    &path,
                    format!("{kind:?} is both preserved and re-established"),
                );
            }
            for kind in &established_kinds {
                if live_invariants.contains_key(kind) && !invalidates.contains(kind) {
                    issue(
                        &mut issues,
                        &format!("{path}.establishes"),
                        format!(
                            "replacing live {kind:?} requires an explicit may_invalidate declaration"
                        ),
                    );
                }
            }
            for kind in &preserves {
                if !live_invariants.contains_key(kind) {
                    issue(
                        &mut issues,
                        &format!("{path}.preserves"),
                        format!("{kind:?} is not live"),
                    );
                }
            }
            for kind in &invalidates {
                if !live_invariants.contains_key(kind) {
                    issue(
                        &mut issues,
                        &format!("{path}.may_invalidate"),
                        format!("{kind:?} is not live"),
                    );
                }
            }

            validate_sorted_unique_strings(
                &mut issues,
                &format!("{path}.requires_debts"),
                &pass.requires_debts,
            );
            validate_sorted_unique_strings(
                &mut issues,
                &format!("{path}.discharges_debts"),
                &pass.discharges_debts,
            );
            for debt in &pass.requires_debts {
                if !live_debts.contains_key(debt) {
                    issue(
                        &mut issues,
                        &format!("{path}.requires_debts"),
                        format!("debt {debt:?} is not live"),
                    );
                }
            }
            for debt in &pass.discharges_debts {
                if !live_debts.contains_key(debt) {
                    issue(
                        &mut issues,
                        &format!("{path}.discharges_debts"),
                        format!("debt {debt:?} is not live"),
                    );
                }
                if !pass.requires_debts.contains(debt) {
                    issue(
                        &mut issues,
                        &format!("{path}.discharges_debts"),
                        format!("debt {debt:?} must also be declared in requires_debts"),
                    );
                }
            }
            let introduced = collect_debts(
                &mut issues,
                &format!("{path}.introduces_debts"),
                &pass.introduces_debts,
            );
            for debt in introduced.values() {
                if debt.introduced_by != pass.id {
                    issue(
                        &mut issues,
                        &format!("{path}.introduces_debts"),
                        format!("debt {:?} has a different introducer", debt.id),
                    );
                }
                if live_debts.contains_key(&debt.id) || pass.discharges_debts.contains(&debt.id) {
                    issue(
                        &mut issues,
                        &format!("{path}.introduces_debts"),
                        format!("debt {:?} is reused or simultaneously discharged", debt.id),
                    );
                }
            }

            match pass.class {
                PassClass::Preserving => {
                    if !invalidates.is_empty() || !introduced.is_empty() {
                        issue(
                            &mut issues,
                            &path,
                            "preserving pass cannot invalidate invariants or introduce debt",
                        );
                    }
                }
                PassClass::Speculative => {
                    if invalidates.is_empty() && introduced.is_empty() {
                        issue(
                            &mut issues,
                            &path,
                            "speculative pass must declare invalidation or introduced debt",
                        );
                    }
                }
                PassClass::Repairing => {
                    if pass.discharges_debts.is_empty() || !introduced.is_empty() {
                        issue(
                            &mut issues,
                            &path,
                            "repairing pass must discharge debt and cannot introduce new debt",
                        );
                    }
                }
            }

            for kind in invalidates {
                live_invariants.remove(&kind);
            }
            for (kind, reference) in establishes {
                live_invariants.insert(kind, reference);
            }
            for debt in &pass.discharges_debts {
                live_debts.remove(debt);
            }
            live_debts.extend(introduced);
        }

        let outputs = collect_invariants(&mut issues, "outputs", &self.outputs);
        if live_invariants != outputs {
            issue(
                &mut issues,
                "outputs",
                "declared outputs do not exactly match the computed live invariant set",
            );
        }
        let remaining = collect_debts(&mut issues, "remaining_debts", &self.remaining_debts);
        if live_debts != remaining {
            issue(
                &mut issues,
                "remaining_debts",
                "declared remaining debts do not exactly match the computed live debt set",
            );
        }
        for debt in live_debts.values() {
            for blocked in &debt.blocks {
                if live_invariants.contains_key(blocked) {
                    issue(
                        &mut issues,
                        "remaining_debts",
                        format!(
                            "live debt {:?} blocks authoritative output {blocked:?}",
                            debt.id
                        ),
                    );
                }
            }
        }

        if issues.is_empty() {
            Ok(())
        } else {
            Err(issues)
        }
    }
}

fn collect_invariants(
    issues: &mut Vec<StageContractIssue>,
    path: &str,
    references: &[InvariantReference],
) -> BTreeMap<InvariantKind, InvariantReference> {
    let mut result = BTreeMap::new();
    let mut previous = None;
    for (index, reference) in references.iter().enumerate() {
        let item_path = format!("{path}[{index}]");
        validate_invariant(issues, &item_path, reference);
        if previous.is_some_and(|kind| kind >= reference.kind) {
            issue(issues, path, "invariant kinds must be unique and sorted");
        }
        previous = Some(reference.kind);
        result.insert(reference.kind, reference.clone());
    }
    result
}

fn validate_invariant(
    issues: &mut Vec<StageContractIssue>,
    path: &str,
    reference: &InvariantReference,
) {
    validate_fingerprint(
        issues,
        &format!("{path}.fingerprint"),
        &reference.fingerprint,
    );
    if reference.certificate.kind != reference.kind.certificate_kind() {
        issue(
            issues,
            &format!("{path}.certificate.kind"),
            "certificate kind does not match invariant kind",
        );
    }
    require_identity(
        issues,
        &format!("{path}.certificate.schema_version"),
        &reference.certificate.schema_version,
    );
    require_identity(
        issues,
        &format!("{path}.certificate.artifact"),
        &reference.certificate.artifact,
    );
    validate_fingerprint(
        issues,
        &format!("{path}.certificate.subject_fingerprint"),
        &reference.certificate.subject_fingerprint,
    );
    validate_fingerprint(
        issues,
        &format!("{path}.certificate.certificate_fingerprint"),
        &reference.certificate.certificate_fingerprint,
    );
    if reference.certificate.subject_fingerprint != reference.fingerprint {
        issue(
            issues,
            &format!("{path}.certificate.subject_fingerprint"),
            "certificate does not name the invariant fingerprint",
        );
    }
}

fn collect_kinds(
    issues: &mut Vec<StageContractIssue>,
    path: &str,
    kinds: &[InvariantKind],
) -> BTreeSet<InvariantKind> {
    let result = kinds.iter().copied().collect::<BTreeSet<_>>();
    if result.len() != kinds.len() || !kinds.windows(2).all(|pair| pair[0] < pair[1]) {
        issue(issues, path, "invariant kinds must be unique and sorted");
    }
    result
}

fn collect_debts(
    issues: &mut Vec<StageContractIssue>,
    path: &str,
    debts: &[StageDebt],
) -> BTreeMap<String, StageDebt> {
    let mut result = BTreeMap::new();
    let mut previous: Option<&str> = None;
    for (index, debt) in debts.iter().enumerate() {
        let item_path = format!("{path}[{index}]");
        require_identity(issues, &format!("{item_path}.id"), &debt.id);
        require_identity(
            issues,
            &format!("{item_path}.introduced_by"),
            &debt.introduced_by,
        );
        if debt.detail.trim().is_empty() {
            issue(issues, &format!("{item_path}.detail"), "detail is empty");
        }
        collect_kinds(issues, &format!("{item_path}.blocks"), &debt.blocks);
        if previous.is_some_and(|id| id >= debt.id.as_str()) {
            issue(issues, path, "debt IDs must be unique and sorted");
        }
        previous = Some(debt.id.as_str());
        result.insert(debt.id.clone(), debt.clone());
    }
    result
}

fn validate_sorted_unique_strings(
    issues: &mut Vec<StageContractIssue>,
    path: &str,
    values: &[String],
) {
    for (index, value) in values.iter().enumerate() {
        require_identity(issues, &format!("{path}[{index}]"), value);
    }
    if !values.windows(2).all(|pair| pair[0] < pair[1]) {
        issue(issues, path, "values must be unique and sorted");
    }
}

fn validate_fingerprint(issues: &mut Vec<StageContractIssue>, path: &str, fingerprint: &str) {
    let digest = fingerprint.strip_prefix(STAGE_FINGERPRINT_PREFIX);
    if digest.is_none_or(|digest| {
        digest.len() != 64
            || !digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    }) {
        issue(
            issues,
            path,
            "fingerprint must be sha256: followed by 64 lowercase hexadecimal characters",
        );
    }
}

fn require_identity(issues: &mut Vec<StageContractIssue>, path: &str, value: &str) {
    if value.trim().is_empty() {
        issue(issues, path, "identity is empty");
    }
}

fn issue(issues: &mut Vec<StageContractIssue>, path: &str, message: impl Into<String>) {
    issues.push(StageContractIssue {
        path: path.to_owned(),
        message: message.into(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invariant(kind: InvariantKind, byte: char) -> InvariantReference {
        let fingerprint = format!("{STAGE_FINGERPRINT_PREFIX}{}", byte.to_string().repeat(64));
        InvariantReference {
            kind,
            fingerprint: fingerprint.clone(),
            certificate: CertificateReference {
                kind: kind.certificate_kind(),
                schema_version: "test-certificate/v1".into(),
                artifact: format!("{kind:?}.json"),
                subject_fingerprint: fingerprint,
                certificate_fingerprint: format!("{STAGE_FINGERPRINT_PREFIX}{}", "f".repeat(64)),
            },
        }
    }

    fn valid_contract() -> StageContract {
        let normalized = invariant(InvariantKind::NormalizedProblem, 'a');
        let placement = invariant(InvariantKind::Placement, 'b');
        let speculative = StageDebt {
            id: "width-unrealized".into(),
            kind: DebtKind::WidthUnrealized,
            introduced_by: "compile-particles".into(),
            detail: "centerlines have not yet realized copper width".into(),
            blocks: vec![InvariantKind::ExactValidation],
        };
        let particle_batch = invariant(InvariantKind::ParticleBatch, 'c');
        let particle_result = invariant(InvariantKind::ParticleResult, 'd');
        StageContract {
            schema_version: STAGE_CONTRACT_SCHEMA_VERSION.into(),
            contract_id: "reference".into(),
            initial_invariants: vec![normalized.clone(), placement.clone()],
            initial_debts: Vec::new(),
            passes: vec![
                StagePassContract {
                    id: "compile-particles".into(),
                    stage: StageKind::ParticleCompile,
                    class: PassClass::Speculative,
                    after: Vec::new(),
                    requires: vec![normalized.clone(), placement.clone()],
                    preserves: vec![InvariantKind::NormalizedProblem, InvariantKind::Placement],
                    may_invalidate: Vec::new(),
                    establishes: vec![particle_batch.clone()],
                    requires_debts: Vec::new(),
                    introduces_debts: vec![speculative.clone()],
                    discharges_debts: Vec::new(),
                },
                StagePassContract {
                    id: "solve-particles".into(),
                    stage: StageKind::ParticleSolve,
                    class: PassClass::Repairing,
                    after: vec!["compile-particles".into()],
                    requires: vec![particle_batch.clone()],
                    preserves: vec![InvariantKind::ParticleBatch],
                    may_invalidate: Vec::new(),
                    establishes: vec![particle_result.clone()],
                    requires_debts: vec!["width-unrealized".into()],
                    introduces_debts: Vec::new(),
                    discharges_debts: vec!["width-unrealized".into()],
                },
            ],
            outputs: vec![normalized, placement, particle_batch, particle_result],
            remaining_debts: Vec::new(),
        }
    }

    #[test]
    fn valid_contract_tracks_invariants_and_debt_exactly() {
        valid_contract().validate().unwrap();
    }

    #[test]
    fn overlap_and_fingerprint_mismatch_fail_closed() {
        let mut contract = valid_contract();
        contract.passes[0]
            .may_invalidate
            .push(InvariantKind::Placement);
        contract.passes[1].requires[0].fingerprint =
            format!("{STAGE_FINGERPRINT_PREFIX}{}", "e".repeat(64));
        let issues = contract.validate().unwrap_err();
        assert!(
            issues
                .iter()
                .any(|issue| issue.message.contains("both preserved and invalidated"))
        );
        assert!(
            issues
                .iter()
                .any(|issue| issue.message.contains("fingerprint/certificate"))
        );
    }

    #[test]
    fn certificate_and_remaining_debt_must_be_exact() {
        let mut contract = valid_contract();
        contract.initial_invariants[0]
            .certificate
            .subject_fingerprint = format!("{STAGE_FINGERPRINT_PREFIX}{}", "0".repeat(64));
        contract.remaining_debts.push(StageDebt {
            id: "invented".into(),
            kind: DebtKind::ViewStale,
            introduced_by: "input".into(),
            detail: "not actually live".into(),
            blocks: vec![InvariantKind::ViewArtifact],
        });
        let issues = contract.validate().unwrap_err();
        assert!(
            issues
                .iter()
                .any(|issue| issue.message.contains("does not name"))
        );
        assert!(issues.iter().any(|issue| issue.path == "remaining_debts"));
    }

    #[test]
    fn replacing_a_live_invariant_requires_explicit_invalidation() {
        let mut contract = valid_contract();
        contract.passes[0]
            .establishes
            .push(invariant(InvariantKind::Placement, 'e'));
        contract.passes[0].establishes.sort_by_key(|item| item.kind);
        contract.outputs = vec![
            invariant(InvariantKind::NormalizedProblem, 'a'),
            invariant(InvariantKind::Placement, 'e'),
            invariant(InvariantKind::ParticleBatch, 'c'),
            invariant(InvariantKind::ParticleResult, 'd'),
        ];
        let issues = contract.validate().unwrap_err();
        assert!(issues.iter().any(|issue| {
            issue
                .message
                .contains("requires an explicit may_invalidate declaration")
        }));
    }

    #[test]
    fn schema_is_strict_and_versioned() {
        let json = serde_json::to_value(valid_contract()).unwrap();
        assert_eq!(json["schema_version"], STAGE_CONTRACT_SCHEMA_VERSION);
        let mut unknown = json;
        unknown["unknown"] = serde_json::json!(true);
        assert!(serde_json::from_value::<StageContract>(unknown).is_err());
    }

    #[test]
    fn fingerprint_requires_the_canonical_sha256_prefix() {
        for invalid in [
            "a".repeat(64),
            format!("blake3:{}", "a".repeat(64)),
            format!("SHA256:{}", "a".repeat(64)),
        ] {
            let mut contract = valid_contract();
            contract.initial_invariants[0].fingerprint = invalid;
            let issues = contract.validate().unwrap_err();
            assert!(issues.iter().any(|issue| {
                issue.path == "initial_invariants[0].fingerprint"
                    && issue.message.contains("sha256:")
            }));
        }
    }
}
