// Copyright (C) 2026 Florian Loitsch.

//! Publish the board KiCad actually checked after refilling its zones.

use crate::{KiCadRunStatus, fs_util, native_report};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub(crate) mod connectivity;
mod hatch;
pub(crate) mod layout;
mod parallel_feed;
mod track_order;
mod unchanged_hatch;
mod width_replay;
pub use connectivity::FillConnectivityProof;
pub use hatch::{HatchedNeckRepair, repair_kicad_hatched_necks};
pub use layout::FillPublication;
pub(crate) use layout::{has_fillable_zones, publish_layout_fill};
pub use parallel_feed::{add_kicad_parallel_feed, shorten_kicad_decoupling_path};
pub(crate) use unchanged_hatch::{
    UNCHANGED_HATCH_CHECKS_WITH_BASELINE, UnchangedHatchBaseline, UnchangedHatchBudget,
    repair_unchanged_hatch_widths,
};
pub use unchanged_hatch::{UnchangedHatchRepair, repair_kicad_unchanged_hatch_widths};
pub use width_replay::FillWidthReplay;

const FINDINGS: [&str; 3] = ["violations", "unconnected_items", "schematic_parity"];
pub(crate) type FindingCounts = BTreeMap<(String, String), usize>;

/// Whether the materialized project's board has copper zones for refill.
pub fn project_has_fillable_zones(project: &Path, board_id: &str) -> Result<bool, String> {
    fs_util::check_board_id(board_id)?;
    let path = project.join(format!("{board_id}.kicad_pcb"));
    let source = fs::read_to_string(&path)
        .map_err(|error| format!("read PCB {}: {error}", path.display()))?;
    let board = crate::kicad_sexpr::parse(&source)?;
    if board.head() != Some("kicad_pcb") {
        return Err("expected a KiCad PCB".into());
    }
    Ok(has_fillable_zones(&board))
}

/// Finish a newly routed, native-complete project through the same copper-only
/// handoff as a layout. Its original DRC remains the refilled result.
pub fn publish_routed_fill(
    project: &Path,
    board_id: &str,
    work_remaining: f64,
    wall_deadline: Option<Instant>,
) -> Option<(FillPublication, f64)> {
    use crate::check_work::{check_inputs, check_seconds};
    use pcb_kicad_file::{check_repairs, kicad_sexpr::parse};

    let skipped = |reason: String| Some((FillPublication::skipped(reason), 0.0));
    if let Err(error) = fs_util::check_board_id(board_id) {
        return skipped(error);
    }
    let board = project.join(format!("{board_id}.kicad_pcb"));
    let text = match fs::read_to_string(&board) {
        Ok(text) => text,
        Err(error) => return skipped(format!("read routed PCB: {error}")),
    };
    let pcb = match parse(&text) {
        Ok(pcb) => pcb,
        Err(error) => return skipped(error),
    };
    if !has_fillable_zones(&pcb) {
        return None;
    }
    let charge = check_seconds(check_inputs(&pcb));
    let estimated_wall = Instant::now() + Duration::from_secs_f64(8.0 * charge);
    let deadline = wall_deadline.map_or(estimated_wall, |deadline| deadline.min(estimated_wall));
    if work_remaining < 2.0 * charge || Instant::now() >= deadline {
        let mut fill = FillPublication::skipped(
            "the route budget did not cover refill and stored-board verification",
        );
        fill.retry_argv = vec![
            "pcb-maker".into(),
            "drc-kicad-board".into(),
            project.display().to_string(),
            board_id.into(),
            project
                .with_extension("saved-fill-drc.json")
                .display()
                .to_string(),
            "--save-filled".into(),
            project.with_extension("saved-fill").display().to_string(),
        ];
        return Some((fill, 0.0));
    }
    if check_repairs::needs_repair(pcb.clone()) {
        return skipped("KiCad's routing verifier needed to repair the refill input".into());
    }
    let expected = native_report::read_kicad_report(&project.join("drc.json")).and_then(|report| {
        native_report::validate_drc_report(&report, &board)?;
        if counts(&report).opens != 0 {
            return Err("the routed board's DRC still reports open connections".into());
        }
        Ok(report)
    });
    let expected = match expected {
        Ok(report) => report,
        Err(error) => return skipped(error),
    };
    use sha2::{Digest, Sha256};
    let fill = publish_layout_fill(
        project,
        board_id,
        pcb,
        format!("{:x}", Sha256::digest(text.as_bytes())),
        layout::FillRefillEvidence::CompletedRoute(&expected),
        false,
        || Instant::now() >= deadline,
    );
    let work = fill.native_checks_charged as f64 * charge;
    Some((fill, work))
}

