// Copyright (C) 2026 Toit contributors.

use pcb_kicad::{
    KiCadBoardStatistics, KiCadCopperStripReport, KiCadGroundZoneConfig,
    KiCadGroundZoneReplacement, KiCadLayerRenderConfig, KiCadLayerRenderReport,
    KiCadPhysicalCopperStatistics, KiCadRouteQuality, SemanticKiCadPose,
    SemanticKiCadTemplateConfig, VerificationReport, drc_design_issues,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

pub const COMPETITIVE_ROUTE_BENCHMARK_SCHEMA_VERSION: u32 = 1;
pub const COMPETITIVE_ROUTE_RESULT_SCHEMA_VERSION: u32 = 1;
pub const COMPETITIVE_ROUTE_COMPARISON_SCHEMA_VERSION: u32 = 1;
pub const COMPETITIVE_ROUTE_CORPUS_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompetitiveRouteBenchmarkConfig {
    pub schema_version: u32,
    pub id: String,
    pub source: CompetitiveRouteSource,
    /// Optional proof obligations for semantic auto-placement. These test
    /// behavior relative to the declared input without prescribing one exact
    /// output pose or placement algorithm.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placement_acceptance: Option<CompetitivePlacementAcceptance>,
    /// Optional observable topology obligations applied independently to both
    /// routed results in a paired comparison.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_acceptance: Option<CompetitiveResultAcceptance>,
    /// Deterministic transformations applied independently after each router
    /// finishes but before native result admission.
    #[serde(default)]
    pub post_route_processors: Vec<CompetitivePostRouteProcessor>,
    pub rules: CompetitiveRouteRules,
    pub freerouting: FreeroutingConfig,
    #[serde(default = "default_true")]
    pub require_zero_source_copper: bool,
    #[serde(default)]
    pub render: KiCadLayerRenderConfig,
    /// Directory receiving the mandatory paired source/result renders. File
    /// names are derived from the run output directory, so a manifest can be
    /// reused without overwriting an earlier experiment.
    pub journal_directory: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompetitivePlacementAcceptance {
    #[serde(default)]
    pub minimum_moved_components: usize,
    #[serde(default)]
    pub minimum_translated_components: usize,
    #[serde(default)]
    pub minimum_rotated_components: usize,
    #[serde(default)]
    pub minimum_connectivity_distance_reduction_mm: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CompetitivePlacementEvidence {
    pub moved_components: usize,
    pub translated_components: usize,
    pub rotated_components: usize,
    pub connectivity_distance_before_mm: f64,
    pub connectivity_distance_after_mm: f64,
    pub connectivity_distance_reduction_mm: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompetitiveRoutingFeedbackPlacementConfig {
    pub routing_config: PathBuf,
    pub pressure_config: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CompetitiveRoutingFeedbackPlacementEvidence {
    pub initial_routing_complete: bool,
    pub selected_routing_complete: bool,
    pub selected_attempt: usize,
    pub attempted_repairs: usize,
    pub total_expansions: u64,
    pub moved_components: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompetitiveResultAcceptance {
    #[serde(default)]
    pub minimum_vias: usize,
    #[serde(default)]
    pub minimum_used_copper_layers: usize,
    #[serde(default)]
    pub minimum_copper_zones: usize,
    #[serde(default)]
    pub minimum_filled_zone_area_mm2: f64,
    #[serde(default)]
    pub differential_pairs: Vec<CompetitiveDifferentialPairAcceptance>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompetitiveDifferentialPairAcceptance {
    pub positive_connection: String,
    pub negative_connection: String,
    pub maximum_length_skew_mm: f64,
    pub maximum_via_count_difference: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CompetitiveDifferentialPairEvidence {
    pub positive_connection: String,
    pub negative_connection: String,
    pub positive_quality: Option<KiCadRouteQuality>,
    pub negative_quality: Option<KiCadRouteQuality>,
    pub length_skew_mm: Option<f64>,
    pub via_count_difference: Option<usize>,
    pub complete: bool,
    pub failure: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CompetitivePostRouteProcessor {
    GroundZoneReplacement { config: KiCadGroundZoneConfig },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CompetitivePostRouteEvidence {
    GroundZoneReplacement {
        config: KiCadGroundZoneConfig,
        replacement: KiCadGroundZoneReplacement,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompetitiveRouteComparisonConfig {
    pub schema_version: u32,
    pub id: String,
    pub route_benchmark: PathBuf,
    pub pcb_maker: PcbMakerComparisonConfig,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PcbMakerComparisonConfig {
    #[serde(default)]
    pub insertion_config: PathBuf,
    #[serde(default)]
    pub sequential_config: PathBuf,
    pub verification_cache_directory: PathBuf,
    pub maximum_seconds: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompetitiveRouteCorpusConfig {
    pub schema_version: u32,
    pub id: String,
    pub cases: Vec<CompetitiveRouteCorpusCaseConfig>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompetitiveRouteCorpusCaseConfig {
    pub id: String,
    pub comparison: PathBuf,
}

impl CompetitiveRouteCorpusConfig {
    pub fn check(&self) -> Result<(), String> {
        if self.schema_version != COMPETITIVE_ROUTE_CORPUS_SCHEMA_VERSION {
            return Err(format!(
                "competitive corpus schema_version must be {}",
                COMPETITIVE_ROUTE_CORPUS_SCHEMA_VERSION
            ));
        }
        check_artifact_id(&self.id, "corpus")?;
        if self.cases.is_empty() {
            return Err("competitive corpus must contain at least one case".into());
        }
        let mut ids = BTreeSet::new();
        for case in &self.cases {
            check_artifact_id(&case.id, "corpus case")?;
            if case.comparison.as_os_str().is_empty() {
                return Err(format!("corpus case {} has no comparison path", case.id));
            }
            if !ids.insert(&case.id) {
                return Err(format!("duplicate corpus case id {}", case.id));
            }
        }
        Ok(())
    }
}

fn check_artifact_id(id: &str, kind: &str) -> Result<(), String> {
    if id.is_empty()
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(format!(
            "{kind} id must contain lowercase ASCII letters, digits, or '-'"
        ));
    }
    Ok(())
}

impl CompetitiveRouteComparisonConfig {
    pub fn check(&self) -> Result<(), String> {
        if self.schema_version != COMPETITIVE_ROUTE_COMPARISON_SCHEMA_VERSION {
            return Err(format!(
                "competitive comparison schema_version must be {}",
                COMPETITIVE_ROUTE_COMPARISON_SCHEMA_VERSION
            ));
        }
        if self.id.is_empty()
            || !self
                .id
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err(
                "comparison id must contain lowercase ASCII letters, digits, or '-'".into(),
            );
        }
        if self.route_benchmark.as_os_str().is_empty()
            || self
                .pcb_maker
                .verification_cache_directory
                .as_os_str()
                .is_empty()
        {
            return Err("comparison input paths must be non-empty".into());
        }
        let has_insertion = !self.pcb_maker.insertion_config.as_os_str().is_empty();
        let has_sequential = !self.pcb_maker.sequential_config.as_os_str().is_empty();
        if has_insertion == has_sequential {
            return Err(
                "pcb-maker comparison must select exactly one of insertion_config or sequential_config"
                    .into(),
            );
        }
        if self.pcb_maker.maximum_seconds == 0 || self.pcb_maker.maximum_seconds > 86_400 {
            return Err("pcb-maker comparison maximum_seconds must be in 1..=86400".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CompetitiveRouteSource {
    MaterializedKicad {
        directory: PathBuf,
        board_id: String,
    },
    /// Copy a complete upstream KiCad project, then remove only its inherited
    /// routed copper before either router sees it.
    ColdKicadProject {
        directory: PathBuf,
        board_id: String,
    },
    SemanticKicad {
        problem: PathBuf,
        placement_config: PathBuf,
        template_config: PathBuf,
        connection_prefix: usize,
        /// Optional routing-failure feedback pass. Its semantic routes are
        /// discarded; only the selected legal placement is materialized, so
        /// both competitors still receive identical zero-copper input.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        routing_feedback: Option<CompetitiveRoutingFeedbackPlacementConfig>,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompetitiveRouteRules {
    pub trace_width_mm: f64,
    pub clearance_mm: f64,
    pub via_diameter_mm: f64,
    pub via_drill_mm: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FreeroutingConfig {
    pub jar: PathBuf,
    pub expected_jar_sha256: String,
    #[serde(default = "default_java_executable")]
    pub java_executable: PathBuf,
    #[serde(default = "default_python_executable")]
    pub python_executable: PathBuf,
    #[serde(default = "default_maximum_seconds")]
    pub maximum_seconds: u64,
    #[serde(default = "default_maximum_passes")]
    pub maximum_passes: u32,
    #[serde(default = "default_maximum_threads")]
    pub maximum_threads: u32,
}

fn default_true() -> bool {
    true
}

fn default_java_executable() -> PathBuf {
    PathBuf::from("java")
}

fn default_python_executable() -> PathBuf {
    PathBuf::from("python3")
}

fn default_maximum_seconds() -> u64 {
    60
}

fn default_maximum_passes() -> u32 {
    100
}

fn default_maximum_threads() -> u32 {
    1
}

impl CompetitiveRouteBenchmarkConfig {
    pub fn check(&self) -> Result<(), String> {
        if self.schema_version != COMPETITIVE_ROUTE_BENCHMARK_SCHEMA_VERSION {
            return Err(format!(
                "competitive route benchmark schema_version must be {}",
                COMPETITIVE_ROUTE_BENCHMARK_SCHEMA_VERSION
            ));
        }
        if self.id.is_empty()
            || !self
                .id
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err("benchmark id must contain lowercase ASCII letters, digits, or '-'".into());
        }
        if self.journal_directory.as_os_str().is_empty() {
            return Err("journal_directory must be non-empty".into());
        }
        if !self.require_zero_source_copper {
            return Err(
                "schema v1 only admits cold route-only benchmarks with zero source copper".into(),
            );
        }
        if let Some(acceptance) = &self.placement_acceptance {
            if !matches!(self.source, CompetitiveRouteSource::SemanticKicad { .. }) {
                return Err("placement_acceptance requires a semantic_kicad source".into());
            }
            if !acceptance
                .minimum_connectivity_distance_reduction_mm
                .is_finite()
                || acceptance.minimum_connectivity_distance_reduction_mm < 0.0
            {
                return Err(
                    "minimum_connectivity_distance_reduction_mm must be finite and non-negative"
                        .into(),
                );
            }
            if acceptance.minimum_moved_components == 0
                && acceptance.minimum_translated_components == 0
                && acceptance.minimum_rotated_components == 0
                && acceptance.minimum_connectivity_distance_reduction_mm == 0.0
            {
                return Err("placement_acceptance must contain a positive requirement".into());
            }
        }
        if let Some(acceptance) = &self.result_acceptance {
            if acceptance.minimum_vias == 0
                && acceptance.minimum_used_copper_layers == 0
                && acceptance.minimum_copper_zones == 0
                && acceptance.minimum_filled_zone_area_mm2 == 0.0
                && acceptance.differential_pairs.is_empty()
            {
                return Err("result_acceptance must contain a positive requirement".into());
            }
            if acceptance.minimum_used_copper_layers > 32 {
                return Err("minimum_used_copper_layers must not exceed 32".into());
            }
            if !acceptance.minimum_filled_zone_area_mm2.is_finite()
                || acceptance.minimum_filled_zone_area_mm2 < 0.0
            {
                return Err("minimum_filled_zone_area_mm2 must be finite and non-negative".into());
            }
            let mut differential_connections = BTreeSet::new();
            for pair in &acceptance.differential_pairs {
                if pair.positive_connection.is_empty()
                    || pair.negative_connection.is_empty()
                    || pair.positive_connection == pair.negative_connection
                    || !pair.maximum_length_skew_mm.is_finite()
                    || pair.maximum_length_skew_mm < 0.0
                {
                    return Err(
                        "differential-pair acceptance contains invalid connections or skew".into(),
                    );
                }
                for connection in [&pair.positive_connection, &pair.negative_connection] {
                    if !differential_connections.insert(connection) {
                        return Err(format!(
                            "differential connection {connection:?} appears in multiple acceptance pairs"
                        ));
                    }
                }
            }
        }
        let rules = &self.rules;
        if !rules.trace_width_mm.is_finite()
            || rules.trace_width_mm <= 0.0
            || !rules.clearance_mm.is_finite()
            || rules.clearance_mm < 0.0
            || !rules.via_diameter_mm.is_finite()
            || rules.via_diameter_mm <= 0.0
            || !rules.via_drill_mm.is_finite()
            || rules.via_drill_mm <= 0.0
            || rules.via_drill_mm >= rules.via_diameter_mm
        {
            return Err("competitive route rules contain invalid geometry".into());
        }
        if self.freerouting.expected_jar_sha256.len() != 64
            || !self
                .freerouting
                .expected_jar_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err("expected_jar_sha256 must be 64 lowercase hexadecimal digits".into());
        }
        if self.freerouting.maximum_seconds == 0
            || self.freerouting.maximum_seconds > 86_400
            || self.freerouting.maximum_passes == 0
            || self.freerouting.maximum_passes > 10_000
            || self.freerouting.maximum_threads == 0
            || self.freerouting.maximum_threads > 1_024
        {
            return Err("invalid bounded Freerouting configuration".into());
        }
        match &self.source {
            CompetitiveRouteSource::MaterializedKicad { board_id, .. }
            | CompetitiveRouteSource::ColdKicadProject { board_id, .. } => {
                check_board_id(board_id)?;
            }
            CompetitiveRouteSource::SemanticKicad {
                routing_feedback, ..
            } => {
                if routing_feedback.as_ref().is_some_and(|feedback| {
                    feedback.routing_config.as_os_str().is_empty()
                        || feedback.pressure_config.as_os_str().is_empty()
                }) {
                    return Err("routing-feedback placement config paths must be non-empty".into());
                }
            }
        }
        Ok(())
    }
}

fn check_board_id(board_id: &str) -> Result<(), String> {
    if board_id.is_empty()
        || board_id.contains('/')
        || board_id.contains('\\')
        || board_id == "."
        || board_id == ".."
    {
        Err("board_id must be a non-empty filename stem".into())
    } else {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompetitiveRouteRunStatus {
    Running,
    Complete,
    Incomplete,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CompetitiveRouteStage {
    pub name: String,
    pub elapsed_micros: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CompetitiveSourceEvidence {
    pub kind: String,
    pub board_id: String,
    pub input_hashes: BTreeMap<String, String>,
    #[serde(default)]
    pub copper_strip: Option<KiCadCopperStripReport>,
    pub board_sha256: String,
    pub directory_sha256: String,
    pub statistics: KiCadBoardStatistics,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placement: Option<CompetitivePlacementEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing_feedback: Option<CompetitiveRoutingFeedbackPlacementEvidence>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CompetitiveToolEvidence {
    pub kicad_cli_version: String,
    pub pcbnew_version: String,
    pub java_version: String,
    pub freerouting_version: Option<String>,
    pub freerouting_jar: PathBuf,
    pub freerouting_jar_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FreeroutingRunEvidence {
    pub maximum_seconds: u64,
    pub maximum_passes: u32,
    pub maximum_threads: u32,
    pub exit_code: i32,
    pub elapsed_micros: u64,
    pub autorouter_passes: usize,
    pub reported_initial_unrouted_items: Option<usize>,
    pub reported_final_unrouted_items: Option<usize>,
    pub reported_cpu_seconds: Option<f64>,
    pub dsn: PathBuf,
    pub dsn_sha256: String,
    pub session: PathBuf,
    pub session_sha256: String,
    pub log: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CompetitiveRouteResultEvidence {
    pub board: PathBuf,
    pub board_sha256: String,
    pub directory_sha256: String,
    pub statistics: KiCadBoardStatistics,
    /// Absolute native report for the routed board. `complete` can be false
    /// when a frozen upstream fixture carries unrelated pre-existing issues;
    /// route-only completion is decided by `route_admission` below.
    pub verification: VerificationReport,
    #[serde(default)]
    pub route_admission: Option<CompetitiveRouteAdmissionEvidence>,
    #[serde(default)]
    pub post_route_processors: Vec<CompetitivePostRouteEvidence>,
    #[serde(default)]
    pub differential_pairs: Vec<CompetitiveDifferentialPairEvidence>,
    pub source_render: KiCadLayerRenderReport,
    pub result_render: KiCadLayerRenderReport,
    pub journal_renders: Vec<PathBuf>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CompetitiveRouteAdmissionEvidence {
    pub contract: String,
    pub complete: bool,
    pub source_baseline: VerificationReport,
    pub result: VerificationReport,
    pub introduced_erc_violations: usize,
    pub introduced_drc_design_violations: usize,
    pub introduced_schematic_parity_issues: usize,
    pub result_selected_net_unconnected_items: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CompetitiveRouteRunReport {
    pub schema_version: u32,
    pub benchmark_id: String,
    pub contract: String,
    pub status: CompetitiveRouteRunStatus,
    pub active_stage: Option<String>,
    pub failure: Option<String>,
    pub config_path: PathBuf,
    pub config_sha256: String,
    pub output_directory: PathBuf,
    pub rules: CompetitiveRouteRules,
    pub source: Option<CompetitiveSourceEvidence>,
    pub tools: Option<CompetitiveToolEvidence>,
    pub routing: Option<FreeroutingRunEvidence>,
    pub result: Option<CompetitiveRouteResultEvidence>,
    pub stages: Vec<CompetitiveRouteStage>,
    pub total_elapsed_micros: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompetitiveComparisonStatus {
    Finished,
    Failed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompetitiveCorpusStatus {
    Finished,
    Failed,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct CompetitiveCorpusTotals {
    pub cases: usize,
    pub finished: usize,
    pub failed: usize,
    pub both_solved: usize,
    pub freerouting_only: usize,
    pub pcb_maker_only: usize,
    pub neither_solved: usize,
    pub cache_hits: usize,
    pub cache_misses: usize,
    pub cache_bypasses: usize,
    pub elapsed_micros: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CompetitiveCorpusRouterResult {
    pub solved: bool,
    pub segments: Option<usize>,
    pub vias: Option<usize>,
    pub stored_segment_length_mm: Option<f64>,
    pub physical_copper: Option<KiCadPhysicalCopperStatistics>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CompetitiveCorpusCaseEvidence {
    pub id: String,
    pub comparison_config: PathBuf,
    pub output_directory: PathBuf,
    pub report: PathBuf,
    pub status: CompetitiveComparisonStatus,
    pub failure: Option<String>,
    pub fixed_placement_matches: bool,
    pub rules_match: bool,
    pub completion_leader: String,
    pub freerouting: CompetitiveCorpusRouterResult,
    pub pcb_maker: CompetitiveCorpusRouterResult,
    pub cache_hits: usize,
    pub cache_misses: usize,
    pub cache_bypasses: usize,
    pub elapsed_micros: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CompetitiveRouteCorpusReport {
    pub schema_version: u32,
    pub corpus_id: String,
    pub contract: String,
    pub status: CompetitiveCorpusStatus,
    pub config_path: PathBuf,
    pub config_sha256: String,
    pub output_directory: PathBuf,
    pub cases: Vec<CompetitiveCorpusCaseEvidence>,
    pub totals: CompetitiveCorpusTotals,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CompetitivePcbMakerEvidence {
    #[serde(default)]
    pub mode: String,
    pub executable: PathBuf,
    pub executable_sha256: String,
    pub maximum_seconds: u64,
    pub exit_code: i32,
    pub timed_out: bool,
    pub elapsed_micros: u64,
    pub log: PathBuf,
    pub cold_prefix_manifest: Option<PathBuf>,
    pub progression_manifest: Option<PathBuf>,
    #[serde(default)]
    pub sequential_route_manifest: Option<PathBuf>,
    pub target_rung: usize,
    pub final_rung: usize,
    pub termination: String,
    pub total_route_expansions: u64,
    pub solved_target: bool,
    pub result_directory: PathBuf,
    pub board: PathBuf,
    pub board_sha256: String,
    pub statistics: KiCadBoardStatistics,
    pub verification: VerificationReport,
    #[serde(default)]
    pub route_admission: Option<CompetitiveRouteAdmissionEvidence>,
    #[serde(default)]
    pub post_route_processors: Vec<CompetitivePostRouteEvidence>,
    #[serde(default)]
    pub differential_pairs: Vec<CompetitiveDifferentialPairEvidence>,
    pub render: KiCadLayerRenderReport,
    pub journal_renders: Vec<PathBuf>,
    pub native_verification_cache: CompetitiveNativeCacheEvidence,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CompetitiveNativeCacheEvidence {
    pub directory: PathBuf,
    pub event_log: PathBuf,
    pub events: usize,
    pub hits: usize,
    pub misses: usize,
    pub bypasses: usize,
    pub unique_keys: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CompetitiveRouteComparisonReport {
    pub schema_version: u32,
    pub benchmark_id: String,
    pub contract: String,
    pub status: CompetitiveComparisonStatus,
    pub failure: Option<String>,
    pub config_path: PathBuf,
    pub config_sha256: String,
    pub output_directory: PathBuf,
    pub semantic_input_hashes: BTreeMap<String, String>,
    pub placement_comparison_grid_mm: f64,
    pub fixed_placement_matches: bool,
    pub rules_match: bool,
    pub freerouting_solved: bool,
    pub pcb_maker_solved: bool,
    pub completion_leader: String,
    pub freerouting_report: PathBuf,
    pub pcb_maker: Option<CompetitivePcbMakerEvidence>,
    pub total_elapsed_micros: u64,
}

impl CompetitiveRouteRunReport {
    pub fn complete(&self) -> bool {
        self.status == CompetitiveRouteRunStatus::Complete
    }
}

pub fn load_competitive_route_benchmark(
    path: &Path,
) -> Result<CompetitiveRouteBenchmarkConfig, String> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let config: CompetitiveRouteBenchmarkConfig = serde_json::from_str(&source)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))?;
    config.check()?;
    Ok(config)
}

pub fn run_freerouting_benchmark(
    config_path: &Path,
    output_directory: &Path,
) -> Result<CompetitiveRouteRunReport, String> {
    let config_path = absolute(config_path)?;
    let config_source = fs::read(&config_path)
        .map_err(|error| format!("failed to read {}: {error}", config_path.display()))?;
    let config: CompetitiveRouteBenchmarkConfig = serde_json::from_slice(&config_source)
        .map_err(|error| format!("failed to parse {}: {error}", config_path.display()))?;
    config.check()?;
    let config_base = config_path
        .parent()
        .ok_or_else(|| "benchmark config has no parent directory".to_string())?;
    let output_directory = absolute(output_directory)?;
    if output_directory.exists() {
        return Err(format!(
            "refusing to overwrite competitive benchmark output {}",
            output_directory.display()
        ));
    }
    fs::create_dir_all(&output_directory).map_err(|error| {
        format!(
            "failed to create competitive benchmark output {}: {error}",
            output_directory.display()
        )
    })?;
    fs::write(
        output_directory.join("benchmark-config.json"),
        &config_source,
    )
    .map_err(|error| format!("failed to snapshot benchmark config: {error}"))?;

    let started = Instant::now();
    let mut report = CompetitiveRouteRunReport {
        schema_version: COMPETITIVE_ROUTE_RESULT_SCHEMA_VERSION,
        benchmark_id: config.id.clone(),
        contract: "cold fixed-placement route-only comparison; regenerate semantic placement when configured; zero inherited tracks/arcs/vias/zones; apply declared rules before DSN export; exact native KiCad admission after SES import".into(),
        status: CompetitiveRouteRunStatus::Running,
        active_stage: None,
        failure: None,
        config_path: config_path.clone(),
        config_sha256: sha256_bytes(&config_source),
        output_directory: output_directory.clone(),
        rules: config.rules.clone(),
        source: None,
        tools: None,
        routing: None,
        result: None,
        stages: Vec::new(),
        total_elapsed_micros: 0,
    };
    write_report(&output_directory, &report)?;

    if let Err(error) =
        run_freerouting_benchmark_inner(&config, config_base, &output_directory, &mut report)
    {
        report.status = CompetitiveRouteRunStatus::Failed;
        report.failure = Some(error);
    }
    report.active_stage = None;
    report.total_elapsed_micros = started.elapsed().as_micros() as u64;
    write_report(&output_directory, &report)?;
    Ok(report)
}

pub fn run_route_corpus(
    corpus_config_path: &Path,
    output_directory: &Path,
) -> Result<CompetitiveRouteCorpusReport, String> {
    let started = Instant::now();
    let corpus_config_path = absolute(corpus_config_path)?;
    let config_source = fs::read(&corpus_config_path)
        .map_err(|error| format!("failed to read {}: {error}", corpus_config_path.display()))?;
    let config: CompetitiveRouteCorpusConfig = serde_json::from_slice(&config_source)
        .map_err(|error| format!("failed to parse {}: {error}", corpus_config_path.display()))?;
    config.check()?;
    let config_base = corpus_config_path
        .parent()
        .ok_or_else(|| "corpus config has no parent directory".to_string())?
        .to_path_buf();
    let output_directory = absolute(output_directory)?;
    if output_directory.exists() {
        return Err(format!(
            "refusing to overwrite competitive corpus output {}",
            output_directory.display()
        ));
    }
    fs::create_dir_all(&output_directory).map_err(|error| {
        format!(
            "failed to create competitive corpus output {}: {error}",
            output_directory.display()
        )
    })?;
    fs::write(output_directory.join("corpus-config.json"), &config_source)
        .map_err(|error| format!("failed to snapshot corpus config: {error}"))?;

    let mut report = CompetitiveRouteCorpusReport {
        schema_version: COMPETITIVE_ROUTE_CORPUS_SCHEMA_VERSION,
        corpus_id: config.id,
        contract: "run every paired comparison independently from its declared cold semantic input; retain each self-contained report and render set; aggregate completion before route quality".into(),
        status: CompetitiveCorpusStatus::Failed,
        config_path: corpus_config_path,
        config_sha256: sha256_bytes(&config_source),
        output_directory: output_directory.clone(),
        cases: Vec::new(),
        totals: CompetitiveCorpusTotals {
            cases: config.cases.len(),
            ..CompetitiveCorpusTotals::default()
        },
    };
    write_corpus_report(&output_directory, &report)?;

    for case in config.cases {
        let comparison_config = resolve_file(&config_base, &case.comparison);
        let case_output = output_directory.join(&case.id);
        let case_report = case_output.join("competitive-comparison.json");
        let evaluated =
            run_route_comparison(&comparison_config, &case_output).and_then(|comparison| {
                summarize_corpus_case(case.id.clone(), comparison_config.clone(), comparison)
            });
        let evidence = match evaluated {
            Ok(evidence) => evidence,
            Err(error) => CompetitiveCorpusCaseEvidence {
                id: case.id,
                comparison_config,
                output_directory: case_output,
                report: case_report,
                status: CompetitiveComparisonStatus::Failed,
                failure: Some(error),
                fixed_placement_matches: false,
                rules_match: false,
                completion_leader: "undetermined".into(),
                freerouting: empty_corpus_router_result(),
                pcb_maker: empty_corpus_router_result(),
                cache_hits: 0,
                cache_misses: 0,
                cache_bypasses: 0,
                elapsed_micros: 0,
            },
        };
        accumulate_corpus_case(&mut report.totals, &evidence);
        report.cases.push(evidence);
        report.totals.elapsed_micros = started.elapsed().as_micros() as u64;
        write_corpus_report(&output_directory, &report)?;
    }

    report.status = if report.totals.failed == 0 {
        CompetitiveCorpusStatus::Finished
    } else {
        CompetitiveCorpusStatus::Failed
    };
    report.totals.elapsed_micros = started.elapsed().as_micros() as u64;
    write_corpus_report(&output_directory, &report)?;
    Ok(report)
}

fn summarize_corpus_case(
    id: String,
    comparison_config: PathBuf,
    comparison: CompetitiveRouteComparisonReport,
) -> Result<CompetitiveCorpusCaseEvidence, String> {
    let freerouting_report: CompetitiveRouteRunReport = read_json(&comparison.freerouting_report)?;
    let freerouting_statistics = freerouting_report
        .result
        .as_ref()
        .map(|result| &result.statistics);
    let pcb_maker_statistics = comparison
        .pcb_maker
        .as_ref()
        .map(|evidence| &evidence.statistics);
    let cache = comparison
        .pcb_maker
        .as_ref()
        .map(|evidence| &evidence.native_verification_cache);
    Ok(CompetitiveCorpusCaseEvidence {
        id,
        comparison_config,
        output_directory: comparison.output_directory.clone(),
        report: comparison
            .output_directory
            .join("competitive-comparison.json"),
        status: comparison.status,
        failure: comparison.failure,
        fixed_placement_matches: comparison.fixed_placement_matches,
        rules_match: comparison.rules_match,
        completion_leader: comparison.completion_leader,
        freerouting: CompetitiveCorpusRouterResult {
            solved: comparison.freerouting_solved,
            segments: freerouting_statistics.map(|statistics| statistics.segments),
            vias: freerouting_statistics.map(|statistics| statistics.vias),
            stored_segment_length_mm: freerouting_statistics
                .map(|statistics| statistics.stored_segment_length_mm),
            physical_copper: freerouting_statistics
                .map(|statistics| statistics.physical_copper.clone()),
        },
        pcb_maker: CompetitiveCorpusRouterResult {
            solved: comparison.pcb_maker_solved,
            segments: pcb_maker_statistics.map(|statistics| statistics.segments),
            vias: pcb_maker_statistics.map(|statistics| statistics.vias),
            stored_segment_length_mm: pcb_maker_statistics
                .map(|statistics| statistics.stored_segment_length_mm),
            physical_copper: pcb_maker_statistics
                .map(|statistics| statistics.physical_copper.clone()),
        },
        cache_hits: cache.map_or(0, |cache| cache.hits),
        cache_misses: cache.map_or(0, |cache| cache.misses),
        cache_bypasses: cache.map_or(0, |cache| cache.bypasses),
        elapsed_micros: comparison.total_elapsed_micros,
    })
}

fn empty_corpus_router_result() -> CompetitiveCorpusRouterResult {
    CompetitiveCorpusRouterResult {
        solved: false,
        segments: None,
        vias: None,
        stored_segment_length_mm: None,
        physical_copper: None,
    }
}

fn accumulate_corpus_case(
    totals: &mut CompetitiveCorpusTotals,
    case: &CompetitiveCorpusCaseEvidence,
) {
    match case.status {
        CompetitiveComparisonStatus::Finished => totals.finished += 1,
        CompetitiveComparisonStatus::Failed => totals.failed += 1,
    }
    if case.status == CompetitiveComparisonStatus::Finished {
        match (case.freerouting.solved, case.pcb_maker.solved) {
            (true, true) => totals.both_solved += 1,
            (true, false) => totals.freerouting_only += 1,
            (false, true) => totals.pcb_maker_only += 1,
            (false, false) => totals.neither_solved += 1,
        }
    }
    totals.cache_hits += case.cache_hits;
    totals.cache_misses += case.cache_misses;
    totals.cache_bypasses += case.cache_bypasses;
}

fn write_corpus_report(
    output_directory: &Path,
    report: &CompetitiveRouteCorpusReport,
) -> Result<(), String> {
    let path = output_directory.join("competitive-corpus.json");
    let serialized = serde_json::to_string_pretty(report).map_err(|error| error.to_string())?;
    fs::write(&path, format!("{serialized}\n"))
        .map_err(|error| format!("failed to write {}: {error}", path.display()))
}

pub fn run_route_comparison(
    comparison_config_path: &Path,
    output_directory: &Path,
) -> Result<CompetitiveRouteComparisonReport, String> {
    let started = Instant::now();
    let comparison_config_path = absolute(comparison_config_path)?;
    let comparison_source = fs::read(&comparison_config_path).map_err(|error| {
        format!(
            "failed to read {}: {error}",
            comparison_config_path.display()
        )
    })?;
    let comparison: CompetitiveRouteComparisonConfig = serde_json::from_slice(&comparison_source)
        .map_err(|error| {
        format!(
            "failed to parse {}: {error}",
            comparison_config_path.display()
        )
    })?;
    comparison.check()?;
    let comparison_base = comparison_config_path
        .parent()
        .ok_or_else(|| "comparison config has no parent directory".to_string())?;
    let route_config_path = resolve_file(comparison_base, &comparison.route_benchmark);
    let route_config = load_competitive_route_benchmark(&route_config_path)?;
    let route_config_base = route_config_path
        .parent()
        .ok_or_else(|| "route benchmark config has no parent directory".to_string())?;
    let semantic_inputs = match &route_config.source {
        CompetitiveRouteSource::SemanticKicad {
            problem,
            placement_config,
            template_config,
            connection_prefix,
            ..
        } => Some((
            resolve_file(route_config_base, problem),
            resolve_file(route_config_base, placement_config),
            resolve_file(route_config_base, template_config),
            *connection_prefix,
        )),
        CompetitiveRouteSource::MaterializedKicad { .. }
        | CompetitiveRouteSource::ColdKicadProject { .. } => None,
    };
    let semantic_routing_feedback_source = matches!(
        &route_config.source,
        CompetitiveRouteSource::SemanticKicad {
            routing_feedback: Some(_),
            ..
        }
    );
    let insertion_config = (!comparison.pcb_maker.insertion_config.as_os_str().is_empty())
        .then(|| resolve_file(comparison_base, &comparison.pcb_maker.insertion_config));
    let sequential_config = (!comparison
        .pcb_maker
        .sequential_config
        .as_os_str()
        .is_empty())
    .then(|| resolve_file(comparison_base, &comparison.pcb_maker.sequential_config));
    match (
        &semantic_inputs,
        &insertion_config,
        &sequential_config,
        semantic_routing_feedback_source,
    ) {
        (Some(_), Some(config), None, false)
        | (Some(_), None, Some(config), true)
        | (None, None, Some(config), false) => {
            let value: serde_json::Value = read_json(config)?;
            validate_internal_routing_rules(&value, &route_config.rules)?;
        }
        (Some(_), _, _, true) => {
            return Err(
                "semantic_kicad routing-feedback comparison requires sequential_config".into(),
            );
        }
        (Some(_), _, _, false) => {
            return Err("semantic_kicad comparison requires insertion_config".into());
        }
        (None, _, _, _) => {
            return Err("materialized or cold KiCad comparison requires sequential_config".into());
        }
    }
    let verification_cache = resolve_file(
        comparison_base,
        &comparison.pcb_maker.verification_cache_directory,
    );

    let output_directory = absolute(output_directory)?;
    if output_directory.exists() {
        return Err(format!(
            "refusing to overwrite competitive comparison output {}",
            output_directory.display()
        ));
    }
    let freerouting = run_freerouting_benchmark(&route_config_path, &output_directory)?;
    fs::write(
        output_directory.join("comparison-config.json"),
        &comparison_source,
    )
    .map_err(|error| format!("failed to snapshot comparison config: {error}"))?;
    let freerouting_report = output_directory.join("competitive-result.json");
    let mut report = CompetitiveRouteComparisonReport {
        schema_version: COMPETITIVE_ROUTE_COMPARISON_SCHEMA_VERSION,
        benchmark_id: comparison.id,
        contract: "run Freerouting and pcb-maker from one frozen zero-copper source; semantic cases independently prune connectivity before placement, while direct KiCad cases share the adapted project byte-for-byte; require exact matching rules, fixed placement at the 0.0001 mm Specctra exchange grid, explicit process budgets, and native KiCad route admission".into(),
        status: CompetitiveComparisonStatus::Failed,
        failure: None,
        config_path: comparison_config_path,
        config_sha256: sha256_bytes(&comparison_source),
        output_directory: output_directory.clone(),
        semantic_input_hashes: freerouting
            .source
            .as_ref()
            .map(|source| source.input_hashes.clone())
            .unwrap_or_default(),
        placement_comparison_grid_mm: 0.0001,
        fixed_placement_matches: false,
        rules_match: true,
        freerouting_solved: freerouting.complete(),
        pcb_maker_solved: false,
        completion_leader: "undetermined".into(),
        freerouting_report,
        pcb_maker: None,
        total_elapsed_micros: 0,
    };
    write_comparison_report(&output_directory, &report)?;

    let result = if let Some(sequential_config) = sequential_config.as_deref() {
        run_pcb_maker_sequential_comparison_inner(
            &comparison.pcb_maker,
            &route_config,
            route_config_base,
            sequential_config,
            &verification_cache,
            &output_directory,
            &freerouting,
        )
    } else if let Some((problem, placement, template, target_rung)) = semantic_inputs {
        run_pcb_maker_comparison_inner(
            &comparison.pcb_maker,
            &route_config,
            route_config_base,
            &problem,
            &placement,
            &template,
            target_rung,
            insertion_config
                .as_deref()
                .ok_or_else(|| "semantic comparison lost insertion config".to_string())?,
            &verification_cache,
            &output_directory,
            &freerouting,
        )
    } else {
        Err("competitive comparison lost its pcb-maker routing configuration".into())
    };
    match result {
        Ok(pcb_maker) => {
            report.fixed_placement_matches = freerouting.source.as_ref().is_some_and(|source| {
                freerouting.result.as_ref().is_some_and(|result| {
                    let source = &source.statistics.component_placement_100nm_sha256;
                    source == &result.statistics.component_placement_100nm_sha256
                        && source == &pcb_maker.statistics.component_placement_100nm_sha256
                })
            });
            let result_acceptance = validate_result_acceptance(
                &route_config,
                pcb_maker.statistics.vias,
                pcb_maker.statistics.physical_copper.used_copper_layers,
                pcb_maker.statistics.copper_zones,
                pcb_maker.statistics.physical_copper.filled_zone_area_mm2,
                &pcb_maker.differential_pairs,
                "pcb-maker",
            );
            report.pcb_maker_solved = pcb_maker.solved_target && result_acceptance.is_ok();
            report.completion_leader = match (report.freerouting_solved, report.pcb_maker_solved) {
                (true, true) | (false, false) => "tie",
                (true, false) => "freerouting",
                (false, true) => "pcb-maker",
            }
            .into();
            if !report.fixed_placement_matches {
                report.failure =
                    Some("competitors did not receive the same fixed component placement".into());
            } else if let Err(error) = result_acceptance {
                report.failure = Some(error);
            } else {
                report.status = CompetitiveComparisonStatus::Finished;
            }
            report.pcb_maker = Some(pcb_maker);
        }
        Err(error) => report.failure = Some(error),
    }
    report.total_elapsed_micros = started.elapsed().as_micros() as u64;
    write_comparison_report(&output_directory, &report)?;
    Ok(report)
}

#[allow(clippy::too_many_arguments)]
fn run_pcb_maker_comparison_inner(
    config: &PcbMakerComparisonConfig,
    route_config: &CompetitiveRouteBenchmarkConfig,
    route_config_base: &Path,
    problem: &Path,
    placement: &Path,
    template: &Path,
    target_rung: usize,
    insertion_config: &Path,
    verification_cache: &Path,
    output_directory: &Path,
    freerouting: &CompetitiveRouteRunReport,
) -> Result<CompetitivePcbMakerEvidence, String> {
    // Completion is an outcome, not a prerequisite for running the other
    // competitor. Requiring Freerouting to solve first censored pcb-maker-only
    // wins and made genuine neither-solved cases indistinguishable from
    // harness failures. The source and result evidence used below remain
    // mandatory regardless of the native completion result.
    let executable = env::current_exe()
        .map_err(|error| format!("failed to locate current pcb-maker executable: {error}"))?;
    let internal_directory = output_directory.join("pcb-maker");
    let log = output_directory.join("pcb-maker.log");
    let cache_event_log = output_directory.join("pcb-maker-kicad-cache-events.jsonl");
    let executable_sha256 = sha256_file(&executable)?;
    let run_started = Instant::now();
    let process = run_logged_with_timeout_outcome(
        Command::new(&executable)
            .arg("solve-semantic-kicad-prefix")
            .arg(problem)
            .arg(placement)
            .arg(template)
            .arg(target_rung.to_string())
            .arg(&internal_directory)
            .arg(insertion_config)
            .env(pcb_kicad::KICAD_VERIFICATION_CACHE_ENV, verification_cache)
            .env(
                pcb_kicad::KICAD_VERIFICATION_CACHE_EVENTS_ENV,
                &cache_event_log,
            ),
        &log,
        Duration::from_secs(config.maximum_seconds),
    )?;
    let elapsed_micros = run_started.elapsed().as_micros() as u64;
    let cold_prefix_manifest = internal_directory.join("cold-prefix.json");
    let progression_manifest = internal_directory.join("solve/progression.json");
    let timed_out = matches!(&process, LoggedProcessOutcome::TimedOut);
    if !cold_prefix_manifest.is_file() && !timed_out {
        return Err(format!(
            "pcb-maker exited with {} without writing {}",
            process.description(),
            cold_prefix_manifest.display()
        ));
    }
    if timed_out && !progression_manifest.is_file() {
        return Err(format!(
            "pcb-maker timed out without a crash-safe progression checkpoint at {}",
            progression_manifest.display()
        ));
    }
    let cold: Option<serde_json::Value> = cold_prefix_manifest
        .is_file()
        .then(|| read_json(&cold_prefix_manifest))
        .transpose()?;
    let progression: Option<serde_json::Value> = progression_manifest
        .is_file()
        .then(|| read_json(&progression_manifest))
        .transpose()?;
    let manifest_target = cold
        .as_ref()
        .map(|cold| json_usize(cold, "target_rung"))
        .transpose()?
        .unwrap_or(target_rung);
    let final_rung = cold
        .as_ref()
        .map(|cold| json_usize(cold, "final_rung"))
        .transpose()?
        .or_else(|| {
            progression
                .as_ref()
                .and_then(|value| value.get("final_rung"))
                .and_then(serde_json::Value::as_u64)
                .and_then(|value| usize::try_from(value).ok())
        })
        .ok_or_else(|| "pcb-maker report has no final_rung".to_string())?;
    if manifest_target != target_rung {
        return Err(format!(
            "pcb-maker manifest target {manifest_target} differs from benchmark target {target_rung}"
        ));
    }
    let final_directory = cold
        .as_ref()
        .map(|cold| json_str(cold, "final_directory"))
        .transpose()?
        .or_else(|| {
            progression
                .as_ref()
                .and_then(|value| value.get("final_directory"))
                .and_then(serde_json::Value::as_str)
        })
        .ok_or_else(|| "pcb-maker report has no final_directory".to_string())?;
    let original_result_directory = PathBuf::from(final_directory);
    let original_result_directory = if original_result_directory.is_absolute() {
        original_result_directory
    } else {
        absolute(&original_result_directory)?
    };
    let board_id = freerouting
        .source
        .as_ref()
        .ok_or_else(|| "Freerouting report has no source evidence".to_string())?
        .board_id
        .clone();
    let result_directory = if route_config.post_route_processors.is_empty() {
        original_result_directory.clone()
    } else {
        let postprocessed = output_directory.join("pcb-maker-postprocessed-result");
        copy_directory(&original_result_directory, &postprocessed)?;
        postprocessed
    };
    let board = result_directory.join(format!("{board_id}.kicad_pcb"));
    let post_route_processors = apply_post_route_processors(route_config, &board)?;
    if !post_route_processors.is_empty() {
        let python = resolve_executable(
            route_config_base,
            &route_config.freerouting.python_executable,
        );
        refill_and_save_zones(
            &python,
            &board,
            &output_directory.join("pcb-maker-zone-fill.log"),
        )?;
    }
    let route_admission = if post_route_processors.is_empty() {
        None
    } else {
        Some(verify_route_result_against_source(
            &output_directory.join("source"),
            &result_directory,
            &board_id,
            true,
        )?)
    };
    let verification_path = result_directory.join("verification.json");
    let verification: VerificationReport = if let Some(admission) = &route_admission {
        admission.result.clone()
    } else if verification_path.is_file() {
        read_json(&verification_path)?
    } else {
        // An exact rollback intentionally contains only the immutable parent
        // artifact. Verify a disposable copy so an incomplete routing outcome
        // remains comparable without mutating or misclassifying the rollback.
        let verification_directory = output_directory.join("pcb-maker-selected-verification");
        copy_directory(&result_directory, &verification_directory)?;
        pcb_kicad::verify_materialized_rung(&verification_directory, &board_id)?
    };
    if verification.board_id != board_id {
        return Err("pcb-maker selected verification report has the wrong board id".into());
    }
    let statistics = pcb_kicad::inspect_kicad_board(&board)?;
    let differential_pairs = measure_differential_pairs(route_config, &board);
    let render = pcb_kicad::render_board_layers(
        &board,
        &output_directory.join("renders/pcb-maker-result"),
        &route_config.render,
    )?;
    let freerouting_render = &freerouting
        .result
        .as_ref()
        .ok_or_else(|| "Freerouting report has no result evidence".to_string())?
        .result_render;
    let journal_renders = publish_comparison_renders(
        route_config,
        route_config_base,
        output_directory,
        freerouting_render,
        &render,
    )?;
    let termination = if timed_out {
        "time_budget_exhausted"
    } else {
        progression
            .as_ref()
            .and_then(|progression| progression.get("termination"))
            .and_then(serde_json::Value::as_str)
            .or_else(|| {
                cold.as_ref()
                    .and_then(|cold| cold.get("progression"))
                    .and_then(|progression| progression.get("termination"))
                    .and_then(serde_json::Value::as_str)
            })
            .unwrap_or("no_progression")
    }
    .to_string();
    let total_route_expansions = progression
        .as_ref()
        .and_then(|progression| progression.get("total_route_expansions"))
        .and_then(serde_json::Value::as_u64)
        .or_else(|| {
            cold.as_ref()
                .and_then(|cold| cold.get("progression"))
                .and_then(|progression| progression.get("total_route_expansions"))
                .and_then(serde_json::Value::as_u64)
        })
        .unwrap_or(0);
    let completed = cold
        .as_ref()
        .and_then(|cold| cold.get("completed"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let native_verification_cache =
        summarize_native_cache_events(verification_cache, &cache_event_log)?;
    let post_route_complete = route_admission
        .as_ref()
        .is_none_or(|admission| admission.complete);
    Ok(CompetitivePcbMakerEvidence {
        mode: "semantic_cold_progression".into(),
        executable: executable.clone(),
        executable_sha256,
        maximum_seconds: config.maximum_seconds,
        exit_code: process.exit_code(),
        timed_out,
        elapsed_micros,
        log,
        cold_prefix_manifest: cold_prefix_manifest
            .is_file()
            .then_some(cold_prefix_manifest),
        progression_manifest: progression_manifest
            .is_file()
            .then_some(progression_manifest),
        sequential_route_manifest: None,
        target_rung,
        final_rung,
        termination,
        total_route_expansions,
        solved_target: process.success()
            && completed
            && final_rung == target_rung
            && verification.complete
            && post_route_complete,
        result_directory,
        board: board.clone(),
        board_sha256: sha256_file(&board)?,
        statistics,
        verification,
        route_admission,
        post_route_processors,
        differential_pairs,
        render,
        journal_renders,
        native_verification_cache,
    })
}

fn run_pcb_maker_sequential_comparison_inner(
    config: &PcbMakerComparisonConfig,
    route_config: &CompetitiveRouteBenchmarkConfig,
    route_config_base: &Path,
    sequential_config: &Path,
    verification_cache: &Path,
    output_directory: &Path,
    freerouting: &CompetitiveRouteRunReport,
) -> Result<CompetitivePcbMakerEvidence, String> {
    let source = freerouting
        .source
        .as_ref()
        .ok_or_else(|| "Freerouting report has no source evidence".to_string())?;
    let executable = env::current_exe()
        .map_err(|error| format!("failed to locate current pcb-maker executable: {error}"))?;
    let executable_sha256 = sha256_file(&executable)?;
    let source_directory = output_directory.join("source");
    let internal_directory = output_directory.join("pcb-maker");
    let log = output_directory.join("pcb-maker.log");
    let cache_event_log = output_directory.join("pcb-maker-kicad-cache-events.jsonl");
    let run_started = Instant::now();
    let process = run_logged_with_timeout_outcome(
        Command::new(&executable)
            .arg("route-kicad-board-sequential")
            .arg(&source_directory)
            .arg(&source.board_id)
            .arg(&internal_directory)
            .arg(sequential_config)
            .env(pcb_kicad::KICAD_VERIFICATION_CACHE_ENV, verification_cache)
            .env(
                pcb_kicad::KICAD_VERIFICATION_CACHE_EVENTS_ENV,
                &cache_event_log,
            ),
        &log,
        Duration::from_secs(config.maximum_seconds),
    )?;
    let elapsed_micros = run_started.elapsed().as_micros() as u64;
    let timed_out = matches!(&process, LoggedProcessOutcome::TimedOut);
    let sequential_route_manifest = internal_directory.join("sequential-route.json");
    if !sequential_route_manifest.is_file() {
        return Err(format!(
            "pcb-maker {} without writing a crash-safe sequential checkpoint at {}",
            process.description(),
            sequential_route_manifest.display()
        ));
    }
    let sequential: pcb_kicad::KiCadSequentialRouterResult = read_json(&sequential_route_manifest)?;
    if sequential.board_id != source.board_id {
        return Err("pcb-maker sequential report selected the wrong board id".into());
    }
    if !sequential.source_board_unchanged
        || sequential.source_board_sha256_before != sequential.source_board_sha256_after
        || sequential.source_board_sha256_before != source.board_sha256
    {
        return Err("pcb-maker sequential run mutated or did not consume the frozen source".into());
    }
    let target_rung = sequential.connection_order.len();
    let final_rung = sequential.completed_connections;
    let termination = match sequential.termination {
        pcb_kicad::KiCadSequentialRouterTermination::Complete => "complete",
        pcb_kicad::KiCadSequentialRouterTermination::RoutingFailed => "routing_failed",
        pcb_kicad::KiCadSequentialRouterTermination::BoundReached => "bound_reached",
    }
    .to_string();
    let result_directory = internal_directory.join("result");
    let board = result_directory.join(format!("{}.kicad_pcb", source.board_id));
    if !board.is_file() {
        return Err(format!(
            "pcb-maker sequential checkpoint has no result board {}",
            board.display()
        ));
    }
    let post_route_processors = apply_post_route_processors(route_config, &board)?;
    if !post_route_processors.is_empty() {
        let python = resolve_executable(
            route_config_base,
            &route_config.freerouting.python_executable,
        );
        refill_and_save_zones(
            &python,
            &board,
            &output_directory.join("pcb-maker-zone-fill.log"),
        )?;
    }
    let route_admission = verify_route_result_against_source(
        &source_directory,
        &result_directory,
        &source.board_id,
        !post_route_processors.is_empty(),
    )?;
    let verification = route_admission.result.clone();
    let statistics = pcb_kicad::inspect_kicad_board(&board)?;
    let differential_pairs = measure_differential_pairs(route_config, &board);
    let render = pcb_kicad::render_board_layers(
        &board,
        &output_directory.join("renders/pcb-maker-result"),
        &route_config.render,
    )?;
    let freerouting_render = &freerouting
        .result
        .as_ref()
        .ok_or_else(|| "Freerouting report has no result evidence".to_string())?
        .result_render;
    let journal_renders = publish_comparison_renders(
        route_config,
        route_config_base,
        output_directory,
        freerouting_render,
        &render,
    )?;
    let native_verification_cache =
        summarize_native_cache_events(verification_cache, &cache_event_log)?;
    let solved_target = process.success()
        && !timed_out
        && sequential.termination == pcb_kicad::KiCadSequentialRouterTermination::Complete
        && final_rung == target_rung
        && route_admission.complete;
    Ok(CompetitivePcbMakerEvidence {
        mode: "direct_cold_kicad_sequential".into(),
        executable: executable.clone(),
        executable_sha256,
        maximum_seconds: config.maximum_seconds,
        exit_code: process.exit_code(),
        timed_out,
        elapsed_micros,
        log,
        cold_prefix_manifest: None,
        progression_manifest: None,
        sequential_route_manifest: Some(sequential_route_manifest),
        target_rung,
        final_rung,
        termination,
        total_route_expansions: sequential.total_expansions,
        solved_target,
        result_directory,
        board: board.clone(),
        board_sha256: sha256_file(&board)?,
        statistics,
        verification,
        route_admission: Some(route_admission),
        post_route_processors,
        differential_pairs,
        render,
        journal_renders,
        native_verification_cache,
    })
}

fn run_freerouting_benchmark_inner(
    config: &CompetitiveRouteBenchmarkConfig,
    config_base: &Path,
    output_directory: &Path,
    report: &mut CompetitiveRouteRunReport,
) -> Result<(), String> {
    let source_directory = output_directory.join("source");
    let prepared = stage(report, output_directory, "prepare_cold_source", || {
        prepare_source(config, config_base, output_directory, &source_directory)
    })?;
    let board_id = prepared.board_id;
    check_board_id(&board_id)?;
    let source_board = source_directory.join(format!("{board_id}.kicad_pcb"));
    let before_rules = pcb_kicad::inspect_kicad_board(&source_board)?;
    require_cold_board(&before_rules)?;

    let route_directory = output_directory.join("router");
    fs::create_dir_all(&route_directory)
        .map_err(|error| format!("failed to create {}: {error}", route_directory.display()))?;
    let dsn = route_directory.join("input.dsn");
    stage(
        report,
        output_directory,
        "apply_rules_and_export_dsn",
        || {
            apply_rules_and_export_dsn(
                &resolve_executable(config_base, &config.freerouting.python_executable),
                &source_board,
                &dsn,
                &config.rules,
                &route_directory.join("pcbnew-export.log"),
            )
        },
    )?;
    let source_statistics = pcb_kicad::inspect_kicad_board(&source_board)?;
    require_cold_board(&source_statistics)?;
    report.source = Some(CompetitiveSourceEvidence {
        kind: prepared.kind,
        board_id: board_id.clone(),
        input_hashes: prepared.input_hashes,
        copper_strip: prepared.copper_strip,
        board_sha256: sha256_file(&source_board)?,
        directory_sha256: sha256_directory(&source_directory)?,
        statistics: source_statistics,
        placement: prepared.placement,
        routing_feedback: prepared.routing_feedback,
    });
    write_report(output_directory, report)?;
    if config.placement_acceptance.is_some() {
        let placement_evidence = report
            .source
            .as_ref()
            .and_then(|source| source.placement.clone());
        stage(
            report,
            output_directory,
            "validate_placement_acceptance",
            || validate_placement_acceptance(config, placement_evidence.as_ref()),
        )?;
    }

    let jar = resolve_file(config_base, &config.freerouting.jar);
    let jar_sha256 = sha256_file(&jar)?;
    if jar_sha256 != config.freerouting.expected_jar_sha256 {
        return Err(format!(
            "Freerouting jar hash mismatch: expected {}, got {jar_sha256}",
            config.freerouting.expected_jar_sha256
        ));
    }
    let java = resolve_executable(config_base, &config.freerouting.java_executable);
    let python = resolve_executable(config_base, &config.freerouting.python_executable);
    let mut tools = stage(report, output_directory, "identify_tools", || {
        identify_tools(&java, &python, &jar, &jar_sha256)
    })?;
    report.tools = Some(tools.clone());
    write_report(output_directory, report)?;

    let internal_render_directory = output_directory.join("renders");
    fs::create_dir_all(&internal_render_directory).map_err(|error| {
        format!(
            "failed to create render directory {}: {error}",
            internal_render_directory.display()
        )
    })?;
    let source_render = stage(report, output_directory, "render_source", || {
        pcb_kicad::render_board_layers(
            &source_board,
            &internal_render_directory.join("source"),
            &config.render,
        )
    })?;

    let session = route_directory.join("output.ses");
    let router_log = route_directory.join("freerouting.log");
    let routing_started = Instant::now();
    let status = stage(report, output_directory, "run_freerouting", || {
        let mut command = Command::new(&java);
        command
            .arg("-Djava.awt.headless=true")
            .arg("--enable-final-field-mutation=ALL-UNNAMED")
            .arg("-jar")
            .arg(&jar)
            .arg("-de")
            .arg(&dsn)
            .arg("-do")
            .arg(&session)
            .arg("-mp")
            .arg(config.freerouting.maximum_passes.to_string())
            .arg("-mt")
            .arg(config.freerouting.maximum_threads.to_string());
        run_logged_with_timeout(
            &mut command,
            &router_log,
            Duration::from_secs(config.freerouting.maximum_seconds),
        )
    })?;
    let routing_elapsed_micros = routing_started.elapsed().as_micros() as u64;
    if !status.success() {
        return Err(format!(
            "Freerouting exited with status {}; see {}",
            status,
            router_log.display()
        ));
    }
    if !session.is_file() {
        return Err(format!(
            "Freerouting exited successfully without writing {}",
            session.display()
        ));
    }
    let router_log_source = fs::read_to_string(&router_log)
        .map_err(|error| format!("failed to read {}: {error}", router_log.display()))?;
    let parsed = parse_freerouting_log(&router_log_source);
    tools.freerouting_version = parsed.version.clone();
    report.tools = Some(tools);
    report.routing = Some(FreeroutingRunEvidence {
        maximum_seconds: config.freerouting.maximum_seconds,
        maximum_passes: config.freerouting.maximum_passes,
        maximum_threads: config.freerouting.maximum_threads,
        exit_code: status.code().unwrap_or(-1),
        elapsed_micros: routing_elapsed_micros,
        autorouter_passes: parsed.autorouter_passes,
        reported_initial_unrouted_items: parsed.initial_unrouted_items,
        reported_final_unrouted_items: parsed.final_unrouted_items,
        reported_cpu_seconds: parsed.cpu_seconds,
        dsn: dsn.clone(),
        dsn_sha256: sha256_file(&dsn)?,
        session: session.clone(),
        session_sha256: sha256_file(&session)?,
        log: router_log.clone(),
    });
    write_report(output_directory, report)?;

    let result_directory = output_directory.join("result");
    copy_directory(&source_directory, &result_directory)?;
    let result_board = result_directory.join(format!("{board_id}.kicad_pcb"));
    let zero_connection_source = matches!(
        &config.source,
        CompetitiveRouteSource::SemanticKicad {
            connection_prefix: 0,
            ..
        }
    );
    if zero_connection_source
        && fs::metadata(&session)
            .map_err(|error| error.to_string())?
            .len()
            == 0
    {
        stage(
            report,
            output_directory,
            "adopt_zero_connection_source",
            || {
                if parsed.initial_unrouted_items != Some(0)
                    || parsed.final_unrouted_items != Some(0)
                {
                    return Err(
                        "empty Freerouting session is admissible only when the router reports zero initial and final unrouted items"
                            .into(),
                    );
                }
                Ok(())
            },
        )?;
    } else {
        stage(report, output_directory, "import_session", || {
            import_specctra_session(
                &python,
                &result_board,
                &session,
                &route_directory.join("pcbnew-import.log"),
            )
        })?;
    }
    let post_route_processors = stage(report, output_directory, "post_route_processors", || {
        apply_post_route_processors(config, &result_board)
    })?;
    if !post_route_processors.is_empty() {
        stage(report, output_directory, "fill_and_save_zones", || {
            refill_and_save_zones(
                &python,
                &result_board,
                &route_directory.join("pcbnew-zone-fill.log"),
            )
        })?;
    }
    let refill_zones = !post_route_processors.is_empty();
    let route_admission = stage(report, output_directory, "verify_result", || {
        verify_route_result_against_source(
            &source_directory,
            &result_directory,
            &board_id,
            refill_zones,
        )
    })?;
    let verification = route_admission.result.clone();
    let route_complete = route_admission.complete;
    let result_statistics = pcb_kicad::inspect_kicad_board(&result_board)?;
    let differential_pairs = measure_differential_pairs(config, &result_board);
    let result_render = stage(report, output_directory, "render_result", || {
        pcb_kicad::render_board_layers(
            &result_board,
            &internal_render_directory.join("result"),
            &config.render,
        )
    })?;
    let journal_renders = stage(report, output_directory, "publish_journal_renders", || {
        publish_journal_renders(
            config,
            config_base,
            output_directory,
            &source_render,
            &result_render,
        )
    })?;
    let result_acceptance = validate_result_acceptance(
        config,
        result_statistics.vias,
        result_statistics.physical_copper.used_copper_layers,
        result_statistics.copper_zones,
        result_statistics.physical_copper.filled_zone_area_mm2,
        &differential_pairs,
        "Freerouting",
    );
    report.result = Some(CompetitiveRouteResultEvidence {
        board: result_board.clone(),
        board_sha256: sha256_file(&result_board)?,
        directory_sha256: sha256_directory(&result_directory)?,
        statistics: result_statistics,
        verification,
        route_admission: Some(route_admission),
        post_route_processors,
        differential_pairs,
        source_render,
        result_render,
        journal_renders,
    });
    write_report(output_directory, report)?;
    if config.result_acceptance.is_some() {
        stage(
            report,
            output_directory,
            "validate_result_acceptance",
            || result_acceptance,
        )?;
    }
    report.status = if route_complete {
        CompetitiveRouteRunStatus::Complete
    } else {
        CompetitiveRouteRunStatus::Incomplete
    };
    write_report(output_directory, report)?;
    Ok(())
}

fn validate_result_acceptance(
    config: &CompetitiveRouteBenchmarkConfig,
    vias: usize,
    used_copper_layers: usize,
    copper_zones: usize,
    filled_zone_area_mm2: f64,
    differential_pairs: &[CompetitiveDifferentialPairEvidence],
    router: &str,
) -> Result<(), String> {
    let Some(required) = &config.result_acceptance else {
        return Ok(());
    };
    let mut failures = Vec::new();
    if vias < required.minimum_vias {
        failures.push(format!("vias {} < {}", vias, required.minimum_vias));
    }
    if used_copper_layers < required.minimum_used_copper_layers {
        failures.push(format!(
            "used copper layers {} < {}",
            used_copper_layers, required.minimum_used_copper_layers
        ));
    }
    if copper_zones < required.minimum_copper_zones {
        failures.push(format!(
            "copper zones {} < {}",
            copper_zones, required.minimum_copper_zones
        ));
    }
    if filled_zone_area_mm2 < required.minimum_filled_zone_area_mm2 {
        failures.push(format!(
            "filled zone area {:.6} mm2 < {:.6} mm2",
            filled_zone_area_mm2, required.minimum_filled_zone_area_mm2
        ));
    }
    for pair in differential_pairs {
        if !pair.complete {
            failures.push(pair.failure.clone().unwrap_or_else(|| {
                format!(
                    "differential pair {}/{} missed its acceptance",
                    pair.positive_connection, pair.negative_connection
                )
            }));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{router} result missed acceptance: {}",
            failures.join(", ")
        ))
    }
}

fn measure_differential_pairs(
    config: &CompetitiveRouteBenchmarkConfig,
    board: &Path,
) -> Vec<CompetitiveDifferentialPairEvidence> {
    let Some(acceptance) = &config.result_acceptance else {
        return Vec::new();
    };
    acceptance
        .differential_pairs
        .iter()
        .map(|pair| {
            let measure = |connection: &str| {
                pcb_kicad::import_route_candidate_from_pcb(board, connection)
                    .and_then(|candidate| pcb_kicad::route_candidate_quality(&candidate))
            };
            let positive = measure(&pair.positive_connection);
            let negative = measure(&pair.negative_connection);
            match (positive, negative) {
                (Ok(positive_quality), Ok(negative_quality)) => {
                    let length_skew_mm =
                        (positive_quality.length_mm - negative_quality.length_mm).abs();
                    let via_count_difference = positive_quality
                        .vias
                        .abs_diff(negative_quality.vias);
                    let complete = length_skew_mm <= pair.maximum_length_skew_mm
                        && via_count_difference <= pair.maximum_via_count_difference;
                    CompetitiveDifferentialPairEvidence {
                        positive_connection: pair.positive_connection.clone(),
                        negative_connection: pair.negative_connection.clone(),
                        positive_quality: Some(positive_quality),
                        negative_quality: Some(negative_quality),
                        length_skew_mm: Some(length_skew_mm),
                        via_count_difference: Some(via_count_difference),
                        complete,
                        failure: (!complete).then(|| {
                            format!(
                                "differential pair {}/{} has {:.6} mm skew (limit {:.6}) and {} via-count difference (limit {})",
                                pair.positive_connection,
                                pair.negative_connection,
                                length_skew_mm,
                                pair.maximum_length_skew_mm,
                                via_count_difference,
                                pair.maximum_via_count_difference
                            )
                        }),
                    }
                }
                (positive, negative) => CompetitiveDifferentialPairEvidence {
                    positive_connection: pair.positive_connection.clone(),
                    negative_connection: pair.negative_connection.clone(),
                    positive_quality: positive.ok(),
                    negative_quality: negative.ok(),
                    length_skew_mm: None,
                    via_count_difference: None,
                    complete: false,
                    failure: Some(format!(
                        "could not import both differential routes {}/{} as canonical acyclic copper",
                        pair.positive_connection, pair.negative_connection
                    )),
                },
            }
        })
        .collect()
}

fn apply_post_route_processors(
    config: &CompetitiveRouteBenchmarkConfig,
    board: &Path,
) -> Result<Vec<CompetitivePostRouteEvidence>, String> {
    config
        .post_route_processors
        .iter()
        .map(|processor| match processor {
            CompetitivePostRouteProcessor::GroundZoneReplacement { config } => {
                let replacement =
                    pcb_kicad::apply_kicad_connection_zone_replacement(board, config)?;
                Ok(CompetitivePostRouteEvidence::GroundZoneReplacement {
                    config: config.clone(),
                    replacement,
                })
            }
        })
        .collect()
}

fn verify_route_result_against_source(
    source_directory: &Path,
    result_directory: &Path,
    board_id: &str,
    refill_zones: bool,
) -> Result<CompetitiveRouteAdmissionEvidence, String> {
    let result_parent = result_directory
        .parent()
        .ok_or_else(|| "route result directory has no parent".to_string())?;
    let baseline_directory = result_parent.join("source-baseline-verification");
    let source_baseline = if baseline_directory.is_dir() {
        let source_board = source_directory.join(format!("{board_id}.kicad_pcb"));
        let baseline_board = baseline_directory.join(format!("{board_id}.kicad_pcb"));
        if sha256_file(&source_board)? != sha256_file(&baseline_board)? {
            return Err("existing route-admission baseline does not match the cold source".into());
        }
        let baseline: VerificationReport =
            read_json(&baseline_directory.join("verification.json"))?;
        if baseline.board_id != board_id {
            return Err("existing route-admission baseline has the wrong board id".into());
        }
        baseline
    } else {
        copy_directory(source_directory, &baseline_directory)?;
        pcb_kicad::verify_materialized_rung(&baseline_directory, board_id)?
    };
    let result = if refill_zones {
        pcb_kicad::verify_materialized_rung_with_zone_refill(result_directory, board_id)?
    } else {
        pcb_kicad::verify_materialized_rung(result_directory, board_id)?
    };
    let source_erc = read_json(&baseline_directory.join("erc.json"))?;
    let result_erc = read_json(&result_directory.join("erc.json"))?;
    let source_drc = read_json(&baseline_directory.join("drc.json"))?;
    let result_drc = read_json(&result_directory.join("drc.json"))?;
    let introduced_erc_violations = introduced_issue_count(
        schematic_erc_issues(&source_erc),
        schematic_erc_issues(&result_erc),
    )?;
    let introduced_drc_design_violations = introduced_issue_count(
        drc_design_issues(&source_drc),
        drc_design_issues(&result_drc),
    )?;
    let introduced_schematic_parity_issues = introduced_issue_count(
        report_array(&source_drc, "schematic_parity"),
        report_array(&result_drc, "schematic_parity"),
    )?;
    let result_selected_net_unconnected_items = result.selected_net_unconnected_items;
    Ok(CompetitiveRouteAdmissionEvidence {
        contract: format!(
            "native KiCad result connectivity must be zero; ERC, DRC design, and schematic-parity findings must be absent or byte-semantically identical to findings already present in the frozen zero-copper source; lib_footprint_mismatch warnings are retained separately as library metadata under the shared native verification policy; issue UUIDs and report ordering are non-semantic; refill_zones={refill_zones}"
        ),
        complete: introduced_erc_violations == 0
            && introduced_drc_design_violations == 0
            && introduced_schematic_parity_issues == 0
            && result_selected_net_unconnected_items == 0,
        source_baseline,
        result,
        introduced_erc_violations,
        introduced_drc_design_violations,
        introduced_schematic_parity_issues,
        result_selected_net_unconnected_items,
    })
}

fn schematic_erc_issues(report: &serde_json::Value) -> Vec<&serde_json::Value> {
    report["sheets"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|sheet| report_array(sheet, "violations"))
        .collect()
}

fn report_array<'a>(report: &'a serde_json::Value, key: &str) -> Vec<&'a serde_json::Value> {
    report[key].as_array().into_iter().flatten().collect()
}

fn introduced_issue_count(
    baseline: Vec<&serde_json::Value>,
    result: Vec<&serde_json::Value>,
) -> Result<usize, String> {
    let mut counts = BTreeMap::<String, usize>::new();
    for issue in baseline {
        *counts.entry(issue_fingerprint(issue)?).or_default() += 1;
    }
    let mut introduced = 0;
    for issue in result {
        let fingerprint = issue_fingerprint(issue)?;
        match counts.get_mut(&fingerprint) {
            Some(count) if *count > 0 => *count -= 1,
            _ => introduced += 1,
        }
    }
    Ok(introduced)
}

fn issue_fingerprint(issue: &serde_json::Value) -> Result<String, String> {
    fn normalize(value: &serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Array(values) => {
                let mut normalized = values.iter().map(normalize).collect::<Vec<_>>();
                normalized.sort_by_key(|value| value.to_string());
                serde_json::Value::Array(normalized)
            }
            serde_json::Value::Object(values) => serde_json::Value::Object(
                values
                    .iter()
                    .filter(|(key, _)| key.as_str() != "uuid")
                    .map(|(key, value)| (key.clone(), normalize(value)))
                    .collect(),
            ),
            _ => value.clone(),
        }
    }
    serde_json::to_string(&normalize(issue)).map_err(|error| error.to_string())
}

fn require_cold_board(statistics: &KiCadBoardStatistics) -> Result<(), String> {
    if statistics.segments != 0
        || statistics.arcs != 0
        || statistics.vias != 0
        || statistics.copper_zones != 0
        || statistics.copper_graphics != 0
    {
        return Err(format!(
            "cold route-only source contains copper: {} segments, {} arcs, {} vias, {} copper zones, {} top-level copper graphics ({} rule areas retained)",
            statistics.segments,
            statistics.arcs,
            statistics.vias,
            statistics.copper_zones,
            statistics.copper_graphics,
            statistics.rule_areas
        ));
    }
    Ok(())
}

struct PreparedSource {
    kind: String,
    board_id: String,
    input_hashes: BTreeMap<String, String>,
    copper_strip: Option<KiCadCopperStripReport>,
    placement: Option<CompetitivePlacementEvidence>,
    routing_feedback: Option<CompetitiveRoutingFeedbackPlacementEvidence>,
}

fn solved_components_from_poses(
    problem: &layout_trace_model::Problem,
    poses: &[pcb_placement::PlacementPose],
) -> Result<Vec<pcb_validate::SolvedComponent>, String> {
    let components = problem
        .components
        .iter()
        .map(|component| (component.id.as_str(), component))
        .collect::<BTreeMap<_, _>>();
    poses
        .iter()
        .map(|pose| {
            let component = components.get(pose.component.as_str()).ok_or_else(|| {
                format!("placement returned unknown component {}", pose.component)
            })?;
            Ok(pcb_validate::SolvedComponent {
                id: pose.component.clone(),
                position: pose.position,
                size: component.size,
                rotation_degrees: pose.rotation_degrees,
            })
        })
        .collect()
}

fn placement_evidence_for_poses(
    problem: &layout_trace_model::Problem,
    connectivity_distance_before_mm: f64,
    poses: &[pcb_placement::PlacementPose],
) -> Result<CompetitivePlacementEvidence, String> {
    const TOLERANCE_MM: f64 = 1.0e-6;
    let declared = problem
        .components
        .iter()
        .map(|component| (component.id.as_str(), component))
        .collect::<BTreeMap<_, _>>();
    if poses.len() != declared.len() {
        return Err("placement evidence requires exactly one pose per component".into());
    }
    let translated = |pose: &&pcb_placement::PlacementPose| {
        let component = declared[pose.component.as_str()];
        let dx = pose.position.x - component.position.x;
        let dy = pose.position.y - component.position.y;
        dx.hypot(dy) > TOLERANCE_MM
    };
    let rotated = |pose: &&pcb_placement::PlacementPose| {
        let component = declared[pose.component.as_str()];
        ((pose.rotation_degrees - component.rotation_degrees + 180.0).rem_euclid(360.0) - 180.0)
            .abs()
            > TOLERANCE_MM
    };
    let translated_components = poses.iter().filter(translated).count();
    let rotated_components = poses.iter().filter(rotated).count();
    let moved_components = poses
        .iter()
        .filter(|pose| translated(pose) || rotated(pose))
        .count();
    let connectivity_distance_after_mm =
        pcb_placement::placement_demand_evidence(problem, poses, [32, 32])?.deposited_demand;
    Ok(CompetitivePlacementEvidence {
        moved_components,
        translated_components,
        rotated_components,
        connectivity_distance_before_mm,
        connectivity_distance_after_mm,
        connectivity_distance_reduction_mm: connectivity_distance_before_mm
            - connectivity_distance_after_mm,
    })
}

fn prepare_source(
    config: &CompetitiveRouteBenchmarkConfig,
    config_base: &Path,
    output_directory: &Path,
    source_directory: &Path,
) -> Result<PreparedSource, String> {
    match &config.source {
        CompetitiveRouteSource::MaterializedKicad {
            directory,
            board_id,
        } => {
            let directory = resolve_file(config_base, directory);
            let mut input_hashes = BTreeMap::new();
            input_hashes.insert("source_directory".into(), sha256_directory(&directory)?);
            copy_directory(&directory, source_directory)?;
            let board = source_directory.join(format!("{board_id}.kicad_pcb"));
            if !board.is_file() {
                return Err(format!("source board {} does not exist", board.display()));
            }
            Ok(PreparedSource {
                kind: "materialized_kicad".into(),
                board_id: board_id.clone(),
                input_hashes,
                copper_strip: None,
                placement: None,
                routing_feedback: None,
            })
        }
        CompetitiveRouteSource::ColdKicadProject {
            directory,
            board_id,
        } => {
            let directory = resolve_file(config_base, directory);
            let mut input_hashes = BTreeMap::new();
            input_hashes.insert(
                "upstream_project_directory".into(),
                sha256_directory(&directory)?,
            );
            copy_directory(&directory, source_directory)?;
            let board = source_directory.join(format!("{board_id}.kicad_pcb"));
            if !board.is_file() {
                return Err(format!("source board {} does not exist", board.display()));
            }
            let copper_strip = pcb_kicad::write_kicad_board_without_copper(&board, &board)?;
            fs::write(
                output_directory.join("source-adaptation.json"),
                format!(
                    "{}\n",
                    serde_json::to_string_pretty(&copper_strip)
                        .map_err(|error| error.to_string())?
                ),
            )
            .map_err(|error| format!("failed to write source-adaptation.json: {error}"))?;
            Ok(PreparedSource {
                kind: "cold_kicad_project".into(),
                board_id: board_id.clone(),
                input_hashes,
                copper_strip: Some(copper_strip),
                placement: None,
                routing_feedback: None,
            })
        }
        CompetitiveRouteSource::SemanticKicad {
            problem,
            placement_config,
            template_config,
            connection_prefix,
            routing_feedback,
        } => {
            let problem_path = resolve_file(config_base, problem);
            let placement_path = resolve_file(config_base, placement_config);
            let template_path = resolve_file(config_base, template_config);
            let mut input_hashes = BTreeMap::new();
            input_hashes.insert("problem".into(), sha256_file(&problem_path)?);
            input_hashes.insert("placement_config".into(), sha256_file(&placement_path)?);
            input_hashes.insert("template_config".into(), sha256_file(&template_path)?);

            let routing_feedback_paths = routing_feedback.as_ref().map(|feedback| {
                (
                    resolve_file(config_base, &feedback.routing_config),
                    resolve_file(config_base, &feedback.pressure_config),
                )
            });
            if let Some((routing_path, pressure_path)) = &routing_feedback_paths {
                input_hashes.insert(
                    "routing_feedback_routing_config".into(),
                    sha256_file(routing_path)?,
                );
                input_hashes.insert(
                    "routing_feedback_pressure_config".into(),
                    sha256_file(pressure_path)?,
                );
            }

            let mut problem: layout_trace_model::Problem = read_json(&problem_path)?;
            problem
                .check_schema()
                .map_err(|error| format!("invalid {}: {error}", problem_path.display()))?;
            let placement_config: pcb_placement::InitialPlacementConfig =
                read_json(&placement_path)?;
            placement_config.check()?;
            let mut template_config: SemanticKiCadTemplateConfig = read_json(&template_path)?;
            retain_connection_prefix(&mut problem, &mut template_config, *connection_prefix)?;
            let placement = pcb_placement::run_initial_placement(&problem, &placement_config)?;
            let mut selected_poses = placement.poses.clone();
            let mut routing_feedback_evidence = None;
            if let Some((routing_path, pressure_path)) = routing_feedback_paths {
                let routing_config: pcb_routing::DutGridRoutingConfig = read_json(&routing_path)?;
                routing_config.check()?;
                let pressure_config: pcb_coordinator::PressureRepairConfig =
                    read_json(&pressure_path)?;
                pressure_config.check()?;
                let initial_components = solved_components_from_poses(&problem, &selected_poses)?;
                let feedback = pcb_coordinator::repair_routing_with_pressure(
                    &problem,
                    &initial_components,
                    &routing_config,
                    &pressure_config,
                )?;
                let initial_routing_complete = feedback.attempts[0]
                    .routing
                    .as_ref()
                    .is_some_and(pcb_routing::DutGridRoutingResult::complete);
                if !feedback.evidence.complete {
                    return Err(format!(
                        "routing-feedback placement remained incomplete after {} repair attempt(s)",
                        feedback.evidence.attempted_repairs
                    ));
                }
                let selected_attempt = feedback.evidence.selected_attempt;
                let moved_components = feedback.attempts[selected_attempt].moved_components.clone();
                routing_feedback_evidence = Some(CompetitiveRoutingFeedbackPlacementEvidence {
                    initial_routing_complete,
                    selected_routing_complete: feedback.evidence.complete,
                    selected_attempt,
                    attempted_repairs: feedback.evidence.attempted_repairs,
                    total_expansions: feedback.evidence.total_expansions,
                    moved_components,
                });
                selected_poses = feedback
                    .candidate
                    .components
                    .iter()
                    .map(|component| pcb_placement::PlacementPose {
                        component: component.id.clone(),
                        position: component.position,
                        rotation_degrees: component.rotation_degrees,
                    })
                    .collect();
                fs::write(
                    output_directory.join("placement-feedback.json"),
                    format!(
                        "{}\n",
                        serde_json::to_string_pretty(&feedback)
                            .map_err(|error| error.to_string())?
                    ),
                )
                .map_err(|error| format!("failed to write placement-feedback.json: {error}"))?;
            }
            let placement_evidence = placement_evidence_for_poses(
                &problem,
                placement.evidence.connectivity_distance_before_mm,
                &selected_poses,
            )?;
            let poses = selected_poses
                .iter()
                .map(|pose| SemanticKiCadPose {
                    component: pose.component.clone(),
                    position: pose.position,
                    rotation_degrees: pose.rotation_degrees,
                })
                .collect::<Vec<_>>();
            let generated_directory = output_directory.join("generated-source");
            let template_report = pcb_kicad::write_semantic_kicad_ladder_template(
                &problem,
                &poses,
                &template_config,
                &generated_directory,
            )?;
            let declaration_path = generated_directory.join("declaration.json");
            let declaration = pcb_kicad::load_declaration(&declaration_path)?;
            let materialized = pcb_kicad::materialize_rung(
                &declaration,
                *connection_prefix,
                &output_directory.join("materialized-source"),
            )?;
            copy_directory(&materialized.output_directory, source_directory)?;
            fs::write(
                output_directory.join("source-generation.json"),
                format!(
                    "{}\n",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "schema_version": 1,
                        "contract": "prune connectivity before placement; regenerate placement; materialize the selected prefix with zero inherited copper",
                        "connection_prefix": connection_prefix,
                        "initial_placement": placement,
                        "selected_placement": selected_poses,
                        "routing_feedback": routing_feedback_evidence,
                        "template": template_report,
                        "materialization": materialized
                    }))
                    .map_err(|error| error.to_string())?
                ),
            )
            .map_err(|error| format!("failed to write source-generation.json: {error}"))?;
            Ok(PreparedSource {
                kind: "semantic_kicad".into(),
                board_id: template_config.board_id,
                input_hashes,
                copper_strip: None,
                placement: Some(placement_evidence),
                routing_feedback: routing_feedback_evidence,
            })
        }
    }
}

fn validate_placement_acceptance(
    config: &CompetitiveRouteBenchmarkConfig,
    actual: Option<&CompetitivePlacementEvidence>,
) -> Result<(), String> {
    let Some(required) = &config.placement_acceptance else {
        return Ok(());
    };
    let actual = actual
        .ok_or_else(|| "placement acceptance has no generated placement evidence".to_string())?;
    let mut failures = Vec::new();
    if actual.moved_components < required.minimum_moved_components {
        failures.push(format!(
            "moved components {} < {}",
            actual.moved_components, required.minimum_moved_components
        ));
    }
    if actual.translated_components < required.minimum_translated_components {
        failures.push(format!(
            "translated components {} < {}",
            actual.translated_components, required.minimum_translated_components
        ));
    }
    if actual.rotated_components < required.minimum_rotated_components {
        failures.push(format!(
            "rotated components {} < {}",
            actual.rotated_components, required.minimum_rotated_components
        ));
    }
    if actual.connectivity_distance_reduction_mm + f64::EPSILON
        < required.minimum_connectivity_distance_reduction_mm
    {
        failures.push(format!(
            "connectivity-distance reduction {:.6} mm < {:.6} mm",
            actual.connectivity_distance_reduction_mm,
            required.minimum_connectivity_distance_reduction_mm
        ));
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "generated placement missed acceptance: {}",
            failures.join(", ")
        ))
    }
}

fn retain_connection_prefix(
    problem: &mut layout_trace_model::Problem,
    template: &mut SemanticKiCadTemplateConfig,
    prefix: usize,
) -> Result<(), String> {
    if template.connection_order.is_empty() {
        return Err("semantic benchmark requires explicit connection_order".into());
    }
    if prefix > template.connection_order.len() {
        return Err(format!(
            "connection_prefix {prefix} exceeds {} configured connections",
            template.connection_order.len()
        ));
    }
    template.connection_order.truncate(prefix);
    let selected = template
        .connection_order
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    problem
        .nets
        .retain(|net| selected.contains(net.electrical_net.as_deref().unwrap_or(&net.id)));
    problem
        .electrical_nets
        .retain(|net| selected.contains(net.id.as_str()));
    problem.check_schema()
}

const APPLY_RULES_AND_EXPORT: &str = r#"
import pcbnew
import sys

board_path, dsn_path = sys.argv[1:3]
track, clearance, via, drill = [int(value) for value in sys.argv[3:7]]
board = pcbnew.LoadBoard(board_path)
if board is None:
    raise RuntimeError("pcbnew.LoadBoard returned None")
classes = board.GetAllNetClasses()
if not classes:
    raise RuntimeError("board has no effective net classes")
for netclass in classes.values():
    netclass.SetTrackWidth(track)
    netclass.SetClearance(clearance)
    netclass.SetViaDiameter(via)
    netclass.SetViaDrill(drill)
pcbnew.SaveBoard(board_path, board)
if not pcbnew.ExportSpecctraDSN(board, dsn_path):
    raise RuntimeError("ExportSpecctraDSN failed")
print(pcbnew.GetBuildVersion())
print(track, clearance, via, drill)
"#;

fn apply_rules_and_export_dsn(
    python: &Path,
    board: &Path,
    dsn: &Path,
    rules: &CompetitiveRouteRules,
    log: &Path,
) -> Result<(), String> {
    let values = [
        mm_to_nanometres(rules.trace_width_mm)?,
        mm_to_nanometres(rules.clearance_mm)?,
        mm_to_nanometres(rules.via_diameter_mm)?,
        mm_to_nanometres(rules.via_drill_mm)?,
    ];
    let output = Command::new(python)
        .arg("-c")
        .arg(APPLY_RULES_AND_EXPORT)
        .arg(board)
        .arg(dsn)
        .args(values.map(|value| value.to_string()))
        .output()
        .map_err(|error| format!("failed to run {}: {error}", python.display()))?;
    write_process_log(log, &output.stdout, &output.stderr)?;
    if !output.status.success() {
        return Err(format!(
            "pcbnew rule application/DSN export failed with {}; see {}",
            output.status,
            log.display()
        ));
    }
    if !dsn.is_file() {
        return Err(format!("pcbnew did not write {}", dsn.display()));
    }
    Ok(())
}

const IMPORT_SESSION: &str = r#"
import pcbnew
import sys

board_path, session_path = sys.argv[1:3]
board = pcbnew.LoadBoard(board_path)
if board is None:
    raise RuntimeError("pcbnew.LoadBoard returned None")
if not pcbnew.ImportSpecctraSES(board, session_path):
    raise RuntimeError("ImportSpecctraSES failed")
pcbnew.SaveBoard(board_path, board)
print(pcbnew.GetBuildVersion())
"#;

fn import_specctra_session(
    python: &Path,
    board: &Path,
    session: &Path,
    log: &Path,
) -> Result<(), String> {
    // SaveBoard may migrate project defaults (including rule severities).
    // Admit the imported copper under the frozen source settings instead.
    let output = with_original_project_settings(board, || {
        Command::new(python)
            .arg("-c")
            .arg(IMPORT_SESSION)
            .arg(board)
            .arg(session)
            .output()
            .map_err(|error| format!("failed to run {}: {error}", python.display()))
    })?;
    write_process_log(log, &output.stdout, &output.stderr)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "pcbnew SES import failed with {}; see {}",
            output.status,
            log.display()
        ))
    }
}

fn with_original_project_settings<T>(
    board: &Path,
    action: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let project = board.with_extension("kicad_pro");
    let original = if project.exists() {
        Some(fs::read(&project).map_err(|error| format!("read {}: {error}", project.display()))?)
    } else {
        None
    };
    let result = action();
    match original {
        Some(bytes) => fs::write(&project, bytes)
            .map_err(|error| format!("restore {} after import: {error}", project.display()))?,
        None if project.exists() => fs::remove_file(&project).map_err(|error| {
            format!("restore absent {} after import: {error}", project.display())
        })?,
        None => {}
    }
    result
}

#[test]
fn session_import_restores_settings_after_success_and_failure() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = env::temp_dir().join(format!(
        "pcb-maker-import-settings-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir(&directory).unwrap();
    let board = directory.join("board.kicad_pcb");
    let project = board.with_extension("kicad_pro");
    for fail in [false, true] {
        fs::write(&project, b"original severity settings").unwrap();
        let result = with_original_project_settings(&board, || {
            fs::write(&project, b"migrated settings that ignore a check").unwrap();
            if fail {
                Err("import failed".to_string())
            } else {
                Ok(())
            }
        });
        assert_eq!(result.is_err(), fail);
        assert_eq!(fs::read(&project).unwrap(), b"original severity settings");
    }
    fs::remove_file(&project).unwrap();
    with_original_project_settings(&board, || {
        fs::write(&project, b"new defaults").unwrap();
        Ok(())
    })
    .unwrap();
    assert!(!project.exists());
    fs::remove_dir(directory).unwrap();
}

const REFILL_AND_SAVE_ZONES: &str = r#"
import pcbnew
import sys

board_path = sys.argv[1]
board = pcbnew.LoadBoard(board_path)
if board is None:
    raise RuntimeError("pcbnew.LoadBoard returned None")
zones = board.Zones()
if len(zones) == 0:
    raise RuntimeError("zone refill requested for a board without zones")
pcbnew.ZONE_FILLER(board).Fill(zones)
pcbnew.SaveBoard(board_path, board)
print(pcbnew.GetBuildVersion())
print(len(zones))
"#;

fn refill_and_save_zones(python: &Path, board: &Path, log: &Path) -> Result<(), String> {
    let output = Command::new(python)
        .arg("-c")
        .arg(REFILL_AND_SAVE_ZONES)
        .arg(board)
        .output()
        .map_err(|error| format!("failed to run {}: {error}", python.display()))?;
    write_process_log(log, &output.stdout, &output.stderr)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "pcbnew zone refill/save failed with {}; see {}",
            output.status,
            log.display()
        ))
    }
}

fn mm_to_nanometres(value: f64) -> Result<i64, String> {
    let scaled = value * 1_000_000.0;
    if !scaled.is_finite() || scaled < 0.0 || scaled > i64::MAX as f64 {
        return Err(format!("cannot convert {value} mm to KiCad internal units"));
    }
    let rounded = scaled.round();
    if (scaled - rounded).abs() > 1e-6 {
        return Err(format!(
            "{value} mm is not exactly representable at KiCad nanometre resolution"
        ));
    }
    Ok(rounded as i64)
}

fn identify_tools(
    java: &Path,
    python: &Path,
    jar: &Path,
    jar_sha256: &str,
) -> Result<CompetitiveToolEvidence, String> {
    let kicad_cli_version = capture_version(Command::new("kicad-cli").arg("version"))?;
    let pcbnew_version = capture_version(
        Command::new(python)
            .arg("-c")
            .arg("import pcbnew; print(pcbnew.GetBuildVersion())"),
    )?;
    let java_version = capture_version(Command::new(java).arg("-version"))?;
    Ok(CompetitiveToolEvidence {
        kicad_cli_version,
        pcbnew_version,
        java_version,
        freerouting_version: None,
        freerouting_jar: jar.to_path_buf(),
        freerouting_jar_sha256: jar_sha256.to_string(),
    })
}

fn capture_version(command: &mut Command) -> Result<String, String> {
    let debug = format!("{command:?}");
    let output = command
        .output()
        .map_err(|error| format!("failed to run {debug}: {error}"))?;
    if !output.status.success() {
        return Err(format!("{debug} exited with {}", output.status));
    }
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    combined
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with("/usr/src/debug/"))
        .map(str::to_string)
        .ok_or_else(|| format!("{debug} produced no version text"))
}

enum LoggedProcessOutcome {
    Exited(ExitStatus),
    TimedOut,
}

impl LoggedProcessOutcome {
    fn success(&self) -> bool {
        matches!(self, Self::Exited(status) if status.success())
    }

    fn exit_code(&self) -> i32 {
        match self {
            Self::Exited(status) => status.code().unwrap_or(-1),
            Self::TimedOut => -1,
        }
    }

    fn description(&self) -> String {
        match self {
            Self::Exited(status) => status.to_string(),
            Self::TimedOut => "after reaching its time budget".into(),
        }
    }
}

fn run_logged_with_timeout(
    command: &mut Command,
    log: &Path,
    timeout: Duration,
) -> Result<ExitStatus, String> {
    match run_logged_with_timeout_outcome(command, log, timeout)? {
        LoggedProcessOutcome::Exited(status) => Ok(status),
        LoggedProcessOutcome::TimedOut => Err(format!(
            "command exceeded {:.3} seconds; partial log is in {}",
            timeout.as_secs_f64(),
            log.display()
        )),
    }
}

fn run_logged_with_timeout_outcome(
    command: &mut Command,
    log: &Path,
    timeout: Duration,
) -> Result<LoggedProcessOutcome, String> {
    let output = File::create(log)
        .map_err(|error| format!("failed to create {}: {error}", log.display()))?;
    let error_output = output
        .try_clone()
        .map_err(|error| format!("failed to clone {}: {error}", log.display()))?;
    command
        .stdout(Stdio::from(output))
        .stderr(Stdio::from(error_output));
    let debug = format!("{command:?}");
    let mut child = command
        .spawn()
        .map_err(|error| format!("failed to spawn {debug}: {error}"))?;
    let started = Instant::now();
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("failed to poll {debug}: {error}"))?
        {
            return Ok(LoggedProcessOutcome::Exited(status));
        }
        if started.elapsed() >= timeout {
            child
                .kill()
                .map_err(|error| format!("failed to terminate timed-out {debug}: {error}"))?;
            let _ = child.wait();
            return Ok(LoggedProcessOutcome::TimedOut);
        }
        thread::sleep(Duration::from_millis(25));
    }
}

#[derive(Default)]
struct ParsedFreeroutingLog {
    version: Option<String>,
    autorouter_passes: usize,
    initial_unrouted_items: Option<usize>,
    final_unrouted_items: Option<usize>,
    cpu_seconds: Option<f64>,
}

fn parse_freerouting_log(source: &str) -> ParsedFreeroutingLog {
    let mut parsed = ParsedFreeroutingLog::default();
    for line in source.lines() {
        if parsed.version.is_none()
            && let Some(start) = line.find("Freerouting v")
        {
            parsed.version = line[start..]
                .split_whitespace()
                .nth(1)
                .map(|version| version.trim_start_matches('v').to_string());
        }
        if line.contains("Auto-router pass #") {
            parsed.autorouter_passes += 1;
            parsed.final_unrouted_items = parse_number_before(line, " unrouted").or(Some(0));
        }
        if line.contains("Auto-router session completed:") {
            parsed.initial_unrouted_items = parse_number_after(line, "started with ");
            parsed.cpu_seconds = parse_float_before_after(line, "using ", " total CPU seconds");
        }
    }
    parsed
}

fn parse_number_after(source: &str, marker: &str) -> Option<usize> {
    source
        .split_once(marker)?
        .1
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

fn parse_number_before(source: &str, marker: &str) -> Option<usize> {
    source
        .split_once(marker)?
        .0
        .split(|character: char| !character.is_ascii_digit())
        .rfind(|piece| !piece.is_empty())?
        .parse()
        .ok()
}

fn parse_float_before_after(source: &str, start: &str, end: &str) -> Option<f64> {
    source
        .split_once(start)?
        .1
        .split_once(end)?
        .0
        .trim()
        .parse()
        .ok()
}

fn validate_internal_routing_rules(
    value: &serde_json::Value,
    rules: &CompetitiveRouteRules,
) -> Result<(), String> {
    fn visit(
        value: &serde_json::Value,
        rules: &CompetitiveRouteRules,
        checked: &mut usize,
    ) -> Result<(), String> {
        match value {
            serde_json::Value::Object(object) => {
                if object.contains_key("trace_width_mm") {
                    *checked += 1;
                    let number = |name: &str| {
                        object
                            .get(name)
                            .and_then(serde_json::Value::as_f64)
                            .ok_or_else(|| format!("routing config has no numeric {name}"))
                    };
                    let actual = [
                        number("trace_width_mm")?,
                        number("clearance_mm")?,
                        number("via_size_mm")?,
                        number("via_drill_mm")?,
                    ];
                    let expected = [
                        rules.trace_width_mm,
                        rules.clearance_mm,
                        rules.via_diameter_mm,
                        rules.via_drill_mm,
                    ];
                    if actual
                        .iter()
                        .zip(expected)
                        .any(|(actual, expected)| (actual - expected).abs() > 1.0e-12)
                    {
                        return Err(format!(
                            "pcb-maker routing geometry {actual:?} differs from benchmark geometry {expected:?}"
                        ));
                    }
                }
                for child in object.values() {
                    visit(child, rules, checked)?;
                }
            }
            serde_json::Value::Array(values) => {
                for child in values {
                    visit(child, rules, checked)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    let mut checked = 0;
    visit(value, rules, &mut checked)?;
    if checked == 0 {
        Err("pcb-maker insertion config contains no routing geometry".into())
    } else {
        Ok(())
    }
}

fn json_usize(value: &serde_json::Value, field: &str) -> Result<usize, String> {
    let raw = value
        .get(field)
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| format!("manifest has no unsigned {field}"))?;
    usize::try_from(raw).map_err(|_| format!("manifest {field} does not fit usize"))
}

fn json_str<'a>(value: &'a serde_json::Value, field: &str) -> Result<&'a str, String> {
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| format!("manifest has no string {field}"))
}

fn summarize_native_cache_events(
    cache_directory: &Path,
    event_log: &Path,
) -> Result<CompetitiveNativeCacheEvidence, String> {
    let source = fs::read_to_string(event_log)
        .map_err(|error| format!("failed to read {}: {error}", event_log.display()))?;
    let mut events = 0;
    let mut hits = 0;
    let mut misses = 0;
    let mut bypasses = 0;
    let mut keys = BTreeSet::new();
    for (index, line) in source.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let event: pcb_kicad::KiCadVerificationCacheTrace =
            serde_json::from_str(line).map_err(|error| {
                format!(
                    "failed to parse {} line {}: {error}",
                    event_log.display(),
                    index + 1
                )
            })?;
        events += 1;
        match event.status {
            pcb_kicad::KiCadVerificationCacheStatus::Hit => hits += 1,
            pcb_kicad::KiCadVerificationCacheStatus::Miss => misses += 1,
            pcb_kicad::KiCadVerificationCacheStatus::Bypassed => bypasses += 1,
        }
        if let Some(key) = event.key {
            keys.insert(key);
        }
    }
    if events == 0 {
        return Err("pcb-maker produced an empty native-cache event log".into());
    }
    Ok(CompetitiveNativeCacheEvidence {
        directory: fs::canonicalize(cache_directory).map_err(|error| {
            format!(
                "failed to canonicalize cache {}: {error}",
                cache_directory.display()
            )
        })?,
        event_log: event_log.to_path_buf(),
        events,
        hits,
        misses,
        bypasses,
        unique_keys: keys.len(),
    })
}

fn publish_comparison_renders(
    route_config: &CompetitiveRouteBenchmarkConfig,
    route_config_base: &Path,
    output_directory: &Path,
    freerouting: &KiCadLayerRenderReport,
    pcb_maker: &KiCadLayerRenderReport,
) -> Result<Vec<PathBuf>, String> {
    let journal_directory = resolve_file(route_config_base, &route_config.journal_directory);
    fs::create_dir_all(&journal_directory)
        .map_err(|error| format!("failed to create {}: {error}", journal_directory.display()))?;
    let run_name = journal_run_name(output_directory)?;
    let entries = [
        (&freerouting.front, "freerouting-result-front.png"),
        (&freerouting.back, "freerouting-result-back.png"),
        (&pcb_maker.front, "pcb-maker-result-front.png"),
        (&pcb_maker.back, "pcb-maker-result-back.png"),
    ];
    let mut published = Vec::new();
    for (from, suffix) in entries {
        let to = journal_directory.join(format!("{run_name}-{suffix}"));
        fs::copy(from, &to).map_err(|error| {
            format!(
                "failed to publish comparison render {} to {}: {error}",
                from.display(),
                to.display()
            )
        })?;
        published.push(
            fs::canonicalize(&to)
                .map_err(|error| format!("failed to canonicalize {}: {error}", to.display()))?,
        );
    }
    Ok(published)
}

fn publish_journal_renders(
    config: &CompetitiveRouteBenchmarkConfig,
    config_base: &Path,
    output_directory: &Path,
    source: &KiCadLayerRenderReport,
    result: &KiCadLayerRenderReport,
) -> Result<Vec<PathBuf>, String> {
    let journal_directory = resolve_file(config_base, &config.journal_directory);
    fs::create_dir_all(&journal_directory)
        .map_err(|error| format!("failed to create {}: {error}", journal_directory.display()))?;
    let run_name = journal_run_name(output_directory)?;
    let entries = [
        (&source.front, "source-front.png"),
        (&source.back, "source-back.png"),
        (&result.front, "result-front.png"),
        (&result.back, "result-back.png"),
    ];
    let mut published = Vec::new();
    for (from, suffix) in entries {
        let to = journal_directory.join(format!("{run_name}-{suffix}"));
        fs::copy(from, &to).map_err(|error| {
            format!(
                "failed to publish render {} to {}: {error}",
                from.display(),
                to.display()
            )
        })?;
        published.push(
            fs::canonicalize(&to)
                .map_err(|error| format!("failed to canonicalize {}: {error}", to.display()))?,
        );
    }
    Ok(published)
}

fn journal_run_name(output_directory: &Path) -> Result<String, String> {
    let leaf = output_directory
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            format!(
                "output directory {} has no usable run name",
                output_directory.display()
            )
        })?;
    let Some(parent) = output_directory.parent() else {
        return Ok(leaf.to_string());
    };
    if !parent.join("competitive-corpus.json").is_file() {
        return Ok(leaf.to_string());
    }
    let corpus = parent
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("corpus directory {} has no usable name", parent.display()))?;
    Ok(format!("{corpus}-{leaf}"))
}

fn stage<T>(
    report: &mut CompetitiveRouteRunReport,
    output_directory: &Path,
    name: &str,
    operation: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    report.active_stage = Some(name.to_string());
    write_report(output_directory, report)?;
    let started = Instant::now();
    match operation() {
        Ok(value) => {
            report.stages.push(CompetitiveRouteStage {
                name: name.to_string(),
                elapsed_micros: started.elapsed().as_micros() as u64,
            });
            report.active_stage = None;
            write_report(output_directory, report)?;
            Ok(value)
        }
        Err(error) => Err(format!("{name}: {error}")),
    }
}

fn write_report(output_directory: &Path, report: &CompetitiveRouteRunReport) -> Result<(), String> {
    let path = output_directory.join("competitive-result.json");
    let serialized = serde_json::to_string_pretty(report).map_err(|error| error.to_string())?;
    fs::write(&path, format!("{serialized}\n"))
        .map_err(|error| format!("failed to write {}: {error}", path.display()))
}

fn write_comparison_report(
    output_directory: &Path,
    report: &CompetitiveRouteComparisonReport,
) -> Result<(), String> {
    let path = output_directory.join("competitive-comparison.json");
    let serialized = serde_json::to_string_pretty(report).map_err(|error| error.to_string())?;
    fs::write(&path, format!("{serialized}\n"))
        .map_err(|error| format!("failed to write {}: {error}", path.display()))
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    serde_json::from_str(&source)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))
}

fn absolute(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        env::current_dir()
            .map(|directory| directory.join(path))
            .map_err(|error| format!("failed to resolve {}: {error}", path.display()))
    }
}

fn resolve_file(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

fn resolve_executable(base: &Path, path: &Path) -> PathBuf {
    if path.components().count() == 1 {
        path.to_path_buf()
    } else {
        resolve_file(base, path)
    }
}

fn copy_directory(source: &Path, destination: &Path) -> Result<(), String> {
    if !source.is_dir() {
        return Err(format!("{} is not a directory", source.display()));
    }
    if destination.exists() {
        return Err(format!("refusing to overwrite {}", destination.display()));
    }
    fs::create_dir_all(destination)
        .map_err(|error| format!("failed to create {}: {error}", destination.display()))?;
    let mut entries = fs::read_dir(source)
        .map_err(|error| format!("failed to read {}: {error}", source.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to enumerate {}: {error}", source.display()))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let file_type = entry
            .file_type()
            .map_err(|error| format!("failed to inspect {}: {error}", entry.path().display()))?;
        let to = destination.join(entry.file_name());
        if file_type.is_dir() {
            copy_directory(&entry.path(), &to)?;
        } else if file_type.is_file() {
            fs::copy(entry.path(), &to).map_err(|error| {
                format!(
                    "failed to copy {} to {}: {error}",
                    entry.path().display(),
                    to.display()
                )
            })?;
        } else {
            return Err(format!(
                "source directory contains unsupported symlink or special file {}",
                entry.path().display()
            ));
        }
    }
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file =
        File::open(path).map_err(|error| format!("failed to open {}: {error}", path.display()))?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn sha256_directory(root: &Path) -> Result<String, String> {
    let mut files = Vec::new();
    collect_files(root, root, &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));
    let mut hash = Sha256::new();
    for (relative, path) in files {
        let relative = relative.to_string_lossy();
        hash.update((relative.len() as u64).to_le_bytes());
        hash.update(relative.as_bytes());
        let bytes = fs::read(&path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn collect_files(
    root: &Path,
    directory: &Path,
    files: &mut Vec<(PathBuf, PathBuf)>,
) -> Result<(), String> {
    if !directory.is_dir() {
        return Err(format!("{} is not a directory", directory.display()));
    }
    for entry in fs::read_dir(directory)
        .map_err(|error| format!("failed to read {}: {error}", directory.display()))?
    {
        let entry = entry
            .map_err(|error| format!("failed to enumerate {}: {error}", directory.display()))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("failed to inspect {}: {error}", entry.path().display()))?;
        if file_type.is_dir() {
            collect_files(root, &entry.path(), files)?;
        } else if file_type.is_file() {
            let relative = entry
                .path()
                .strip_prefix(root)
                .map_err(|error| error.to_string())?
                .to_path_buf();
            files.push((relative, entry.path()));
        } else {
            return Err(format!(
                "cannot hash symlink or special file {}",
                entry.path().display()
            ));
        }
    }
    Ok(())
}

fn write_process_log(path: &Path, stdout: &[u8], stderr: &[u8]) -> Result<(), String> {
    let mut bytes = Vec::with_capacity(stdout.len() + stderr.len() + 2);
    bytes.extend_from_slice(stdout);
    if !stdout.ends_with(b"\n") {
        bytes.push(b'\n');
    }
    bytes.extend_from_slice(stderr);
    fs::write(path, bytes).map_err(|error| format!("failed to write {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_complete_and_incomplete_freerouting_logs() {
        let source = "Freerouting v2.2.4 (build-date: today)\nAuto-router pass #1 was completed (3 unrouted)\nAuto-router pass #2 was completed\nAuto-router session completed: started with 24 unrouted nets, completed in 5.90 seconds, final score: 994.50, using 4.72 total CPU seconds\n";
        let parsed = parse_freerouting_log(source);
        assert_eq!(parsed.version.as_deref(), Some("2.2.4"));
        assert_eq!(parsed.autorouter_passes, 2);
        assert_eq!(parsed.initial_unrouted_items, Some(24));
        assert_eq!(parsed.final_unrouted_items, Some(0));
        assert_eq!(parsed.cpu_seconds, Some(4.72));
    }

    #[test]
    fn route_admission_separates_metadata_but_preserves_design_findings() {
        let metadata = serde_json::json!({
            "type": "lib_footprint_mismatch", "severity": "warning",
            "description": "Footprint differs from library", "items": []
        });
        let clearance = serde_json::json!({
            "type": "clearance", "severity": "warning",
            "description": "Copper too close", "items": []
        });
        let baseline = serde_json::json!({"violations": [metadata, clearance]});
        let mut changed_metadata = metadata.clone();
        changed_metadata["description"] = serde_json::json!("Different footprint mismatch");
        let mut result = serde_json::json!({"violations": [changed_metadata, clearance]});
        let introduced = |result: &serde_json::Value| {
            introduced_issue_count(drc_design_issues(&baseline), drc_design_issues(result)).unwrap()
        };
        assert_eq!(introduced(&result), 0);
        // An additional occurrence of a design warning still rejects admission.
        result["violations"].as_array_mut().unwrap().push(clearance);
        assert_eq!(introduced(&result), 1);
        // Even the same library finding is a design issue when severity is error.
        result["violations"][0]["severity"] = serde_json::json!("error");
        assert_eq!(introduced(&result), 2);
        // Unknown warning types are not silently classified as metadata.
        result["violations"][0]["type"] = serde_json::json!("new_kicad_rule");
        result["violations"][0]["severity"] = serde_json::json!("warning");
        assert_eq!(introduced(&result), 2);
    }

    #[test]
    fn route_admission_ignores_issue_identity_and_order_but_not_new_geometry() {
        let baseline = serde_json::json!({
            "type": "silk_edge_clearance",
            "severity": "warning",
            "items": [
                {"description": "board edge", "pos": {"x": 1, "y": 2}, "uuid": "a"},
                {"description": "silk", "pos": {"x": 3, "y": 4}, "uuid": "b"}
            ]
        });
        let reordered = serde_json::json!({
            "severity": "warning",
            "type": "silk_edge_clearance",
            "items": [
                {"description": "silk", "pos": {"y": 4, "x": 3}, "uuid": "new-b"},
                {"description": "board edge", "pos": {"y": 2, "x": 1}, "uuid": "new-a"}
            ]
        });
        let moved = serde_json::json!({
            "type": "silk_edge_clearance",
            "severity": "warning",
            "items": [
                {"description": "board edge", "pos": {"x": 1, "y": 2}},
                {"description": "silk", "pos": {"x": 3, "y": 5}}
            ]
        });

        assert_eq!(
            introduced_issue_count(vec![&baseline], vec![&reordered]).unwrap(),
            0
        );
        assert_eq!(
            introduced_issue_count(vec![&baseline], vec![&reordered, &moved]).unwrap(),
            1
        );
    }

    #[test]
    fn config_rejects_non_cold_and_unbounded_runs() {
        let mut config = CompetitiveRouteBenchmarkConfig {
            schema_version: 1,
            id: "control".into(),
            source: CompetitiveRouteSource::MaterializedKicad {
                directory: "input".into(),
                board_id: "board".into(),
            },
            placement_acceptance: None,
            result_acceptance: None,
            post_route_processors: Vec::new(),
            rules: CompetitiveRouteRules {
                trace_width_mm: 0.25,
                clearance_mm: 0.2,
                via_diameter_mm: 0.7,
                via_drill_mm: 0.3,
            },
            freerouting: FreeroutingConfig {
                jar: "freerouting.jar".into(),
                expected_jar_sha256:
                    "f5ed374182900ccc78e473518bbb9f6b869f4a07159495f663a76f52bb10523b".into(),
                java_executable: "java".into(),
                python_executable: "python3".into(),
                maximum_seconds: 60,
                maximum_passes: 100,
                maximum_threads: 1,
            },
            require_zero_source_copper: true,
            render: KiCadLayerRenderConfig::default(),
            journal_directory: "journal".into(),
        };
        config.check().unwrap();
        config.source = CompetitiveRouteSource::SemanticKicad {
            problem: "problem.json".into(),
            placement_config: "placement.json".into(),
            template_config: "template.json".into(),
            connection_prefix: 0,
            routing_feedback: None,
        };
        config.check().unwrap();
        if let CompetitiveRouteSource::SemanticKicad {
            routing_feedback, ..
        } = &mut config.source
        {
            *routing_feedback = Some(CompetitiveRoutingFeedbackPlacementConfig {
                routing_config: PathBuf::new(),
                pressure_config: "pressure.json".into(),
            });
        }
        assert!(
            config
                .check()
                .unwrap_err()
                .contains("routing-feedback placement config paths")
        );
        if let CompetitiveRouteSource::SemanticKicad {
            routing_feedback, ..
        } = &mut config.source
        {
            *routing_feedback = None;
        }
        config.require_zero_source_copper = false;
        assert!(config.check().unwrap_err().contains("zero source copper"));
        config.require_zero_source_copper = true;
        config.freerouting.maximum_seconds = 0;
        assert!(config.check().unwrap_err().contains("bounded Freerouting"));
    }

    #[test]
    fn placement_acceptance_requires_observed_translation_rotation_and_improvement() {
        let mut config = CompetitiveRouteBenchmarkConfig {
            schema_version: 1,
            id: "autoplace-control".into(),
            source: CompetitiveRouteSource::SemanticKicad {
                problem: "problem.json".into(),
                placement_config: "placement.json".into(),
                template_config: "template.json".into(),
                connection_prefix: 2,
                routing_feedback: None,
            },
            placement_acceptance: Some(CompetitivePlacementAcceptance {
                minimum_moved_components: 1,
                minimum_translated_components: 1,
                minimum_rotated_components: 1,
                minimum_connectivity_distance_reduction_mm: 2.0,
            }),
            result_acceptance: None,
            post_route_processors: Vec::new(),
            rules: CompetitiveRouteRules {
                trace_width_mm: 0.4,
                clearance_mm: 0.3,
                via_diameter_mm: 0.8,
                via_drill_mm: 0.4,
            },
            freerouting: FreeroutingConfig {
                jar: "freerouting.jar".into(),
                expected_jar_sha256:
                    "f5ed374182900ccc78e473518bbb9f6b869f4a07159495f663a76f52bb10523b".into(),
                java_executable: "java".into(),
                python_executable: "python3".into(),
                maximum_seconds: 60,
                maximum_passes: 100,
                maximum_threads: 1,
            },
            require_zero_source_copper: true,
            render: KiCadLayerRenderConfig::default(),
            journal_directory: "journal".into(),
        };
        config.check().unwrap();
        let evidence = CompetitivePlacementEvidence {
            moved_components: 1,
            translated_components: 1,
            rotated_components: 1,
            connectivity_distance_before_mm: 12.8,
            connectivity_distance_after_mm: 10.0,
            connectivity_distance_reduction_mm: 2.8,
        };
        validate_placement_acceptance(&config, Some(&evidence)).unwrap();

        let unchanged = CompetitivePlacementEvidence {
            moved_components: 0,
            translated_components: 0,
            rotated_components: 0,
            connectivity_distance_before_mm: 12.8,
            connectivity_distance_after_mm: 12.8,
            connectivity_distance_reduction_mm: 0.0,
        };
        let error = validate_placement_acceptance(&config, Some(&unchanged)).unwrap_err();
        assert!(error.contains("moved components 0 < 1"));
        assert!(error.contains("translated components 0 < 1"));
        assert!(error.contains("rotated components 0 < 1"));
        assert!(error.contains("reduction 0.000000 mm < 2.000000 mm"));

        config.result_acceptance = Some(CompetitiveResultAcceptance {
            minimum_vias: 1,
            minimum_used_copper_layers: 2,
            minimum_copper_zones: 2,
            minimum_filled_zone_area_mm2: 100.0,
            differential_pairs: Vec::new(),
        });
        config.check().unwrap();
        validate_result_acceptance(&config, 1, 2, 2, 100.0, &[], "control").unwrap();
        let error = validate_result_acceptance(&config, 0, 1, 0, 0.0, &[], "control").unwrap_err();
        assert!(error.contains("control result missed acceptance"));
        assert!(error.contains("vias 0 < 1"));
        assert!(error.contains("used copper layers 1 < 2"));
        assert!(error.contains("copper zones 0 < 2"));
        assert!(error.contains("filled zone area 0.000000 mm2 < 100.000000 mm2"));

        config.source = CompetitiveRouteSource::MaterializedKicad {
            directory: "input".into(),
            board_id: "board".into(),
        };
        assert!(
            config
                .check()
                .unwrap_err()
                .contains("requires a semantic_kicad source")
        );
    }

    #[test]
    fn comparison_requires_bounded_execution_and_exact_matching_rules() {
        let mut comparison = CompetitiveRouteComparisonConfig {
            schema_version: 1,
            id: "control-comparison".into(),
            route_benchmark: "route.json".into(),
            pcb_maker: PcbMakerComparisonConfig {
                insertion_config: "insertion.json".into(),
                sequential_config: PathBuf::new(),
                verification_cache_directory: "cache".into(),
                maximum_seconds: 60,
            },
        };
        comparison.check().unwrap();
        comparison.pcb_maker.maximum_seconds = 0;
        assert!(comparison.check().unwrap_err().contains("maximum_seconds"));
        comparison.pcb_maker.maximum_seconds = 60;
        comparison.pcb_maker.sequential_config = "sequential.json".into();
        assert!(comparison.check().unwrap_err().contains("exactly one"));
        comparison.pcb_maker.insertion_config = PathBuf::new();
        comparison.check().unwrap();

        let rules = CompetitiveRouteRules {
            trace_width_mm: 0.25,
            clearance_mm: 0.2,
            via_diameter_mm: 0.7,
            via_drill_mm: 0.3,
        };
        let exact = serde_json::json!({
            "local_routing_portfolio": [{
                "trace_width_mm": 0.25,
                "clearance_mm": 0.2,
                "via_size_mm": 0.7,
                "via_drill_mm": 0.3
            }]
        });
        validate_internal_routing_rules(&exact, &rules).unwrap();
        let mut per_net_mismatch = exact.clone();
        per_net_mismatch["local_routing_portfolio"][0]["connection_rules"] = serde_json::json!({
            "GND": {"trace_width_mm": 0.8, "clearance_mm": 0.28,
                    "via_size_mm": 0.7, "via_drill_mm": 0.3}
        });
        assert!(
            validate_internal_routing_rules(&per_net_mismatch, &rules).is_err(),
            "a uniform external-router benchmark must not admit different per-net rules"
        );
        let mismatch = serde_json::json!({
            "trace_width_mm": 0.25,
            "clearance_mm": 0.2,
            "via_size_mm": 0.8,
            "via_drill_mm": 0.4
        });
        assert!(
            validate_internal_routing_rules(&mismatch, &rules)
                .unwrap_err()
                .contains("differs")
        );
    }

    #[test]
    fn corpus_requires_unique_safe_case_ids() {
        let mut corpus = CompetitiveRouteCorpusConfig {
            schema_version: COMPETITIVE_ROUTE_CORPUS_SCHEMA_VERSION,
            id: "growth-smoke".into(),
            cases: vec![
                CompetitiveRouteCorpusCaseConfig {
                    id: "prefix-01".into(),
                    comparison: "prefix01/comparison.json".into(),
                },
                CompetitiveRouteCorpusCaseConfig {
                    id: "prefix-02".into(),
                    comparison: "prefix02/comparison.json".into(),
                },
            ],
        };
        corpus.check().unwrap();
        corpus.cases[1].id = "prefix-01".into();
        assert!(corpus.check().unwrap_err().contains("duplicate"));
        corpus.cases[1].id = "../escape".into();
        assert!(corpus.check().unwrap_err().contains("corpus case id"));
    }
}
