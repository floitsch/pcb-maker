// Copyright (C) 2026 Toit contributors.

//! KiCad's own verification of a board directory (ERC, DRC, schematic
//! parity) and its cache.

use super::*;

mod refill;
use self::refill::Capture;
pub use self::refill::CheckedRefill;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct VerificationReport {
    pub board_id: String,
    pub complete: bool,
    pub erc_violations: usize,
    pub drc_design_violations: usize,
    /// An admitted native check included schematic parity.
    #[serde(default)]
    pub schematic_parity_checked: bool,
    pub schematic_parity_issues: usize,
    pub selected_net_unconnected_items: usize,
    pub intentional_no_connect_groups: usize,
    pub library_metadata_warnings: usize,
    /// ERC's library warnings by type (`is_erc_library_warning`), not in
    /// `erc_violations`.
    #[serde(default)]
    pub erc_library_warnings: BTreeMap<String, usize>,
}

pub const KICAD_VERIFICATION_CACHE_ENV: &str = "PCB_MAKER_KICAD_VERIFICATION_CACHE";

pub const KICAD_VERIFICATION_CACHE_EVENTS_ENV: &str = "PCB_MAKER_KICAD_VERIFICATION_CACHE_EVENTS";

const KICAD_VERIFICATION_CACHE_SCHEMA_VERSION: u32 = 2;