#[derive(Clone, Debug, Serialize)]
pub struct FillDrcCounts {
    pub errors: usize,
    pub warnings: usize,
    pub exclusions: usize,
    pub opens: usize,
    pub schematic_parity: usize,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub possibly_capped_types: BTreeMap<&'static str, Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct FillHandoff {
    pub status: KiCadRunStatus,
    pub source_board_sha256: String,
    pub saved_board_sha256: String,
    pub preserved_source_forms: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_preservation_note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reordered_copper_forms: Option<usize>,
    pub schematic_parity_checked: bool,
    pub source_as_stored: FillDrcCounts,
    pub saved_as_stored: FillDrcCounts,
    pub native_checks: usize,
    pub connectivity_checks: usize,
    pub wall_seconds: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connectivity_proof: Option<FillConnectivityProof>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width_witness_replay: Option<FillWidthReplay>,
    /// Finding counts by native report field and rule type, matched to the
    /// original as-stored report by full finding identity.
    pub inherited_findings: BTreeMap<&'static str, BTreeMap<String, usize>>,
}

pub(crate) fn counts(report: &Value) -> FillDrcCounts {
    let exclusions =
        |finding: &Value| finding["severity"] == "exclusion" || finding["excluded"] == true;
    let severity = |level| {
        report["violations"]
            .as_array()
            .expect("validated findings")
            .iter()
            .filter(|finding| finding["severity"] == level && !exclusions(finding))
            .count()
    };
    let mut possibly_capped_types = BTreeMap::new();
    for field in FINDINGS {
        let limit = if field == "unconnected_items" {
            499
        } else {
            crate::verdict::KICAD_FINDING_CAP
        };
        let mut counts = BTreeMap::<String, usize>::new();
        for finding in report[field].as_array().expect("validated findings") {
            let kind = finding["type"].as_str().expect("validated rule type");
            *counts.entry(kind.into()).or_default() += 1;
        }
        let capped: Vec<_> = counts
            .into_iter()
            .filter_map(|(kind, count)| (count >= limit).then_some(kind))
            .collect();
        if !capped.is_empty() {
            possibly_capped_types.insert(field, capped);
        }
    }
    FillDrcCounts {
        errors: severity("error"),
        warnings: severity("warning"),
        exclusions: report["violations"]
            .as_array()
            .expect("validated findings")
            .iter()
            .filter(|finding| exclusions(finding))
            .count(),
        opens: report["unconnected_items"]
            .as_array()
            .expect("validated opens")
            .len(),
        schematic_parity: report["schematic_parity"]
            .as_array()
            .expect("validated parity")
            .len(),
        possibly_capped_types,
    }
}

pub(crate) fn keys(report: &Value, field: &str) -> FindingCounts {
    let mut counts = BTreeMap::new();
    for mut finding in report[field]
        .as_array()
        .expect("validated findings")
        .clone()
    {
        finding["items"]
            .as_array_mut()
            .expect("validated finding items")
            .sort_by_cached_key(Value::to_string);
        let kind = finding["type"]
            .as_str()
            .expect("validated rule type")
            .to_owned();
        *counts.entry((kind, finding.to_string())).or_default() += 1;
    }
    counts
}

