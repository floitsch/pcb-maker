// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Versioned, fail-closed contract for bounded route-family portfolios.
//!
//! This is the production form of the V5 experimental contract. It deliberately
//! validates certificates and their epochs before a portfolio can be handed to
//! assignment. It does not yet generate portfolios or wire them into the engine.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt::{self, Write};

use crate::copper_pair::{
    FixedCopperPairClassification, FixedCopperRoute, classify_prepared_copper_pair,
    prepare_fixed_copper_route,
};
use crate::geometry::EPSILON;
use crate::model::Vec2;

pub const LEGACY_ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION: &str =
    "layout-trace.route-family-portfolio/v2";
pub const FIXED_WITNESS_ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION: &str =
    "layout-trace.route-family-portfolio/v3";
pub const ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION: &str = "layout-trace.route-family-portfolio/v4";
/// V5 preserves the V4 fixed-context and exact serialized-witness contract,
/// while admitting ordered multi-run families with certified transitions.
/// Older versions retain their historical single-run restriction.
pub const MULTI_RUN_ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION: &str =
    "layout-trace.route-family-portfolio/v5";
pub const FIXED_WITNESS_PAIR_ANALYSIS_METHOD: &str =
    "exhaustive_fixed_full_width_serialized_runs/v1";
pub const FIXED_CONTEXT_ANALYSIS_METHOD: &str = "exhaustive_fixed_context_serialized_runs/v1";
pub const FIXED_CONTEXT_FINGERPRINT_METHOD: &str = "layout-trace.fixed-context-fingerprint/v1";
pub const FIXED_WITNESS_NO_SCALAR_CAPACITY_MODEL: &str =
    "fixed_full_width_witness_pairwise/no_scalar_capacity/v3";
pub const EXACT_VIA_CLEARANCE_MODEL: &str = "exact_point_against_polygonal_obstacles/v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationIssue {
    pub code: &'static str,
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FingerprintError {
    pub path: String,
    pub message: String,
}

impl fmt::Display for FingerprintError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.path, self.message)
    }
}

impl Error for FingerprintError {}

impl ValidationIssue {
    fn new(code: &'static str, path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code,
            path: path.into(),
            message: message.into(),
        }
    }
}

#[derive(Debug)]
pub enum RouteFamilyIrError {
    Json(serde_json::Error),
    Validation(Vec<ValidationIssue>),
}

impl fmt::Display for RouteFamilyIrError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(error) => write!(formatter, "invalid route-family JSON: {error}"),
            Self::Validation(issues) => {
                write!(formatter, "invalid route-family portfolio")?;
                for issue in issues.iter().take(5) {
                    write!(
                        formatter,
                        "; {} at {}: {}",
                        issue.code, issue.path, issue.message
                    )?;
                }
                if issues.len() > 5 {
                    write!(formatter, "; and {} more", issues.len() - 5)?;
                }
                Ok(())
            }
        }
    }
}