const KICAD_VERIFICATION_POLICY: &str = "verify-materialized-rung-v2:erc-without-library-warnings:drc-all:schematic-parity:intentional-no-connect-groups";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadVerificationCacheStatus {
    Hit,
    Miss,
    Bypassed,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct KiCadVerificationCacheTrace {
    pub schema_version: u32,
    pub status: KiCadVerificationCacheStatus,
    pub key: Option<String>,
    pub input_sha256: Option<String>,
    pub kicad_cli_version: String,
    pub verifier_sha256: String,
    pub policy: String,
    pub refill_zones: bool,
    pub cache_root: PathBuf,
    pub cache_entry: Option<PathBuf>,
    pub elapsed_micros: u64,
    pub detail: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct KiCadVerificationCacheEntry {
    schema_version: u32,
    key: String,
    input_sha256: String,
    kicad_cli_version: String,
    verifier_sha256: String,
    policy: String,
    refill_zones: bool,
    erc_sha256: String,
    drc_sha256: String,
    report: VerificationReport,
}

pub fn verify_materialized_rung(
    directory: &Path,
    board_id: &str,
) -> Result<VerificationReport, String> {
    verification_preview::capture(directory, board_id, || {
        remove_if_present(&directory.join("verification.json"))?;
        let pcb_path = directory.join(format!("{board_id}.kicad_pcb"));
        let pcb_source = fs::read_to_string(&pcb_path)
            .map_err(|error| format!("failed to read {}: {error}", pcb_path.display()))?;
        let pcb = parse(&pcb_source)?;
        verify_materialized_rung_with_options(directory, board_id, count_head(&pcb, "zone") > 0)
    })
}

/// The ordinary final layout check may keep the fill it just verified for
/// its own publication. A failed optional capture is charged separately.
pub fn verify_layout_rung(
    directory: &Path,
    board_id: &str,
) -> Result<(VerificationReport, Option<CheckedRefill>, usize), String> {
    if !native_drc_uses_english() || env::var_os(KICAD_VERIFICATION_CACHE_ENV).is_some() {
        return verify_materialized_rung(directory, board_id).map(|report| (report, None, 1));
    }
    let board = directory.join(format!("{board_id}.kicad_pcb"));
    let drc_path = directory.join("drc.json");
    match refill::capture_layout(directory, &board, &drc_path) {
        Capture::Verified(drc, snapshot) => {
            let receipt = file_sha256(&drc_path)
                .ok()
                .map(|report_sha256| CheckedRefill {
                    snapshot,
                    drc: drc.clone(),
                    report_name: "drc.json",
                    report_sha256,
                });
            verification_preview::capture(directory, board_id, || {
                verify_with_schematic(directory, board_id, true, SchematicDrc::Checked(drc))
            })
            .map(|report| (report, receipt, 1))
        }
        fallback => verify_materialized_rung(directory, board_id).map(|report| {
            (
                report,
                None,
                1 + usize::from(matches!(fallback, Capture::Failed)),
            )
        }),
    }
}

/// Finalize evidence for a layout's additive copper repair. Its private
/// project contains the selected board's unchanged schematic and ERC,
/// and the repair stage's freshly checked DRC for the exact new board.
pub fn record_verified_additive_board(
    directory: &Path,
    board_id: &str,
    parity_requested: bool,
) -> Result<VerificationReport, String> {
    verification_preview::capture(directory, board_id, || {
        let erc = read_json(&directory.join("erc.json"))?;
        let drc = read_json(&directory.join("drc.json"))?;
        write_verification_report(directory, board_id, &erc, &drc, parity_requested)
    })
}

/// Proof that a fresh all-track parity DRC applies to this routed project.
/// It can travel with a selected attempt when the ladder renames its directory.
#[derive(Clone, Debug)]
pub struct ClosedReuseDrc {
    inputs: ClosedReuseInputs,
    report: String,
    report_name: &'static str,
    partial: bool,
    captured_refill: Option<std::sync::Arc<refill::Snapshot>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ClosedReuseInputs {
    parent: PathBuf,
    project: String,
    native: String,
    refill: bool,
}

pub(crate) fn local_library_table(table: &Expr) -> bool {
    let suffix = match table.head() {
        Some("fp_lib_table") => "pretty",
        Some("sym_lib_table") => "kicad_sym",
        _ => return false,
    };
    table
        .children()
        .iter()
        .filter(|item| item.head() == Some("lib"))
        .all(|item| {
            let Some(relative) =
                form_atom(item, "uri", 1).and_then(|uri| uri.strip_prefix("${KIPRJMOD}/"))
            else {
                return false;
            };
            form_atom(item, "type", 1) == Some("KiCad")
                && !relative.contains(['$', '\\'])
                && Path::new(relative).extension().is_some_and(|ext| ext == suffix)
                && Path::new(relative).components().all(|item| {
                    matches!(item, std::path::Component::Normal(part) if !part.to_string_lossy().starts_with('.'))
                })
        })
}

fn closed_reuse_inputs(
    directory: &Path,
    board_id: &str,
    partial: bool,
    key: impl FnOnce(&Path, bool) -> Result<String, String>,
) -> Option<ClosedReuseInputs> {
    let board = directory.join(format!("{board_id}.kicad_pcb"));
    let pcb = parse(&fs::read_to_string(&board).ok()?).ok()?;
    let refill = pcb.child("zone").is_some();
    if partial {
        // The usual check repairs duplicate-priority pours and null panel
        // edges before refilling. Its input must match the unmodified trial.
        if refill && check_repairs::needs_repair(pcb) {
            return None;
        }
    } else {
        let footprints = pcb
            .children()
            .iter()
            .filter(|item| matches!(item.head(), Some("footprint" | "module")));
        // Preserve the closed-copper path's existing zone eligibility.
        if pcb
            .children()
            .iter()
            .chain(footprints.flat_map(Expr::children))
            .any(|item| item.head() == Some("zone") && item.child("keepout").is_none())
        {
            return None;
        }
    }
    let schematic = directory.join(format!("{board_id}.kicad_sch"));
    // Relative sheet dependencies may resolve outside the project directory.
    if parse(&fs::read_to_string(schematic).ok()?)
        .ok()?
        .child("sheet")
        .is_some()
    {
        return None;
    }
    for name in ["fp-lib-table", "sym-lib-table"] {
        let path = directory.join(name);
        if path.is_file() && !local_library_table(&parse(&fs::read_to_string(path).ok()?).ok()?) {
            return None;
        }
    }
    match kicad_cache_input_sha256(directory).ok()? {
        KiCadCacheInput::Cacheable(project) => Some(ClosedReuseInputs {
            parent: directory.parent()?.canonicalize().ok()?,
            project,
            native: key(&board, refill).ok()?,
            refill,
        }),
        KiCadCacheInput::Bypassed(_) => None,
    }
}

impl ClosedReuseDrc {
    /// Whether the ordinary board verification used this same report.
    pub fn shares_final_verification(&self) -> bool {
        self.report_name == "drc.json"
    }

    /// Copy the checked report only when the exact saved board, its local
    /// project inputs and the report still match what KiCad checked.
    pub fn copy_if_current(&self, directory: &Path, board_id: &str, output: &Path) -> bool {
        let inputs = closed_reuse_inputs(
            directory,
            board_id,
            self.partial,
            native_report::route_reuse_drc_input_key,
        );
        self.copy_report(directory, output, inputs)
    }

    /// Transfer the fill only while its exact routed project and the
    /// freshly checked raw DRC artifact still match the selected rung.
    pub fn refill_if_current(&self, directory: &Path, board_id: &str) -> Option<CheckedRefill> {
        let snapshot = self.captured_refill.as_ref()?;
        let board = directory.join(format!("{board_id}.kicad_pcb"));
        if !snapshot.matches(&board) {
            return None;
        }
        let inputs = closed_reuse_inputs(
            directory,
            board_id,
            self.partial,
            native_report::route_reuse_drc_input_key,
        );
        let bytes = self.checked_report_bytes(directory, inputs)?;
        Some(CheckedRefill {
            snapshot: snapshot.clone(),
            drc: serde_json::from_slice(&bytes).ok()?,
            report_name: self.report_name,
            report_sha256: self.report.clone(),
        })
    }

    fn copy_report(
        &self,
        directory: &Path,
        output: &Path,
        inputs: Option<ClosedReuseInputs>,
    ) -> bool {
        self.checked_report_bytes(directory, inputs)
            .is_some_and(|report| write_atomically(output, &report).is_ok())
    }

    fn checked_report_bytes(
        &self,
        directory: &Path,
        inputs: Option<ClosedReuseInputs>,
    ) -> Option<Vec<u8>> {
        if inputs.as_ref() != Some(&self.inputs) {
            return None;
        }
        let report = fs::read(directory.join(self.report_name)).ok()?;
        (format!("{:x}", Sha256::digest(&report)) == self.report).then_some(report)
    }
}

/// Use one strict current-board DRC for both normal verification and reuse
/// comparison when all routed copper is already present and the board has
/// no copper pours. Where the strict path cannot prove equivalence, the
/// first model-complete result can use the guarded refill-aware check.
pub fn verify_closed_reuse_rung(
    directory: &Path,
    board_id: &str,
    allow_refilled_proof: bool,
    capture_refill: bool,
) -> Result<(VerificationReport, Option<ClosedReuseDrc>, usize), String> {
    verify_reuse_rung(
        directory,
        board_id,
        false,
        allow_refilled_proof,
        capture_refill,
    )
}

/// Check all tracks first for a model-complete incremental trial. The
/// ordinary verification can share a report containing only optional library
/// warnings. Otherwise the raw report is used only for the reuse comparison.
/// The final element is the number of DRC checks to charge to the trial.
pub fn verify_partial_reuse_rung(
    directory: &Path,
    board_id: &str,
    capture_refill: bool,
) -> Result<(VerificationReport, Option<ClosedReuseDrc>, usize), String> {
    if env::var_os(KICAD_VERIFICATION_CACHE_ENV).is_some() {
        return verify_materialized_rung(directory, board_id).map(|report| (report, None, 1));
    }
    verify_reuse_rung(directory, board_id, true, false, capture_refill)
}

fn verify_reuse_rung(
    directory: &Path,
    board_id: &str,
    partial: bool,
    allow_refilled_proof: bool,
    capture_refill: bool,
) -> Result<(VerificationReport, Option<ClosedReuseDrc>, usize), String> {
    const DRC: &str = "drc.json";
    const ALL_TRACK_DRC: &str = "reuse-all-track-drc.json";
    let input_key = || {
        closed_reuse_inputs(
            directory,
            board_id,
            partial,
            native_report::route_reuse_drc_input_key,
        )
    };
    let Some(inputs) = input_key() else {
        if allow_refilled_proof {
            return verify_partial_reuse_rung(directory, board_id, capture_refill);
        }
        return verify_materialized_rung(directory, board_id).map(|report| (report, None, 1));
    };
    let mut captured_refill = None;
    let mut extra_checks = 0;
    let (drc, report_name, checks) = if partial {
        let pcb = directory.join(format!("{board_id}.kicad_pcb"));
        let raw = directory.join(ALL_TRACK_DRC);
        let capture = if capture_refill && inputs.refill {
            refill::capture(directory, &pcb, &raw)
        } else {
            Capture::Skipped
        };
        let report = match capture {
            Capture::Verified(report, proof) => {
                captured_refill = Some(proof);
                Ok(report)
            }
            fallback => {
                extra_checks = usize::from(matches!(fallback, Capture::Failed));
                native_report::run_route_reuse_drc(&pcb, &raw, inputs.refill)
            }
        };
        match report {
            Ok(report) if input_key().as_ref() == Some(&inputs) => {
                if can_share_partial_drc(&report) {
                    fs::rename(directory.join(ALL_TRACK_DRC), directory.join(DRC))
                        .map_err(|error| format!("could not publish reuse DRC: {error}"))?;
                    (SchematicDrc::Checked(report), DRC, 1)
                } else {
                    // Preserve ordinary verification and keep the raw
                    // report for the later comparison of unchanged inputs.
                    (SchematicDrc::Ordinary, ALL_TRACK_DRC, 2)
                }
            }
            _ => {
                return verify_materialized_rung(directory, board_id)
                    .map(|report| (report, None, 2 + extra_checks));
            }
        }
    } else {
        (SchematicDrc::AllTracks, DRC, 1)
    };
    let report = verification_preview::capture(directory, board_id, || {
        verify_with_schematic(directory, board_id, inputs.refill, drc)
    })?;
    let inputs_unchanged = input_key().as_ref() == Some(&inputs);
    if partial && checks == 1 && !inputs_unchanged {
        // The strict report also served as the ordinary check. Refresh it
        // before publishing if inputs changed while the attempt finished.
        return verify_materialized_rung(directory, board_id)
            .map(|report| (report, None, 2 + extra_checks));
    }
    let proof = if inputs_unchanged {
        file_sha256(&directory.join(report_name))
            .ok()
            .map(|report| ClosedReuseDrc {
                inputs,
                report,
                report_name,
                partial,
                captured_refill,
            })
    } else {
        None
    };
    Ok((report, proof, checks + extra_checks))
}

fn can_share_partial_drc(report: &serde_json::Value) -> bool {
    // Reporting every track error can only add design findings, and the
    // ordinary panel check can only excuse entries from unconnected_items.
    // There must be none of either to transform.
    ["schematic_parity", "unconnected_items"]
        .iter()
        .all(|field| report[field].as_array().is_some_and(Vec::is_empty))
        && report["violations"].as_array().is_some_and(|findings| {
            findings.iter().all(is_library_metadata_warning)
                && (findings.is_empty() || native_drc_uses_english())
        })
}

fn native_drc_uses_english() -> bool {
    // The all-track command confirms native parity with LC_ALL=C. Avoid
    // substituting its warning text when the ordinary command is localized.
    let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(env::var_os)
        .find(|value| !value.is_empty());
    env::var_os("LANGUAGE").is_none_or(|language| language.is_empty())
        && locale.is_none_or(|locale| {
            matches!(
                locale.to_str(),
                Some("C" | "C.UTF-8" | "POSIX" | "en_US.UTF-8")
            )
        })
}

fn verify_materialized_rung_with_options(
    directory: &Path,
    board_id: &str,
    refill_zones: bool,
) -> Result<VerificationReport, String> {
    if !directory.join(format!("{board_id}.kicad_sch")).exists() {
        return verify_board_only(directory, board_id, refill_zones);
    }
    if let Some(cache_root) = env::var_os(KICAD_VERIFICATION_CACHE_ENV) {
        verify_materialized_rung_cached(directory, board_id, refill_zones, Path::new(&cache_root))
    } else {
        verify_materialized_rung_uncached(directory, board_id, refill_zones)
    }
}

fn verify_materialized_rung_uncached(
    directory: &Path,
    board_id: &str,
    refill_zones: bool,
) -> Result<VerificationReport, String> {
    verify_with_schematic(directory, board_id, refill_zones, SchematicDrc::Ordinary)
}

enum SchematicDrc {
    Ordinary,
    AllTracks,
    Checked(serde_json::Value),
}

fn verify_with_schematic(
    directory: &Path,
    board_id: &str,
    refill_zones: bool,
    check: SchematicDrc,
) -> Result<VerificationReport, String> {
    remove_if_present(&directory.join("verification.json"))?;
    let schematic = directory.join(format!("{board_id}.kicad_sch"));
    let pcb = directory.join(format!("{board_id}.kicad_pcb"));
    let erc_path = directory.join("erc.json");
    let drc_path = directory.join("drc.json");
    run_kicad_report(
        &[
            "sch",
            "erc",
            "--format",
            "json",
            "--severity-all",
            "--output",
        ],
        &erc_path,
        &schematic,
    )?;
    let drc = match check {
        SchematicDrc::Checked(report) => report,
        SchematicDrc::AllTracks => {
            native_report::run_route_reuse_drc(&pcb, &drc_path, refill_zones)?
        }
        SchematicDrc::Ordinary => {
            let drc_arguments = native_report::ordinary_drc_arguments(true, refill_zones, false);
            run_kicad_report(&drc_arguments, &drc_path, &pcb)?;
            read_json(&drc_path)?
        }
    };
    let erc: serde_json::Value = read_json(&erc_path)?;
    write_verification_report(directory, board_id, &erc, &drc, true)
}

/// A project without a schematic (a bare board, as in board-only benchmark
/// sets): DRC without schematic parity, and an empty ERC report in its place.
fn verify_board_only(
    directory: &Path,
    board_id: &str,
    refill_zones: bool,
) -> Result<VerificationReport, String> {
    remove_if_present(&directory.join("verification.json"))?;
    let pcb = directory.join(format!("{board_id}.kicad_pcb"));
    let drc_path = directory.join("drc.json");
    let drc_arguments = native_report::ordinary_drc_arguments(false, refill_zones, false);
    run_kicad_report(&drc_arguments, &drc_path, &pcb)?;
    let drc: serde_json::Value = read_json(&drc_path)?;
    let erc = serde_json::json!({
        "source": format!("{board_id}.kicad_sch"),
        "date": drc["date"].as_str().unwrap_or("unknown"),
        "kicad_version": "none: board-only project, no schematic",
        "coordinate_units": "mm",
        "sheets": [],
    });
    write_typed_json(&directory.join("erc.json"), &erc)?;
    write_verification_report(directory, board_id, &erc, &drc, false)
}

pub(crate) fn write_verification_report(
    directory: &Path,
    board_id: &str,
    erc: &serde_json::Value,
    drc: &serde_json::Value,
    parity_requested: bool,
) -> Result<VerificationReport, String> {
    remove_if_present(&directory.join("verification.json"))?;
    let report = assess_verification_reports(board_id, erc, drc, parity_requested)?;
    write_typed_json(&directory.join("verification.json"), &report)?;
    Ok(report)
}

pub(crate) fn assess_verification_reports(
    board_id: &str,
    erc: &serde_json::Value,
    drc: &serde_json::Value,
    parity_requested: bool,
) -> Result<VerificationReport, String> {
    validate_report(
        erc,
        ReportKind::Erc,
        Path::new(&format!("{board_id}.kicad_sch")),
    )?;
    validate_report(
        drc,
        ReportKind::Drc,
        Path::new(&format!("{board_id}.kicad_pcb")),
    )?;
    let erc_violations = erc_design_issues(erc).len();
    let mut erc_library_warnings = BTreeMap::<String, usize>::new();
    for finding in erc["sheets"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|sheet| sheet["violations"].as_array().into_iter().flatten())
        .filter(|finding| is_erc_library_warning(finding))
    {
        *erc_library_warnings
            .entry(finding["type"].as_str().unwrap_or("").to_string())
            .or_default() += 1;
    }
    let schematic_parity_issues = drc["schematic_parity"].as_array().map_or(0, Vec::len);
    let mut drc_design_violations = 0;
    let mut library_metadata_warnings = 0;
    for violation in drc["violations"].as_array().into_iter().flatten() {
        if is_library_metadata_warning(violation) {
            library_metadata_warnings += 1;
        } else {
            drc_design_violations += 1;
        }
    }
    let mut intentional_no_connect_groups = 0;
    let mut selected_net_unconnected_items = 0;
    for violation in drc["unconnected_items"].as_array().into_iter().flatten() {
        if is_intentional_no_connect_group(violation) {
            intentional_no_connect_groups += 1;
        } else {
            selected_net_unconnected_items += 1;
        }
    }
    let complete = erc_violations == 0
        && drc_design_violations == 0
        && schematic_parity_issues == 0
        && selected_net_unconnected_items == 0;
    let report = VerificationReport {
        board_id: board_id.to_string(),
        complete,
        erc_violations,
        drc_design_violations,
        schematic_parity_checked: parity_requested && drc["schematic_parity"].is_array(),
        schematic_parity_issues,
        selected_net_unconnected_items,
        intentional_no_connect_groups,
        library_metadata_warnings,
        erc_library_warnings,
    };
    Ok(report)
}

enum KiCadCacheInput {
    Cacheable(String),
    Bypassed(String),
}

fn verify_materialized_rung_cached(
    directory: &Path,
    board_id: &str,
    refill_zones: bool,
    cache_root: &Path,
) -> Result<VerificationReport, String> {
    remove_if_present(&directory.join("verification.json"))?;
    let started = Instant::now();
    let kicad_cli_version = kicad_cli_version()?;
    let verifier_sha256 = kicad_verifier_implementation_sha256();
    let input_sha256 = match kicad_cache_input_sha256(directory)? {
        KiCadCacheInput::Cacheable(hash) => hash,
        KiCadCacheInput::Bypassed(detail) => {
            let report = verify_materialized_rung_uncached(directory, board_id, refill_zones)?;
            write_kicad_cache_trace(
                directory,
                &KiCadVerificationCacheTrace {
                    schema_version: KICAD_VERIFICATION_CACHE_SCHEMA_VERSION,
                    status: KiCadVerificationCacheStatus::Bypassed,
                    key: None,
                    input_sha256: None,
                    kicad_cli_version,
                    verifier_sha256,
                    policy: KICAD_VERIFICATION_POLICY.into(),
                    refill_zones,
                    cache_root: cache_root.to_path_buf(),
                    cache_entry: None,
                    elapsed_micros: started.elapsed().as_micros() as u64,
                    detail,
                },
            )?;
            return Ok(report);
        }
    };
    let key = {
        let mut hash = Sha256::new();
        hash.update(KICAD_VERIFICATION_CACHE_SCHEMA_VERSION.to_le_bytes());
        hash.update(KICAD_VERIFICATION_POLICY.as_bytes());
        hash.update([u8::from(refill_zones)]);
        hash.update((kicad_cli_version.len() as u64).to_le_bytes());
        hash.update(kicad_cli_version.as_bytes());
        hash.update(verifier_sha256.as_bytes());
        hash.update(input_sha256.as_bytes());
        format!("{:x}", hash.finalize())
    };
    fs::create_dir_all(cache_root).map_err(|error| {
        format!(
            "failed to create KiCad verification cache {}: {error}",
            cache_root.display()
        )
    })?;
    let cache_entry = cache_root.join(&key);
    if cache_entry.is_dir() {
        let entry: KiCadVerificationCacheEntry =
            read_typed_json(&cache_entry.join("cache-entry.json"))?;
        if entry.schema_version != KICAD_VERIFICATION_CACHE_SCHEMA_VERSION
            || entry.key != key
            || entry.input_sha256 != input_sha256
            || entry.kicad_cli_version != kicad_cli_version
            || entry.verifier_sha256 != verifier_sha256
            || entry.policy != KICAD_VERIFICATION_POLICY
            || entry.refill_zones != refill_zones
            || entry.report.board_id != board_id
        {
            return Err(format!(
                "KiCad verification cache entry {} does not match its key",
                cache_entry.display()
            ));
        }
        let cached_erc = cache_entry.join("erc.json");
        let cached_drc = cache_entry.join("drc.json");
        if file_sha256(&cached_erc)? != entry.erc_sha256
            || file_sha256(&cached_drc)? != entry.drc_sha256
        {
            return Err(format!(
                "KiCad verification cache entry {} has corrupted reports",
                cache_entry.display()
            ));
        }
        let assessed = assess_verification_reports(
            board_id,
            &read_json(&cached_erc)?,
            &read_json(&cached_drc)?,
            true,
        )?;
        if assessed != entry.report {
            return Err(format!(
                "KiCad verification cache entry {} has an inconsistent assessment",
                cache_entry.display()
            ));
        }
        fs::copy(&cached_erc, directory.join("erc.json"))
            .map_err(|error| format!("failed to restore cached ERC report: {error}"))?;
        fs::copy(&cached_drc, directory.join("drc.json"))
            .map_err(|error| format!("failed to restore cached DRC report: {error}"))?;
        write_typed_json(&directory.join("verification.json"), &entry.report)?;
        write_kicad_cache_trace(
            directory,
            &KiCadVerificationCacheTrace {
                schema_version: KICAD_VERIFICATION_CACHE_SCHEMA_VERSION,
                status: KiCadVerificationCacheStatus::Hit,
                key: Some(key),
                input_sha256: Some(input_sha256),
                kicad_cli_version,
                verifier_sha256,
                policy: KICAD_VERIFICATION_POLICY.into(),
                refill_zones,
                cache_root: cache_root.to_path_buf(),
                cache_entry: Some(cache_entry),
                elapsed_micros: started.elapsed().as_micros() as u64,
                detail: "restored exact ERC, DRC, and interpreted verification reports".into(),
            },
        )?;
        return Ok(entry.report);
    }

    let report = verify_materialized_rung_uncached(directory, board_id, refill_zones)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock precedes UNIX epoch: {error}"))?
        .as_nanos();
    let temporary = cache_root.join(format!(".{key}.{}.{}.tmp", std::process::id(), nonce));
    fs::create_dir(&temporary).map_err(|error| {
        format!(
            "failed to create temporary cache entry {}: {error}",
            temporary.display()
        )
    })?;
    let erc = directory.join("erc.json");
    let drc = directory.join("drc.json");
    fs::copy(&erc, temporary.join("erc.json"))
        .map_err(|error| format!("failed to cache ERC report: {error}"))?;
    fs::copy(&drc, temporary.join("drc.json"))
        .map_err(|error| format!("failed to cache DRC report: {error}"))?;
    fs::copy(
        directory.join("verification.json"),
        temporary.join("verification.json"),
    )
    .map_err(|error| format!("failed to cache verification report: {error}"))?;
    let entry = KiCadVerificationCacheEntry {
        schema_version: KICAD_VERIFICATION_CACHE_SCHEMA_VERSION,
        key: key.clone(),
        input_sha256: input_sha256.clone(),
        kicad_cli_version: kicad_cli_version.clone(),
        verifier_sha256: verifier_sha256.clone(),
        policy: KICAD_VERIFICATION_POLICY.into(),
        refill_zones,
        erc_sha256: file_sha256(&erc)?,
        drc_sha256: file_sha256(&drc)?,
        report: report.clone(),
    };
    write_typed_json(&temporary.join("cache-entry.json"), &entry)?;
    if let Err(error) = fs::rename(&temporary, &cache_entry) {
        if cache_entry.is_dir() {
            fs::remove_dir_all(&temporary).map_err(|cleanup| {
                format!(
                    "cache entry race ({error}) and failed to remove {}: {cleanup}",
                    temporary.display()
                )
            })?;
        } else {
            return Err(format!(
                "failed to publish cache entry {}: {error}",
                cache_entry.display()
            ));
        }
    }
    write_kicad_cache_trace(
        directory,
        &KiCadVerificationCacheTrace {
            schema_version: KICAD_VERIFICATION_CACHE_SCHEMA_VERSION,
            status: KiCadVerificationCacheStatus::Miss,
            key: Some(key),
            input_sha256: Some(input_sha256),
            kicad_cli_version,
            verifier_sha256,
            policy: KICAD_VERIFICATION_POLICY.into(),
            refill_zones,
            cache_root: cache_root.to_path_buf(),
            cache_entry: Some(cache_entry),
            elapsed_micros: started.elapsed().as_micros() as u64,
            detail: "ran native KiCad ERC/DRC and published an immutable cache entry".into(),
        },
    )?;
    Ok(report)
}

/// Identifies the native-verification implementation independently of the
/// router executable that happens to call it.
///
/// The cache intentionally invalidates conservatively after any change to this
/// crate or the resolved Rust dependency graph. Changes confined to placement
/// and routing crates retain native KiCad verification hits when their emitted
/// KiCad inputs are byte-identical.
pub fn kicad_verifier_implementation_sha256() -> String {
    let mut hash = Sha256::new();
    hash.update(b"pcb-maker-kicad-verifier-implementation-v1");
    // lib.rs held all of these until 2026-10-21. They were one crate,
    // pcb-kicad, until 2026-11-11: the key covers the roots of the crates
    // it became and the files it covered then, at their new paths (a
    // file of a crate above this one is read as bytes, not compiled).
    hash.update(include_bytes!("../../pcb-kicad/src/lib.rs"));
    hash.update(include_bytes!("lib.rs"));
    hash.update(include_bytes!("../../pcb-kicad-file/src/lib.rs"));
    hash.update(include_bytes!("../../pcb-kicad-file/src/kicad_sexpr.rs"));
    hash.update(include_bytes!("../../pcb-kicad-file/src/kicad_nets.rs"));
    hash.update(include_bytes!("verification.rs"));
    hash.update(include_bytes!("../../pcb-kicad-file/src/fs_util.rs"));
    hash.update(include_bytes!("native_report.rs"));
    // sub_boards.rs held outline_pieces.rs (its DRC excuse) until
    // 2026-11-11.
    hash.update(include_bytes!("../../pcb-kicad-file/src/outline_pieces.rs"));
    hash.update(include_bytes!("../../pcb-kicad-place/src/sub_boards.rs"));
    hash.update(include_bytes!("../../../Cargo.lock"));
    format!("{:x}", hash.finalize())
}

fn kicad_cli_version() -> Result<String, String> {
    let output = Command::new(crate::native_report::kicad_cli())
        .arg("version")
        .output()
        .map_err(|error| format!("failed to identify kicad-cli: {error}"))?;
    if !output.status.success() {
        return Err(format!("kicad-cli version exited with {}", output.status));
    }
    String::from_utf8(output.stdout)
        .map(|version| version.trim().to_string())
        .map_err(|error| format!("kicad-cli version was not UTF-8: {error}"))
}

fn kicad_cache_input_sha256(directory: &Path) -> Result<KiCadCacheInput, String> {
    for table in ["sym-lib-table", "fp-lib-table"] {
        let path = directory.join(table);
        if !path.is_file() {
            continue;
        }
        let source = fs::read_to_string(&path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        for tail in source.split("(uri \"").skip(1) {
            let Some(uri) = tail.split('"').next() else {
                return Ok(KiCadCacheInput::Bypassed(format!(
                    "could not parse library URI in {table}"
                )));
            };
            if !uri.starts_with("${KIPRJMOD}/") {
                return Ok(KiCadCacheInput::Bypassed(format!(
                    "{table} references external library {uri:?}"
                )));
            }
        }
    }
    let mut files = Vec::new();
    collect_kicad_cache_inputs(directory, directory, &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));
    if files.is_empty() {
        return Err(format!(
            "{} contains no KiCad design inputs",
            directory.display()
        ));
    }
    let mut hash = Sha256::new();
    hash.update(b"pcb-maker-kicad-verification-input-v1");
    for (relative, absolute) in files {
        let relative = relative.as_os_str().as_encoded_bytes();
        let contents = fs::read(&absolute)
            .map_err(|error| format!("failed to read {}: {error}", absolute.display()))?;
        hash.update((relative.len() as u64).to_le_bytes());
        hash.update(relative);
        hash.update((contents.len() as u64).to_le_bytes());
        hash.update(contents);
    }
    Ok(KiCadCacheInput::Cacheable(format!("{:x}", hash.finalize())))
}

fn collect_kicad_cache_inputs(
    root: &Path,
    directory: &Path,
    files: &mut Vec<(PathBuf, PathBuf)>,
) -> Result<(), String> {
    for entry in fs::read_dir(directory)
        .map_err(|error| format!("failed to read {}: {error}", directory.display()))?
    {
        let entry = entry.map_err(|error| format!("failed to read directory entry: {error}"))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("failed to inspect {}: {error}", entry.path().display()))?;
        if file_type.is_symlink() {
            return Err(format!(
                "KiCad verification cache rejects symlink {}",
                entry.path().display()
            ));
        }
        if file_type.is_dir() {
            collect_kicad_cache_inputs(root, &entry.path(), files)?;
            continue;
        }
        if !file_type.is_file() || !is_kicad_cache_input(&entry.path()) {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(root)
            .map_err(|error| error.to_string())?
            .to_path_buf();
        files.push((relative, entry.path()));
    }
    Ok(())
}

fn is_kicad_cache_input(path: &Path) -> bool {
    let name = path.file_name().and_then(|name| name.to_str());
    if matches!(name, Some("sym-lib-table" | "fp-lib-table")) {
        return true;
    }
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("kicad_sch" | "kicad_pcb" | "kicad_pro" | "kicad_sym" | "kicad_mod" | "kicad_dru")
    )
}

fn write_kicad_cache_trace(
    directory: &Path,
    trace: &KiCadVerificationCacheTrace,
) -> Result<(), String> {
    write_typed_json(&directory.join("verification-cache.json"), trace)?;
    if let Some(event_path) = env::var_os(KICAD_VERIFICATION_CACHE_EVENTS_ENV) {
        let event_path = PathBuf::from(event_path);
        if let Some(parent) = event_path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "failed to create cache event directory {}: {error}",
                    parent.display()
                )
            })?;
        }
        let mut events = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&event_path)
            .map_err(|error| {
                format!(
                    "failed to open cache event log {}: {error}",
                    event_path.display()
                )
            })?;
        // One write of the whole line: processes appending to one log at
        // once do not interleave within a line (O_APPEND).
        let line = serde_json::to_string(trace).map_err(|error| error.to_string())? + "\n";
        events.write_all(line.as_bytes()).map_err(|error| {
            format!(
                "failed to append cache event log {}: {error}",
                event_path.display()
            )
        })?;
    }
    Ok(())
}