/// Summarize the first unmatched findings on one side of an exact comparison.
fn unmatched_findings(ours: &FindingCounts, other: &FindingCounts) -> String {
    ours.iter()
        .filter(|(key, count)| **count > other.get(*key).copied().unwrap_or(0))
        .take(2)
        .filter_map(|((kind, json), _)| {
            let finding: Value = serde_json::from_str(json).ok()?;
            let severity = finding["severity"].as_str().unwrap_or("unknown severity");
            let state = if finding["excluded"] == true || severity == "exclusion" {
                "excluded"
            } else {
                "active"
            };
            let items = finding["items"]
                .as_array()?
                .iter()
                .filter_map(|item| {
                    item["description"]
                        .as_str()
                        .or_else(|| item["uuid"].as_str())
                })
                .take(3)
                .collect::<Vec<_>>()
                .join(" and ");
            let detail = if items.is_empty() {
                finding["description"].as_str().unwrap_or("")
            } else {
                &items
            };
            let mut detail_text = format!("{kind} ({state} {severity})");
            if !detail.is_empty() {
                detail_text.push_str(&format!(" involving {detail}"));
            }
            Some(detail_text)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// KiCad may reorder the footprints when saving, changing which library
/// warnings fit in its sample. In that case check both modes on the
/// actual saved board, with the usual exact comparison of all findings.
fn capped_library_order_changed(before: &Value, filled: &Value, stored: &Value) -> bool {
    if same_metadata(before, filled, stored).is_err()
        || ["unconnected_items", "schematic_parity"]
            .iter()
            .any(|field| keys(filled, field) != keys(stored, field))
    {
        return false;
    }
    let capped = |report: &Value| {
        counts(report)
            .possibly_capped_types
            .remove("violations")
            .unwrap_or_default()
    };
    let a = capped(filled);
    let b = capped(stored);
    let shared: Vec<_> = a
        .iter()
        .filter(|kind| {
            b.contains(kind)
                && matches!(
                    kind.as_str(),
                    "lib_footprint_issues" | "lib_footprint_mismatch"
                )
        })
        .collect();
    if shared.is_empty() {
        return false;
    }
    let others = |report| {
        let mut findings = keys(report, "violations");
        findings.retain(|(kind, text), _| {
            !shared.contains(&kind)
                || !serde_json::from_str(text)
                    .is_ok_and(|f| native_report::is_library_metadata_warning(&f))
        });
        findings
    };
    others(filled) == others(stored)
}

pub(crate) fn compare_reports(
    before: &Value,
    filled: &Value,
    saved: &Value,
) -> Result<BTreeMap<&'static str, BTreeMap<String, usize>>, String> {
    same_metadata(before, filled, saved)?;
    for field in FINDINGS {
        let saved = keys(saved, field);
        let filled = keys(filled, field);
        if filled != saved {
            let mut error =
                format!("stored board did not reproduce the refilled native DRC {field}");
            for (check, details) in [
                ("refilled", unmatched_findings(&filled, &saved)),
                ("stored", unmatched_findings(&saved, &filled)),
            ] {
                if !details.is_empty() {
                    error.push_str(&format!(". Only the {check} check reported {details}"));
                }
            }
            return Err(error);
        }
    }
    Ok(inherited_findings(before, saved))
}

/// `compare_reports` without the items of one-net findings
/// (`pcb_kicad_check::net_witness`): which copper of a joint KiCad names
/// for a narrow connection follows the path of the checked file, so two
/// checks of the same copper in different directories differ in them.
pub(crate) fn compare_board_reports(
    board: &pcb_kicad_file::kicad_sexpr::Expr,
    before: &Value,
    filled: &Value,
    saved: &Value,
) -> Result<BTreeMap<&'static str, BTreeMap<String, usize>>, String> {
    let [before, filled, saved] = [before, filled, saved]
        .map(|report| pcb_kicad_check::net_witness::net_witness_neutral(board, report));
    compare_reports(&before, &filled, &saved)
}

pub(crate) fn same_metadata(before: &Value, filled: &Value, saved: &Value) -> Result<(), String> {
    for field in [
        "kicad_version",
        "coordinate_units",
        "included_severities",
        "ignored_checks",
    ] {
        if before[field] != filled[field] || filled[field] != saved[field] {
            return Err(format!(
                "native DRC changed {field} across the source, refill and reopen checks"
            ));
        }
    }
    Ok(())
}

fn inherited_findings(
    before: &Value,
    saved: &Value,
) -> BTreeMap<&'static str, BTreeMap<String, usize>> {
    let mut inherited = BTreeMap::new();
    for field in FINDINGS {
        let saved = keys(saved, field);
        let before = keys(before, field);
        let mut matching = BTreeMap::new();
        for (key, count) in saved {
            let kept = count.min(*before.get(&key).unwrap_or(&0));
            if kept > 0 {
                *matching.entry(key.0).or_default() += kept;
            }
        }
        inherited.insert(field, matching);
    }
    inherited
}