impl Error for RouteFamilyIrError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Json(error) => Some(error),
            Self::Validation(_) => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RouteFamilyPortfolio {
    pub schema_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feasibility_mode: Option<RouteFamilyFeasibilityMode>,
    pub portfolio_id: String,
    pub placement_revision: u64,
    /// Required in V4 and absent from V2/V3 wire data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub geometry_revision: Option<u64>,
    pub corridor_epochs: Vec<CorridorEpoch>,
    pub family_generation: FamilyGenerationCertificate,
    pub passage_certificates: Vec<PassageCapacityCertificate>,
    pub variables: Vec<RouteFamilyVariable>,
    /// Required in V4. V2/V3 wire data omits this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixed_context: Option<FixedContextCertificate>,
    pub pair_analysis: PairAnalysisCertificate,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RouteFamilyFeasibilityMode {
    /// Legacy V2: scalar passage capacities conservatively constrain every
    /// branch width charged to a semantic passage.
    #[default]
    ExclusivePassageCapacity,
    /// V3: selected full-width witness geometry is authoritative. Every
    /// cross-electrical family pair is checked for crossings and copper
    /// clearance; scalar passage charging is disabled.
    FixedFullWidthWitnessPairwise,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CorridorEpoch {
    pub layer: String,
    pub corridor_revision: u64,
    pub semantic_fingerprint: String,
    pub cut_basis_fingerprint: String,
    pub cut_basis_complete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FamilyGenerationCertificate {
    pub method: String,
    pub max_families_per_variable: usize,
    pub search_complete: bool,
    pub budget_exhausted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PassageCapacityCertificate {
    pub layer: String,
    pub key: String,
    pub placement_revision: u64,
    pub corridor_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub available_width: Option<f64>,
    pub minimum_clearance: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enforcement: Option<PassageEnforcementMode>,
    pub capacity_model: String,
    pub certified: bool,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PassageEnforcementMode {
    #[default]
    ScalarWidth,
    FixedWitnessPairwise,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RouteFamilyVariable {
    /// Stable independent assignment identity. In production this is a route
    /// branch, not an electrical-net identity.
    pub branch: String,
    /// Copper identity used to exempt same-net geometry from pair conflicts.
    pub electrical_net: String,
    pub required_width: f64,
    /// Required edge-to-edge clearance for this branch's serialized copper.
    /// Required by fixed-witness V3; absent from legacy V2 wire data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_clearance: Option<f64>,
    pub families: Vec<RouteFamily>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RouteFamily {
    pub family_id: String,
    pub placement_revision: u64,
    pub complete_route: bool,
    pub cost: f64,
    pub runs: Vec<RouteFamilyRun>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RouteFamilyRun {
    pub run_id: String,
    pub run_index: usize,
    pub layer: String,
    pub start_point: usize,
    pub end_point: usize,
    pub corridor_revision: u64,
    pub transition_from_previous: RequiredNullable<RouteTransition>,
    pub witness: RouteWitness,
    pub cut_word_certificate: CutWordCertificate,
    pub semantic_passage_keys: Vec<String>,
    pub passage_mapping_complete: bool,
}

/// A nullable value whose key is nevertheless required during deserialization.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum RequiredNullable<T> {
    Null,
    Value(T),
}

impl<T> RequiredNullable<T> {
    fn as_ref(&self) -> Option<&T> {
        match self {
            Self::Null => None,
            Self::Value(value) => Some(value),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RouteTransition {
    pub kind: RouteTransitionKind,
    pub position: FamilyPoint,
    pub certified: bool,
    /// Physical annulus serialized by V5 for layer-changing transitions.
    /// Older schemas and same-layer transitions omit it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<RouteViaGeometry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RouteViaGeometry {
    pub diameter: f64,
    pub drill: f64,
    pub clearance_model: String,
    pub minimum_clearance: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RouteTransitionKind {
    Continuous,
    Via,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FamilyPoint {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RouteWitness {
    pub polyline: Vec<FamilyPoint>,
    pub placement_revision: u64,
    pub realizable: bool,
    pub clearance_certified: bool,
    pub clearance_model: String,
    pub minimum_clearance: f64,
    pub required_width: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CutWordCertificate {
    pub cut_basis_fingerprint: String,
    pub basis_complete: bool,
    pub verified: bool,
    pub word: Vec<CutEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CutEvent {
    pub obstacle: String,
    pub direction: CutDirection,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CutDirection {
    Positive,
    Negative,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PairAnalysisCertificate {
    pub method: String,
    pub placement_revision: u64,
    pub corridor_epoch_fingerprint: String,
    pub candidate_set_fingerprint: String,
    pub search_complete: bool,
    /// Required in fixed-witness V3 mode: number of cross-electrical family
    /// pairs classified, including compatible pairs absent from `conflicts`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub analyzed_family_pairs: Option<u64>,
    /// Required in V4 and deliberately separate from the candidate-set hash.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixed_context_fingerprint: Option<String>,
    /// Full cross-electrical candidate-family x context-run Cartesian count,
    /// including trivially clear different-layer pairs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub analyzed_family_context_pairs: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context_conflicts: Vec<FamilyContextConflict>,
    pub conflicts: Vec<FamilyConflict>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FixedContextCertificate {
    pub method: String,
    pub placement_revision: u64,
    pub geometry_revision: u64,
    pub fixed_context_fingerprint: String,
    pub search_complete: bool,
    pub runs: Vec<FixedContextRun>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FixedContextRun {
    pub context_id: String,
    pub branch: String,
    pub electrical_net: String,
    pub layer: String,
    pub required_width: f64,
    pub required_clearance: f64,
    pub polyline: Vec<FamilyPoint>,
    pub placement_revision: u64,
    pub geometry_revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FamilyContextConflict {
    pub family_id: String,
    pub context_id: String,
    pub kind: FamilyConflictKind,
    pub certified: bool,
    pub evidence: ConflictEvidence,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FamilyConflict {
    pub left_family_id: String,
    pub right_family_id: String,
    pub kind: FamilyConflictKind,
    pub certified: bool,
    pub evidence: ConflictEvidence,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FamilyConflictKind {
    CenterlineCrossing,
    CopperClearance,
    LaneOrder,
    TopologicalOrder,
    SharedVia,
}

impl FamilyConflictKind {
    /// Whether this witness incompatibility must be resolved before a
    /// discrete family assignment can be selected. A copper-clearance
    /// shortfall preserves the centerline topology and is therefore owned by
    /// continuous full-width repair plus final exact validation.
    pub const fn requires_discrete_separation(self) -> bool {
        !matches!(self, Self::CopperClearance)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ConflictEvidence {
    pub layer: RequiredNullable<String>,
    pub passage_key: RequiredNullable<String>,
    pub detail: String,
}

pub fn parse_and_validate_route_family_portfolio(
    json: &str,
) -> Result<RouteFamilyPortfolio, RouteFamilyIrError> {
    let portfolio: RouteFamilyPortfolio =
        serde_json::from_str(json).map_err(RouteFamilyIrError::Json)?;
    portfolio
        .validate()
        .map_err(RouteFamilyIrError::Validation)?;
    Ok(portfolio)
}

impl RouteFamilyPortfolio {
    fn is_fixed_witness_schema(&self) -> bool {
        self.schema_version == FIXED_WITNESS_ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION
            || self.schema_version == ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION
            || self.schema_version == MULTI_RUN_ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION
    }

    fn is_fixed_context_schema(&self) -> bool {
        self.schema_version == ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION
            || self.schema_version == MULTI_RUN_ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION
    }

    fn is_multi_run_schema(&self) -> bool {
        self.schema_version == MULTI_RUN_ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION
    }

    pub fn effective_feasibility_mode(&self) -> RouteFamilyFeasibilityMode {
        self.feasibility_mode
            .unwrap_or(RouteFamilyFeasibilityMode::ExclusivePassageCapacity)
    }

    pub fn validate(&self) -> Result<(), Vec<ValidationIssue>> {
        let issues = self.validation_issues();
        if issues.is_empty() {
            Ok(())
        } else {
            Err(issues)
        }
    }

    pub fn seal_pair_analysis_fingerprints(&mut self) -> Result<(), FingerprintError> {
        self.pair_analysis.candidate_set_fingerprint = self.candidate_set_fingerprint()?;
        self.pair_analysis.corridor_epoch_fingerprint = self.corridor_epoch_fingerprint();
        if self.is_fixed_context_schema() {
            let fingerprint = self.fixed_context_fingerprint()?;
            let context = self
                .fixed_context
                .as_mut()
                .ok_or_else(|| FingerprintError {
                    path: "$.fixed_context".into(),
                    message: "V4 requires fixed context before sealing fingerprints".into(),
                })?;
            context.fixed_context_fingerprint = fingerprint.clone();
            self.pair_analysis.fixed_context_fingerprint = Some(fingerprint);
        }
        Ok(())
    }

    /// Recompute exhaustive fixed-witness family/family and family/context
    /// conflicts from serialized copper, then bind the exact fingerprints.
    /// Experimental and external V3+ producers should finalize here rather
    /// than constructing a trusted sparse conflict certificate themselves.
    pub fn rebuild_and_seal_fixed_witness_analysis(&mut self) -> Result<(), Vec<ValidationIssue>> {
        if !self.is_fixed_witness_schema()
            || self.effective_feasibility_mode()
                != RouteFamilyFeasibilityMode::FixedFullWidthWitnessPairwise
        {
            return Err(vec![ValidationIssue::new(
                "fixed_witness_rebuild_mode_required",
                "$.feasibility_mode",
                "fixed-witness analysis can only be rebuilt for a V3+ fixed-witness portfolio",
            )]);
        }
        let mut issues = Vec::new();
        let (family_pairs, family_conflicts) = recompute_fixed_witness_pairs(self, &mut issues);
        let (context_pairs, context_conflicts) = recompute_fixed_context_pairs(self, &mut issues);
        if !issues.is_empty() {
            return Err(issues);
        }
        let mut family_conflicts = family_conflicts
            .into_iter()
            .map(
                |((left_family_id, right_family_id), conflict)| FamilyConflict {
                    left_family_id,
                    right_family_id,
                    kind: conflict.kind,
                    certified: true,
                    evidence: ConflictEvidence {
                        layer: RequiredNullable::Value(conflict.layer),
                        passage_key: RequiredNullable::Null,
                        detail: conflict.detail,
                    },
                },
            )
            .collect::<Vec<_>>();
        family_conflicts.sort_by(|left, right| {
            (&left.left_family_id, &left.right_family_id)
                .cmp(&(&right.left_family_id, &right.right_family_id))
        });
        let mut context_conflicts = context_conflicts
            .into_iter()
            .map(
                |((family_id, context_id), conflict)| FamilyContextConflict {
                    family_id,
                    context_id,
                    kind: conflict.kind,
                    certified: true,
                    evidence: ConflictEvidence {
                        layer: RequiredNullable::Value(conflict.layer),
                        passage_key: RequiredNullable::Null,
                        detail: conflict.detail,
                    },
                },
            )
            .collect::<Vec<_>>();
        context_conflicts.sort_by(|left, right| {
            (&left.family_id, &left.context_id).cmp(&(&right.family_id, &right.context_id))
        });
        self.pair_analysis.method = FIXED_WITNESS_PAIR_ANALYSIS_METHOD.into();
        self.pair_analysis.placement_revision = self.placement_revision;
        self.pair_analysis.search_complete = true;
        self.pair_analysis.analyzed_family_pairs = Some(family_pairs);
        self.pair_analysis.conflicts = family_conflicts;
        if self.is_fixed_context_schema() {
            self.pair_analysis.analyzed_family_context_pairs = Some(context_pairs);
            self.pair_analysis.context_conflicts = context_conflicts;
        } else {
            self.pair_analysis.analyzed_family_context_pairs = None;
            self.pair_analysis.context_conflicts.clear();
        }
        self.seal_pair_analysis_fingerprints().map_err(|error| {
            vec![ValidationIssue::new(
                "fixed_witness_rebuild_fingerprint_failed",
                error.path,
                error.message,
            )]
        })?;
        self.validate()
    }

    pub fn fixed_context_fingerprint(&self) -> Result<String, FingerprintError> {
        let context = self
            .fixed_context
            .as_ref()
            .ok_or_else(|| FingerprintError {
                path: "$.fixed_context".into(),
                message: "fixed-context fingerprint requires a context certificate".into(),
            })?;
        let mut runs = context.runs.iter().collect::<Vec<_>>();
        runs.sort_by(|left, right| left.context_id.cmp(&right.context_id));
        for run in &runs {
            if !run.required_width.is_finite() || !run.required_clearance.is_finite() {
                return Err(FingerprintError {
                    path: format!("$.fixed_context.runs[context_id={:?}]", run.context_id),
                    message: "context fingerprint cannot encode non-finite copper dimensions"
                        .into(),
                });
            }
            if run
                .polyline
                .iter()
                .any(|point| !point.x.is_finite() || !point.y.is_finite())
            {
                return Err(FingerprintError {
                    path: format!(
                        "$.fixed_context.runs[context_id={:?}].polyline",
                        run.context_id
                    ),
                    message: "context fingerprint cannot encode non-finite geometry".into(),
                });
            }
        }
        let value = Value::Array(vec![
            Value::String(FIXED_CONTEXT_FINGERPRINT_METHOD.into()),
            Value::String(context.method.clone()),
            Value::from(context.placement_revision),
            Value::from(context.geometry_revision),
            Value::Array(
                runs.into_iter()
                    .map(|run| {
                        serde_json::to_value(run).map_err(|error| FingerprintError {
                            path: format!("$.fixed_context.runs[context_id={:?}]", run.context_id),
                            message: format!("context run cannot be serialized: {error}"),
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            ),
        ]);
        Ok(canonical_hash(&value))
    }

    pub fn candidate_set_fingerprint(&self) -> Result<String, FingerprintError> {
        let mut candidates: Vec<_> = self
            .variables
            .iter()
            .flat_map(|variable| {
                variable.families.iter().map(move |family| {
                    (
                        variable.branch.as_str(),
                        variable.electrical_net.as_str(),
                        variable.required_width,
                        variable.required_clearance,
                        family.family_id.as_str(),
                        family,
                    )
                })
            })
            .collect();
        candidates.sort_by(|left, right| (left.0, left.4).cmp(&(right.0, right.4)));
        let values = candidates
            .into_iter()
            .map(
                |(branch, electrical_net, width, clearance, family_id, family)| {
                    if !width.is_finite() {
                        return Err(FingerprintError {
                            path: format!("$.variables[branch={branch:?}].required_width"),
                            message: "candidate fingerprint cannot encode a non-finite width"
                                .to_owned(),
                        });
                    }
                    if clearance.is_some_and(|value| !value.is_finite()) {
                        return Err(FingerprintError {
                            path: format!("$.variables[branch={branch:?}].required_clearance"),
                            message: "candidate fingerprint cannot encode a non-finite clearance"
                                .to_owned(),
                        });
                    }
                    ensure_family_finite(family)?;
                    let family_value =
                        serde_json::to_value(family).map_err(|error| FingerprintError {
                            path: format!(
                                "$.variables[branch={branch:?}].families[family_id={family_id:?}]"
                            ),
                            message: format!("candidate cannot be serialized: {error}"),
                        })?;
                    let mut fields = vec![
                        Value::String(branch.to_owned()),
                        Value::String(electrical_net.to_owned()),
                        Value::from(width),
                    ];
                    if self.is_fixed_witness_schema() {
                        fields.push(clearance.map_or(Value::Null, Value::from));
                    }
                    fields.push(Value::String(family_id.to_owned()));
                    fields.push(family_value);
                    Ok(Value::Array(fields))
                },
            )
            .collect::<Result<Vec<_>, FingerprintError>>()?;
        Ok(canonical_hash(&Value::Array(values)))
    }

    pub fn corridor_epoch_fingerprint(&self) -> String {
        let mut epochs: Vec<_> = self.corridor_epochs.iter().collect();
        epochs.sort_by(|left, right| {
            (
                left.layer.as_str(),
                left.corridor_revision,
                left.semantic_fingerprint.as_str(),
                left.cut_basis_fingerprint.as_str(),
                left.cut_basis_complete,
            )
                .cmp(&(
                    right.layer.as_str(),
                    right.corridor_revision,
                    right.semantic_fingerprint.as_str(),
                    right.cut_basis_fingerprint.as_str(),
                    right.cut_basis_complete,
                ))
        });
        let value = Value::Array(
            epochs
                .into_iter()
                .map(|epoch| {
                    Value::Array(vec![
                        Value::String(epoch.layer.clone()),
                        Value::from(epoch.corridor_revision),
                        Value::String(epoch.semantic_fingerprint.clone()),
                        Value::String(epoch.cut_basis_fingerprint.clone()),
                        Value::Bool(epoch.cut_basis_complete),
                    ])
                })
                .collect(),
        );
        canonical_hash(&value)
    }

    pub fn validation_issues(&self) -> Vec<ValidationIssue> {
        let mut issues = Vec::new();
        if self.schema_version != ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION
            && self.schema_version != MULTI_RUN_ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION
            && self.schema_version != FIXED_WITNESS_ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION
            && self.schema_version != LEGACY_ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION
        {
            issues.push(ValidationIssue::new(
                "schema_version_unsupported",
                "$.schema_version",
                "unsupported route-family portfolio version",
            ));
        }
        let fixed_schema = self.is_fixed_witness_schema();
        let v4 = self.is_fixed_context_schema();
        if v4 && self.geometry_revision.is_none() {
            issues.push(ValidationIssue::new(
                "v4_geometry_revision_missing",
                "$.geometry_revision",
                "V4 requires the current geometry revision for fixed-context provenance",
            ));
        }
        if !v4 && self.geometry_revision.is_some() {
            issues.push(ValidationIssue::new(
                "geometry_revision_schema_mismatch",
                "$.geometry_revision",
                "top-level geometry revision is authoritative V4 wire data",
            ));
        }
        if fixed_schema && self.feasibility_mode.is_none() {
            issues.push(ValidationIssue::new(
                "v3_feasibility_mode_missing",
                "$.feasibility_mode",
                "fixed-witness V3/V4 requires an explicit feasibility mode",
            ));
        }
        if fixed_schema
            && self.effective_feasibility_mode()
                != RouteFamilyFeasibilityMode::FixedFullWidthWitnessPairwise
        {
            issues.push(ValidationIssue::new(
                "v3_fixed_witness_mode_required",
                "$.feasibility_mode",
                "V3/V4 supports only fixed_full_width_witness_pairwise; scalar capacity is legacy V2",
            ));
        }
        if self.schema_version == LEGACY_ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION
            && self.effective_feasibility_mode()
                != RouteFamilyFeasibilityMode::ExclusivePassageCapacity
        {
            issues.push(ValidationIssue::new(
                "legacy_feasibility_mode",
                "$.feasibility_mode",
                "V2 portfolios support only legacy scalar passage capacity",
            ));
        }
        require_nonempty(&self.portfolio_id, "$.portfolio_id", &mut issues);
        if self.corridor_epochs.is_empty() {
            issues.push(ValidationIssue::new(
                "schema_min_items",
                "$.corridor_epochs",
                "at least one corridor epoch is required",
            ));
        }

        let mut epochs = HashMap::new();
        for (index, epoch) in self.corridor_epochs.iter().enumerate() {
            let path = format!("$.corridor_epochs[{index}]");
            require_nonempty(&epoch.layer, &format!("{path}.layer"), &mut issues);
            require_nonempty(
                &epoch.semantic_fingerprint,
                &format!("{path}.semantic_fingerprint"),
                &mut issues,
            );
            require_nonempty(
                &epoch.cut_basis_fingerprint,
                &format!("{path}.cut_basis_fingerprint"),
                &mut issues,
            );
            if epochs.insert(epoch.layer.as_str(), epoch).is_some() {
                issues.push(ValidationIssue::new(
                    "duplicate_corridor_layer",
                    format!("{path}.layer"),
                    "layer has more than one corridor epoch",
                ));
            }
            if !epoch.cut_basis_complete {
                issues.push(ValidationIssue::new(
                    "cut_basis_incomplete",
                    path,
                    "route-family admission requires a complete cut basis",
                ));
            }
        }

        let generation = &self.family_generation;
        require_nonempty(
            &generation.method,
            "$.family_generation.method",
            &mut issues,
        );
        if !(1..=16).contains(&generation.max_families_per_variable) {
            issues.push(ValidationIssue::new(
                if generation.max_families_per_variable == 0 {
                    "schema_minimum"
                } else {
                    "schema_maximum"
                },
                "$.family_generation.max_families_per_variable",
                "family bound must be between 1 and 16",
            ));
        }
        if !generation.search_complete {
            issues.push(ValidationIssue::new(
                "family_search_incomplete",
                "$.family_generation.search_complete",
                "bounded family enumeration must complete",
            ));
        }
        if generation.budget_exhausted {
            issues.push(ValidationIssue::new(
                "family_budget_exhausted",
                "$.family_generation.budget_exhausted",
                "an exhausted search is not a complete portfolio",
            ));
        }

        let mut passages = HashMap::new();
        for (index, certificate) in self.passage_certificates.iter().enumerate() {
            let path = format!("$.passage_certificates[{index}]");
            require_nonempty(&certificate.layer, &format!("{path}.layer"), &mut issues);
            require_nonempty(&certificate.key, &format!("{path}.key"), &mut issues);
            require_nonempty(
                &certificate.capacity_model,
                &format!("{path}.capacity_model"),
                &mut issues,
            );
            if fixed_schema && certificate.enforcement.is_none() {
                issues.push(ValidationIssue::new(
                    "v3_passage_enforcement_missing",
                    format!("{path}.enforcement"),
                    "fixed-witness V3/V4 requires explicit passage enforcement",
                ));
            }
            let enforcement = certificate
                .enforcement
                .unwrap_or(PassageEnforcementMode::ScalarWidth);
            match (self.effective_feasibility_mode(), enforcement) {
                (
                    RouteFamilyFeasibilityMode::ExclusivePassageCapacity,
                    PassageEnforcementMode::ScalarWidth,
                ) => match certificate.available_width {
                    Some(width) => require_positive_finite(
                        width,
                        &format!("{path}.available_width"),
                        &mut issues,
                    ),
                    None => issues.push(ValidationIssue::new(
                        "scalar_capacity_missing",
                        format!("{path}.available_width"),
                        "legacy scalar passage enforcement requires an available width",
                    )),
                },
                (
                    RouteFamilyFeasibilityMode::FixedFullWidthWitnessPairwise,
                    PassageEnforcementMode::FixedWitnessPairwise,
                ) => {
                    if certificate.capacity_model
                        != "fixed_full_width_witness_pairwise/no_scalar_capacity/v3"
                    {
                        issues.push(ValidationIssue::new(
                            "fixed_witness_capacity_model_unsupported",
                            format!("{path}.capacity_model"),
                            "fixed-witness V3 requires the exact no-scalar capacity proof model",
                        ));
                    }
                    if certificate.available_width.is_some() {
                        issues.push(ValidationIssue::new(
                            "fixed_witness_scalar_capacity",
                            format!("{path}.available_width"),
                            "fixed-witness pairwise enforcement must not invent a scalar passage width",
                        ));
                    }
                }
                _ => issues.push(ValidationIssue::new(
                    "passage_enforcement_mode_mismatch",
                    format!("{path}.enforcement"),
                    "passage enforcement must match the portfolio feasibility mode",
                )),
            }
            require_nonnegative_finite(
                certificate.minimum_clearance,
                &format!("{path}.minimum_clearance"),
                &mut issues,
            );
            let key = (certificate.layer.as_str(), certificate.key.as_str());
            if passages.insert(key, certificate).is_some() {
                issues.push(ValidationIssue::new(
                    "duplicate_passage_certificate",
                    path.clone(),
                    "stable passage certificate is duplicated",
                ));
            }
            match epochs.get(certificate.layer.as_str()) {
                None => issues.push(ValidationIssue::new(
                    "passage_layer_missing",
                    format!("{path}.layer"),
                    "no corridor epoch exists for this passage layer",
                )),
                Some(epoch) if certificate.corridor_revision != epoch.corridor_revision => {
                    issues.push(ValidationIssue::new(
                        "stale_passage_corridor_revision",
                        format!("{path}.corridor_revision"),
                        "passage capacity belongs to a different corridor epoch",
                    ));
                }
                Some(_) => {}
            }
            if certificate.placement_revision != self.placement_revision {
                issues.push(ValidationIssue::new(
                    "stale_passage_placement_revision",
                    format!("{path}.placement_revision"),
                    "passage capacity belongs to a different placement",
                ));
            }
            if !certificate.certified {
                issues.push(ValidationIssue::new(
                    "capacity_uncertified",
                    format!("{path}.certified"),
                    "advisory capacity cannot admit a family",
                ));
            }
        }

        if self.variables.is_empty() {
            issues.push(ValidationIssue::new(
                "schema_min_items",
                "$.variables",
                "at least one branch variable is required",
            ));
        }
        let mut branch_ids = HashSet::new();
        let mut family_to_variable = HashMap::new();
        let mut run_ids = HashSet::new();
        for (variable_index, variable) in self.variables.iter().enumerate() {
            let variable_path = format!("$.variables[{variable_index}]");
            require_nonempty(
                &variable.branch,
                &format!("{variable_path}.branch"),
                &mut issues,
            );
            require_nonempty(
                &variable.electrical_net,
                &format!("{variable_path}.electrical_net"),
                &mut issues,
            );
            require_positive_finite(
                variable.required_width,
                &format!("{variable_path}.required_width"),
                &mut issues,
            );
            if fixed_schema {
                match variable.required_clearance {
                    Some(clearance) => require_nonnegative_finite(
                        clearance,
                        &format!("{variable_path}.required_clearance"),
                        &mut issues,
                    ),
                    None => issues.push(ValidationIssue::new(
                        "v3_required_clearance_missing",
                        format!("{variable_path}.required_clearance"),
                        "fixed-witness V3/V4 requires serialized branch clearance",
                    )),
                }
            } else if let Some(clearance) = variable.required_clearance {
                require_nonnegative_finite(
                    clearance,
                    &format!("{variable_path}.required_clearance"),
                    &mut issues,
                );
            }
            if !branch_ids.insert(variable.branch.as_str()) {
                issues.push(ValidationIssue::new(
                    "duplicate_branch",
                    format!("{variable_path}.branch"),
                    "branch assignment identity must be unique",
                ));
            }
            if variable.families.is_empty() {
                issues.push(ValidationIssue::new(
                    "schema_min_items",
                    format!("{variable_path}.families"),
                    "each branch variable needs at least one family",
                ));
            }
            if variable.families.len() > 16 {
                issues.push(ValidationIssue::new(
                    "schema_max_items",
                    format!("{variable_path}.families"),
                    "at most 16 families may be serialized per branch variable",
                ));
            }
            if variable.families.len() > generation.max_families_per_variable {
                issues.push(ValidationIssue::new(
                    "family_limit_exceeded",
                    format!("{variable_path}.families"),
                    "serialized frontier exceeds its declared bound",
                ));
            }
            for (family_index, family) in variable.families.iter().enumerate() {
                let family_path = format!("{variable_path}.families[{family_index}]");
                require_nonempty(
                    &family.family_id,
                    &format!("{family_path}.family_id"),
                    &mut issues,
                );
                if family_to_variable
                    .insert(
                        family.family_id.as_str(),
                        (variable.branch.as_str(), variable.electrical_net.as_str()),
                    )
                    .is_some()
                {
                    issues.push(ValidationIssue::new(
                        "duplicate_family_id",
                        format!("{family_path}.family_id"),
                        "family IDs are global conflict-edge identities",
                    ));
                }
                if family.placement_revision != self.placement_revision {
                    issues.push(ValidationIssue::new(
                        "stale_family_placement_revision",
                        format!("{family_path}.placement_revision"),
                        "family witness belongs to a different placement",
                    ));
                }
                if !family.complete_route {
                    issues.push(ValidationIssue::new(
                        "route_family_incomplete",
                        format!("{family_path}.complete_route"),
                        "assignment candidates must cover a complete net route",
                    ));
                }
                require_nonnegative_finite(
                    family.cost,
                    &format!("{family_path}.cost"),
                    &mut issues,
                );
                if family.runs.is_empty() {
                    issues.push(ValidationIssue::new(
                        "schema_min_items",
                        format!("{family_path}.runs"),
                        "a complete route needs at least one run",
                    ));
                }
                if self.effective_feasibility_mode()
                    == RouteFamilyFeasibilityMode::FixedFullWidthWitnessPairwise
                    && family.runs.len() != 1
                    && !self.is_multi_run_schema()
                {
                    issues.push(ValidationIssue::new(
                        "fixed_witness_requires_single_via_free_run",
                        format!("{family_path}.runs"),
                        "fixed-witness V3/V4 admits exactly one complete via-free run",
                    ));
                }
                let mut ordered_runs: Vec<_> = family.runs.iter().collect();
                ordered_runs.sort_by_key(|run| run.run_index);
                if ordered_runs
                    .iter()
                    .map(|run| run.run_index)
                    .ne(0..ordered_runs.len())
                {
                    issues.push(ValidationIssue::new(
                        "run_indices_not_contiguous",
                        format!("{family_path}.runs"),
                        "run indices must be exactly 0..N-1",
                    ));
                }
                for (run_index, run) in ordered_runs.iter().enumerate() {
                    let run_path = format!("{family_path}.runs[{run_index}]");
                    require_nonempty(&run.run_id, &format!("{run_path}.run_id"), &mut issues);
                    require_nonempty(&run.layer, &format!("{run_path}.layer"), &mut issues);
                    if !run_ids.insert(run.run_id.as_str()) {
                        issues.push(ValidationIssue::new(
                            "duplicate_run_id",
                            format!("{run_path}.run_id"),
                            "run IDs must be globally unique",
                        ));
                    }
                    match epochs.get(run.layer.as_str()) {
                        None => issues.push(ValidationIssue::new(
                            "run_layer_missing",
                            format!("{run_path}.layer"),
                            "no corridor epoch exists for this run layer",
                        )),
                        Some(epoch) if run.corridor_revision != epoch.corridor_revision => {
                            issues.push(ValidationIssue::new(
                                "stale_run_corridor_revision",
                                format!("{run_path}.corridor_revision"),
                                "run belongs to a different corridor epoch",
                            ));
                        }
                        Some(_) => {}
                    }
                    if run.end_point <= run.start_point {
                        issues.push(ValidationIssue::new(
                            "route_run_point_range_empty",
                            format!("{run_path}.end_point"),
                            "run point range must advance through the route",
                        ));
                    }
                    let covered_points = run
                        .end_point
                        .checked_sub(run.start_point)
                        .and_then(|span| span.checked_add(1));
                    if self.is_multi_run_schema()
                        && covered_points != Some(run.witness.polyline.len())
                    {
                        issues.push(ValidationIssue::new(
                            "route_run_point_range_witness_mismatch",
                            format!("{run_path}.witness.polyline"),
                            "V5 run point range must exactly cover its serialized witness",
                        ));
                    }
                    if self.is_multi_run_schema() && run_index == 0 && run.start_point != 0 {
                        issues.push(ValidationIssue::new(
                            "route_first_run_nonzero_start",
                            format!("{run_path}.start_point"),
                            "V5 complete routes must start at point zero",
                        ));
                    }
                    validate_witness(
                        &run.witness,
                        variable.required_width,
                        variable.required_clearance,
                        self.effective_feasibility_mode(),
                        self.placement_revision,
                        &run_path,
                        &mut issues,
                    );
                    validate_cut_word(
                        &run.cut_word_certificate,
                        epochs.get(run.layer.as_str()).copied(),
                        &run_path,
                        &mut issues,
                    );
                    let mut passage_keys = HashSet::new();
                    for passage_key in &run.semantic_passage_keys {
                        require_nonempty(
                            passage_key,
                            &format!("{run_path}.semantic_passage_keys"),
                            &mut issues,
                        );
                        if !passage_keys.insert(passage_key.as_str()) {
                            issues.push(ValidationIssue::new(
                                "schema_unique_items",
                                format!("{run_path}.semantic_passage_keys"),
                                "semantic passage keys must be unique",
                            ));
                        }
                        if !passages.contains_key(&(run.layer.as_str(), passage_key.as_str())) {
                            issues.push(ValidationIssue::new(
                                "missing_passage_certificate",
                                format!("{run_path}.semantic_passage_keys"),
                                "no capacity certificate exists for this passage",
                            ));
                        }
                    }
                    if !run.passage_mapping_complete {
                        issues.push(ValidationIssue::new(
                            "passage_mapping_incomplete",
                            format!("{run_path}.passage_mapping_complete"),
                            "all candidate passages must be serialized",
                        ));
                    }
                    validate_transition(
                        run,
                        ordered_runs.get(run_index.wrapping_sub(1)).copied(),
                        run_index,
                        variable.required_clearance,
                        &run_path,
                        &family_path,
                        &mut issues,
                    );
                }
            }
        }

        if family_to_variable.is_empty() {
            issues.push(ValidationIssue::new(
                "empty_portfolio",
                "$.variables",
                "at least one candidate family is required",
            ));
        }
        validate_fixed_context(self, v4, &epochs, &family_to_variable, &mut issues);
        validate_pair_analysis(self, &epochs, &passages, &family_to_variable, &mut issues);
        issues
    }
}

fn validate_witness(
    witness: &RouteWitness,
    net_width: f64,
    required_clearance: Option<f64>,
    feasibility_mode: RouteFamilyFeasibilityMode,
    placement_revision: u64,
    run_path: &str,
    issues: &mut Vec<ValidationIssue>,
) {
    let path = format!("{run_path}.witness");
    if witness.polyline.len() < 2 {
        issues.push(ValidationIssue::new(
            "schema_min_items",
            format!("{path}.polyline"),
            "witness polyline needs at least two points",
        ));
    }
    for (index, point) in witness.polyline.iter().enumerate() {
        require_finite(point.x, &format!("{path}.polyline[{index}].x"), issues);
        require_finite(point.y, &format!("{path}.polyline[{index}].y"), issues);
    }
    if witness.placement_revision != placement_revision {
        issues.push(ValidationIssue::new(
            "stale_witness_placement_revision",
            format!("{path}.placement_revision"),
            "witness belongs to a different placement",
        ));
    }
    if !witness.realizable {
        issues.push(ValidationIssue::new(
            "witness_unrealizable",
            format!("{path}.realizable"),
            "candidate needs a realizable geometric witness",
        ));
    }
    if !witness.clearance_certified {
        issues.push(ValidationIssue::new(
            "clearance_uncertified",
            format!("{path}.clearance_certified"),
            "centerline seeds are insufficient for admission",
        ));
    }
    require_nonempty(
        &witness.clearance_model,
        &format!("{path}.clearance_model"),
        issues,
    );
    require_nonnegative_finite(
        witness.minimum_clearance,
        &format!("{path}.minimum_clearance"),
        issues,
    );
    require_positive_finite(
        witness.required_width,
        &format!("{path}.required_width"),
        issues,
    );
    if witness.required_width != net_width {
        issues.push(ValidationIssue::new(
            "witness_width_mismatch",
            format!("{path}.required_width"),
            "witness width differs from its net demand",
        ));
    }
    if feasibility_mode == RouteFamilyFeasibilityMode::FixedFullWidthWitnessPairwise {
        if witness.clearance_model != "exact_polyline_against_polygonal_obstacles" {
            issues.push(ValidationIssue::new(
                "fixed_witness_clearance_model_unsupported",
                format!("{path}.clearance_model"),
                "fixed-witness V3 requires exact polygonal obstacle/board clearance",
            ));
        }
        if let Some(clearance) = required_clearance {
            let required_radius = net_width * 0.5 + clearance;
            if !required_radius.is_finite() {
                issues.push(ValidationIssue::new(
                    "fixed_witness_required_radius_nonfinite",
                    format!("{path}.minimum_clearance"),
                    "required width/clearance radius overflowed",
                ));
            } else if witness.minimum_clearance + EPSILON < required_radius {
                issues.push(ValidationIssue::new(
                    "fixed_witness_minimum_clearance_shortfall",
                    format!("{path}.minimum_clearance"),
                    format!(
                        "minimum centerline clearance {} is below required radius {required_radius}",
                        witness.minimum_clearance
                    ),
                ));
            }
        }
    }
}

fn validate_cut_word(
    cut: &CutWordCertificate,
    epoch: Option<&CorridorEpoch>,
    run_path: &str,
    issues: &mut Vec<ValidationIssue>,
) {
    let path = format!("{run_path}.cut_word_certificate");
    require_nonempty(
        &cut.cut_basis_fingerprint,
        &format!("{path}.cut_basis_fingerprint"),
        issues,
    );
    if !cut.basis_complete {
        issues.push(ValidationIssue::new(
            "cut_basis_incomplete",
            format!("{path}.basis_complete"),
            "cut-word certificate marks its basis incomplete",
        ));
    }
    if !cut.verified {
        issues.push(ValidationIssue::new(
            "homotopy_unverified",
            format!("{path}.verified"),
            "cut word must be verified against this epoch",
        ));
    }
    if let Some(epoch) = epoch {
        if cut.cut_basis_fingerprint != epoch.cut_basis_fingerprint {
            issues.push(ValidationIssue::new(
                "cut_basis_fingerprint_mismatch",
                format!("{path}.cut_basis_fingerprint"),
                "cut word is scoped to a different basis",
            ));
        }
    }
    for (index, event) in cut.word.iter().enumerate() {
        require_nonempty(
            &event.obstacle,
            &format!("{path}.word[{index}].obstacle"),
            issues,
        );
    }
    if cut
        .word
        .windows(2)
        .any(|pair| pair[0].obstacle == pair[1].obstacle && pair[0].direction != pair[1].direction)
    {
        issues.push(ValidationIssue::new(
            "cut_word_not_reduced",
            format!("{path}.word"),
            "adjacent inverse cut events must be cancelled",
        ));
    }
}

fn validate_transition(
    run: &RouteFamilyRun,
    previous: Option<&RouteFamilyRun>,
    run_index: usize,
    required_clearance: Option<f64>,
    run_path: &str,
    family_path: &str,
    issues: &mut Vec<ValidationIssue>,
) {
    let transition = run.transition_from_previous.as_ref();
    if run_index == 0 {
        if transition.is_some() {
            issues.push(ValidationIssue::new(
                "first_run_has_transition",
                format!("{run_path}.transition_from_previous"),
                "the first run cannot transition from a predecessor",
            ));
        }
        return;
    }
    let Some(previous) = previous else {
        return;
    };
    let Some(transition) = transition else {
        issues.push(ValidationIssue::new(
            "run_transition_missing",
            format!("{run_path}.transition_from_previous"),
            "composed runs need a certified transition",
        ));
        if previous.end_point != run.start_point {
            disconnected_ranges(family_path, issues);
        }
        return;
    };
    let expected = if previous.layer == run.layer {
        RouteTransitionKind::Continuous
    } else {
        RouteTransitionKind::Via
    };
    if transition.kind != expected {
        issues.push(ValidationIssue::new(
            "run_transition_kind_mismatch",
            format!("{run_path}.transition_from_previous.kind"),
            "transition kind does not match adjacent layers",
        ));
    }
    if !transition.certified {
        issues.push(ValidationIssue::new(
            "run_transition_uncertified",
            format!("{run_path}.transition_from_previous.certified"),
            "layer/run transition must be realizable",
        ));
    }
    if transition.kind == RouteTransitionKind::Via && run_index > 0 {
        match transition.via.as_ref() {
            Some(via) => {
                require_positive_finite(
                    via.diameter,
                    &format!("{run_path}.transition_from_previous.via.diameter"),
                    issues,
                );
                require_positive_finite(
                    via.drill,
                    &format!("{run_path}.transition_from_previous.via.drill"),
                    issues,
                );
                if via.drill > via.diameter {
                    issues.push(ValidationIssue::new(
                        "run_transition_via_drill_exceeds_diameter",
                        format!("{run_path}.transition_from_previous.via.drill"),
                        "via drill must not exceed its copper diameter",
                    ));
                }
                if via.clearance_model != EXACT_VIA_CLEARANCE_MODEL {
                    issues.push(ValidationIssue::new(
                        "run_transition_via_clearance_model_unsupported",
                        format!("{run_path}.transition_from_previous.via.clearance_model"),
                        "V5 vias require the exact point-against-polygonal-obstacles model",
                    ));
                }
                require_nonnegative_finite(
                    via.minimum_clearance,
                    &format!("{run_path}.transition_from_previous.via.minimum_clearance"),
                    issues,
                );
                if let Some(required_clearance) = required_clearance {
                    let required_radius = via.diameter * 0.5 + required_clearance;
                    if required_radius.is_finite()
                        && via.minimum_clearance + EPSILON < required_radius
                    {
                        issues.push(ValidationIssue::new(
                            "run_transition_via_clearance_shortfall",
                            format!("{run_path}.transition_from_previous.via.minimum_clearance"),
                            "serialized via clearance is smaller than its copper radius plus required clearance",
                        ));
                    }
                }
            }
            None => issues.push(ValidationIssue::new(
                "run_transition_via_geometry_missing",
                format!("{run_path}.transition_from_previous.via"),
                "V5 layer-changing transitions require physical via geometry",
            )),
        }
    } else if transition.via.is_some() {
        issues.push(ValidationIssue::new(
            "run_transition_unexpected_via_geometry",
            format!("{run_path}.transition_from_previous.via"),
            "same-layer transitions must not serialize via geometry",
        ));
    }
    let previous_end = previous.witness.polyline.last();
    let current_start = run.witness.polyline.first();
    if previous_end.is_none()
        || current_start.is_none()
        || previous_end.is_some_and(|point| !same_point(point, &transition.position))
        || current_start.is_some_and(|point| !same_point(point, &transition.position))
    {
        issues.push(ValidationIssue::new(
            "run_transition_position_mismatch",
            format!("{run_path}.transition_from_previous.position"),
            "transition must coincide with both witness endpoints",
        ));
    }
    if previous.end_point != run.start_point {
        disconnected_ranges(family_path, issues);
    }
}

fn disconnected_ranges(family_path: &str, issues: &mut Vec<ValidationIssue>) {
    issues.push(ValidationIssue::new(
        "route_run_point_range_disconnected",
        format!("{family_path}.runs"),
        "adjacent run point ranges do not meet",
    ));
}

fn validate_fixed_context(
    portfolio: &RouteFamilyPortfolio,
    v4: bool,
    epochs: &HashMap<&str, &CorridorEpoch>,
    family_to_variable: &HashMap<&str, (&str, &str)>,
    issues: &mut Vec<ValidationIssue>,
) {
    let Some(context) = portfolio.fixed_context.as_ref() else {
        if v4 {
            issues.push(ValidationIssue::new(
                "v4_fixed_context_missing",
                "$.fixed_context",
                "V4 requires authoritative fixed outside-component copper",
            ));
        }
        return;
    };
    if !v4 {
        issues.push(ValidationIssue::new(
            "fixed_context_schema_mismatch",
            "$.fixed_context",
            "fixed context is authoritative V4 wire data and cannot appear in V2/V3",
        ));
    }
    if context.method != FIXED_CONTEXT_ANALYSIS_METHOD {
        issues.push(ValidationIssue::new(
            "fixed_context_method_unsupported",
            "$.fixed_context.method",
            "V4 requires the exact serialized fixed-context classifier",
        ));
    }
    if context.placement_revision != portfolio.placement_revision {
        issues.push(ValidationIssue::new(
            "stale_fixed_context_placement_revision",
            "$.fixed_context.placement_revision",
            "fixed context belongs to a different placement",
        ));
    }
    if Some(context.geometry_revision) != portfolio.geometry_revision {
        issues.push(ValidationIssue::new(
            "stale_fixed_context_geometry_revision",
            "$.fixed_context.geometry_revision",
            "fixed context belongs to a different geometry revision",
        ));
    }
    if !context.search_complete {
        issues.push(ValidationIssue::new(
            "fixed_context_incomplete",
            "$.fixed_context.search_complete",
            "omitted outside copper cannot constrain assignment safely",
        ));
    }
    require_nonempty(
        &context.fixed_context_fingerprint,
        "$.fixed_context.fixed_context_fingerprint",
        issues,
    );
    let mut ids = HashSet::new();
    let selected_branches = family_to_variable
        .values()
        .map(|(branch, _)| *branch)
        .collect::<HashSet<_>>();
    let mut context_branch_nets = HashMap::new();
    for (index, run) in context.runs.iter().enumerate() {
        let path = format!("$.fixed_context.runs[{index}]");
        require_nonempty(&run.context_id, &format!("{path}.context_id"), issues);
        require_nonempty(&run.branch, &format!("{path}.branch"), issues);
        require_nonempty(
            &run.electrical_net,
            &format!("{path}.electrical_net"),
            issues,
        );
        require_nonempty(&run.layer, &format!("{path}.layer"), issues);
        if !epochs.contains_key(run.layer.as_str()) {
            issues.push(ValidationIssue::new(
                "fixed_context_layer_missing",
                format!("{path}.layer"),
                "fixed context copper references a layer without a current corridor epoch",
            ));
        }
        if !ids.insert(run.context_id.as_str()) {
            issues.push(ValidationIssue::new(
                "duplicate_fixed_context_id",
                format!("{path}.context_id"),
                "fixed context run IDs must be globally unique",
            ));
        }
        if selected_branches.contains(run.branch.as_str()) {
            issues.push(ValidationIssue::new(
                "fixed_context_selected_branch_overlap",
                format!("{path}.branch"),
                "selected-component copper cannot also be authoritative fixed outside context",
            ));
        }
        if context_branch_nets
            .insert(run.branch.as_str(), run.electrical_net.as_str())
            .is_some_and(|prior| prior != run.electrical_net.as_str())
        {
            issues.push(ValidationIssue::new(
                "fixed_context_branch_net_mismatch",
                format!("{path}.electrical_net"),
                "all runs and via annuli for one outside branch must share one electrical net",
            ));
        }
        require_positive_finite(
            run.required_width,
            &format!("{path}.required_width"),
            issues,
        );
        require_nonnegative_finite(
            run.required_clearance,
            &format!("{path}.required_clearance"),
            issues,
        );
        if run.polyline.is_empty() {
            issues.push(ValidationIssue::new(
                "schema_min_items",
                format!("{path}.polyline"),
                "fixed context copper needs at least one point",
            ));
        }
        for (point_index, point) in run.polyline.iter().enumerate() {
            require_finite(
                point.x,
                &format!("{path}.polyline[{point_index}].x"),
                issues,
            );
            require_finite(
                point.y,
                &format!("{path}.polyline[{point_index}].y"),
                issues,
            );
        }
        if run.placement_revision != context.placement_revision {
            issues.push(ValidationIssue::new(
                "stale_fixed_context_run_placement_revision",
                format!("{path}.placement_revision"),
                "fixed context run belongs to a different placement",
            ));
        }
        if run.geometry_revision != context.geometry_revision {
            issues.push(ValidationIssue::new(
                "stale_fixed_context_run_geometry_revision",
                format!("{path}.geometry_revision"),
                "fixed context run belongs to a different geometry revision",
            ));
        }
    }
    match portfolio.fixed_context_fingerprint() {
        Ok(expected) if expected != context.fixed_context_fingerprint => {
            issues.push(ValidationIssue::new(
                "fixed_context_fingerprint_mismatch",
                "$.fixed_context.fixed_context_fingerprint",
                "fixed context fingerprint does not cover these exact outside runs",
            ));
        }
        Err(error) => issues.push(ValidationIssue::new(
            "fixed_context_fingerprint_unavailable",
            error.path,
            error.message,
        )),
        Ok(_) => {}
    }
}

fn validate_pair_analysis<'a>(
    portfolio: &'a RouteFamilyPortfolio,
    epochs: &HashMap<&'a str, &'a CorridorEpoch>,
    passages: &HashMap<(&'a str, &'a str), &'a PassageCapacityCertificate>,
    family_to_variable: &HashMap<&'a str, (&'a str, &'a str)>,
    issues: &mut Vec<ValidationIssue>,
) {
    let pair = &portfolio.pair_analysis;
    let fixed_mode = portfolio.effective_feasibility_mode()
        == RouteFamilyFeasibilityMode::FixedFullWidthWitnessPairwise;
    let fixed_schema = portfolio.is_fixed_witness_schema();
    let v4 = portfolio.is_fixed_context_schema();
    require_nonempty(&pair.method, "$.pair_analysis.method", issues);
    if fixed_mode && pair.method != FIXED_WITNESS_PAIR_ANALYSIS_METHOD {
        issues.push(ValidationIssue::new(
            "fixed_witness_pair_method_unsupported",
            "$.pair_analysis.method",
            "fixed-witness V3/V4 requires the exact serialized-run pair classifier",
        ));
    }
    if fixed_schema && pair.analyzed_family_pairs.is_none() {
        issues.push(ValidationIssue::new(
            "v3_analyzed_family_pairs_missing",
            "$.pair_analysis.analyzed_family_pairs",
            "fixed-witness V3/V4 requires an explicit analyzed family-pair count",
        ));
    }
    if v4 && pair.fixed_context_fingerprint.is_none() {
        issues.push(ValidationIssue::new(
            "v4_fixed_context_fingerprint_missing",
            "$.pair_analysis.fixed_context_fingerprint",
            "V4 pair analysis must bind the authoritative outside copper",
        ));
    }
    if v4 && pair.analyzed_family_context_pairs.is_none() {
        issues.push(ValidationIssue::new(
            "v4_analyzed_family_context_pairs_missing",
            "$.pair_analysis.analyzed_family_context_pairs",
            "V4 requires an explicit full family-context Cartesian count",
        ));
    }
    if !v4
        && (pair.fixed_context_fingerprint.is_some()
            || pair.analyzed_family_context_pairs.is_some()
            || !pair.context_conflicts.is_empty())
    {
        issues.push(ValidationIssue::new(
            "fixed_context_pair_fields_schema_mismatch",
            "$.pair_analysis",
            "fixed-context pair fields are authoritative V4 wire data",
        ));
    }
    require_nonempty(
        &pair.corridor_epoch_fingerprint,
        "$.pair_analysis.corridor_epoch_fingerprint",
        issues,
    );
    require_nonempty(
        &pair.candidate_set_fingerprint,
        "$.pair_analysis.candidate_set_fingerprint",
        issues,
    );
    if pair.placement_revision != portfolio.placement_revision {
        issues.push(ValidationIssue::new(
            "stale_pair_placement_revision",
            "$.pair_analysis.placement_revision",
            "pair certificate belongs to a different placement",
        ));
    }
    match portfolio.candidate_set_fingerprint() {
        Ok(fingerprint) if pair.candidate_set_fingerprint != fingerprint => {
            issues.push(ValidationIssue::new(
                "candidate_set_fingerprint_mismatch",
                "$.pair_analysis.candidate_set_fingerprint",
                "pair certificate does not cover this exact candidate frontier",
            ));
        }
        Err(error) => issues.push(ValidationIssue::new(
            "candidate_set_fingerprint_unavailable",
            error.path,
            error.message,
        )),
        Ok(_) => {}
    }
    if pair.corridor_epoch_fingerprint != portfolio.corridor_epoch_fingerprint() {
        issues.push(ValidationIssue::new(
            "corridor_epoch_fingerprint_mismatch",
            "$.pair_analysis.corridor_epoch_fingerprint",
            "pair certificate belongs to different corridor/cut epochs",
        ));
    }
    if !pair.search_complete {
        issues.push(ValidationIssue::new(
            "pair_analysis_incomplete",
            "$.pair_analysis.search_complete",
            "unlisted candidate pairs are safe only after complete analysis",
        ));
    }
    if v4 {
        let expected_context_fingerprint = portfolio
            .fixed_context
            .as_ref()
            .map(|context| context.fixed_context_fingerprint.as_str());
        if pair.fixed_context_fingerprint.as_deref() != expected_context_fingerprint {
            issues.push(ValidationIssue::new(
                "pair_fixed_context_fingerprint_mismatch",
                "$.pair_analysis.fixed_context_fingerprint",
                "pair analysis is bound to different outside copper",
            ));
        }
    }
    let (expected_pairs, expected_fixed_conflicts) = if fixed_mode {
        recompute_fixed_witness_pairs(portfolio, issues)
    } else {
        (0, HashMap::new())
    };
    if fixed_mode {
        if pair.analyzed_family_pairs != Some(expected_pairs) {
            issues.push(ValidationIssue::new(
                "fixed_witness_pair_matrix_incomplete",
                "$.pair_analysis.analyzed_family_pairs",
                format!(
                    "fixed-witness mode requires all {expected_pairs} cross-electrical family pairs; certificate records {:?}",
                    pair.analyzed_family_pairs
                ),
            ));
        }
    }
    let (expected_context_pairs, expected_context_conflicts) = if v4 {
        recompute_fixed_context_pairs(portfolio, issues)
    } else {
        (0, HashMap::new())
    };
    if v4 && pair.analyzed_family_context_pairs != Some(expected_context_pairs) {
        issues.push(ValidationIssue::new(
            "fixed_context_pair_matrix_incomplete",
            "$.pair_analysis.analyzed_family_context_pairs",
            format!(
                "V4 requires all {expected_context_pairs} cross-electrical family-context pairs; certificate records {:?}",
                pair.analyzed_family_context_pairs
            ),
        ));
    }
    let mut seen_edges = HashSet::new();
    for (index, conflict) in pair.conflicts.iter().enumerate() {
        let path = format!("$.pair_analysis.conflicts[{index}]");
        require_nonempty(
            &conflict.left_family_id,
            &format!("{path}.left_family_id"),
            issues,
        );
        require_nonempty(
            &conflict.right_family_id,
            &format!("{path}.right_family_id"),
            issues,
        );
        require_nonempty(
            &conflict.evidence.detail,
            &format!("{path}.evidence.detail"),
            issues,
        );
        let left_variable = family_to_variable.get(conflict.left_family_id.as_str());
        let right_variable = family_to_variable.get(conflict.right_family_id.as_str());
        if left_variable.is_none() || right_variable.is_none() {
            issues.push(ValidationIssue::new(
                "conflict_family_missing",
                path.clone(),
                "conflict endpoint is absent from this candidate set",
            ));
            continue;
        }
        if left_variable.is_some_and(|left| right_variable.is_some_and(|right| left.1 == right.1)) {
            issues.push(ValidationIssue::new(
                "conflict_same_electrical_net",
                path.clone(),
                "same-electrical-net conflict edges are meaningless",
            ));
        }
        let (left, right) = if conflict.left_family_id <= conflict.right_family_id {
            (&conflict.left_family_id, &conflict.right_family_id)
        } else {
            (&conflict.right_family_id, &conflict.left_family_id)
        };
        if !seen_edges.insert((left.as_str(), right.as_str())) {
            issues.push(ValidationIssue::new(
                "duplicate_conflict_edge",
                path.clone(),
                "conflict edge is duplicated",
            ));
        }
        if !conflict.certified {
            issues.push(ValidationIssue::new(
                "conflict_uncertified",
                format!("{path}.certified"),
                "diagnostic conflict evidence cannot constrain admission",
            ));
        }
        if conflict.kind == FamilyConflictKind::CopperClearance && !fixed_mode {
            issues.push(ValidationIssue::new(
                "copper_clearance_conflict_mode",
                format!("{path}.kind"),
                "copper-clearance conflicts require fixed-full-width-witness mode",
            ));
        }
        if fixed_mode {
            match expected_fixed_conflicts.get(&(left.to_string(), right.to_string())) {
                Some(expected) if expected.kind == conflict.kind => {
                    let exact_layer = conflict.evidence.layer.as_ref() == Some(&expected.layer);
                    let null_passage = conflict.evidence.passage_key.as_ref().is_none();
                    if !exact_layer || !null_passage || conflict.evidence.detail != expected.detail
                    {
                        issues.push(ValidationIssue::new(
                            "fixed_witness_conflict_evidence_mismatch",
                            format!("{path}.evidence"),
                            "fixed-witness conflict evidence must exactly match canonical recomputation",
                        ));
                    }
                }
                Some(expected) => issues.push(ValidationIssue::new(
                    "fixed_witness_conflict_kind_mismatch",
                    format!("{path}.kind"),
                    format!(
                        "serialized witnesses classify this pair as {:?}, not {:?}",
                        expected.kind, conflict.kind
                    ),
                )),
                None => issues.push(ValidationIssue::new(
                    "fixed_witness_spurious_conflict",
                    path.clone(),
                    "serialized full-width witnesses classify this pair as clear",
                )),
            }
        }
        let layer = conflict.evidence.layer.as_ref().map(String::as_str);
        let passage_key = conflict.evidence.passage_key.as_ref().map(String::as_str);
        if layer.is_some_and(|layer| !epochs.contains_key(layer)) {
            issues.push(ValidationIssue::new(
                "conflict_layer_missing",
                format!("{path}.evidence.layer"),
                "conflict references an unknown corridor layer",
            ));
        }
        if passage_key.is_some()
            && (layer.is_none() || !passages.contains_key(&(layer.unwrap(), passage_key.unwrap())))
        {
            issues.push(ValidationIssue::new(
                "conflict_passage_missing",
                format!("{path}.evidence.passage_key"),
                "conflict references an uncertified or unknown passage",
            ));
        }
    }
    if fixed_mode {
        for ((left, right), expected) in expected_fixed_conflicts {
            if !seen_edges.contains(&(left.as_str(), right.as_str())) {
                issues.push(ValidationIssue::new(
                    "fixed_witness_conflict_missing",
                    "$.pair_analysis.conflicts",
                    format!(
                        "serialized witnesses require a {:?} conflict for {left:?} and {right:?}",
                        expected.kind
                    ),
                ));
            }
        }
    }
    let mut seen_context_edges = HashSet::new();
    let context_by_id = portfolio
        .fixed_context
        .as_ref()
        .map(|context| {
            context
                .runs
                .iter()
                .map(|run| (run.context_id.as_str(), run))
                .collect::<HashMap<_, _>>()
        })
        .unwrap_or_default();
    for (index, conflict) in pair.context_conflicts.iter().enumerate() {
        let path = format!("$.pair_analysis.context_conflicts[{index}]");
        require_nonempty(&conflict.family_id, &format!("{path}.family_id"), issues);
        require_nonempty(&conflict.context_id, &format!("{path}.context_id"), issues);
        require_nonempty(
            &conflict.evidence.detail,
            &format!("{path}.evidence.detail"),
            issues,
        );
        let family = family_to_variable.get(conflict.family_id.as_str());
        let context = context_by_id.get(conflict.context_id.as_str());
        if family.is_none() {
            issues.push(ValidationIssue::new(
                "context_conflict_family_missing",
                format!("{path}.family_id"),
                "unary conflict family is absent from this candidate set",
            ));
        }
        if context.is_none() {
            issues.push(ValidationIssue::new(
                "context_conflict_run_missing",
                format!("{path}.context_id"),
                "unary conflict context run is absent from this certificate",
            ));
        }
        if family
            .is_some_and(|family| context.is_some_and(|context| family.1 == context.electrical_net))
        {
            issues.push(ValidationIssue::new(
                "context_conflict_same_electrical_net",
                path.clone(),
                "same-electrical-net fixed context is DRC-exempt",
            ));
        }
        if !seen_context_edges.insert((conflict.family_id.as_str(), conflict.context_id.as_str())) {
            issues.push(ValidationIssue::new(
                "duplicate_context_conflict_edge",
                path.clone(),
                "family-context conflict edge is duplicated",
            ));
        }
        if !conflict.certified {
            issues.push(ValidationIssue::new(
                "context_conflict_uncertified",
                format!("{path}.certified"),
                "diagnostic fixed-context evidence cannot forbid an option",
            ));
        }
        match expected_context_conflicts
            .get(&(conflict.family_id.clone(), conflict.context_id.clone()))
        {
            Some(expected) if expected.kind == conflict.kind => {
                let exact_layer = conflict.evidence.layer.as_ref() == Some(&expected.layer);
                let null_passage = conflict.evidence.passage_key.as_ref().is_none();
                if !exact_layer || !null_passage || conflict.evidence.detail != expected.detail {
                    issues.push(ValidationIssue::new(
                        "fixed_context_conflict_evidence_mismatch",
                        format!("{path}.evidence"),
                        "fixed-context conflict evidence must exactly match canonical recomputation",
                    ));
                }
            }
            Some(expected) => issues.push(ValidationIssue::new(
                "fixed_context_conflict_kind_mismatch",
                format!("{path}.kind"),
                format!(
                    "serialized family and context classify as {:?}, not {:?}",
                    expected.kind, conflict.kind
                ),
            )),
            None => issues.push(ValidationIssue::new(
                "fixed_context_spurious_conflict",
                path,
                "serialized family and context copper classify as clear",
            )),
        }
    }
    if v4 {
        for ((family, context), expected) in expected_context_conflicts {
            if !seen_context_edges.contains(&(family.as_str(), context.as_str())) {
                issues.push(ValidationIssue::new(
                    "fixed_context_conflict_missing",
                    "$.pair_analysis.context_conflicts",
                    format!(
                        "serialized copper requires a {:?} unary conflict for family {family:?} against context {context:?}",
                        expected.kind
                    ),
                ));
            }
        }
    }
}

struct RecomputedFixedConflict {
    kind: FamilyConflictKind,
    layer: String,
    detail: String,
}

struct SerializedCopperPrimitive {
    layer: String,
    label: String,
    points: Vec<Vec2>,
    width: f64,
    clearance: f64,
}

fn family_copper_primitives(
    variable: &RouteFamilyVariable,
    family: &RouteFamily,
) -> Vec<SerializedCopperPrimitive> {
    let Some(clearance) = variable.required_clearance else {
        return Vec::new();
    };
    let mut runs = family.runs.iter().collect::<Vec<_>>();
    runs.sort_by_key(|run| run.run_index);
    let mut primitives = Vec::new();
    for (run_index, run) in runs.iter().enumerate() {
        primitives.push(SerializedCopperPrimitive {
            layer: run.layer.clone(),
            label: format!("run:{}", run.run_id),
            points: run
                .witness
                .polyline
                .iter()
                .map(|point| Vec2::new(point.x, point.y))
                .collect(),
            width: variable.required_width,
            clearance,
        });
        if run_index == 0 {
            continue;
        }
        let RequiredNullable::Value(transition) = &run.transition_from_previous else {
            continue;
        };
        if transition.kind != RouteTransitionKind::Via {
            continue;
        }
        let Some(via) = transition.via.as_ref() else {
            continue;
        };
        let point = Vec2::new(transition.position.x, transition.position.y);
        for layer in [runs[run_index - 1].layer.as_str(), run.layer.as_str()] {
            primitives.push(SerializedCopperPrimitive {
                layer: layer.to_owned(),
                label: format!("via:{}@{}", run.run_id, layer),
                points: vec![point],
                width: via.diameter,
                clearance,
            });
        }
    }
    primitives
}

fn context_copper_primitive(run: &FixedContextRun) -> SerializedCopperPrimitive {
    SerializedCopperPrimitive {
        layer: run.layer.clone(),
        label: format!("context:{}", run.context_id),
        points: run
            .polyline
            .iter()
            .map(|point| Vec2::new(point.x, point.y))
            .collect(),
        width: run.required_width,
        clearance: run.required_clearance,
    }
}

fn classify_serialized_primitives(
    first: &[SerializedCopperPrimitive],
    second: &[SerializedCopperPrimitive],
    prefix: &str,
    segment_names: (&str, &str),
    include_primitive_labels: bool,
) -> Result<Option<RecomputedFixedConflict>, String> {
    let mut first_clearance = None;
    for first_primitive in first {
        for second_primitive in second {
            if first_primitive.layer != second_primitive.layer {
                continue;
            }
            let first_route = prepare_fixed_copper_route(FixedCopperRoute {
                polyline: &first_primitive.points,
                width: first_primitive.width,
                clearance: first_primitive.clearance,
            })
            .map_err(|error| {
                format!(
                    "could not prepare serialized primitive {:?}: {error}",
                    first_primitive.label
                )
            })?;
            let second_route = prepare_fixed_copper_route(FixedCopperRoute {
                polyline: &second_primitive.points,
                width: second_primitive.width,
                clearance: second_primitive.clearance,
            })
            .map_err(|error| {
                format!(
                    "could not prepare serialized primitive {:?}: {error}",
                    second_primitive.label
                )
            })?;
            let classification = classify_prepared_copper_pair(&first_route, &second_route)
                .map_err(|error| {
                    format!(
                        "could not classify serialized primitives {:?} + {:?}: {error}",
                        first_primitive.label, second_primitive.label
                    )
                })?;
            let evidence = classification.evidence();
            let detail = if include_primitive_labels {
                format!(
                    "{prefix}: first_primitive={}, second_primitive={}, {}={}, {}={}, centerline_distance={:.17}, required_distance={:.17}, segment_pairs_analyzed={}",
                    first_primitive.label,
                    second_primitive.label,
                    segment_names.0,
                    evidence.first_segment_index,
                    segment_names.1,
                    evidence.second_segment_index,
                    evidence.minimum_centerline_distance,
                    evidence.required_centerline_distance,
                    evidence.segment_pairs_analyzed,
                )
            } else {
                format!(
                    "{prefix}, {}={}, {}={}, centerline_distance={:.17}, required_distance={:.17}, segment_pairs_analyzed={}",
                    segment_names.0,
                    evidence.first_segment_index,
                    segment_names.1,
                    evidence.second_segment_index,
                    evidence.minimum_centerline_distance,
                    evidence.required_centerline_distance,
                    evidence.segment_pairs_analyzed,
                )
            };
            let conflict = RecomputedFixedConflict {
                kind: match classification {
                    FixedCopperPairClassification::Clear(_) => continue,
                    FixedCopperPairClassification::CenterlineIntersection(_) => {
                        FamilyConflictKind::CenterlineCrossing
                    }
                    FixedCopperPairClassification::ClearanceShortfall(_) => {
                        FamilyConflictKind::CopperClearance
                    }
                },
                layer: first_primitive.layer.clone(),
                detail,
            };
            if conflict.kind == FamilyConflictKind::CenterlineCrossing {
                return Ok(Some(conflict));
            }
            if first_clearance.is_none() {
                first_clearance = Some(conflict);
            }
        }
    }
    Ok(first_clearance)
}

fn recompute_fixed_witness_pairs(
    portfolio: &RouteFamilyPortfolio,
    issues: &mut Vec<ValidationIssue>,
) -> (u64, HashMap<(String, String), RecomputedFixedConflict>) {
    struct SerializedFixedFamily<'a> {
        variable_index: usize,
        variable: &'a RouteFamilyVariable,
        family: &'a RouteFamily,
        primitives: Vec<SerializedCopperPrimitive>,
    }

    let serialized = portfolio
        .variables
        .iter()
        .enumerate()
        .flat_map(|(variable_index, variable)| {
            variable
                .families
                .iter()
                .map(move |family| SerializedFixedFamily {
                    variable_index,
                    variable,
                    family,
                    primitives: family_copper_primitives(variable, family),
                })
        })
        .collect::<Vec<_>>();
    let mut analyzed = 0_u64;
    let mut conflicts = HashMap::new();
    for (left_index, left) in serialized.iter().enumerate() {
        for right in serialized.iter().skip(left_index + 1) {
            if left.variable_index == right.variable_index
                || left.variable.electrical_net == right.variable.electrical_net
            {
                continue;
            }
            analyzed += 1;
            let (first_id, second_id, first_primitives, second_primitives) =
                if left.family.family_id <= right.family.family_id {
                    (
                        &left.family.family_id,
                        &right.family.family_id,
                        &left.primitives,
                        &right.primitives,
                    )
                } else {
                    (
                        &right.family.family_id,
                        &left.family.family_id,
                        &right.primitives,
                        &left.primitives,
                    )
                };
            let prefix = format!(
                "fixed full-width witnesses: first_family={first_id}, second_family={second_id}"
            );
            let conflict = match classify_serialized_primitives(
                first_primitives,
                second_primitives,
                &prefix,
                ("first_segment", "second_segment"),
                portfolio.is_multi_run_schema(),
            ) {
                Ok(conflict) => conflict,
                Err(error) => {
                    issues.push(ValidationIssue::new(
                        "fixed_witness_pair_recomputation_failed",
                        "$.pair_analysis",
                        error,
                    ));
                    continue;
                }
            };
            let Some(conflict) = conflict else {
                continue;
            };
            conflicts.insert((first_id.clone(), second_id.clone()), conflict);
        }
    }
    (analyzed, conflicts)
}

fn recompute_fixed_context_pairs(
    portfolio: &RouteFamilyPortfolio,
    issues: &mut Vec<ValidationIssue>,
) -> (u64, HashMap<(String, String), RecomputedFixedConflict>) {
    struct SerializedFamily<'a> {
        variable: &'a RouteFamilyVariable,
        family: &'a RouteFamily,
        primitives: Vec<SerializedCopperPrimitive>,
    }
    let families = portfolio
        .variables
        .iter()
        .flat_map(|variable| {
            variable
                .families
                .iter()
                .map(move |family| SerializedFamily {
                    variable,
                    family,
                    primitives: family_copper_primitives(variable, family),
                })
        })
        .collect::<Vec<_>>();
    let Some(context) = portfolio.fixed_context.as_ref() else {
        return (0, HashMap::new());
    };
    let context_primitives = context
        .runs
        .iter()
        .map(context_copper_primitive)
        .collect::<Vec<_>>();
    let mut analyzed = 0_u64;
    let mut conflicts = HashMap::new();
    for family in &families {
        for (context_index, run) in context.runs.iter().enumerate() {
            if family.variable.electrical_net == run.electrical_net {
                continue;
            }
            analyzed += 1;
            let family_id = family.family.family_id.clone();
            let context_id = run.context_id.clone();
            let prefix = format!("fixed context copper: family={family_id}, context={context_id}");
            let conflict = match classify_serialized_primitives(
                &family.primitives,
                std::slice::from_ref(&context_primitives[context_index]),
                &prefix,
                ("family_segment", "context_segment"),
                portfolio.is_multi_run_schema(),
            ) {
                Ok(conflict) => conflict,
                Err(error) => {
                    issues.push(ValidationIssue::new(
                        "fixed_context_pair_recomputation_failed",
                        "$.pair_analysis",
                        error,
                    ));
                    continue;
                }
            };
            let Some(conflict) = conflict else {
                continue;
            };
            conflicts.insert((family_id, context_id), conflict);
        }
    }
    (analyzed, conflicts)
}

fn require_nonempty(value: &str, path: &str, issues: &mut Vec<ValidationIssue>) {
    if value.is_empty() {
        issues.push(ValidationIssue::new(
            "schema_min_length",
            path,
            "string must not be empty",
        ));
    }
}

fn require_finite(value: f64, path: &str, issues: &mut Vec<ValidationIssue>) {
    if !value.is_finite() {
        issues.push(ValidationIssue::new(
            "schema_number_not_finite",
            path,
            "number must be finite",
        ));
    }
}

fn require_positive_finite(value: f64, path: &str, issues: &mut Vec<ValidationIssue>) {
    require_finite(value, path, issues);
    if value <= 0.0 {
        issues.push(ValidationIssue::new(
            "schema_exclusive_minimum",
            path,
            "number must be greater than zero",
        ));
    }
}

fn require_nonnegative_finite(value: f64, path: &str, issues: &mut Vec<ValidationIssue>) {
    require_finite(value, path, issues);
    if value < 0.0 {
        issues.push(ValidationIssue::new(
            "schema_minimum",
            path,
            "number must be nonnegative",
        ));
    }
}

fn same_point(left: &FamilyPoint, right: &FamilyPoint) -> bool {
    approximately_equal(left.x, right.x) && approximately_equal(left.y, right.y)
}

fn ensure_family_finite(family: &RouteFamily) -> Result<(), FingerprintError> {
    if !family.cost.is_finite() {
        return Err(FingerprintError {
            path: format!("family[{:?}].cost", family.family_id),
            message: "candidate fingerprint cannot encode a non-finite cost".to_owned(),
        });
    }
    for run in &family.runs {
        let witness = &run.witness;
        if !witness.minimum_clearance.is_finite() || !witness.required_width.is_finite() {
            return Err(FingerprintError {
                path: format!("run[{:?}].witness", run.run_id),
                message: "candidate fingerprint cannot encode non-finite witness dimensions"
                    .to_owned(),
            });
        }
        for (index, point) in witness.polyline.iter().enumerate() {
            if !point.x.is_finite() || !point.y.is_finite() {
                return Err(FingerprintError {
                    path: format!("run[{:?}].witness.polyline[{index}]", run.run_id),
                    message: "candidate fingerprint cannot encode a non-finite point".to_owned(),
                });
            }
        }
        if let Some(transition) = run.transition_from_previous.as_ref() {
            if !transition.position.x.is_finite() || !transition.position.y.is_finite() {
                return Err(FingerprintError {
                    path: format!("run[{:?}].transition_from_previous.position", run.run_id),
                    message: "candidate fingerprint cannot encode a non-finite transition"
                        .to_owned(),
                });
            }
            if transition.via.as_ref().is_some_and(|via| {
                !via.diameter.is_finite()
                    || !via.drill.is_finite()
                    || !via.minimum_clearance.is_finite()
            }) {
                return Err(FingerprintError {
                    path: format!("run[{:?}].transition_from_previous.via", run.run_id),
                    message: "candidate fingerprint cannot encode non-finite via geometry"
                        .to_owned(),
                });
            }
        }
    }
    Ok(())
}

fn approximately_equal(left: f64, right: f64) -> bool {
    if left == right {
        return true;
    }
    let scale = left.abs().max(right.abs()).max(1.0);
    (left - right).abs() <= 1.0e-9 * scale
}

fn canonical_hash(value: &Value) -> String {
    let mut encoded = String::new();
    write_canonical_json(value, &mut encoded);
    let digest = Sha256::digest(encoded.as_bytes());
    let mut output = String::with_capacity(64);
    for byte in digest {
        write!(&mut output, "{byte:02x}").expect("writing to a string cannot fail");
    }
    output
}

fn write_canonical_json(value: &Value, output: &mut String) {
    match value {
        Value::Null => output.push_str("null"),
        Value::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
        Value::Number(value) => output.push_str(&value.to_string()),
        Value::String(value) => {
            output.push_str(&serde_json::to_string(value).expect("a JSON string always serializes"))
        }
        Value::Array(values) => {
            output.push('[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                write_canonical_json(value, output);
            }
            output.push(']');
        }
        Value::Object(values) => {
            output.push('{');
            let mut keys: Vec<_> = values.keys().collect();
            keys.sort_unstable();
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                output.push_str(
                    &serde_json::to_string(key).expect("a JSON object key always serializes"),
                );
                output.push(':');
                write_canonical_json(&values[key], output);
            }
            output.push('}');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn east_fixed_context_ladder_fingerprints_are_fail_closed() {
        let manifest_source = include_str!(
            "../../../benchmarks/imported/layout-trace/esp32-slices/evidence/east-route-family-fixed-context-7e9e146.json"
        );
        let manifest: Value = serde_json::from_str(manifest_source).unwrap();
        let runs = manifest["runs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|run| serde_json::from_value::<FixedContextRun>(run.clone()).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(runs.len(), 19);

        // The source artifact was emitted from these f64 values. Parsing its
        // decimal representation must recover the same bits; serde_json's
        // best-effort parser otherwise double-rounds this coordinate down by
        // one ULP before any routing code sees it.
        let affected = runs.iter().find(|run| run.branch == "E_DEC_A_GND").unwrap();
        assert_eq!(
            affected.polyline[3].x.to_bits(),
            58.230000000000004_f64.to_bits()
        );

        let serialized_runs = serde_json::to_string(&runs).unwrap();
        let reparsed_runs: Vec<FixedContextRun> = serde_json::from_str(&serialized_runs).unwrap();
        assert_eq!(runs.len(), reparsed_runs.len());
        assert_eq!(runs, reparsed_runs);
        for (original, reparsed) in runs.iter().zip(&reparsed_runs) {
            assert_eq!(
                original.required_width.to_bits(),
                reparsed.required_width.to_bits()
            );
            assert_eq!(
                original.required_clearance.to_bits(),
                reparsed.required_clearance.to_bits()
            );
            assert_eq!(original.polyline.len(), reparsed.polyline.len());
            for (original, reparsed) in original.polyline.iter().zip(&reparsed.polyline) {
                assert_eq!(original.x.to_bits(), reparsed.x.to_bits());
                assert_eq!(original.y.to_bits(), reparsed.y.to_bits());
            }
        }

        for ladder in manifest["ladders"].as_array().unwrap() {
            let selected = ladder["context_branches"]
                .as_array()
                .unwrap()
                .iter()
                .map(|branch| branch.as_str().unwrap())
                .collect::<HashSet<_>>();
            let mut selected_runs = reparsed_runs
                .iter()
                .filter(|run| selected.contains(run.branch.as_str()))
                .cloned()
                .collect::<Vec<_>>();
            selected_runs.sort_by(|left, right| left.context_id.cmp(&right.context_id));
            let value = Value::Array(vec![
                Value::String(FIXED_CONTEXT_FINGERPRINT_METHOD.into()),
                manifest["provenance"]["method"].clone(),
                manifest["provenance"]["placement_revision"].clone(),
                manifest["provenance"]["geometry_revision"].clone(),
                serde_json::to_value(selected_runs).unwrap(),
            ]);
            assert_eq!(
                canonical_hash(&value),
                ladder["expected_fixed_context_fingerprint"]
                    .as_str()
                    .unwrap(),
                "{} correctly rounded fixed-context data drifted",
                ladder["id"].as_str().unwrap()
            );
            assert_ne!(
                ladder["expected_fixed_context_fingerprint"],
                ladder["legacy_best_effort_fixed_context_fingerprint"]
            );
        }
        assert_eq!(
            manifest["ladders"][3]["expected_fixed_context_fingerprint"],
            manifest["provenance"]["current_correctly_rounded_fixed_context_fingerprint"]
        );
        assert_eq!(
            manifest["ladders"][3]["legacy_best_effort_fixed_context_fingerprint"],
            manifest["provenance"]["legacy_best_effort_recomputed_fixed_context_fingerprint"]
        );
        assert_eq!(
            manifest["provenance"]["source_stored_fixed_context_fingerprint"],
            manifest["provenance"]["current_correctly_rounded_fixed_context_fingerprint"]
        );
        assert_eq!(
            manifest["provenance"]["source_stored_fingerprint_matches_legacy_best_effort_recomputation"],
            Value::Bool(false)
        );
        assert_eq!(
            manifest["provenance"]["source_stored_fingerprint_matches_current_correctly_rounded_recomputation"],
            Value::Bool(true)
        );
    }

    fn fixture() -> RouteFamilyPortfolio {
        let mut portfolio = RouteFamilyPortfolio {
            schema_version: LEGACY_ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION.to_owned(),
            feasibility_mode: None,
            portfolio_id: "rust-v5-positive".to_owned(),
            placement_revision: 7,
            geometry_revision: None,
            corridor_epochs: vec![CorridorEpoch {
                layer: "top".to_owned(),
                corridor_revision: 11,
                semantic_fingerprint: "synthetic-semantic-v1".to_owned(),
                cut_basis_fingerprint: "synthetic-cut-v1".to_owned(),
                cut_basis_complete: true,
            }],
            family_generation: FamilyGenerationCertificate {
                method: "bounded-test-generator".to_owned(),
                max_families_per_variable: 1,
                search_complete: true,
                budget_exhausted: false,
            },
            passage_certificates: vec![PassageCapacityCertificate {
                layer: "top".to_owned(),
                key: "center".to_owned(),
                placement_revision: 7,
                corridor_revision: 11,
                available_width: Some(2.0),
                minimum_clearance: 0.25,
                enforcement: None,
                capacity_model: "synthetic-certified-cross-section-v1".to_owned(),
                certified: true,
            }],
            variables: vec![RouteFamilyVariable {
                branch: "signal".to_owned(),
                electrical_net: "signal".to_owned(),
                required_width: 0.5,
                required_clearance: None,
                families: vec![RouteFamily {
                    family_id: "signal/family-0".to_owned(),
                    placement_revision: 7,
                    complete_route: true,
                    cost: 1.0,
                    runs: vec![RouteFamilyRun {
                        run_id: "signal/family-0/run-0".to_owned(),
                        run_index: 0,
                        layer: "top".to_owned(),
                        start_point: 0,
                        end_point: 1,
                        corridor_revision: 11,
                        transition_from_previous: RequiredNullable::Null,
                        witness: RouteWitness {
                            polyline: vec![
                                FamilyPoint { x: 0.0, y: 1.0 },
                                FamilyPoint { x: 2.0, y: 1.0 },
                            ],
                            placement_revision: 7,
                            realizable: true,
                            clearance_certified: true,
                            clearance_model: "synthetic-exact-clearance-v1".to_owned(),
                            minimum_clearance: 0.25,
                            required_width: 0.5,
                        },
                        cut_word_certificate: CutWordCertificate {
                            cut_basis_fingerprint: "synthetic-cut-v1".to_owned(),
                            basis_complete: true,
                            verified: true,
                            word: vec![],
                        },
                        semantic_passage_keys: vec!["center".to_owned()],
                        passage_mapping_complete: true,
                    }],
                }],
            }],
            fixed_context: None,
            pair_analysis: PairAnalysisCertificate {
                method: "complete-pair-analysis-v1".to_owned(),
                placement_revision: 7,
                corridor_epoch_fingerprint: "pending".to_owned(),
                candidate_set_fingerprint: "pending".to_owned(),
                search_complete: true,
                analyzed_family_pairs: None,
                fixed_context_fingerprint: None,
                analyzed_family_context_pairs: None,
                context_conflicts: vec![],
                conflicts: vec![],
            },
        };
        portfolio.seal_pair_analysis_fingerprints().unwrap();
        portfolio
    }

    fn codes(portfolio: &RouteFamilyPortfolio) -> HashSet<&'static str> {
        portfolio
            .validation_issues()
            .into_iter()
            .map(|issue| issue.code)
            .collect()
    }

    fn clear_fixed_pair_fixture() -> RouteFamilyPortfolio {
        let mut portfolio = fixture();
        portfolio.schema_version = FIXED_WITNESS_ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION.into();
        portfolio.feasibility_mode =
            Some(RouteFamilyFeasibilityMode::FixedFullWidthWitnessPairwise);
        portfolio.passage_certificates[0].available_width = None;
        portfolio.passage_certificates[0].enforcement =
            Some(PassageEnforcementMode::FixedWitnessPairwise);
        portfolio.passage_certificates[0].capacity_model =
            FIXED_WITNESS_NO_SCALAR_CAPACITY_MODEL.into();
        portfolio.variables[0].required_clearance = Some(0.25);
        portfolio.variables[0].families[0].runs[0]
            .witness
            .clearance_model = "exact_polyline_against_polygonal_obstacles".into();
        portfolio.variables[0].families[0].runs[0]
            .witness
            .minimum_clearance = 1.0;
        let mut second = portfolio.variables[0].clone();
        second.branch = "other".into();
        second.electrical_net = "other".into();
        second.families[0].family_id = "other/family-0".into();
        second.families[0].runs[0].run_id = "other/family-0/run-0".into();
        for point in &mut second.families[0].runs[0].witness.polyline {
            point.y += 1.0;
        }
        portfolio.variables.push(second);
        portfolio.pair_analysis.method = FIXED_WITNESS_PAIR_ANALYSIS_METHOD.into();
        portfolio.pair_analysis.analyzed_family_pairs = Some(1);
        portfolio.pair_analysis.conflicts.clear();
        portfolio.seal_pair_analysis_fingerprints().unwrap();
        portfolio.validate().unwrap();
        portfolio
    }

    fn v4_context_fixture(runs: Vec<FixedContextRun>) -> RouteFamilyPortfolio {
        let mut portfolio = clear_fixed_pair_fixture();
        portfolio.schema_version = ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION.into();
        portfolio.geometry_revision = Some(13);
        for (index, layer) in runs
            .iter()
            .map(|run| run.layer.as_str())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .enumerate()
        {
            if portfolio
                .corridor_epochs
                .iter()
                .any(|epoch| epoch.layer == layer)
            {
                continue;
            }
            portfolio.corridor_epochs.push(CorridorEpoch {
                layer: layer.into(),
                corridor_revision: 20 + index as u64,
                semantic_fingerprint: format!("synthetic-{layer}-semantic-v1"),
                cut_basis_fingerprint: format!("synthetic-{layer}-cuts-v1"),
                cut_basis_complete: true,
            });
        }
        portfolio.fixed_context = Some(FixedContextCertificate {
            method: FIXED_CONTEXT_ANALYSIS_METHOD.into(),
            placement_revision: 7,
            geometry_revision: 13,
            fixed_context_fingerprint: String::new(),
            search_complete: true,
            runs,
        });
        let mut recompute_issues = Vec::new();
        let (analyzed, expected) = recompute_fixed_context_pairs(&portfolio, &mut recompute_issues);
        assert!(recompute_issues.is_empty(), "{recompute_issues:?}");
        portfolio.pair_analysis.analyzed_family_context_pairs = Some(analyzed);
        portfolio.pair_analysis.context_conflicts = expected
            .into_iter()
            .map(
                |((family_id, context_id), expected)| FamilyContextConflict {
                    family_id,
                    context_id,
                    kind: expected.kind,
                    certified: true,
                    evidence: ConflictEvidence {
                        layer: RequiredNullable::Value(expected.layer),
                        passage_key: RequiredNullable::Null,
                        detail: expected.detail,
                    },
                },
            )
            .collect();
        portfolio
            .pair_analysis
            .context_conflicts
            .sort_by(|left, right| {
                (&left.family_id, &left.context_id).cmp(&(&right.family_id, &right.context_id))
            });
        portfolio.seal_pair_analysis_fingerprints().unwrap();
        portfolio.validate().unwrap();
        portfolio
    }

    fn context_run(
        context_id: &str,
        electrical_net: &str,
        layer: &str,
        points: Vec<FamilyPoint>,
    ) -> FixedContextRun {
        FixedContextRun {
            context_id: context_id.into(),
            branch: format!("outside-{context_id}"),
            electrical_net: electrical_net.into(),
            layer: layer.into(),
            required_width: 0.5,
            required_clearance: 0.25,
            polyline: points,
            placement_revision: 7,
            geometry_revision: 13,
        }
    }

    #[test]
    fn positive_fixture_round_trips_and_validates() {
        let portfolio = fixture();
        let json = serde_json::to_string(&portfolio).unwrap();
        let parsed = parse_and_validate_route_family_portfolio(&json).unwrap();
        assert_eq!(parsed, portfolio);
    }

    #[test]
    fn v4_fixed_context_is_separately_fingerprinted_and_counts_all_layers() {
        let clear_bottom = context_run(
            "bottom-run",
            "fixed",
            "bottom",
            vec![FamilyPoint { x: 1.0, y: 1.0 }],
        );
        let first = v4_context_fixture(vec![clear_bottom]);
        assert_eq!(first.pair_analysis.analyzed_family_context_pairs, Some(2));
        assert!(first.pair_analysis.context_conflicts.is_empty());
        let candidate_hash = first.candidate_set_fingerprint().unwrap();
        let first_context_hash = first.fixed_context_fingerprint().unwrap();

        let mut second = first.clone();
        let context = second.fixed_context.as_mut().unwrap();
        context.runs[0].layer = "top".into();
        context.runs[0].polyline[0].x = 50.0;
        second.seal_pair_analysis_fingerprints().unwrap();
        assert_eq!(second.candidate_set_fingerprint().unwrap(), candidate_hash);
        assert_ne!(
            second.fixed_context_fingerprint().unwrap(),
            first_context_hash
        );
        assert_eq!(second.pair_analysis.analyzed_family_context_pairs, Some(2));
        second.validate().unwrap();
    }

    #[test]
    fn v4_rejects_missing_authoritative_context_fields_without_mutating_v3() {
        let v3 = clear_fixed_pair_fixture();
        v3.validate().unwrap();

        let mut v4 = v3.clone();
        v4.schema_version = ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION.into();
        let issues = codes(&v4);
        assert!(issues.contains("v4_geometry_revision_missing"));
        assert!(issues.contains("v4_fixed_context_missing"));
        assert!(issues.contains("v4_fixed_context_fingerprint_missing"));
        assert!(issues.contains("v4_analyzed_family_context_pairs_missing"));

        let mut v3_with_v4_provenance = v3;
        v3_with_v4_provenance.geometry_revision = Some(13);
        assert!(codes(&v3_with_v4_provenance).contains("geometry_revision_schema_mismatch"));
    }

    #[test]
    fn v4_validator_recomputes_unary_context_conflicts_and_provenance() {
        let crossing = context_run(
            "crossing-run",
            "fixed",
            "top",
            vec![
                FamilyPoint { x: 1.0, y: 0.0 },
                FamilyPoint { x: 1.0, y: 3.0 },
            ],
        );
        let portfolio = v4_context_fixture(vec![crossing]);
        assert_eq!(
            portfolio.pair_analysis.analyzed_family_context_pairs,
            Some(2)
        );
        assert_eq!(portfolio.pair_analysis.context_conflicts.len(), 2);

        let mut missing = portfolio.clone();
        missing.pair_analysis.context_conflicts.pop();
        assert!(codes(&missing).contains("fixed_context_conflict_missing"));

        let mut forged = portfolio.clone();
        forged.pair_analysis.context_conflicts[0].evidence.detail = "forged".into();
        assert!(codes(&forged).contains("fixed_context_conflict_evidence_mismatch"));

        let mut wrong_count = portfolio.clone();
        wrong_count.pair_analysis.analyzed_family_context_pairs = Some(1);
        assert!(codes(&wrong_count).contains("fixed_context_pair_matrix_incomplete"));

        let mut stale = portfolio.clone();
        stale.fixed_context.as_mut().unwrap().runs[0].geometry_revision = 12;
        assert!(codes(&stale).contains("stale_fixed_context_run_geometry_revision"));
        assert!(codes(&stale).contains("fixed_context_fingerprint_mismatch"));

        let mut stale_context = portfolio.clone();
        let context = stale_context.fixed_context.as_mut().unwrap();
        context.geometry_revision = 12;
        for run in &mut context.runs {
            run.geometry_revision = 12;
        }
        stale_context.seal_pair_analysis_fingerprints().unwrap();
        assert!(codes(&stale_context).contains("stale_fixed_context_geometry_revision"));

        let mut selected_as_context = portfolio.clone();
        selected_as_context.fixed_context.as_mut().unwrap().runs[0].branch = "signal".into();
        selected_as_context
            .seal_pair_analysis_fingerprints()
            .unwrap();
        assert!(codes(&selected_as_context).contains("fixed_context_selected_branch_overlap"));

        let mut fictitious_layer = portfolio.clone();
        fictitious_layer.fixed_context.as_mut().unwrap().runs[0].layer = "ghost".into();
        fictitious_layer.seal_pair_analysis_fingerprints().unwrap();
        assert!(codes(&fictitious_layer).contains("fixed_context_layer_missing"));
    }

    #[test]
    fn one_point_via_context_conflicts_cross_net_but_exempts_same_net() {
        let via = context_run(
            "outside#via:0:point:2:top",
            "fixed",
            "top",
            vec![FamilyPoint { x: 1.0, y: 1.0 }],
        );
        let cross_net = v4_context_fixture(vec![via.clone()]);
        assert!(
            cross_net
                .pair_analysis
                .context_conflicts
                .iter()
                .any(|conflict| conflict.family_id == "signal/family-0")
        );

        let same_net = v4_context_fixture(vec![FixedContextRun {
            electrical_net: "signal".into(),
            ..via
        }]);
        assert_eq!(
            same_net.pair_analysis.analyzed_family_context_pairs,
            Some(1)
        );
        assert!(
            same_net
                .pair_analysis
                .context_conflicts
                .iter()
                .all(|conflict| conflict.family_id != "signal/family-0")
        );
    }

    #[test]
    fn v4_rejects_inconsistent_electrical_identity_within_one_context_branch() {
        let mut portfolio = v4_context_fixture(vec![
            context_run(
                "far-a",
                "fixed",
                "top",
                vec![FamilyPoint { x: 50.0, y: 50.0 }],
            ),
            context_run(
                "far-b",
                "fixed",
                "top",
                vec![FamilyPoint { x: 60.0, y: 60.0 }],
            ),
        ]);
        let context = portfolio.fixed_context.as_mut().unwrap();
        context.runs[1].branch = context.runs[0].branch.clone();
        context.runs[1].electrical_net = "different-fixed-net".into();
        portfolio.seal_pair_analysis_fingerprints().unwrap();

        assert!(codes(&portfolio).contains("fixed_context_branch_net_mismatch"));
    }

    #[test]
    fn python_compatible_fingerprints_are_stable() {
        let portfolio = fixture();
        assert_eq!(
            portfolio.corridor_epoch_fingerprint(),
            "a42cdc121f7b27115d9944405531e9048f69bffba041ab39a1bb6bea5454b2c4"
        );
        assert_eq!(
            portfolio.candidate_set_fingerprint().unwrap(),
            "5e34e8e257daf065cb908b972d6d4b84d94df81b27b2b22beb8d3be923d17066"
        );
    }

    #[test]
    fn known_incomplete_controls_fail_closed() {
        let mut portfolio = fixture();
        portfolio.family_generation.search_complete = false;
        assert!(codes(&portfolio).contains("family_search_incomplete"));

        let mut portfolio = fixture();
        portfolio.family_generation.budget_exhausted = true;
        assert!(codes(&portfolio).contains("family_budget_exhausted"));

        let mut portfolio = fixture();
        portfolio.variables[0].families[0].complete_route = false;
        assert!(codes(&portfolio).contains("route_family_incomplete"));

        let mut portfolio = fixture();
        portfolio.variables[0].families[0].runs[0]
            .cut_word_certificate
            .verified = false;
        assert!(codes(&portfolio).contains("homotopy_unverified"));

        let mut portfolio = fixture();
        portfolio.variables[0].families[0].runs[0]
            .witness
            .clearance_certified = false;
        assert!(codes(&portfolio).contains("clearance_uncertified"));

        let mut portfolio = fixture();
        portfolio.variables[0].families[0].runs[0].passage_mapping_complete = false;
        assert!(codes(&portfolio).contains("passage_mapping_incomplete"));

        let mut portfolio = fixture();
        portfolio.pair_analysis.search_complete = false;
        assert!(codes(&portfolio).contains("pair_analysis_incomplete"));
    }

    #[test]
    fn witness_mutation_invalidates_complete_pair_analysis() {
        let mut portfolio = fixture();
        portfolio.variables[0].families[0].runs[0].witness.polyline[1].x = 3.0;
        assert!(codes(&portfolio).contains("candidate_set_fingerprint_mismatch"));
    }

    #[test]
    fn legacy_v2_scalar_portfolio_remains_readable_without_new_mode_fields() {
        let mut value = serde_json::to_value(fixture()).unwrap();
        value["schema_version"] =
            Value::String(LEGACY_ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION.into());
        value.as_object_mut().unwrap().remove("feasibility_mode");
        value["passage_certificates"][0]
            .as_object_mut()
            .unwrap()
            .remove("enforcement");
        value["pair_analysis"]
            .as_object_mut()
            .unwrap()
            .remove("analyzed_family_pairs");
        value["variables"][0]
            .as_object_mut()
            .unwrap()
            .remove("required_clearance");
        value["pair_analysis"]["candidate_set_fingerprint"] = Value::String(
            "5e34e8e257daf065cb908b972d6d4b84d94df81b27b2b22beb8d3be923d17066".into(),
        );
        let portfolio =
            parse_and_validate_route_family_portfolio(&serde_json::to_string(&value).unwrap())
                .unwrap();
        assert_eq!(portfolio.feasibility_mode, None);
        assert_eq!(portfolio.passage_certificates[0].enforcement, None);
        assert_eq!(portfolio.variables[0].required_clearance, None);
        let reserialized = serde_json::to_value(&portfolio).unwrap();
        assert!(reserialized.get("feasibility_mode").is_none());
        assert!(
            reserialized["passage_certificates"][0]
                .get("enforcement")
                .is_none()
        );
        assert!(
            reserialized["pair_analysis"]
                .get("analyzed_family_pairs")
                .is_none()
        );
        assert!(
            reserialized["variables"][0]
                .get("required_clearance")
                .is_none()
        );
    }

    #[test]
    fn fixed_witness_mode_rejects_scalar_capacity_and_stale_witness_commitment() {
        let mut portfolio = fixture();
        portfolio.schema_version = FIXED_WITNESS_ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION.into();
        portfolio.feasibility_mode =
            Some(RouteFamilyFeasibilityMode::FixedFullWidthWitnessPairwise);
        portfolio.passage_certificates[0].enforcement =
            Some(PassageEnforcementMode::FixedWitnessPairwise);
        portfolio.passage_certificates[0].available_width = None;
        portfolio.passage_certificates[0].capacity_model =
            FIXED_WITNESS_NO_SCALAR_CAPACITY_MODEL.into();
        portfolio.variables[0].required_clearance = Some(0.25);
        portfolio.variables[0].families[0].runs[0]
            .witness
            .clearance_model = "exact_polyline_against_polygonal_obstacles".into();
        portfolio.variables[0].families[0].runs[0]
            .witness
            .minimum_clearance = 1.0;
        portfolio.pair_analysis.method = FIXED_WITNESS_PAIR_ANALYSIS_METHOD.into();
        portfolio.pair_analysis.analyzed_family_pairs = Some(0);
        portfolio.seal_pair_analysis_fingerprints().unwrap();
        portfolio.validate().unwrap();

        portfolio.passage_certificates[0].available_width = Some(2.0);
        assert!(codes(&portfolio).contains("fixed_witness_scalar_capacity"));
        portfolio.passage_certificates[0].available_width = None;
        portfolio.variables[0].families[0].runs[0].witness.polyline[1].y += 0.1;
        assert!(codes(&portfolio).contains("candidate_set_fingerprint_mismatch"));
    }

    #[test]
    fn fixed_witness_validation_recomputes_sparse_conflicts_from_serialized_runs() {
        let mut portfolio = clear_fixed_pair_fixture();
        for point in &mut portfolio.variables[1].families[0].runs[0].witness.polyline {
            point.y -= 0.5;
        }
        portfolio.seal_pair_analysis_fingerprints().unwrap();
        assert!(codes(&portfolio).contains("fixed_witness_conflict_missing"));

        let mut portfolio = clear_fixed_pair_fixture();
        portfolio.pair_analysis.conflicts.push(FamilyConflict {
            left_family_id: "other/family-0".into(),
            right_family_id: "signal/family-0".into(),
            kind: FamilyConflictKind::CopperClearance,
            certified: true,
            evidence: ConflictEvidence {
                layer: RequiredNullable::Value("top".into()),
                passage_key: RequiredNullable::Null,
                detail: "invented conflict".into(),
            },
        });
        assert!(codes(&portfolio).contains("fixed_witness_spurious_conflict"));

        let mut portfolio = clear_fixed_pair_fixture();
        for point in &mut portfolio.variables[1].families[0].runs[0].witness.polyline {
            point.y -= 0.5;
        }
        let mut recompute_issues = Vec::new();
        let (_, expected) = recompute_fixed_witness_pairs(&portfolio, &mut recompute_issues);
        assert!(recompute_issues.is_empty());
        let expected = expected.values().next().unwrap();
        portfolio.pair_analysis.conflicts.push(FamilyConflict {
            left_family_id: "other/family-0".into(),
            right_family_id: "signal/family-0".into(),
            kind: expected.kind,
            certified: true,
            evidence: ConflictEvidence {
                layer: RequiredNullable::Value(expected.layer.clone()),
                passage_key: RequiredNullable::Null,
                detail: expected.detail.clone(),
            },
        });
        portfolio.seal_pair_analysis_fingerprints().unwrap();
        portfolio.validate().unwrap();

        portfolio.pair_analysis.conflicts[0].evidence.detail = "forged distances".into();
        assert!(codes(&portfolio).contains("fixed_witness_conflict_evidence_mismatch"));
        portfolio.pair_analysis.conflicts[0].evidence.detail = expected.detail.clone();
        portfolio.pair_analysis.conflicts[0].evidence.layer = RequiredNullable::Null;
        assert!(codes(&portfolio).contains("fixed_witness_conflict_evidence_mismatch"));
    }

    #[test]
    fn fixed_witness_validation_binds_exact_clearance_model_and_required_radius() {
        let mut portfolio = clear_fixed_pair_fixture();
        portfolio.variables[0].families[0].runs[0]
            .witness
            .clearance_model = "claimed-exact".into();
        portfolio.seal_pair_analysis_fingerprints().unwrap();
        assert!(codes(&portfolio).contains("fixed_witness_clearance_model_unsupported"));

        let mut portfolio = clear_fixed_pair_fixture();
        portfolio.variables[0].families[0].runs[0]
            .witness
            .minimum_clearance = 0.49;
        portfolio.seal_pair_analysis_fingerprints().unwrap();
        assert!(codes(&portfolio).contains("fixed_witness_minimum_clearance_shortfall"));

        let mut portfolio = clear_fixed_pair_fixture();
        portfolio.variables[0].required_width = f64::MAX;
        portfolio.variables[0].required_clearance = Some(f64::MAX);
        portfolio.variables[0].families[0].runs[0]
            .witness
            .required_width = f64::MAX;
        portfolio.variables[0].families[0].runs[0]
            .witness
            .minimum_clearance = f64::MAX;
        portfolio.seal_pair_analysis_fingerprints().unwrap();
        assert!(codes(&portfolio).contains("fixed_witness_required_radius_nonfinite"));
    }

    #[test]
    fn fixed_witness_validation_rejects_multi_run_and_missing_v3_wire_fields() {
        let mut portfolio = clear_fixed_pair_fixture();
        let mut second_run = portfolio.variables[0].families[0].runs[0].clone();
        second_run.run_id = "signal/family-0/run-1".into();
        second_run.run_index = 1;
        second_run.start_point = 1;
        second_run.end_point = 2;
        second_run.transition_from_previous = RequiredNullable::Value(RouteTransition {
            kind: RouteTransitionKind::Continuous,
            position: second_run.witness.polyline[0],
            certified: true,
            via: None,
        });
        portfolio.variables[0].families[0].runs.push(second_run);
        assert!(codes(&portfolio).contains("fixed_witness_requires_single_via_free_run"));

        let mut value = serde_json::to_value(clear_fixed_pair_fixture()).unwrap();
        value.as_object_mut().unwrap().remove("feasibility_mode");
        value["passage_certificates"][0]
            .as_object_mut()
            .unwrap()
            .remove("enforcement");
        value["variables"][0]
            .as_object_mut()
            .unwrap()
            .remove("required_clearance");
        value["pair_analysis"]
            .as_object_mut()
            .unwrap()
            .remove("analyzed_family_pairs");
        let portfolio: RouteFamilyPortfolio = serde_json::from_value(value).unwrap();
        let codes = codes(&portfolio);
        assert!(codes.contains("v3_feasibility_mode_missing"));
        assert!(codes.contains("v3_passage_enforcement_missing"));
        assert!(codes.contains("v3_required_clearance_missing"));
        assert!(codes.contains("v3_analyzed_family_pairs_missing"));
    }

    fn v5_multi_run_fixture() -> RouteFamilyPortfolio {
        let mut portfolio = v4_context_fixture(Vec::new());
        portfolio.schema_version = MULTI_RUN_ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION.into();
        portfolio.corridor_epochs.push(CorridorEpoch {
            layer: "bottom".into(),
            corridor_revision: 12,
            semantic_fingerprint: "synthetic-bottom-semantic-v1".into(),
            cut_basis_fingerprint: "synthetic-bottom-cut-v1".into(),
            cut_basis_complete: true,
        });
        let family = &mut portfolio.variables[0].families[0];
        let mut continuation = family.runs[0].clone();
        let transition = FamilyPoint { x: 1.0, y: 1.0 };
        family.runs[0].end_point = 1;
        family.runs[0].witness.polyline = vec![FamilyPoint { x: 0.0, y: 1.0 }, transition];
        continuation.run_id = "signal/family-0/run-1".into();
        continuation.run_index = 1;
        continuation.layer = "bottom".into();
        continuation.start_point = 1;
        continuation.end_point = 2;
        continuation.corridor_revision = 12;
        continuation.transition_from_previous = RequiredNullable::Value(RouteTransition {
            kind: RouteTransitionKind::Via,
            position: transition,
            certified: true,
            via: Some(RouteViaGeometry {
                diameter: 0.8,
                drill: 0.4,
                clearance_model: EXACT_VIA_CLEARANCE_MODEL.into(),
                minimum_clearance: 1.0,
            }),
        });
        continuation.witness.polyline = vec![transition, FamilyPoint { x: 2.0, y: 1.0 }];
        continuation.cut_word_certificate.cut_basis_fingerprint = "synthetic-bottom-cut-v1".into();
        continuation.semantic_passage_keys.clear();
        family.runs.push(continuation);
        portfolio.seal_pair_analysis_fingerprints().unwrap();
        portfolio.validate().unwrap();
        portfolio
    }

    #[test]
    fn v5_admits_only_certified_contiguous_multi_run_witnesses() {
        let portfolio = v5_multi_run_fixture();
        let json = serde_json::to_string(&portfolio).unwrap();
        assert_eq!(
            parse_and_validate_route_family_portfolio(&json).unwrap(),
            portfolio
        );

        let mut uncertified = portfolio.clone();
        let RequiredNullable::Value(transition) =
            &mut uncertified.variables[0].families[0].runs[1].transition_from_previous
        else {
            unreachable!()
        };
        transition.certified = false;
        uncertified.seal_pair_analysis_fingerprints().unwrap();
        assert!(codes(&uncertified).contains("run_transition_uncertified"));

        let mut mismatched_range = portfolio.clone();
        mismatched_range.variables[0].families[0].runs[1].end_point = 3;
        mismatched_range.seal_pair_analysis_fingerprints().unwrap();
        assert!(codes(&mismatched_range).contains("route_run_point_range_witness_mismatch"));

        let mut reversed_range = portfolio;
        reversed_range.variables[0].families[0].runs[1].end_point = 0;
        let reversed_codes = codes(&reversed_range);
        assert!(reversed_codes.contains("route_run_point_range_empty"));
        assert!(reversed_codes.contains("route_run_point_range_witness_mismatch"));
    }

    #[test]
    fn v5_rebuild_classifies_via_annuli_against_families_and_context() {
        let mut portfolio = v5_multi_run_fixture();
        let mut other = portfolio.variables[0].clone();
        other.branch = "via-neighbor".into();
        other.electrical_net = "via-neighbor".into();
        other.families[0].family_id = "via-neighbor/family-0".into();
        other.families[0].runs.truncate(1);
        let run = &mut other.families[0].runs[0];
        run.run_id = "via-neighbor/family-0/run-0".into();
        run.start_point = 0;
        run.end_point = 1;
        run.witness.polyline = vec![
            FamilyPoint { x: 1.85, y: 0.5 },
            FamilyPoint { x: 1.85, y: 1.5 },
        ];
        portfolio.variables.push(other);
        portfolio
            .fixed_context
            .as_mut()
            .unwrap()
            .runs
            .push(FixedContextRun {
                context_id: "fixed-near-via".into(),
                branch: "fixed".into(),
                electrical_net: "fixed".into(),
                layer: "bottom".into(),
                required_width: 0.2,
                required_clearance: 0.25,
                polyline: vec![
                    FamilyPoint { x: 1.85, y: 0.5 },
                    FamilyPoint { x: 1.85, y: 1.5 },
                ],
                placement_revision: portfolio.placement_revision,
                geometry_revision: portfolio.geometry_revision.unwrap(),
            });

        portfolio.rebuild_and_seal_fixed_witness_analysis().unwrap();

        assert_eq!(portfolio.pair_analysis.analyzed_family_pairs, Some(3));
        assert!(!portfolio.pair_analysis.conflicts.is_empty());
        assert!(
            portfolio
                .pair_analysis
                .conflicts
                .iter()
                .any(|conflict| conflict.evidence.detail.contains("via:"))
        );
        assert_eq!(
            portfolio.pair_analysis.analyzed_family_context_pairs,
            Some(3)
        );
        assert!(!portfolio.pair_analysis.context_conflicts.is_empty());
        assert!(
            portfolio
                .pair_analysis
                .context_conflicts
                .iter()
                .any(|conflict| conflict.evidence.detail.contains("via:"))
        );
    }

    #[test]
    fn v3_rejects_legacy_scalar_capacity_mode() {
        let mut portfolio = fixture();
        portfolio.schema_version = FIXED_WITNESS_ROUTE_FAMILY_PORTFOLIO_SCHEMA_VERSION.into();
        portfolio.feasibility_mode = Some(RouteFamilyFeasibilityMode::ExclusivePassageCapacity);
        portfolio.passage_certificates[0].enforcement = Some(PassageEnforcementMode::ScalarWidth);
        portfolio.variables[0].required_clearance = Some(0.25);
        portfolio.pair_analysis.analyzed_family_pairs = Some(0);
        portfolio.seal_pair_analysis_fingerprints().unwrap();
        assert!(codes(&portfolio).contains("v3_fixed_witness_mode_required"));
    }

    #[test]
    fn programmatic_non_finite_candidate_never_panics() {
        let mut portfolio = fixture();
        portfolio.variables[0].families[0].cost = f64::NAN;
        let codes = codes(&portfolio);
        assert!(codes.contains("schema_number_not_finite"));
        assert!(codes.contains("candidate_set_fingerprint_unavailable"));
    }

    #[test]
    fn unknown_or_missing_fields_fail_during_parse() {
        let portfolio = fixture();
        let mut value = serde_json::to_value(&portfolio).unwrap();
        value["outer_loop_hint"] = Value::String("not in v2".to_owned());
        assert!(matches!(
            parse_and_validate_route_family_portfolio(&value.to_string()),
            Err(RouteFamilyIrError::Json(_))
        ));

        let mut value = serde_json::to_value(fixture()).unwrap();
        value["schema_version"] =
            Value::String("layout-trace.route-family-portfolio/v1".to_owned());
        assert!(matches!(
            parse_and_validate_route_family_portfolio(&value.to_string()),
            Err(RouteFamilyIrError::Validation(_))
        ));

        let mut value = serde_json::to_value(&portfolio).unwrap();
        value.as_object_mut().unwrap().remove("pair_analysis");
        assert!(matches!(
            parse_and_validate_route_family_portfolio(&value.to_string()),
            Err(RouteFamilyIrError::Json(_))
        ));

        let mut value = serde_json::to_value(&portfolio).unwrap();
        value["variables"][0]["families"][0]["runs"][0]
            .as_object_mut()
            .unwrap()
            .remove("transition_from_previous");
        assert!(matches!(
            parse_and_validate_route_family_portfolio(&value.to_string()),
            Err(RouteFamilyIrError::Json(_))
        ));
    }
}