fn is_intentional_no_connect_group(violation: &serde_json::Value) -> bool {
    let Some(items) = violation["items"].as_array() else {
        return false;
    };
    if items.is_empty() {
        return false;
    }
    let mut component = None;
    for item in items {
        let Some(description) = item["description"].as_str() else {
            return false;
        };
        let Some(net_start) = description.find("[unconnected-(") else {
            return false;
        };
        let Some(net_end) = description[net_start..].find("] of ") else {
            return false;
        };
        let after_net = &description[net_start + net_end + 5..];
        let Some(reference) = after_net.split(" on ").next() else {
            return false;
        };
        match component {
            None => component = Some(reference),
            Some(expected) if expected == reference => {}
            Some(_) => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_reuse_accepts_only_contained_native_libraries() {
        let library = |kind, uri| {
            local_library_table(
                &parse(&format!(
                    r#"(fp_lib_table (lib (name "fixture") (type "{kind}") (uri "{uri}")))"#
                ))
                .unwrap(),
            )
        };
        assert!(library("KiCad", "${KIPRJMOD}/parts/fixture.pretty"));
        assert!(!library("KiCad", "${KIPRJMOD}/../fixture.pretty"));
        assert!(!library("KiCad", "${KIPRJMOD}/parts/../../fixture.pretty"));
        assert!(!library("KiCad", "${KIPRJMOD}/.hidden/fixture.pretty"));
        assert!(!library("KiCad", "${KIPRJMOD}/${other}/fixture.pretty"));
        assert!(!library("Eagle", "${KIPRJMOD}/fixture.pretty"));
    }

    #[test]
    fn closed_reuse_report_follows_renamed_attempt_but_rejects_mutation() {
        check_reuse_report_invalidation(false, true);
    }

    #[test]
    fn partial_reuse_report_rejects_changed_dependencies_and_repairable_boards() {
        check_reuse_report_invalidation(true, false);
    }

    #[test]
    fn a_refilled_shared_report_still_rejects_mutation_and_input_repairs() {
        check_reuse_report_invalidation(true, true);
    }

    #[test]
    fn partial_reuse_needs_a_complete_report_without_design_findings() {
        let mut report = serde_json::json!({
            "schematic_parity": [],
            "unconnected_items": [],
            "violations": [],
        });
        assert!(can_share_partial_drc(&report));
        for field in ["schematic_parity", "unconnected_items", "violations"] {
            report[field] = serde_json::json!([{"type": "clearance", "severity": "warning"}]);
            assert!(!can_share_partial_drc(&report), "{field}");
            report[field] = serde_json::Value::Null;
            assert!(!can_share_partial_drc(&report), "{field} missing");
            report[field] = serde_json::json!([]);
        }
        report["violations"] = serde_json::json!([
            {"type": "lib_footprint_mismatch", "severity": "error"}
        ]);
        assert!(!can_share_partial_drc(&report));
    }

    fn check_reuse_report_invalidation(partial: bool, shared: bool) {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let scratch = env::temp_dir().join(format!(
            "pcb-maker-reuse-{}-{nonce}-{partial}-{shared}",
            std::process::id(),
        ));
        let attempt = scratch.join("rung");
        let selected = scratch.join("selected");
        fs::create_dir_all(&attempt).unwrap();
        let board = if partial {
            "(kicad_pcb (zone))"
        } else {
            "(kicad_pcb)"
        };
        let report_name = if shared {
            "drc.json"
        } else {
            "reuse-all-track-drc.json"
        };
        let files = [
            ("case.kicad_pcb", board),
            ("case.kicad_sch", "(kicad_sch)"),
            ("case.kicad_pro", "{}"),
            ("case.kicad_dru", "(version 1)"),
            (report_name, "{\"violations\":[]}"),
        ];
        for (name, text) in files {
            fs::write(attempt.join(name), text).unwrap();
        }
        let project_inputs = |path: &Path| {
            closed_reuse_inputs(path, "case", partial, |_, refill| {
                Ok(format!("native DRC with refill {refill}"))
            })
        };
        let proof = ClosedReuseDrc {
            inputs: project_inputs(&attempt).unwrap(),
            report: file_sha256(&attempt.join(report_name)).unwrap(),
            report_name,
            partial,
            captured_refill: None,
        };
        assert_eq!(proof.shares_final_verification(), shared);
        if partial {
            assert!(proof.inputs.refill);
            assert!(closed_reuse_inputs(&attempt, "case", false, |_, _| unreachable!()).is_none());
        }
        fs::rename(&attempt, &selected).unwrap();
        let copy = scratch.join("raw.json");
        let copy_report = |path: &Path| proof.copy_report(path, &copy, project_inputs(path));
        assert!(copy_report(&selected));
        assert_eq!(fs::read_to_string(&copy).unwrap(), files[4].1);
        for (name, text) in files {
            let path = selected.join(name);
            fs::write(&path, format!("{text}\n")).unwrap();
            assert!(!copy_report(&selected), "{name}");
            fs::write(&path, text).unwrap();
        }
        let unsupported = if partial {
            "(kicad_pcb (zone) (gr_line (start 0 0) (end 0 0) (layer \"Edge.Cuts\")))"
        } else {
            "(kicad_pcb (zone))"
        };
        fs::write(selected.join("case.kicad_pcb"), unsupported).unwrap();
        assert!(project_inputs(&selected).is_none());
        fs::write(
            selected.join("case.kicad_pcb"),
            "(kicad_pcb (zone (keepout (footprints not_allowed))))",
        )
        .unwrap();
        assert!(project_inputs(&selected).unwrap().refill);
        assert!(!copy_report(&selected));
        fs::write(selected.join("case.kicad_pcb"), files[0].1).unwrap();
        let other_parent = scratch.join("other");
        fs::create_dir(&other_parent).unwrap();
        let moved = other_parent.join("selected");
        fs::rename(selected, &moved).unwrap();
        assert!(!copy_report(&moved));
        fs::remove_dir_all(&scratch).unwrap();
    }

    #[test]
    fn verification_cache_hashes_design_inputs_and_ignores_reports() {
        assert_eq!(kicad_verifier_implementation_sha256().len(), 64);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = env::temp_dir().join(format!(
            "pcb-maker-kicad-cache-input-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("board.kicad_pcb"), "(kicad_pcb)\n").unwrap();
        fs::write(directory.join("board.kicad_pro"), "{}\n").unwrap();
        fs::write(
            directory.join("sym-lib-table"),
            r#"(sym_lib_table (lib (uri "${KIPRJMOD}/Generated.kicad_sym")))"#,
        )
        .unwrap();
        let digest = match kicad_cache_input_sha256(&directory).unwrap() {
            KiCadCacheInput::Cacheable(digest) => digest,
            KiCadCacheInput::Bypassed(reason) => panic!("unexpected bypass: {reason}"),
        };
        fs::write(directory.join("erc.json"), "generated report\n").unwrap();
        let after_report = match kicad_cache_input_sha256(&directory).unwrap() {
            KiCadCacheInput::Cacheable(digest) => digest,
            KiCadCacheInput::Bypassed(reason) => panic!("unexpected bypass: {reason}"),
        };
        assert_eq!(digest, after_report);
        fs::write(directory.join("board.kicad_pcb"), "(kicad_pcb (zone))\n").unwrap();
        let after_board = match kicad_cache_input_sha256(&directory).unwrap() {
            KiCadCacheInput::Cacheable(digest) => digest,
            KiCadCacheInput::Bypassed(reason) => panic!("unexpected bypass: {reason}"),
        };
        assert_ne!(digest, after_board);

        fs::write(
            directory.join("fp-lib-table"),
            r#"(fp_lib_table (lib (uri "/external/library.pretty")))"#,
        )
        .unwrap();
        assert!(matches!(
            kicad_cache_input_sha256(&directory).unwrap(),
            KiCadCacheInput::Bypassed(reason) if reason.contains("external library")
        ));
        fs::remove_dir_all(directory).unwrap();
    }
}