fn explain_cap(error: String, refilled: &Value, stored: &Value) -> String {
    let library_cap = [refilled, stored].into_iter().any(|report| {
        counts(report)
            .possibly_capped_types
            .get("violations")
            .is_some_and(|kinds| {
                kinds.iter().any(|kind| {
                    matches!(
                        kind.as_str(),
                        "lib_footprint_issues" | "lib_footprint_mismatch"
                    )
                })
            })
    });
    if library_cap
        && !std::env::var_os("PCB_MAKER_KICAD_USER_LIBS")
            .is_some_and(|value| !value.is_empty() && value != "0")
    {
        format!(
            "{error}. KiCad's footprint library report may be capped. With the project's libraries installed, retry with PCB_MAKER_KICAD_USER_LIBS=1."
        )
    } else {
        error
    }
}

fn preserve_source_fill(
    original: &[u8],
    native: &[u8],
    order_check: Option<(&Value, &Value)>,
) -> Result<(Vec<u8>, Option<usize>), String> {
    use pcb_kicad_file::kicad_sexpr::{encode, parse};

    let mut source = parse(std::str::from_utf8(original).map_err(|error| error.to_string())?)?;
    let native = parse(std::str::from_utf8(native).map_err(|error| error.to_string())?)?;
    let reordered = if let Some((refilled, stored)) = order_check {
        if !track_order::same_measurements(&source, refilled, stored) {
            return Err("the staged reports disagree on their measured copper rules".into());
        }
        Some(
            track_order::reorder(&mut source, &native)
                .ok_or("KiCad's saved tracks do not give a different unambiguous order")?,
        )
    } else {
        None
    };
    layout::copy_fill_with_native_teardrops(&mut source, native)?;
    Ok((format!("{}\n", encode(&source)).into_bytes(), reordered))
}

struct CheckedSavedFill {
    stored: Value,
    hash: String,
    connectivity: Option<FillConnectivityProof>,
    width_witness_replay: Option<FillWidthReplay>,
    inherited: BTreeMap<&'static str, BTreeMap<String, usize>>,
}

#[derive(PartialEq, Eq)]
enum SourceForms {
    Preserved,
    Reordered,
    Native,
}

/// Copy the full input project, ask KiCad to refill and save the copy, then
/// reopen its stored bytes for a fresh native DRC before publishing it.
pub fn save_filled_kicad_board(
    source: &Path,
    board_id: &str,
    report: &Path,
    output: &Path,
) -> Result<FillHandoff, String> {
    let started = Instant::now();
    fs_util::check_board_id(board_id)?;
    let absolute_entry = |path: &Path| -> Result<PathBuf, String> {
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let parent = fs::canonicalize(parent).map_err(|error| {
            crate::input_error(format!("cannot open {}: {error}", parent.display()))
        })?;
        let name = path.file_name().ok_or_else(|| {
            crate::input_error("give a new fill output directory and a distinct report file")
        })?;
        Ok(parent.join(name))
    };
    let source = fs::canonicalize(source).map_err(|error| crate::input_error(error.to_string()))?;
    let output = absolute_entry(output)?;
    let report = absolute_entry(report)?;
    if fs::symlink_metadata(&output).is_ok()
        || output.starts_with(&source)
        || report.starts_with(&source)
        || report.starts_with(&output)
        || output.starts_with(&report)
    {
        return Err(crate::input_error(
            "use a fresh fill output directory and a separate report path outside the source project",
        ));
    }
    let stage = fs_util::temporary_next_to(&output);
    let saved = (|| {
        fs_util::copy_directory_tree(&source, &stage)?;
        native_report::remove_stale_board_run_reports(&stage)?;
        let board = stage.join(format!("{board_id}.kicad_pcb"));
        let parity =
            board.with_extension("kicad_sch").is_file() || board.with_extension("sch").is_file();
        let before_hash = fs_util::file_sha256(&board)?;
        let original = fs::read(&board).map_err(|error| error.to_string())?;
        let proof_context = layout::project_inputs(&stage);
        let checked_inputs = ["kicad_pro", "kicad_dru", "kicad_sch", "sch"]
            .map(|extension| board.with_extension(extension));
        let project_context = || {
            checked_inputs
                .iter()
                .map(|path| {
                    if path.is_file() {
                        fs_util::file_sha256(path).map(Some)
                    } else {
                        Ok(None)
                    }
                })
                .collect::<Result<Vec<_>, _>>()
        };
        let context = project_context()?;
        let before = native_report::run_fill_handoff_drc(
            &board,
            &stage.join("drc-before-fill.json"),
            parity,
            false,
        )?;
        if before_hash != fs_util::file_sha256(&board)? {
            return Err("KiCad's initial check wrote to the staged board".into());
        }
        let filled_path = stage.join("drc-refilled.json");
        let filled = native_report::run_fill_handoff_drc(&board, &filled_path, parity, true)?;
        let native = fs::read(&board).map_err(|error| error.to_string())?;
        let stored_path = stage.join("drc.json");
        if context != project_context()? {
            return Err(
                "KiCad changed the project's design rules or schematic while saving the board"
                    .into(),
            );
        }
        // The stored board is compared without the items of one-net
        // findings (`net_witness`): which of a joint's tracks KiCad names
        // follows the checked path. The witness replays keep KiCad's items.
        let source_pcb = pcb_kicad_file::kicad_sexpr::parse(
            std::str::from_utf8(&original).map_err(|error| error.to_string())?,
        )?;
        let mut native_checks = 2;
        let mut connectivity_checks = 0;
        let mut verify = |forms: SourceForms| -> Result<CheckedSavedFill, String> {
            let hash = fs_util::file_sha256(&board)?;
            native_checks += 1;
            let stored = native_report::run_fill_handoff_drc(&board, &stored_path, parity, false)?;
            if hash != fs_util::file_sha256(&board)? {
                return Err("checking the stored fill changed the board bytes".into());
            }
            let mut connectivity = None;
            let mut width_witness_replay = None;
            let inherited = if forms == SourceForms::Reordered {
                let original_pcb = pcb_kicad_file::kicad_sexpr::parse(
                    std::str::from_utf8(&original).map_err(|error| error.to_string())?,
                )?;
                native_checks += 1;
                let reopened = native_report::check_kept_route_input(
                    &board,
                    &stage.join("drc-refilled-as-published.json"),
                    parity,
                    true,
                    || false,
                )?;
                if !track_order::same_measurements(&original_pcb, &filled, &reopened)
                    || !track_order::same_measurements(&original_pcb, &filled, &stored)
                {
                    return Err("the ordered board changed a native measurement".into());
                }
                if let Err(error) = compare_reports(&before, &reopened, &stored)
                    && !connectivity::eligible(&before, &reopened, &stored)
                {
                    return Err(explain_cap(error, &reopened, &stored));
                }
                connectivity = Some(connectivity::prove(
                    &board,
                    &original,
                    &filled,
                    &stored,
                    &mut connectivity_checks,
                    || false,
                    Some(&reopened),
                )?);
                inherited_findings(&before, &stored)
            } else {
                match compare_board_reports(&source_pcb, &before, &filled, &stored) {
                    Ok(inherited) => inherited,
                    Err(_) if capped_library_order_changed(&before, &filled, &stored) => {
                        native_checks += 1;
                        let reopened = native_report::check_kept_route_input(
                            &board,
                            &stage.join("drc-refilled-as-published.json"),
                            parity,
                            true,
                            || false,
                        )?;
                        compare_board_reports(&source_pcb, &before, &reopened, &stored)?
                    }
                    Err(error)
                        if forms == SourceForms::Preserved
                            && connectivity::eligible(&before, &filled, &stored) =>
                    {
                        let proof = if connectivity::has_changed_fill_witnesses(&filled, &stored) {
                            connectivity::prove_saved_islands(
                                &board,
                                &native,
                                &filled,
                                &stored,
                                &mut connectivity_checks,
                            )
                            .map_err(|why| {
                                format!("native filled-island proof failed: {why}. Inspect the named nets, filled polygons and their attachments in a private KiCad copy before retrying --save-filled")
                            })
                        } else {
                            connectivity::prove(
                                &board,
                                &original,
                                &filled,
                                &stored,
                                &mut connectivity_checks,
                                || false,
                                None,
                            )
                        };
                        connectivity = Some(proof.map_err(|why| format!("{error}. {why}"))?);
                        inherited_findings(&before, &stored)
                    }
                    Err(error) if forms == SourceForms::Preserved => {
                        let trial = width_replay::Trial {
                            board: &board,
                            original: &original,
                            before: &before,
                            filled: &filled,
                            stored: &stored,
                            parity,
                        };
                        let Some((replay, proof)) = trial
                            .prove(&mut native_checks, &mut connectivity_checks)
                            .map_err(|reason| format!("{error}. Width witness replay: {reason}"))?
                        else {
                            return Err(explain_cap(error, &filled, &stored));
                        };
                        width_witness_replay = Some(replay);
                        connectivity = Some(proof);
                        inherited_findings(&before, &stored)
                    }
                    Err(error) => return Err(explain_cap(error, &filled, &stored)),
                }
            };
            if fs_util::file_sha256(&board)? != hash || project_context()? != context {
                return Err("the saved board or project changed during its native checks".into());
            }
            Ok(CheckedSavedFill {
                stored,
                hash,
                connectivity,
                width_witness_replay,
                inherited,
            })
        };
        let preserved = (|| {
            let inputs = proof_context.as_ref().map_err(Clone::clone)?;
            if &layout::project_inputs(&stage)? != inputs {
                return Err("the staged project changed while KiCad saved its fill".into());
            }
            let (spliced, _) = preserve_source_fill(&original, &native, None)?;
            fs_util::write_atomically(&board, &spliced)?;
            verify(SourceForms::Preserved)
        })();
        let (verified, source_preservation_note, reordered_copper_forms) = match preserved {
            Ok(verified) => (verified, None, None),
            Err(reason) => {
                let ordered = (|| {
                    let inputs = proof_context.as_ref().map_err(Clone::clone)?;
                    if &layout::project_inputs(&stage)? != inputs {
                        return Err(
                            "the staged inputs changed before ordering the saved fill".to_owned()
                        );
                    }
                    let stored = native_report::read_kicad_report(&stored_path)?;
                    let (spliced, changed) =
                        preserve_source_fill(&original, &native, Some((&filled, &stored)))?;
                    fs_util::write_atomically(&board, &spliced)?;
                    Ok::<_, String>((verify(SourceForms::Reordered)?, changed))
                })();
                match ordered {
                    Ok((verified, changed)) => (verified, None, changed),
                    Err(order_reason) => {
                        let reopened_path = stage.join("drc-refilled-as-published.json");
                        if reopened_path.is_file() {
                            fs::remove_file(&reopened_path).map_err(|error| error.to_string())?;
                        }
                        fs_util::write_atomically(&board, &native)?;
                        let verified = verify(SourceForms::Native).map_err(|error| {
                            format!("{error}. Source forms could not be preserved: {reason}. Track order retry: {order_reason}")
                        })?;
                        (verified, (native != original).then_some(reason), None)
                    }
                }
            }
        };
        if let Ok(inputs) = proof_context
            && layout::project_inputs(&stage)? != inputs
        {
            return Err("the staged project changed during fill verification".into());
        }
        let saved_as_stored = counts(&verified.stored);
        let complete = saved_as_stored.errors == 0
            && saved_as_stored.opens == 0
            && saved_as_stored.schematic_parity == 0;
        let result = FillHandoff {
            status: if complete {
                KiCadRunStatus::Complete
            } else {
                KiCadRunStatus::Incomplete
            },
            source_board_sha256: before_hash,
            saved_board_sha256: verified.hash,
            preserved_source_forms: source_preservation_note.is_none(),
            source_preservation_note,
            reordered_copper_forms,
            schematic_parity_checked: parity,
            source_as_stored: counts(&before),
            saved_as_stored,
            native_checks,
            connectivity_checks,
            wall_seconds: started.elapsed().as_secs_f64(),
            connectivity_proof: verified.connectivity,
            width_witness_replay: verified.width_witness_replay,
            inherited_findings: verified.inherited,
        };
        fs_util::write_typed_json(&stage.join("fill-handoff.json"), &result)?;
        fs_util::write_atomically(
            &report,
            &fs::read(filled_path).map_err(|error| error.to_string())?,
        )
        .map_err(|error| format!("failed to publish {}: {error}", report.display()))?;
        fs::create_dir(&output)
            .map_err(|error| format!("failed to reserve {}: {error}", output.display()))?;
        if let Err(error) = fs::rename(&stage, &output) {
            let _ = fs::remove_dir(&output);
            return Err(format!("failed to publish {}: {error}", output.display()));
        }
        Ok(result)
    })();
    if saved.is_err() {
        let _ = fs::remove_dir_all(&stage);
    }
    saved
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// OpenFC's routed board as KiCad 10.0.8 reports it from two
    /// directories (names of 2 and of 16 characters): the same narrow
    /// connection of +3.3V, other tracks of the joint named.
    #[test]
    fn a_width_finding_naming_other_copper_of_the_joint_is_the_same_finding() {
        let board = pcb_kicad_file::kicad_sexpr::parse(
            r#"(kicad_pcb
            (segment (start 154.09 89.3) (end 153.89 89.1) (width 0.09) (layer "F.Cu") (net "+3.3V") (uuid "bc4723a3-c7b5-5a3c-9294-ceb62c43bbf2"))
            (segment (start 154.09 89.4) (end 153.99 89.3) (width 0.09) (layer "F.Cu") (net "+3.3V") (uuid "c266b2fc-254e-50cd-a2db-8f2d0cbd218d"))
            (segment (start 154.29 89.4) (end 154.09 89.4) (width 0.09) (layer "F.Cu") (net "+3.3V") (uuid "f840d2c2-c499-5de0-8e4f-e8d4c569ece8"))
            (segment (start 160 89.4) (end 161 89.4) (width 0.09) (layer "F.Cu") (net "GND") (uuid "00000000-0000-0000-0000-000000000001")))"#,
        )
        .unwrap();
        let report = |width: &str, items: [(&str, &str, [f64; 2]); 2]| {
            json!({
                "coordinate_units": "mm", "kicad_version": "10.0.8",
                "unconnected_items": [], "schematic_parity": [],
                "violations": [{
                    "type": "connection_width", "severity": "warning",
                    "description": format!("Copper connection too narrow (board setup constraints minimum connection width 0.0900 mm; actual {width} mm) (F.Cu)"),
                    "items": items.map(|(uuid, length, [x, y])| json!({
                        "uuid": uuid, "pos": {"x": x, "y": y},
                        "description": format!("Track [+3.3V] on F.Cu, length {length} mm"),
                    })),
                }]
            })
        };
        let long = (
            "bc4723a3-c7b5-5a3c-9294-ceb62c43bbf2",
            "0.2828",
            [154.09, 89.3],
        );
        let short = (
            "c266b2fc-254e-50cd-a2db-8f2d0cbd218d",
            "0.1414",
            [154.09, 89.4],
        );
        let middle = (
            "f840d2c2-c499-5de0-8e4f-e8d4c569ece8",
            "0.2000",
            [154.29, 89.4],
        );
        let refilled = report("0.0463", [long, middle]);
        let stored = report("0.0463", [middle, short]);
        assert!(compare_reports(&refilled, &refilled, &stored).is_err());
        let inherited = compare_board_reports(&board, &refilled, &refilled, &stored).unwrap();
        assert_eq!(inherited["violations"]["connection_width"], 1);
        // Another measured width or copper of another net is another finding.
        let narrower = report("0.0400", [middle, short]);
        assert!(compare_board_reports(&board, &refilled, &refilled, &narrower).is_err());
        let other = (
            "00000000-0000-0000-0000-000000000001",
            "1.0000",
            [160.0, 89.4],
        );
        let elsewhere = report("0.0463", [other, other]);
        assert!(compare_board_reports(&board, &refilled, &refilled, &elsewhere).is_err());
    }

    #[test]
    fn native_fill_counts_boolean_and_severity_exclusions_without_double_counting() {
        let report = json!({
            "unconnected_items":[], "schematic_parity":[],
            "violations":[
                {"type":"old_exclusion", "severity":"exclusion"},
                {"type":"both", "severity":"exclusion", "excluded":true},
                {"type":"excluded_error", "severity":"error", "excluded":true},
                {"type":"excluded_warning", "severity":"warning", "excluded":true},
                {"type":"active_error", "severity":"error", "excluded":false},
                {"type":"implicit_active_error", "severity":"error"},
                {"type":"active_warning", "severity":"warning", "excluded":false}
            ]
        });
        let counts = counts(&report);
        assert_eq!(
            (counts.errors, counts.warnings, counts.exclusions),
            (2, 1, 4)
        );
    }

    #[test]
    fn repaired_project_fill_check_distinguishes_rule_areas_and_copper_zones() {
        let project = fs_util::temporary_next_to(&std::env::temp_dir().join("repair-fill-mode"));
        fs::create_dir(&project).unwrap();
        let board = project.join("repair.kicad_pcb");
        assert!(project_has_fillable_zones(&project, "repair").is_err());
        fs::write(
            &board,
            r#"(kicad_pcb
                 (zone (layer "F.Cu") (keepout (tracks not_allowed)))
                 (zone (layer "F.Cu") (fill no))
                 (footprint "a" (zone (layer "B.Cu") (keepout (pads not_allowed)))))"#,
        )
        .unwrap();
        assert!(!project_has_fillable_zones(&project, "repair").unwrap());
        fs::write(
            &board,
            r#"(kicad_pcb (footprint "a" (zone (layer "B.Cu") (fill yes))))"#,
        )
        .unwrap();
        assert!(project_has_fillable_zones(&project, "repair").unwrap());
        fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn direct_route_fill_checks_are_omitted_without_zones_or_work() {
        let project = fs_util::temporary_next_to(&std::env::temp_dir().join("route-fill"));
        fs::create_dir(&project).unwrap();
        let board = project.join("plane.kicad_pcb");
        fs::write(
            &board,
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (2 "B.Cu" signal)))"#,
        )
        .unwrap();
        assert!(publish_routed_fill(&project, "plane", 40.0, None).is_none());
        let filled_plane = r#"(kicad_pcb (layers (0 "F.Cu" signal) (2 "B.Cu" signal))
          (zone (net 1) (layer "F.Cu") (fill yes) (polygon (pts (xy 0 0) (xy 10 0) (xy 10 10)))))"#;
        fs::write(&board, filled_plane).unwrap();
        let (deferred, work) = publish_routed_fill(&project, "plane", 0.0, None).unwrap();
        assert_eq!(deferred.status, "skipped");
        assert_eq!(deferred.retry_argv[5], "--save-filled");
        assert_eq!(deferred.native_checks, 0);
        assert_eq!(work, 0.0);
        assert_eq!(fs::read_to_string(board).unwrap(), filled_plane);
        assert!(!project.join("drc-saved-fill.json").exists());
        fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn only_drift_in_capped_library_warnings_triggers_an_order_retry() {
        let rows: Vec<_> = (0..crate::verdict::KICAD_FINDING_CAP).map(|i| json!({
            "type":"lib_footprint_issues","severity":"warning","items":[{"uuid":i.to_string()}]
        })).collect();
        let before = json!({"violations":rows,"unconnected_items":[],"schematic_parity":[]});
        let mut stored = before.clone();
        stored["violations"][0]["items"][0]["uuid"] = json!("different");
        assert!(capped_library_order_changed(&before, &before, &stored));
        assert!(compare_reports(&before, &before, &stored).is_err());
        stored["violations"][0]["severity"] = json!("error");
        assert!(!capped_library_order_changed(&before, &before, &stored));
    }

    #[test]
    fn a_lost_exclusion_names_the_native_rule_and_both_objects() {
        let stored = json!({
            "violations": [{
                "type": "copper_edge_clearance",
                "severity": "error",
                "items": [
                    {"description": "Segment on Edge.Cuts", "uuid": "edge"},
                    {"description": "Pad 1 [Net-(D21-K)] of SW21 on B.Cu", "uuid": "pad",
                     "pos": {"x": 34.714413, "y": 109.028576}}
                ]
            }],
            "unconnected_items": [], "schematic_parity": []
        });
        let mut refilled = stored.clone();
        let finding = &mut refilled["violations"][0];
        finding["excluded"] = json!(true);
        finding["items"][1]["pos"]["y"] = json!(109.028577);

        let error = compare_reports(&refilled, &refilled, &stored).unwrap_err();
        assert!(error.contains("refilled check reported copper_edge_clearance (excluded error)"));
        assert!(error.contains("stored check reported copper_edge_clearance (active error)"));
        assert!(error.contains("Segment on Edge.Cuts"));
        assert!(error.contains("Pad 1 [Net-(D21-K)] of SW21 on B.Cu"));
        assert!(compare_reports(&refilled, &refilled, &refilled).is_ok());
    }
}
