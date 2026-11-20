// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The commands agents drive, with their exit codes and verdicts.
//!
//! Every run of `route-kicad-board`, `layout-kicad-board` and
//! `place-kicad-board` ends with a `verdict` in its JSON report and an exit
//! code that says the same (`pcb_kicad::verdict`): 0 complete, 1 incomplete,
//! 2 usage or input error, 3 incomplete and cut short by the wall clock,
//! 4 internal error or panic.

use pcb_kicad::{KiCadReference, KiCadRunFacts, KiCadRunStatus, KiCadVerdict};

mod free_dimension;
mod pour_policies;
mod schematic_layout;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::Instant,
};

/// How a command did not finish.
pub enum Failure {
    /// Bad arguments or input: exit 2.
    Usage(String),
    /// Anything else: exit 4.
    Internal(String),
}

/// Where a run's report goes, for an error or a panic after the run
/// started.
struct ReportTarget {
    command: String,
    report: PathBuf,
    outputs: BTreeMap<String, String>,
    started: Instant,
}

static TARGET: Mutex<Option<ReportTarget>> = Mutex::new(None);

/// The exit code of the verdict this run reached (-1: none yet).
static DECIDED: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);

/// The exit code of the verdict the run reached, if it reached one.
pub fn decided_exit() -> Option<u8> {
    u8::try_from(DECIDED.load(std::sync::atomic::Ordering::Relaxed)).ok()
}

fn set_target(command: &str, report: PathBuf, outputs: BTreeMap<String, String>) {
    if let Ok(mut target) = TARGET.lock() {
        *target = Some(ReportTarget {
            command: command.into(),
            report,
            outputs,
            started: Instant::now(),
        });
    }
}

/// Forgets where the run's report goes: a run that wrote its error's
/// verdict itself.
fn clear_target() {
    if let Ok(mut target) = TARGET.lock() {
        target.take();
    }
}

/// Writes the error's verdict as `<out-dir>/verdict.json` when the run got
/// as far as having an output directory. A usage error writes only into an
/// output directory the run already created, or, when a constraint needs
/// more room than the outline has (`room`), into one holding only the
/// verdict, which a retry may use again (`check_fresh_output`), and so
/// when kicad-cli is missing or older than KiCad 10 (`check_kicad`); an
/// internal error creates it. The run's own report (board-router.json,
/// ...) is left as it is, or absent.
pub fn report_failure(status: KiCadRunStatus, message: &str) {
    let Some(target) = TARGET.lock().ok().and_then(|mut target| target.take()) else {
        return;
    };
    let Some(directory) = target.report.parent() else {
        return;
    };
    let room = (status == KiCadRunStatus::UsageError)
        .then(pcb_kicad::noted_room)
        .flatten();
    // No kicad-cli (`check_kicad`): the verdict says what to install. An
    // input error writes no verdict otherwise unless it noted what would
    // fix it (the room a constraint needs, a lattice too large for the
    // memory).
    let kicad =
        status == KiCadRunStatus::UsageError && message.contains(pcb_kicad::KICAD_REQUIREMENT);
    if !directory.exists()
        && ((status == KiCadRunStatus::UsageError
            && room.is_none()
            && !kicad
            && pcb_kicad::noted_input_actions().is_empty())
            || fs::create_dir_all(directory).is_err())
    {
        return;
    }
    let mut verdict = pcb_kicad::error_verdict(
        &target.command,
        status,
        message,
        target.started.elapsed().as_secs_f64(),
        existing(&target.outputs),
    );
    // A constraint the outline cannot hold (a row longer than the board):
    // the room it needs, as numbers.
    if let Some(room) = room {
        if let Some(size) = room.suggested_outline_mm {
            verdict
                .next_actions
                .push(pcb_kicad::room_action(&room, size));
        }
        verdict.room = Some(room);
    }
    let _ = write_verdict(directory, None, &verdict);
}

/// Writes `verdict` as `<directory>/verdict.json` and, when the run wrote
/// its report (`report`), as the report's `verdict` too.
fn write_verdict(
    directory: &Path,
    report: Option<&Path>,
    verdict: &KiCadVerdict,
) -> Result<(), String> {
    let mut value = serde_json::to_value(verdict).map_err(|error| error.to_string())?;
    // KiCad DRC runs this process asked for and how many the cache answered.
    value["drc_cache"] =
        serde_json::to_value(pcb_kicad::drc_cache_counts()).map_err(|error| error.to_string())?;
    // Both files are written whole or not at all (a temporary file renamed
    // into place): a run killed meanwhile leaves no half report.
    let path = directory.join("verdict.json");
    pcb_kicad::write_atomically(
        &path,
        serde_json::to_string_pretty(&value)
            .map_err(|error| error.to_string())?
            .as_bytes(),
    )?;
    let Some(report) = report else {
        return Ok(());
    };
    let text = fs::read_to_string(report)
        .map_err(|error| format!("failed to read {}: {error}", report.display()))?;
    let mut content: serde_json::Value = serde_json::from_str(&text)
        .map_err(|error| format!("failed to parse {}: {error}", report.display()))?;
    // What the run repaired for KiCad's DRC, beside the router's numbers.
    if let Some(repairs) = value.get("repairs") {
        content["repairs"] = repairs.clone();
    }
    content["verdict"] = value;
    pcb_kicad::write_atomically(
        report,
        serde_json::to_string_pretty(&content)
            .map_err(|error| error.to_string())?
            .as_bytes(),
    )
}

/// Prints the verdict: its line and next actions on stdout, and for a
/// result that is not complete one line on stderr.
fn announce(verdict: &KiCadVerdict, report: &Path) {
    DECIDED.store(
        i32::from(verdict.exit_code),
        std::sync::atomic::Ordering::Relaxed,
    );
    if let Some(line) = pcb_kicad::custom_rules_line(&verdict.custom_rules, &verdict.lengths) {
        println!("{line}");
    }
    if let Some(why) = &verdict.pin_lines {
        println!("pin lines: {why} (verdict.pin_lines)");
    }
    println!(
        "verdict: {} (exit {}): {}",
        serde_json::to_value(verdict.status)
            .unwrap_or_default()
            .as_str()
            .unwrap_or(""),
        verdict.exit_code,
        verdict.reason
    );
    for action in &verdict.next_actions {
        println!("next: {action}");
    }
    if verdict.status != KiCadRunStatus::Complete {
        eprintln!("{}; see {}", verdict.reason, report.display());
    }
}

/// A path as the report gives it: absolute, so that an agent in another
/// directory finds it.
fn path_text(path: &Path) -> String {
    std::path::absolute(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
}

/// The outputs that exist (KiCad's reports are missing when its check did
/// not run).
fn existing(outputs: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    outputs
        .iter()
        .filter(|(key, path)| *key == "verdict" || Path::new(path).exists())
        .map(|(key, path)| (key.clone(), path.clone()))
        .collect()
}

/// Splits `--name value` flags from the positional arguments.
fn split_flags(
    arguments: Vec<String>,
    flags: &[&str],
    usage: &str,
) -> Result<(Vec<String>, BTreeMap<String, String>), Failure> {
    let mut positional = Vec::new();
    let mut values = BTreeMap::new();
    let mut iterator = arguments.into_iter();
    while let Some(argument) = iterator.next() {
        if let Some(flag) = flags.iter().find(|flag| **flag == argument) {
            let value = iterator
                .next()
                .ok_or_else(|| Failure::Usage(format!("{flag} needs a value; {usage}")))?;
            values.insert(flag.to_string(), value);
        } else if argument.starts_with("--") {
            return Err(Failure::Usage(format!(
                "unknown option {argument}; {usage}"
            )));
        } else {
            positional.push(argument);
        }
    }
    Ok((positional, values))
}

fn positional_count(
    arguments: &[String],
    range: std::ops::RangeInclusive<usize>,
    usage: &str,
) -> Result<(), Failure> {
    if range.contains(&arguments.len()) {
        Ok(())
    } else {
        Err(Failure::Usage(usage.to_string()))
    }
}

/// The project directory holds `<board-id>.kicad_pcb`.
fn check_project(directory: &str, board_id: &str) -> Result<(), Failure> {
    pcb_kicad::check_board_id(board_id).map_err(Failure::Usage)?;
    let board = Path::new(directory).join(format!("{board_id}.kicad_pcb"));
    if !Path::new(directory).is_dir() {
        return Err(Failure::Usage(format!(
            "project directory {directory} does not exist"
        )));
    }
    if !board.is_file() {
        return Err(Failure::Usage(format!(
            "{} does not exist (the board id is the file stem)",
            board.display()
        )));
    }
    Ok(())
}

fn check_fresh_output(directory: &str) -> Result<(), Failure> {
    let path = Path::new(directory);
    // A directory holding only the verdict of a run that ended in an input
    // error (the room a constraint needs) is taken again: the retry with
    // the fixed input goes where the first run went.
    let only_verdict = fs::read_dir(path)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.file_name())
                .collect::<Vec<_>>()
        })
        .is_ok_and(|names| names == [std::ffi::OsString::from("verdict.json")])
        && fs::read_to_string(path.join("verdict.json"))
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .is_some_and(|verdict| verdict["status"] == "usage_error");
    if only_verdict {
        fs::remove_dir_all(path)
            .map_err(|error| Failure::Usage(format!("failed to remove {directory}: {error}")))?;
    }
    if path.exists() {
        return Err(Failure::Usage(format!(
            "output directory {directory} already exists; give a new one"
        )));
    }
    Ok(())
}

fn check_file(path: &str) -> Result<(), Failure> {
    if !Path::new(path).is_file() {
        return Err(Failure::Usage(format!("{path} does not exist")));
    }
    Ok(())
}

fn read_json<T: serde::de::DeserializeOwned>(path: &str) -> Result<T, Failure> {
    let text = fs::read_to_string(path)
        .map_err(|error| Failure::Usage(format!("failed to read {path}: {error}")))?;
    serde_json::from_str(&text)
        .map_err(|error| Failure::Usage(format!("failed to parse {path}: {error}")))
}

/// A library error: the input's (marked by `pcb_kicad::input_error`) or
/// internal.
fn classify(error: String) -> Failure {
    if pcb_kicad::is_input_error(&error) {
        Failure::Usage(error)
    } else {
        Failure::Internal(error)
    }
}

/// The machine's work rates for this run, before any work
/// (`pcb_kicad::calibrate_work_rate`): measured, or pinned by
/// `PCB_WORK_RATE` or the router settings `work_rate` and
/// `placer_work_rate`. Printed, and written into the verdict.
fn calibrate_work_rate(router: Option<&pcb_kicad::KiCadBoardRouterConfig>) -> Result<(), Failure> {
    let pinned = match router.map(|config| (config.work_rate, config.placer_work_rate)) {
        Some((Some(expansions), placer)) => Some((expansions, placer)),
        Some((None, Some(_))) => {
            return Err(Failure::Usage(
                "the router setting placer_work_rate needs work_rate beside it".into(),
            ));
        }
        _ => None,
    };
    pcb_kicad::calibrate_work_rate(pinned)
        .map(|_| ())
        .map_err(Failure::Usage)
}

/// The reference project of `--reference`: `none`, a directory, or the
/// default.
fn reference_option(
    values: &BTreeMap<String, String>,
    default: Option<&str>,
    board_id: &str,
) -> Result<Option<PathBuf>, Failure> {
    match values.get("--reference").map(String::as_str).or(default) {
        None | Some("none") => Ok(None),
        Some(directory) => {
            check_project(directory, board_id)?;
            Ok(Some(PathBuf::from(directory)))
        }
    }
}

/// Whether two paths name one directory.
fn same_directory(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Fills in the reference's errors when the result has errors that count.
fn check_reference(
    facts: &mut KiCadRunFacts,
    reference: Option<&Path>,
    board_id: &str,
    report: &Path,
) {
    let Some(project) = reference else {
        return;
    };
    if !facts.needs_reference() {
        return;
    }
    eprintln!(
        "checking the reference board {} for the findings it has already",
        project.display()
    );
    facts.reference = match pcb_kicad::reference_findings(project, board_id, report) {
        Ok(tally) => KiCadReference::Findings {
            from: project.to_path_buf(),
            errors: tally.errors,
            findings: tally.findings,
            unconnected: tally.unconnected,
        },
        Err(error) => KiCadReference::Failed {
            from: project.to_path_buf(),
            error: error.lines().next().unwrap_or("").to_string(),
        },
    };
}

/// Without a reference (or with one KiCad could not check), what of the
/// result is the input's (`pcb_kicad::input_items`: the parts kept where
/// the input has them, its zones and board graphics unchanged), for the
/// verdict's `input_findings`; only when the result has errors that count.
/// The designer's boards a verdict compares the classes' layers with
/// (`note_designer_class_layers`): the reference's, then the input's.
fn designer_boards(reference: Option<&Path>, source: &Path, board_id: &str) -> Vec<PathBuf> {
    let board = format!("{board_id}.kicad_pcb");
    reference
        .map(|project| project.join(&board))
        .into_iter()
        .chain(std::iter::once(source.join(&board)))
        .collect()
}

fn note_reference_pours(
    facts: &mut KiCadRunFacts,
    reference: Option<&Path>,
    board_id: &str,
    result: &Path,
) {
    if let Some(reference) = reference {
        let board = format!("{board_id}.kicad_pcb");
        facts.designer_pours =
            pcb_kicad::compare_kicad_designer_pours(&reference.join(&board), &result.join(&board))
                .ok();
    }
}

fn check_input(facts: &mut KiCadRunFacts, source: &Path, result: &Path, board_id: &str) {
    if matches!(facts.reference, KiCadReference::Findings { .. }) || !facts.needs_reference() {
        return;
    }
    let board = format!("{board_id}.kicad_pcb");
    let read = |directory: &Path| {
        let path = directory.join(&board);
        fs::read_to_string(&path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))
    };
    match read(source)
        .and_then(|input| Ok((input, read(result)?)))
        .and_then(|(input, result)| pcb_kicad::input_items(&input, &result))
    {
        Ok(items) => facts.input = Some(items),
        Err(error) => eprintln!(
            "warning: the input's own findings are not told apart ({}): every finding counts",
            error.lines().next().unwrap_or("")
        ),
    }
}

fn note_placement_constraints(
    facts: &mut KiCadRunFacts,
    constraints: &[pcb_kicad::KiCadConstraintStatus],
    refinement_enabled: bool,
) {
    facts.decoupling_constrained = constraints
        .iter()
        .filter(|status| matches!(status.kind.as_str(), "near" | "pad_near_objective"))
        .map(|status| status.part.clone())
        .collect();
    let mut strict_misses = 0;
    let mut body_pin_misses = 0;
    let mut refinable = Vec::new();
    let mut geometry_misses = Vec::new();
    facts.constraints_missed = constraints
        .iter()
        .filter_map(|status| {
            if let Some(geometry) = &status.near_geometry {
                let source = geometry.pad.as_deref().map_or_else(
                    || format!("{}'s body", status.part),
                    |pad| format!("{}:{pad}", status.part),
                );
                let mut explanation = format!(
                    "{} to {} must be at least {:.3} mm with the target at its reported pose, these footprint bodies and sides, and {:.3} mm body spacing. The brief asks for {} mm.",
                    source, geometry.pin_of, geometry.minimum_at_target_pose_mm,
                    geometry.body_spacing_mm, geometry.max_mm,
                );
                if !geometry.faces_without_room.is_empty() {
                    explanation.push_str(&format!(
                        " The board bounds leave no room on the following target faces ({}).",
                        geometry.faces_without_room.join(", "),
                    ));
                }
                geometry_misses.push(explanation);
                if status.pad_near.as_ref().is_some_and(|pair| pair.satisfied == Some(false)) {
                    strict_misses += 1;
                }
                return Some(format!(
                    "near {} to {} ({:.6} mm, limit {} mm)",
                    source, geometry.pin_of, geometry.measured_mm, geometry.max_mm,
                ));
            }
            if let Some(near) = status.pad_near.as_ref().filter(|_| status.kind == "near")
                && let (Some(limit), Some(overage), Some(satisfied)) =
                    (near.max_mm, near.overage_mm, near.satisfied)
            {
                return (!satisfied).then(|| {
                    strict_misses += 1;
                    if near.refinable && !refinement_enabled {
                        refinable.push(format!("{}:{} to {}", status.part, near.pad, near.pin_of));
                    }
                    format!(
                        "near {}:{} to {} ({:.6} mm, limit {} mm, overage {:.6} mm)",
                        status.part, near.pad, near.pin_of, near.distance_mm, limit, overage
                    )
                });
            }
            if !status.satisfied && let Some(near) = &status.body_near_pin {
                body_pin_misses += 1;
                return Some(format!(
                    "near {} body to {} ({:.6} mm, limit {} mm)",
                    status.part, near.pin_of, near.distance_mm, near.max_mm,
                ));
            }
            if !status.satisfied
                && status.kind == "sub_board"
                && let Some(detail) = &status.detail
            {
                return Some(format!("sub_board {} ({detail})", status.part));
            }
            (!status.satisfied).then(|| match &status.other {
                Some(other) => format!(
                    "{} {} {} ({:.2} mm)",
                    status.kind, status.part, other, status.violation_mm
                ),
                None => format!(
                    "{} {} ({:.2} mm)",
                    status.kind, status.part, status.violation_mm
                ),
            })
        })
        .collect();
    facts.strict_pad_near_misses = strict_misses;
    facts.body_near_pin_misses = body_pin_misses;
    facts.refinable_pad_near_misses = refinable;
    facts.geometry_near_misses = geometry_misses;
}

/// kicad-cli before any work (`pcb_kicad::check_kicad_cli`, its version
/// asked once and cached): missing, failing or older than KiCad 10, a run
/// that needs it stops with an input error naming the requirement (exit 2;
/// `report_failure` writes the verdict when `set_target` was called). A run
/// that can go without KiCad (`optional`: KiCad's check off, a placement,
/// a netlist's import) goes on, with the problem as a warning for its
/// verdict's next actions.
fn check_kicad(optional: bool) -> Result<Option<String>, Failure> {
    match pcb_kicad::check_kicad_cli() {
        Ok(_) => Ok(None),
        Err(error) if optional => {
            let warning = format!(
                "{error}; this run goes on without KiCad, but KiCad's check of the board and the steps after it need it"
            );
            // On stdout: stderr keeps its one line for a failure.
            println!("warning: {warning}");
            Ok(Some(warning))
        }
        Err(error) => Err(Failure::Usage(error)),
    }
}

/// The verdict's warnings for the widths and clearances the router file
/// asked for below the project's (`allow_below_project_minimum`).
fn override_warnings(config: &pcb_kicad::KiCadBoardRouterConfig) -> Vec<String> {
    config
        .project_rules
        .iter()
        .flat_map(|report| &report.overrides)
        .map(|item| {
            format!(
                "rules below the project's minimum (allow_below_project_minimum): {}; KiCad's DRC holds the board to the project's rules, so expect its findings there",
                item.line()
            )
        })
        .collect()
}

const ROUTE_USAGE: &str = "usage: pcb-maker route-kicad-board <project-dir> <board-id> <out-dir> [router-config.json|auto] [--reference <project-dir>|none] [--reuse-routes <previous.kicad_pcb>] [--compare-incumbent <project-dir>] [--strip]";
const LAYOUT_USAGE: &str = "usage: pcb-maker layout-kicad-board <project-dir> <board-id> <out-dir> [router-config.json|auto] [layout-config.json] [--reference <project-dir>|none] [--reuse-routes <previous.kicad_pcb>] [--placement-seed N] [--check-sub-boards]";
const PLACE_USAGE: &str = "usage: pcb-maker place-kicad-board <project-dir> <board-id> <out-dir> [layout-config.json|placer-config.json] [--reference <project-dir>] [--router <router-config.json|auto>] [--measure-work-rate]";

fn note_second_ring_retry(
    verdict: &mut KiCadVerdict,
    project: &Path,
    board_id: &str,
    config: &pcb_kicad::KiCadBoardRouterConfig,
) {
    if !verdict
        .kicad_unconnected_items
        .is_some_and(|count| count > 0)
    {
        return;
    }
    let pads = pcb_kicad::bga_second_ring_open_pads(project, board_id, config);
    if pads.is_empty() {
        return;
    }
    let sample = pads
        .iter()
        .take(8)
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    verdict.next_actions.push(format!(
        "{} native open BGA pads match the second-ring escape rule, including {sample}. Try `\"bga_fanout_second_ring\": true` in the router JSON with the same placement and compare KiCad's opens and findings.",
        pads.len()
    ));
    verdict.bga_second_ring_open_pads = pads;
}

fn route(arguments: Vec<String>) -> Result<KiCadRunStatus, Failure> {
    let command = "route-kicad-board";
    // --strip: the board's own tracks and vias go first (default: kept).
    let strip = arguments.iter().any(|argument| argument == "--strip");
    let arguments = arguments
        .into_iter()
        .filter(|argument| argument != "--strip")
        .collect();
    let (positional, values) = split_flags(
        arguments,
        &["--reference", "--reuse-routes", "--compare-incumbent"],
        ROUTE_USAGE,
    )?;
    positional_count(&positional, 3..=4, ROUTE_USAGE)?;
    let (source, board_id, output) = (&positional[0], &positional[1], &positional[2]);
    check_project(source, board_id)?;
    let reference = reference_option(&values, Some(source), board_id)?;
    let mut config =
        crate::board_router_config(positional.get(3).map(String::as_str), source, board_id)
            .map_err(Failure::Usage)?;
    let comparison = values.get("--compare-incumbent").map(|incumbent| {
        check_project(incumbent, board_id)?;
        if config.skip_native_verification {
            return Err(Failure::Usage("--compare-incumbent requires native verification".into()));
        }
        config.ladder_budget_seconds.filter(|seconds| seconds.is_finite() && *seconds > 0.0)
            .ok_or_else(|| Failure::Usage("--compare-incumbent requires an explicit positive ladder_budget_seconds in the router JSON".into()))?;
        let incumbent = prospective_output_path(Path::new(incumbent))?;
        let target = prospective_output_path(Path::new(output))?;
        if target.starts_with(&incumbent) || target.starts_with(prospective_output_path(Path::new(source))?) {
            return Err(Failure::Usage("use a fresh retry output directory outside both input projects".into()));
        }
        let baseline = pcb_kicad::FixedRouteIncumbent::read(Path::new(source), &incumbent, board_id)
            .map_err(Failure::Usage)?;
        Ok(baseline)
    }).transpose()?;
    check_fresh_output(output)?;
    let reuse = values.get("--reuse-routes");
    // The reference board's vias in pads admit vias in those footprints'
    // enclosed pads (the board's own did already).
    if let Some(project) = reference.as_deref()
        && !same_directory(project, Path::new(source))
    {
        pcb_kicad::admit_reference_vias_in_pads(&mut config, project, board_id);
        pcb_kicad::admit_reference_via_spans(&mut config, project, board_id);
    }
    if strip {
        config.existing_tracks = pcb_kicad::KiCadExistingTracks::Strip;
    }
    if reuse.is_some() && config.existing_tracks == pcb_kicad::KiCadExistingTracks::Strip {
        return Err(Failure::Usage(
            "--reuse-routes needs the keep existing_tracks policy".into(),
        ));
    }
    // A layer-limited skew or length rule's nets prefer the side of its
    // layers the designer's board (the reference's, else the input's own
    // tracks, `--strip` or not) routes them on (`length_rule_layers`).
    pcb_kicad::admit_length_rule_layers(
        &mut config,
        &Path::new(source).join(format!("{board_id}.kicad_pcb")),
        &designer_boards(reference.as_deref(), Path::new(source), board_id),
    );
    // Route mode: differential pairs couple where the project's rules ask
    // for them, unless the settings say (`diff_pairs_choice`).
    config.diff_pairs_by_need = true;
    config.native_candidate_ranker = Some(pcb_kicad::route_candidate_ranker(
        Path::new(source),
        board_id,
        reference.as_deref(),
    ));
    let output = Path::new(output);
    let mut outputs = BTreeMap::from([
        ("project".to_string(), path_text(output)),
        (
            "board".to_string(),
            path_text(&output.join(format!("{board_id}.kicad_pcb"))),
        ),
        ("drc".to_string(), path_text(&output.join("drc.json"))),
        (
            "report".to_string(),
            path_text(&output.join("board-router.json")),
        ),
        (
            "verdict".to_string(),
            path_text(&output.join("verdict.json")),
        ),
    ]);
    if comparison.is_some() {
        outputs.insert(
            "fixed_route_comparison".into(),
            path_text(&output.join("fixed-route-comparison")),
        );
    }
    let report = output.join("board-router.json");
    set_target(command, report.clone(), outputs.clone());
    let kicad_warning = check_kicad(config.skip_native_verification)?;
    let pour_warnings = check_pours(source, board_id, &config, true)?;
    calibrate_work_rate(Some(&config))?;
    set_target(command, report.clone(), outputs.clone());
    // Test hook: a panic once the run has started (tests/cli_exit_codes.rs).
    if std::env::var_os("PCB_MAKER_TEST_PANIC").is_some() {
        panic!("PCB_MAKER_TEST_PANIC is set");
    }
    let started = Instant::now();
    let wall_deadline = config
        .deadline_seconds
        .and_then(|seconds| {
            std::time::Duration::try_from_secs_f64(
                seconds * pcb_kicad::work_rate().wall_stretch(false),
            )
            .ok()
        })
        .and_then(|duration| started.checked_add(duration));
    let result = match reuse {
        Some(previous) => pcb_kicad::route_kicad_board_reusing(
            Path::new(source),
            board_id,
            output,
            Path::new(previous),
            &config,
        ),
        None => pcb_kicad::route_kicad_board(Path::new(source), board_id, output, &config),
    }
    .map_err(|error| {
        if error
            == format!(
                "output directory {} already exists; give a new one",
                output.display()
            )
        {
            clear_target();
        }
        classify(error)
    })?;
    let pending = if result
        .route_reuse
        .as_ref()
        .is_some_and(|reuse| reuse.status == "accepted")
    {
        " pending"
    } else {
        ""
    };
    let copper = saved_copper_line(&result);
    println!(
        "routed {}/{}{pending} connections ({} unconnected terminals), pours {}, {copper}, {} iterations; lowering {:.2}s routing {:.2}s internal check {:.2}s ({} violations) native {:.2}s complete={}",
        result.routed_connections,
        result.routable_connections,
        result.unconnected_terminals,
        result.pours,
        result.iterations,
        result.lowering_seconds,
        result.routing_seconds,
        result.internal_verification_seconds,
        result.internal_violations.len(),
        result.native_verification_seconds,
        result
            .native
            .as_ref()
            .map(|native| native.complete)
            .unwrap_or(false),
    );
    if let Some(kept) = &result.kept_copper {
        println!(
            "kept: the board's {} track segments, {} arcs and {} vias on {} nets, {} connections they made, {} nets complete with them alone",
            kept.segments,
            kept.arcs,
            kept.vias,
            kept.nets.len(),
            kept.connections_made,
            kept.nets_complete.len()
        );
    }
    // Test hook: KiCad's report gone before the verdict reads it
    // (tests/cli_exit_codes.rs).
    if std::env::var_os("PCB_MAKER_TEST_DROP_DRC").is_some() {
        let _ = fs::remove_file(output.join("drc.json"));
    }
    let mut facts = pcb_kicad::route_facts(command, &result, output).map_err(Failure::Internal)?;
    facts.source_project = Some(PathBuf::from(source));
    facts.work_seconds = Some(result.work_seconds);
    if let Some(accounting) = &result.wall_accounting {
        let charged = &accounting.charged;
        facts.pour_work_seconds = Some(charged.pour_seconds);
        facts.lattice_work_seconds = Some(charged.lattice_seconds + charged.lowering_seconds);
    }
    facts.outputs = existing(&outputs);
    facts.input_warnings.extend(kicad_warning);
    facts.input_warnings.extend(pour_warnings);
    facts.input_warnings.extend(override_warnings(&config));
    pcb_kicad::note_designer_class_layers(
        &mut facts,
        &result,
        &designer_boards(reference.as_deref(), Path::new(source), board_id),
    );
    pcb_kicad::note_designer_widths(
        &mut facts,
        &result,
        &designer_boards(reference.as_deref(), Path::new(source), board_id),
        &output.join(format!("{board_id}.kicad_pcb")),
    );
    note_reference_pours(
        &mut facts,
        values.get("--reference").and(reference.as_deref()),
        board_id,
        output,
    );
    check_reference(
        &mut facts,
        reference.as_deref(),
        board_id,
        &output.join("reference-drc.json"),
    );
    check_input(&mut facts, Path::new(source), output, board_id);
    facts.wall_seconds = started.elapsed().as_secs_f64();
    let mut verdict = pcb_kicad::judge(&facts);
    let mut work_remaining = (config.ladder_budget() - result.work_seconds).max(0.0);
    let compare_reserve = comparison
        .as_ref()
        .map_or(0.0, |incumbent| incumbent.required_verification_work());
    if verdict.status == KiCadRunStatus::Complete
        && !result
            .input_connectivity
            .as_ref()
            .is_some_and(|input| input.status == "unchanged")
        && let Some((fill, work)) = pcb_kicad::publish_routed_fill(
            output,
            board_id,
            (work_remaining - compare_reserve).max(0.0),
            wall_deadline,
        )
    {
        work_remaining -= work;
        facts.work_seconds = Some(result.work_seconds + work);
        if fill.status == "saved" {
            facts.outputs.insert(
                "saved_fill_drc".into(),
                path_text(&output.join("drc-saved-fill.json")),
            );
        }
        facts.fill_publication = Some(fill);
    }
    let comparison =
        comparison.map(|baseline| baseline.compare(output, work_remaining, wall_deadline));
    if let Some(comparison) = &comparison {
        facts.work_seconds =
            Some(facts.work_seconds.unwrap_or_default() + comparison.verification_work_seconds);
        facts.outputs.extend(existing(&outputs));
    }
    if comparison.is_some() || facts.fill_publication.is_some() {
        facts.wall_seconds = started.elapsed().as_secs_f64();
        verdict = pcb_kicad::judge(&facts);
    }
    if let Some(fill) = &facts.fill_publication
        && fill.status == "skipped"
    {
        let reason = fill
            .reason
            .as_deref()
            .unwrap_or("native fill publication was deferred");
        let mut action = format!(
            "The route passed native verification after refill. The saved fill is unverified. {reason}."
        );
        if !fill.retry_argv.is_empty() {
            let command = fill
                .retry_argv
                .iter()
                .map(|arg| shell_argument(arg))
                .collect::<Vec<_>>()
                .join(" ");
            action.push_str(&format!(
                " To save and verify a filled copy, run {command}."
            ));
        }
        verdict.next_actions.insert(0, action);
    }
    note_second_ring_retry(&mut verdict, output, board_id, &config);
    verdict.note_via_cost_retry(
        &facts,
        &result,
        &output.join(format!("{board_id}.kicad_pcb")),
        &config,
        positional.get(3).map(String::as_str),
    );
    if let Some(comparison) = comparison {
        verdict.next_actions.push(format!(
            "See fixed_route_comparison in verdict.json for the result of this retry. {}.",
            comparison.reason.trim_end_matches('.')
        ));
        verdict.fixed_route_comparison = Some(comparison);
    }
    write_verdict(output, Some(&report), &verdict).map_err(Failure::Internal)?;
    announce(&verdict, &output.join("verdict.json"));
    Ok(verdict.status)
}

fn saved_copper_line(result: &pcb_kicad::KiCadBoardRouterResult) -> String {
    result.saved_copper.map_or_else(
        || "saved copper unmeasured".to_string(),
        |copper| {
            format!(
                "{} vias, {:.1} mm of saved track",
                copper.vias, copper.track_length_mm
            )
        },
    )
}

fn print_layout(
    result: &pcb_kicad::KiCadBoardLayoutResult,
    missed: &[String],
) -> Result<(), String> {
    if let Some(size) = result
        .outline_sizing
        .iter()
        .find(|trial| trial.used)
        .and_then(|trial| trial.size_mm)
    {
        println!(
            "outline: {:.1} x {:.1} mm, sized from the parts ({} tried)",
            size[0],
            size[1],
            result.outline_sizing.len()
        );
    }
    println!(
        "placed in {:.1}s; first route {:.1}s: {} open, {} vias, {:.0} mm",
        result.placement_seconds,
        result.first_route_seconds,
        result.first_unconnected_terminals,
        result.first_vias,
        result.first_length_mm,
    );
    for step in &result.moves {
        println!(
            "move {} {:?} -> {:?}: {} nets rerouted in {:.1}s, {} open, {} vias, {:.0} mm{}",
            step.reference,
            step.from,
            step.to,
            step.rerouted_nets,
            step.seconds,
            step.unconnected_terminals,
            step.vias,
            step.length_mm,
            if step.kept { " (kept)" } else { "" },
        );
    }
    let saved_copper = saved_copper_line(&result.routed);
    let coverage = if let Some(copper) = result.routed.kept_copper.as_ref() {
        format!(
            "routed {}/{} pending connections, kept copper on {} nets ({} already complete), {saved_copper}",
            result.routed.routed_connections,
            result.routed.routable_connections,
            copper.nets.len(),
            copper.nets_complete.len(),
        )
    } else {
        format!(
            "routed {}/{} connections, {saved_copper}",
            result.routed.routed_connections, result.routed.routable_connections,
        )
    };
    println!(
        "final: {coverage}, {}",
        // KiCad's raw counts: every DRC finding of any severity, cosmetic
        // ones included; the verdict below says which count.
        match result.routed.native.as_ref() {
            Some(native) => format!(
                "KiCad: {} unconnected, {} DRC findings of any severity (the verdict judges them)",
                native.selected_net_unconnected_items, native.drc_design_violations
            ),
            None => "KiCad did not check".to_string(),
        },
    );
    println!(
        "constraints: {} of {} kept{}",
        result.constraints.len() - missed.len(),
        result.constraints.len(),
        if missed.is_empty() {
            String::new()
        } else {
            format!("; missed: {}", missed.join(", "))
        }
    );
    println!(
        "silkscreen: {} labels moved{}",
        result.labels.moved,
        if result.labels.stuck.is_empty() {
            String::new()
        } else {
            format!(", no room for {:?}", result.labels.stuck)
        }
    );
    if let Some(quality) = &result.quality {
        let serde_json::Value::Object(report) =
            serde_json::to_value(quality).map_err(|error| error.to_string())?
        else {
            unreachable!();
        };
        let decoupling = &report["decoupling"];
        if decoupling["capacitors"].as_u64().unwrap_or(0) > 0 {
            println!(
                "decoupling: capacitors a median {} mm from a supply pin, {:.0} % of them within 3 mm",
                decoupling["capacitor_median_mm"],
                decoupling["capacitors_within_3mm"].as_f64().unwrap_or(0.0) * 100.0
            );
        }
        for (key, label) in [
            ("crystals", "crystal"),
            ("switch_inductors", "switch inductor"),
            ("esd", "ESD protector"),
        ] {
            for entry in report[key].as_array().into_iter().flatten() {
                let (name, distance) = match entry {
                    serde_json::Value::Array(pair) => (pair[0].clone(), pair[1].clone()),
                    other => (other["reference"].clone(), other["longest_mm"].clone()),
                };
                match distance.as_f64() {
                    Some(distance) => println!(
                        "{label} {}: {distance} mm from its pins",
                        name.as_str().unwrap_or("?")
                    ),
                    // Nothing to measure from (no IC or connector pin on
                    // its nets): trial 5's "null mm".
                    None => println!(
                        "{label} {}: no pin of its nets to measure from",
                        name.as_str().unwrap_or("?")
                    ),
                }
            }
        }
        for hot in report["hot_parts"].as_array().into_iter().flatten() {
            let planes: Vec<&str> = hot["planes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|layer| layer.as_str())
                .collect();
            println!(
                "hot part {}: {} mm² pad on {}, {}, {} vias in the pad, {} mm² of tracks near it",
                hot["reference"].as_str().unwrap_or("?"),
                hot["pad_mm2"],
                hot["net"].as_str().unwrap_or("?"),
                if planes.is_empty() {
                    "no plane".to_string()
                } else {
                    format!("planes on {}", planes.join(", "))
                },
                hot["vias_in_pad"],
                hot["track_mm2"]
            );
        }
        let cost = &report["cost"];
        let totals: Vec<String> = cost["fabs"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|fab| {
                let currency = fab["currency"].as_str().unwrap_or("");
                let assembly = fab["assembly"].as_f64();
                let board = fab["total"].as_f64().unwrap_or(0.0) - assembly.unwrap_or(0.0);
                let assembly = assembly.map_or(String::new(), |assembly| {
                    format!(" (+ {assembly} {currency} assembly)")
                });
                format!(
                    "{} {:.2} {currency}{assembly}",
                    fab["fab"].as_str().unwrap_or(""),
                    board
                )
            })
            .collect();
        println!(
            "cost of {} boards (estimate, parts excluded): {}",
            cost["quantity"],
            totals.join(", ")
        );
        for hint in cost["hints"].as_array().into_iter().flatten() {
            println!("cost hint: {}", hint.as_str().unwrap_or(""));
        }
    }
    Ok(())
}

/// Reads a constraints file the placer config names, and returns what the
/// input check warns of (printed now, before the run): keepouts over parts
/// the placer keeps where they are, and such parts on top of each other
/// (`pcb_kicad::placement_input_warnings`).
fn check_constraints(
    source: &str,
    board_id: &str,
    placer: &pcb_kicad::KiCadBoardPlacerConfig,
    router: &pcb_kicad::KiCadBoardRouterConfig,
) -> Result<Vec<String>, Failure> {
    if let Some(pcb_kicad::KiCadConstraintsSource::File(path)) = &placer.constraints {
        pcb_kicad::read_placement_constraints(&Path::new(source).join(path))
            .map_err(Failure::Usage)?;
    }
    let with_pair_selection;
    let placer = if router.diff_pair_exclusions.is_empty() {
        placer
    } else {
        let board_settings = router
            .with_board_settings(Path::new(source), board_id)
            .map_err(classify)?;
        with_pair_selection = pcb_kicad::KiCadBoardPlacerConfig {
            router_rules: Some(std::sync::Arc::new(board_settings.into_owned())),
            ..placer.clone()
        };
        &with_pair_selection
    };
    let warnings = pcb_kicad::placement_input_warnings(Path::new(source), board_id, placer)
        .map_err(Failure::Usage)?;
    for warning in &warnings {
        eprintln!("warning: {warning}");
    }
    Ok(warnings)
}

/// The input check of the router file's `keep_whole` regions
/// (`pcb_kicad::keep_whole_input_warnings`): the other nets' pads they
/// would cut off, printed now, before the run. `placed`: route mode (the
/// board's positions are final).
fn check_pours(
    source: &str,
    board_id: &str,
    config: &pcb_kicad::KiCadBoardRouterConfig,
    placed: bool,
) -> Result<Vec<String>, Failure> {
    let warnings =
        pcb_kicad::keep_whole_input_warnings(Path::new(source), board_id, config, placed)
            .map_err(Failure::Usage)?;
    for warning in &warnings {
        eprintln!("warning: {warning}");
    }
    Ok(warnings)
}

/// The placement's facts from its report (`board-placer.json`).
fn placement_facts(
    command: &str,
    report: &serde_json::Value,
    refinement_enabled: bool,
) -> KiCadRunFacts {
    let strings = |key: &str| -> Vec<String> {
        report[key]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|value| value.as_str().map(str::to_string))
            .collect()
    };
    let mut facts = KiCadRunFacts::new(command);
    facts.routes = false;
    facts.bus_groups = serde_json::from_value(report["bus_groups"].clone()).ok();
    facts.panel_copies = serde_json::from_value(report["panel_copies"].clone()).ok();
    facts.sub_boards = serde_json::from_value(report["sub_boards"].clone()).ok();
    // The movable parts without a legal place, each once, a repeated
    // reference with its uuid (`unplaced_parts`); without them (an older
    // report) by name, with the illegal parts' names not yet there (a
    // fixed part or a board mark a part was left on among them).
    let parts: Vec<String> = report["unplaced_parts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|part| part["label"].as_str().map(str::to_string))
        .collect();
    if !parts.is_empty() {
        facts.unplaced = parts;
    } else {
        facts.unplaced = strings("unplaced");
        for part in strings("illegal") {
            if !facts.unplaced.contains(&part) {
                facts.unplaced.push(part);
            }
        }
    }
    facts.placement_hints = strings("hints");
    facts.rear_placement_recovery =
        serde_json::from_value(report["rear_placement_recovery"].clone()).ok();
    facts.automatic_rear_retry =
        serde_json::from_value(report["automatic_rear_retry"].clone()).ok();
    facts.outline_from_input = report["outline_from_input"].as_bool().unwrap_or(false);
    facts.placement_actions = report["unplaced_parts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|part| {
            part["local_diagnosis"]["next_action"]
                .as_str()
                .map(str::to_string)
        })
        .collect();
    facts.closed_pads =
        serde_json::from_value(report["pad_exits"]["closed"].clone()).unwrap_or_default();
    let constraints: Vec<pcb_kicad::KiCadConstraintStatus> =
        serde_json::from_value(report["constraints"].clone()).unwrap_or_default();
    note_placement_constraints(&mut facts, &constraints, refinement_enabled);
    facts.work_seconds = report["work_seconds"].as_f64();
    facts.cut = pcb_kicad::wall_cuts();
    facts.room = serde_json::from_value(report["room"].clone()).ok();
    facts.skeleton_race = serde_json::from_value(report["skeleton_race"].clone()).ok();
    facts
}

fn note_unplaced_antenna_edges(
    verdict: &mut KiCadVerdict,
    source: &Path,
    board_id: &str,
    placed: &serde_json::Value,
) {
    if verdict
        .sub_boards
        .as_ref()
        .is_some_and(|panel| panel.boards_mixed())
    {
        return;
    }
    let constraints = placed["constraints"].as_array().into_iter().flatten();
    let constrained: std::collections::BTreeSet<_> = constraints
        .filter(|entry| {
            matches!(
                entry["kind"].as_str(),
                Some(
                    "edge"
                        | "overhang"
                        | "place"
                        | "region"
                        | "rotation"
                        | "relative"
                        | "row"
                        | "pair_row"
                )
            )
        })
        .flat_map(|entry| [entry["part"].as_str(), entry["other"].as_str()])
        .flatten()
        .collect();
    let selected: BTreeMap<_, _> = placed["unplaced_parts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|part| {
            let reference = part["reference"].as_str()?;
            let uuid = part["uuid"].as_str()?;
            (part["status"] == "unplaced"
                && part["label"] == reference
                && part["held_by"].as_array().is_none_or(Vec::is_empty)
                && !constrained.contains(reference))
            .then_some((reference, uuid))
        })
        .collect();
    if selected.is_empty() {
        return;
    }
    let board = source.join(format!("{board_id}.kicad_pcb"));
    let Ok(entries) =
        pcb_kicad_import::constraints_template::unplaced_antenna_edges(&board, &selected)
    else {
        return;
    };
    verdict.next_actions.splice(
        ..0,
        entries.into_iter().take(3).map(|(reference, edge)| {
            format!(
                "{reference} has no legal placement and its RF footprint has an antenna keepout. If the enclosure permits the antenna to extend beyond the PCB, append {edge} to edge in the placement constraints and retry."
            )
        }),
    );
}

/// The input check's warnings followed by the placement's
/// `constraint_warnings` (board-placer.json) it did not already give: both
/// lead the verdict's next actions.
fn constraint_warnings(mut warnings: Vec<String>, placed: &serde_json::Value) -> Vec<String> {
    for warning in placed["constraint_warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
    {
        // The placer's own "fixed parts C1 and C2 overlap" is the input
        // check's warning without its detail when that one is there.
        if !warnings
            .iter()
            .any(|known| known == warning || known.starts_with(&format!("{warning} (")))
        {
            warnings.push(warning.to_string());
        }
    }
    warnings
}

/// Use a separate range of seeds for the retry. Leave room after the usual
/// first seed pool, or start after the seeds this layout actually ran.
fn alternate_placement_seed(seed: u64, seeds_run: u64) -> u64 {
    let next = seed.wrapping_add(seeds_run.max(1));
    if next > seed { next.max(101) } else { next }
}

fn shell_argument(argument: &str) -> String {
    format!("'{}'", argument.replace('\'', "'\\''"))
}

/// A copyable shell command using the same source and settings and a new
/// output directory. The positional paths may be relative in the original.
fn placement_retry_action(
    positional: &[String],
    flags: &BTreeMap<String, String>,
    output: &Path,
    seed: u64,
) -> Option<String> {
    fn quote_path(path: &Path) -> String {
        shell_argument(&path_text(path))
    }

    let mut name = output.file_name()?.to_os_string();
    name.push(format!("-placement-{seed}"));
    let retry = output.with_file_name(name);
    if !retry
        .symlink_metadata()
        .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
    {
        return None;
    }
    let mut command = format!(
        "pcb-maker layout-kicad-board {} {} {}",
        quote_path(Path::new(&positional[0])),
        shell_argument(&positional[1]),
        quote_path(&retry)
    );
    if let Some(router) = positional.get(3) {
        let router = if router == "auto" {
            router.clone()
        } else {
            quote_path(Path::new(router))
        };
        command.push_str(&format!(" {router}"));
    }
    if let Some(layout) = positional.get(4) {
        command.push_str(&format!(" {}", quote_path(Path::new(layout))));
    }
    if let Some(reference) = flags.get("--reference") {
        let reference = if reference == "none" {
            reference.to_owned()
        } else {
            quote_path(Path::new(reference))
        };
        command.push_str(&format!(" --reference {reference}"));
    }
    if let Some(reuse) = flags.get("--reuse-routes") {
        command.push_str(&format!(" --reuse-routes {}", quote_path(Path::new(reuse))));
    }
    for flag in ["--plane-escape-budget", "--terminal-launch-budget"] {
        if let Some(seconds) = flags.get(flag) {
            command.push_str(&format!(" {flag} {}", shell_argument(seconds)));
        }
    }
    Some(format!(
        "try an alternate placement with the same constraints and routing settings in a new output directory: {command} --placement-seed {seed}"
    ))
}

fn layout(arguments: Vec<String>) -> Result<KiCadRunStatus, Failure> {
    let command = "layout-kicad-board";
    let check_sub_boards = arguments
        .iter()
        .any(|argument| argument == "--check-sub-boards");
    let arguments = arguments
        .into_iter()
        .filter(|argument| argument != "--check-sub-boards")
        .collect();
    let (positional, values) = split_flags(
        arguments,
        &[
            "--reference",
            "--reuse-routes",
            "--placement-seed",
            "--plane-escape-budget",
            "--terminal-launch-budget",
        ],
        LAYOUT_USAGE,
    )?;
    positional_count(&positional, 3..=5, LAYOUT_USAGE)?;
    let placement_seed = values
        .get("--placement-seed")
        .map(|seed| {
            seed.parse::<u64>().map_err(|_| {
                Failure::Usage(format!(
                    "--placement-seed needs a non-negative 64-bit integer, got {seed}"
                ))
            })
        })
        .transpose()?;
    let (source, board_id, output) = (&positional[0], &positional[1], &positional[2]);
    check_project(source, board_id)?;
    if !check_sub_boards {
        check_fresh_output(output)?;
    }
    let reference = reference_option(&values, None, board_id)?;
    let mut router =
        crate::board_router_config(positional.get(3).map(String::as_str), source, board_id)
            .map_err(Failure::Usage)?;
    let reuse = values.get("--reuse-routes");
    if let Some(prior) = reuse {
        if check_sub_boards {
            return Err(Failure::Usage("--reuse-routes needs a routing run".into()));
        }
        if !Path::new(prior).is_file() {
            return Err(Failure::Usage(format!(
                "reuse routes board not found: {prior}"
            )));
        }
        if router.tuning.engine.as_deref() == Some("topological") {
            return Err(Failure::Usage(
                "--reuse-routes requires the detailed router".into(),
            ));
        }
    }
    if let Some(project) = reference.as_deref()
        && !same_directory(project, Path::new(source))
    {
        pcb_kicad::admit_reference_vias_in_pads(&mut router, project, board_id);
        pcb_kicad::admit_reference_via_spans(&mut router, project, board_id);
    }
    // A layer-limited skew or length rule's nets prefer the side of its
    // layers the designer's board routes them on (`length_rule_layers`).
    pcb_kicad::admit_length_rule_layers(
        &mut router,
        &Path::new(source).join(format!("{board_id}.kicad_pcb")),
        &designer_boards(reference.as_deref(), Path::new(source), board_id),
    );
    let mut config: pcb_kicad::KiCadBoardLayoutConfig = match positional.get(4) {
        Some(path) => {
            let text = fs::read_to_string(path)
                .map_err(|error| Failure::Usage(format!("failed to read {path}: {error}")))?;
            let invalid = |error| Failure::Usage(format!("failed to parse {path}: {error}"));
            let mut config: pcb_kicad::KiCadBoardLayoutConfig =
                serde_json::from_str(&text).map_err(invalid)?;
            let input: serde_json::Value = serde_json::from_str(&text).map_err(invalid)?;
            config.body_near_inferred_spacing =
                input.pointer("/placer/tuning/spacing_mm").is_none();
            config
        }
        None => pcb_kicad::KiCadBoardLayoutConfig {
            body_near_inferred_spacing: true,
            ..Default::default()
        },
    };
    if let Some(seed) = placement_seed {
        config.placer.tuning.seed = seed;
    }
    if let Some(seconds) = values.get("--plane-escape-budget") {
        let reserve = seconds.parse::<f64>().ok()
            .filter(|seconds| seconds.is_finite() && *seconds > 0.0 && *seconds < config.total_seconds)
            .ok_or_else(|| Failure::Usage("--plane-escape-budget needs positive seconds smaller than the layout total_seconds".into()))?;
        if check_sub_boards || config.free_dimension.is_some() {
            return Err(Failure::Usage(
                "--plane-escape-budget needs one ordinary routing layout".into(),
            ));
        }
        if router.skip_native_verification {
            return Err(Failure::Usage(
                "--plane-escape-budget needs native verification".into(),
            ));
        }
        config.plane_escape_budget = Some(reserve);
        config.total_seconds -= reserve;
    }
    if let Some(seconds) = values.get("--terminal-launch-budget") {
        let reserve = seconds.parse::<f64>().ok()
            .filter(|seconds| seconds.is_finite() && *seconds > 0.0 && *seconds < config.total_seconds)
            .ok_or_else(|| Failure::Usage("--terminal-launch-budget needs positive seconds smaller than the layout total_seconds".into()))?;
        if check_sub_boards
            || config.free_dimension.is_some()
            || config.plane_escape_budget.is_some()
            || config.stop_after_first_route
            || reuse.is_some()
            || router.skip_native_verification
            || router.tuning.engine.as_deref() == Some("topological")
        {
            return Err(Failure::Usage(
                "--terminal-launch-budget needs one ordinary native-verified detailed layout without another recovery reservation".into(),
            ));
        }
        config.terminal_launch_budget = Some(reserve);
        config.total_seconds -= reserve;
    }
    if !config.terminal_launch_pads.is_empty()
        && (config.terminal_launch_budget.is_none()
            || !pcb_kicad::valid_terminal_launch_pads(&config.terminal_launch_pads))
    {
        return Err(Failure::Usage(
            "terminal_launch_pads needs --terminal-launch-budget and one or two distinct REF:PAD values".into(),
        ));
    }
    config.reuse_routes = reuse.map(PathBuf::from);
    // The reference board's places say which sub-board of a panel each
    // part belongs to (`sub_boards`).
    config.placer.sub_board_reference = reference
        .as_ref()
        .map(|project| project.join(format!("{board_id}.kicad_pcb")));
    if check_sub_boards {
        let source = Path::new(source);
        let placer = pcb_kicad::placement_check_config(source, board_id, &config, &router)
            .map_err(classify)?;
        let sub_boards =
            pcb_kicad::check_sub_board_assignments(source, board_id, &placer).map_err(classify)?;
        let action = sub_boards.as_ref().and_then(|sub| sub.assignment_action());
        let status = if action.is_some() {
            KiCadRunStatus::Incomplete
        } else {
            KiCadRunStatus::Complete
        };
        let report = serde_json::json!({
            "check": "sub_board_assignments",
            "enabled": placer.sub_boards,
            "status": status,
            "sub_boards": sub_boards,
            "next_actions": action.into_iter().collect::<Vec<_>>(),
        });
        println!("{report:#}");
        return Ok(status);
    }
    let mut input_warnings = check_constraints(source, board_id, &config.placer, &router)?;
    set_target(
        command,
        Path::new(output).join("board-layout.json"),
        BTreeMap::from([(
            "verdict".to_string(),
            path_text(&Path::new(output).join("verdict.json")),
        )]),
    );
    input_warnings.extend(check_kicad(router.skip_native_verification)?);
    input_warnings.extend(check_pours(source, board_id, &router, false)?);
    input_warnings.extend(override_warnings(&router));
    // A free dimension: the layout at several sizes (src/agent_cli/free_dimension.rs).
    if let Some(free) = &config.free_dimension {
        if reuse.is_some() {
            return Err(Failure::Usage(
                "--reuse-routes requires a single outline size".into(),
            ));
        }
        let layout_json = match positional.get(4) {
            Some(path) => read_json::<serde_json::Value>(path)?,
            None => serde_json::json!({}),
        };
        return free_dimension::layout(free_dimension::FreeDimensionLayout {
            source,
            board_id,
            output: Path::new(output),
            router_argument: positional.get(3).map(String::as_str),
            router: &router,
            layout_json,
            config: &config,
            free,
            reference: values.get("--reference").map(String::as_str),
        });
    }
    let output = Path::new(output);
    let result_directory = output.join("result");
    let outputs = BTreeMap::from([
        ("project".to_string(), path_text(&result_directory)),
        (
            "board".to_string(),
            path_text(&result_directory.join(format!("{board_id}.kicad_pcb"))),
        ),
        (
            "drc".to_string(),
            path_text(&result_directory.join("drc.json")),
        ),
        (
            "report".to_string(),
            path_text(&output.join("board-layout.json")),
        ),
        (
            "placement".to_string(),
            path_text(&output.join("placed/placement.html")),
        ),
        (
            "placement_report".to_string(),
            path_text(&output.join("placed/board-placer.json")),
        ),
        (
            "verdict".to_string(),
            path_text(&output.join("verdict.json")),
        ),
    ]);
    calibrate_work_rate(Some(&router))?;
    let report = output.join("board-layout.json");
    set_target(command, report.clone(), outputs.clone());
    let started = Instant::now();
    let result = match pcb_kicad::layout_kicad_board(
        Path::new(source),
        board_id,
        output,
        &config,
        &router,
    ) {
        Ok(result) => result,
        Err(error) => {
            // Parts without a legal place end the layout before it routes:
            // an incomplete result, which the placement's report explains.
            let placed = fs::read_to_string(output.join("placed/board-placer.json"))
                .ok()
                .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok());
            let Some(placed) = placed.filter(|placed| {
                ["unplaced", "illegal"].iter().any(|key| {
                    placed[*key]
                        .as_array()
                        .is_some_and(|parts| !parts.is_empty())
                })
            }) else {
                return Err(classify(error));
            };
            let mut facts = placement_facts(command, &placed, config.placer.tuning.refine_pad_near);
            if config.reuse_routes.is_some() {
                facts.route_reuse = Some(pcb_kicad::KiCadRouteReuseOutcome::placement_blocked());
            }
            facts.input_warnings = constraint_warnings(input_warnings, &placed);
            facts.outputs = existing(&outputs);
            facts.wall_seconds = started.elapsed().as_secs_f64();
            let mut verdict = pcb_kicad::judge(&facts);
            note_unplaced_antenna_edges(&mut verdict, Path::new(source), board_id, &placed);
            verdict.error = Some(error);
            // No board-layout.json: there is no layout (scripts take its
            // absence for a layout that did not finish).
            write_verdict(output, None, &verdict).map_err(Failure::Internal)?;
            announce(&verdict, &output.join("verdict.json"));
            return Ok(verdict.status);
        }
    };
    let mut facts = pcb_kicad::route_facts(command, &result.routed, &result_directory)
        .map_err(Failure::Internal)?;
    facts.source_project = Some(PathBuf::from(source));
    facts.fill_publication = result.fill_publication.clone();
    facts.plane_escape = result.plane_escape.clone();
    facts.terminal_launch_retry = result.terminal_launch_retry.clone();
    facts.terminal_launch_advice = result.terminal_launch_advice.clone();
    facts.prepared_row_escape_advice = result.prepared_row_escape_advice.clone();
    facts.local_open_repair = result.local_open_repair.clone();
    if let Some((repair, zone)) = facts
        .local_open_repair
        .as_mut()
        .zip(facts.zone_island_recovery.as_ref())
    {
        repair.retry_argv.clone_from(&zone.retry_argv);
    }
    if let Some(reuse) = &mut facts.route_reuse {
        reuse.note_layout_placement(
            result
                .route_reuse_placement
                .as_ref()
                .map_or(0, |placement| placement.moved_to_previous),
        );
    }
    note_placement_constraints(
        &mut facts,
        &result.constraints,
        config.placer.tuning.refine_pad_near,
    );
    print_layout(&result, &facts.constraints_missed).map_err(Failure::Internal)?;
    facts.board_mark_overlaps = result.labels.overlaps.clone();
    // Nothing ran after the first route: no move, no final ladder, nothing
    // stopped or cut it (trial 5's SmartSpin2k: the verdict said neither).
    facts.first_route_only = result.moves.is_empty()
        && result.final_ladder_budget_seconds.is_none()
        && !result.stopped_after_first_route
        && result.cut.is_empty();
    if result.first_unconnected_terminals > 0 && facts.first_route_only {
        facts.first_route_spent = Some([result.first_route_work_seconds, result.total_seconds]);
    }
    facts.silkscreen = Some(result.labels.silkscreen.clone());
    facts.race = result.race.clone();
    facts.decoupling = result
        .quality
        .as_ref()
        .map(|quality| quality.decoupling.clone());
    if facts
        .decoupling
        .as_ref()
        .is_some_and(pcb_kicad::decoupling_needs_attention)
    {
        facts.decoupling_targets =
            pcb_kicad_import::constraints_template::schematic_decoupling_targets(
                &result_directory,
                board_id,
            )
            .unwrap_or_default();
    }
    // The placement's room (the parts and the outline the layout kept)
    // and what it warned of.
    let micro_moved = result
        .body_near_grid_retry
        .as_ref()
        .is_some_and(|retry| retry.status == "accepted");
    let placement_directory = if micro_moved { "result" } else { "placed" };
    let placed = fs::read_to_string(output.join(placement_directory).join("board-placer.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .unwrap_or_default();
    facts.room = serde_json::from_value(placed["room"].clone()).ok();
    facts.outline_from_input = placed["outline_from_input"].as_bool().unwrap_or(false);
    facts.rear_routing_candidates =
        serde_json::from_value(placed["rear_routing_candidates"].clone()).ok();
    facts.automatic_rear_retry =
        serde_json::from_value(placed["automatic_rear_retry"].clone()).ok();
    facts.skeleton_race = serde_json::from_value(placed["skeleton_race"].clone()).ok();
    // The routed placement's (the race may keep another than the
    // placer's first-ranked).
    facts.closed_pads = result
        .pad_exits
        .as_ref()
        .map(|exits| exits.closed.clone())
        .unwrap_or_default();
    facts.bus_groups = serde_json::from_value(placed["bus_groups"].clone()).ok();
    facts.panel_copies = serde_json::from_value(placed["panel_copies"].clone()).ok();
    facts.sub_boards = serde_json::from_value(placed["sub_boards"].clone()).ok();
    facts.fixed_parts = placed["footprints"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|footprint| footprint["fixed"].as_bool() == Some(true))
        .filter_map(|footprint| footprint["reference"].as_str().map(str::to_string))
        .collect();
    facts.note_authored_layout_constraints(Path::new(source), &config.placer);
    facts.input_warnings = constraint_warnings(input_warnings, &placed);
    facts.cut = result.cut.clone();
    facts.work_seconds = Some(result.work_seconds + result.routed.work_seconds);
    facts.kicad_check_work_seconds = Some(result.kicad_check_work_seconds);
    facts.outputs = existing(&outputs);
    if micro_moved {
        facts.outputs.extend([
            (
                "placement".into(),
                path_text(&result_directory.join("placement.html")),
            ),
            (
                "placement_report".into(),
                path_text(&result_directory.join("board-placer.json")),
            ),
            (
                "pre_grid_placement_report".into(),
                path_text(&output.join("placed/board-placer.json")),
            ),
        ]);
    }
    if let Some(path) = result
        .local_open_repair
        .as_ref()
        .and_then(|repair| repair.report)
    {
        facts
            .outputs
            .insert("local_open_repair".into(), path_text(&output.join(path)));
    }
    if let Some(path) = result
        .generated_pad_via_recovery
        .as_ref()
        .and_then(|recovery| recovery.report)
    {
        facts.outputs.insert(
            "generated_pad_via_recovery".into(),
            path_text(&output.join(path)),
        );
    }
    if let Some(path) = result
        .unchanged_hatch_width_repair
        .as_ref()
        .and_then(|recovery| recovery.report.as_ref())
    {
        facts.outputs.insert(
            "unchanged_hatch_width_repair".into(),
            path_text(&output.join(path)),
        );
    }
    if let Some(escape) = &result.plane_escape {
        let directory = if escape.status == "accepted" {
            result_directory.clone()
        } else {
            output.join("plane-escape")
        };
        let report = directory.join("plane-escape.json");
        if report.is_file() {
            facts
                .outputs
                .insert("plane_escape".into(), path_text(&report));
        }
    }
    let terminal_launch_report = output.join("terminal-launch/terminal-launch.json");
    if result.terminal_launch_retry.is_some() && terminal_launch_report.is_file() {
        facts.outputs.insert(
            "terminal_launch_retry".into(),
            path_text(&terminal_launch_report),
        );
    }
    if result
        .fill_publication
        .as_ref()
        .is_some_and(|fill| fill.status == "saved")
    {
        facts.outputs.insert(
            "saved_fill_drc".into(),
            path_text(&result_directory.join("drc-saved-fill.json")),
        );
    }
    pcb_kicad::note_designer_class_layers(
        &mut facts,
        &result.routed,
        &designer_boards(reference.as_deref(), Path::new(source), board_id),
    );
    pcb_kicad::note_designer_widths(
        &mut facts,
        &result.routed,
        &designer_boards(reference.as_deref(), Path::new(source), board_id),
        &result_directory.join(format!("{board_id}.kicad_pcb")),
    );
    note_reference_pours(
        &mut facts,
        values.get("--reference").and(reference.as_deref()),
        board_id,
        &result_directory,
    );
    if let Some(reference) = values.get("--reference").and(reference.as_deref()) {
        let board = format!("{board_id}.kicad_pcb");
        pcb_kicad::note_reference_keepouts(
            &mut facts,
            &reference.join(&board),
            &result_directory.join(&board),
            &config.placer,
            Path::new(source),
        );
    }
    check_reference(
        &mut facts,
        reference.as_deref(),
        board_id,
        &output.join("reference-drc.json"),
    );
    check_input(&mut facts, Path::new(source), &result_directory, board_id);
    facts.wall_seconds = started.elapsed().as_secs_f64();
    let mut verdict = pcb_kicad::judge(&facts);
    note_second_ring_retry(&mut verdict, &result_directory, board_id, &router);
    verdict.generated_pad_via_recovery = result.generated_pad_via_recovery.clone();
    verdict.body_near_grid_retry = result.body_near_grid_retry.clone();
    verdict.unchanged_hatch_width_repair = result.unchanged_hatch_width_repair.clone();
    if verdict
        .unchanged_hatch_width_repair
        .as_ref()
        .is_some_and(|repair| !repair.retry_argv.is_empty())
    {
        verdict.next_actions.push("The layout ended before the same-hatch width repair could run. unchanged_hatch_width_repair.retry_argv gives the command to try on a separate project with the original hatch settings.".into());
    }
    if verdict.kicad_unconnected_items.is_some_and(|n| n > 0) {
        verdict.generated_pad_via_retries = pcb_kicad::suggest_generated_pad_via_repairs(
            &result_directory,
            board_id,
            &output.join("placed"),
            &output.join("generated-pad-via-retry"),
        );
        for retry in verdict.generated_pad_via_retries.iter().rev() {
            if let Some(reason) = &retry.native_evidence_limit {
                verdict.next_actions.insert(
                    0,
                    format!(
                        "A generated via in {} is near the native open at {}. {reason}",
                        retry.host_pad, retry.open_pad,
                    ),
                );
                continue;
            }
            let command = retry
                .retry_argv
                .iter()
                .map(|argument| shell_argument(argument))
                .collect::<Vec<_>>()
                .join(" ");
            verdict.next_actions.insert(0, format!(
                "Native open {} on {} ({}) is near a router-generated filled via in {} on {}. Try the bounded relocation with `{command}`. The command keeps the via ring in its land and checks KiCad's stored and refilled results before publishing.",
                retry.open_pad, retry.open_layer, retry.net, retry.host_pad, retry.host_layer,
            ));
        }
    }
    if verdict.status == KiCadRunStatus::Incomplete && verdict.kicad_checked {
        verdict.generated_dangling_retry = pcb_kicad::suggest_generated_dangling_leaf(
            &result_directory,
            board_id,
            &output.join("placed"),
            &output.join("generated-dangling-retry"),
        );
        if let Some(leaf) = &verdict.generated_dangling_retry {
            let command = leaf
                .retry_argv
                .iter()
                .map(|argument| shell_argument(argument))
                .collect::<Vec<_>>()
                .join(" ");
            verdict.next_actions.insert(0, format!(
                "KiCad reports a {:.3} mm dangling track on {} ({}, UUID {}). The placed input and routed track identify generated copper. Try `{command}` to remove it with native connectivity checks both as stored and after refill.",
                leaf.length_mm, leaf.net, leaf.layer, leaf.track,
            ));
        }
    }
    if let Some(fill) = &result.fill_publication
        && !fill.retry_argv.is_empty()
    {
        let command = fill
            .retry_argv
            .iter()
            .map(|arg| shell_argument(arg))
            .collect::<Vec<_>>()
            .join(" ");
        let action = format!(
            "The layout budget ended before the zone fills could be saved and verified. To deliver a board with its fills on disk, run {command}."
        );
        if verdict.status == KiCadRunStatus::Complete {
            verdict.next_actions.insert(0, action);
        } else {
            verdict.next_actions.push(action);
        }
    }
    verdict.note_via_cost_retry(
        &facts,
        &result.routed,
        &result_directory.join(format!("{board_id}.kicad_pcb")),
        &router,
        positional.get(3).map(String::as_str),
    );
    if verdict.zone_island_recovery.is_none()
        && let Some(repair) = &result.local_open_repair
        && repair.status == "skipped"
        && !repair.retry_argv.is_empty()
        && matches!(
            repair.reason.as_deref(),
            Some(
                "insufficient_remaining_work"
                    | "insufficient_remaining_work_after_fill_reserve"
                    | "wall_guard_expired"
            )
        )
    {
        let command = repair
            .retry_argv
            .iter()
            .map(|arg| shell_argument(arg))
            .collect::<Vec<_>>()
            .join(" ");
        verdict.next_actions.push(format!("layout budget left local open repair for a separate run. Retry on the selected board with `{command}`"));
    }
    if result.plane_escape_reserved_seconds.is_none()
        && !result.stopped_after_first_route
        && verdict.status == KiCadRunStatus::Incomplete
        && verdict.kicad_unconnected_items == Some(1)
        && verdict.constraints_missed.is_empty()
        && pcb_kicad::has_plane_escape_open(&result_directory, board_id)
    {
        verdict.next_actions.push(format!(
            "try a paired plane escape with the same fixed placement: pcb-maker repair-kicad-opens {} {} --plane-escape {} {} <router.json>; set ladder_budget_seconds in router.json for this reroute",
            shell_argument(&path_text(&result_directory)),
            shell_argument(board_id),
            shell_argument(&path_text(&output.join("plane-escape-retry"))),
            shell_argument(&path_text(&output.join("placed"))),
        ));
    }
    if placement_seed.is_none()
        && !result.stopped_after_first_route
        && verdict.status == KiCadRunStatus::Incomplete
        && verdict.kicad_unconnected_items.is_some_and(|open| open > 0)
        && verdict.new_findings_strict.is_empty()
        && verdict.constraints_missed.is_empty()
        && facts.boards_mixed().is_none()
        && placed["movable"]
            .as_u64()
            .is_some_and(|movable| movable > 0)
        && let Some(seeds_run) = placed["seeds_run"].as_u64()
    {
        let seed = alternate_placement_seed(config.placer.tuning.seed, seeds_run);
        if let Some(action) = placement_retry_action(&positional, &values, output, seed) {
            verdict.alternate_placement_seed = Some(seed);
            let observations =
                pcb_kicad::with_input_observations(Vec::new(), &verdict.input_observations);
            verdict
                .next_actions
                .insert(verdict.next_actions.len() - observations.len(), action);
        }
    }
    write_verdict(output, Some(&report), &verdict).map_err(Failure::Internal)?;
    announce(&verdict, &output.join("verdict.json"));
    Ok(verdict.status)
}

fn place(arguments: Vec<String>) -> Result<KiCadRunStatus, Failure> {
    let command = "place-kicad-board";
    // --measure-work-rate: the rates measured at the start, as a layout
    // takes them (default: the reference's, pinned).
    let measure = arguments
        .iter()
        .any(|argument| argument == "--measure-work-rate");
    let arguments = arguments
        .into_iter()
        .filter(|argument| argument != "--measure-work-rate")
        .collect();
    let (positional, values) = split_flags(arguments, &["--reference", "--router"], PLACE_USAGE)?;
    positional_count(&positional, 3..=4, PLACE_USAGE)?;
    let (source, board_id, output) = (&positional[0], &positional[1], &positional[2]);
    check_project(source, board_id)?;
    let reference = reference_option(&values, None, board_id)?;
    check_fresh_output(output)?;
    // A layout config (import-kicad-netlist's layout.json) is the
    // layout's: its placer object, budget and race settings; a file
    // holding only the placer object works too.
    let mut layout: pcb_kicad::KiCadBoardLayoutConfig = match positional.get(3) {
        Some(path) => match read_json::<serde_json::Value>(path)? {
            serde_json::Value::Object(layout) if layout.contains_key("placer") => {
                serde_json::from_value(serde_json::Value::Object(layout))
                    .map_err(|error| Failure::Usage(format!("failed to parse {path}: {error}")))?
            }
            value => pcb_kicad::KiCadBoardLayoutConfig {
                placer: serde_json::from_value(value)
                    .map_err(|error| Failure::Usage(format!("failed to parse {path}: {error}")))?,
                ..Default::default()
            },
        },
        None => Default::default(),
    };
    // The reference board's places say which sub-board of a panel each
    // part belongs to (`sub_boards`).
    layout.placer.sub_board_reference = reference
        .as_ref()
        .map(|project| project.join(format!("{board_id}.kicad_pcb")));
    // The router configuration the layout would take (--router, default
    // auto: the project's rules): its clearances, and the escape demand's
    // measurements route with it.
    let router =
        crate::board_router_config(values.get("--router").map(String::as_str), source, board_id)
            .map_err(Failure::Usage)?;
    let mut input_warnings = check_constraints(source, board_id, &layout.placer, &router)?;
    // A placement needs no KiCad, but what comes after it does.
    input_warnings.extend(check_kicad(true)?);
    // The rates first: the escape demand's measurements and the
    // placement's budget count work.
    if measure {
        calibrate_work_rate(Some(&router))?;
    } else {
        let pinned = match (router.work_rate, router.placer_work_rate) {
            (Some(expansions), placer) => Some((expansions, placer)),
            (None, Some(_)) => {
                return Err(Failure::Usage(
                    "the router setting placer_work_rate needs work_rate beside it".into(),
                ));
            }
            (None, None) => None,
        };
        pcb_kicad::calibrate_reference_by_default(pinned).map_err(Failure::Usage)?;
    }
    // What the layout of this config places with (the layout's share of
    // its budget, 600 s of work by default; the race's seeds; the escape
    // demand measured with the router's rules), so that the check's yes is
    // the layout's at the same work rates. PCB_PLACER_WORK_SECONDS sets
    // another budget.
    let mut config =
        pcb_kicad::placement_check_config(Path::new(source), board_id, &layout, &router)
            .map_err(classify)?;
    if let Some(seconds) = std::env::var("PCB_PLACER_WORK_SECONDS")
        .ok()
        .and_then(|value| value.parse().ok())
    {
        config.work_seconds = Some(seconds);
    }
    let output = Path::new(output);
    let outputs = BTreeMap::from([
        ("project".to_string(), path_text(output)),
        (
            "board".to_string(),
            path_text(&output.join(format!("{board_id}.kicad_pcb"))),
        ),
        (
            "report".to_string(),
            path_text(&output.join("board-placer.json")),
        ),
        (
            "placement".to_string(),
            path_text(&output.join("placement.html")),
        ),
        (
            "verdict".to_string(),
            path_text(&output.join("verdict.json")),
        ),
    ]);
    let report = output.join("board-placer.json");
    set_target(command, report.clone(), outputs.clone());
    let started = Instant::now();
    let result = pcb_kicad::place_kicad_board(Path::new(source), board_id, output, &config)
        .map_err(classify)?;
    // Parts are the board's footprints; the placer's other objects are the
    // board's own marks (texts, graphics) and rule areas it keeps clear of
    // (the trial's "placed 38 of 52 components" counted them, 2026-10-26).
    let parts = result.footprints.len();
    let others = result.components.saturating_sub(parts);
    println!(
        "placed {} of {} parts ({} kept where they are{}), {} nets: wirelength source {:.0} mm, global {:.0}, legal {:.0}, final {:.0}; {} global iterations, overflow {:.3}; unplaced {:?}, illegal {:?}; {:.2}s",
        result.movable,
        parts,
        parts.saturating_sub(result.movable),
        if others == 0 {
            String::new()
        } else {
            format!("; {others} board marks and rule areas kept clear of")
        },
        result.nets,
        result.wirelength_source_mm,
        result.wirelength_global_mm,
        result.wirelength_legal_mm,
        result.wirelength_final_mm,
        result.global_iterations,
        result.global_overflow,
        result.unplaced,
        result.illegal,
        result.seconds,
    );
    let placed =
        serde_json::to_value(&result).map_err(|error| Failure::Internal(error.to_string()))?;
    let mut facts = placement_facts(command, &placed, config.tuning.refine_pad_near);
    facts.note_authored_layout_constraints(Path::new(source), &config);
    facts.input_warnings = constraint_warnings(input_warnings, &placed);
    facts.outputs = existing(&outputs);
    facts.wall_seconds = started.elapsed().as_secs_f64();
    let mut verdict = pcb_kicad::judge(&facts);
    note_unplaced_antenna_edges(&mut verdict, Path::new(source), board_id, &placed);
    write_verdict(output, Some(&report), &verdict).map_err(Failure::Internal)?;
    announce(&verdict, &output.join("verdict.json"));
    Ok(verdict.status)
}

const DRC_USAGE: &str = "usage: pcb-maker drc-kicad-board <project-dir> <board-id> <report.json> [--no-refill] [--input <project-dir>] [--stitch-zone-island <out-dir> | --save-filled <out-dir> | --localize-zone-widths]";

/// Compare output paths before their missing directories have been created.
fn prospective_output_path(path: &Path) -> Result<PathBuf, Failure> {
    let absolute = std::path::absolute(path).map_err(|error| Failure::Usage(error.to_string()))?;
    let mut resolved = PathBuf::new();
    for part in absolute.components() {
        match part {
            std::path::Component::ParentDir => {
                resolved.pop();
            }
            std::path::Component::CurDir => (),
            _ => {
                resolved.push(part.as_os_str());
                if let Ok(canonical) = fs::canonicalize(&resolved) {
                    resolved = canonical;
                }
            }
        }
    }
    Ok(resolved)
}

fn check_stitch_paths(
    repair: &Path,
    report: &Path,
    board_id: &str,
    projects: impl IntoIterator<Item = impl AsRef<Path>>,
) -> Result<(), Failure> {
    let repair = prospective_output_path(repair)?;
    let scratch_path = report.with_extension("project");
    let scratch = prospective_output_path(&scratch_path)?;
    for ancestor in report
        .ancestors()
        .skip(1)
        .filter(|ancestor| !ancestor.as_os_str().is_empty())
    {
        if prospective_output_path(ancestor)?.starts_with(&repair) {
            return Err(Failure::Usage(
                "put the baseline DRC report beside the repair output directory, such as before.json beside repaired/".into(),
            ));
        }
    }
    let report_entry = report
        .file_name()
        .zip(report.parent())
        .map(|(name, parent)| {
            let parent = if parent.as_os_str().is_empty() {
                Path::new(".")
            } else {
                parent
            };
            prospective_output_path(parent).map(|directory| directory.join(name))
        })
        .transpose()?;
    let report = prospective_output_path(report)?;
    if report.starts_with(&repair) || scratch.starts_with(&repair) || repair.starts_with(&scratch) {
        return Err(Failure::Usage(
            "put the baseline DRC report beside the repair output directory, such as before.json beside repaired/".into(),
        ));
    }
    for project in projects {
        let project = prospective_output_path(project.as_ref())?;
        if repair.starts_with(&project) {
            return Err(Failure::Usage(
                "put the repair output outside the input project".into(),
            ));
        }
        if report == project || project.starts_with(&scratch) {
            return Err(Failure::Usage(
                "use a separate DRC report path. The given path would replace an input project"
                    .into(),
            ));
        }
        let project_file = |path: &Path| {
            path.starts_with(&project)
                && (path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extension.starts_with("kicad_"))
                    || path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| matches!(name, "fp-lib-table" | "sym-lib-table")))
        };
        if project_file(&report) || report_entry.as_deref().is_some_and(project_file) {
            return Err(Failure::Usage(
                "use a separate DRC report path. The given path would replace a project input file"
                    .into(),
            ));
        }
        for name in [
            format!("{board_id}.kicad_pcb"),
            format!("{board_id}.kicad_pro"),
            format!("{board_id}.kicad_dru"),
            format!("{board_id}.kicad_sch"),
            "fp-lib-table".into(),
            "sym-lib-table".into(),
            "constraints.json".into(),
            "layout.json".into(),
        ] {
            if report == prospective_output_path(&project.join(name))? {
                return Err(Failure::Usage(
                    "use a separate DRC report path. The given path would replace a project input file".into(),
                ));
            }
        }
    }
    if scratch == report || fs::symlink_metadata(&scratch_path).is_ok() {
        return Err(Failure::Usage(
            "use a different baseline DRC report name, such as before.json, to avoid replacing the temporary project path".into(),
        ));
    }
    Ok(())
}

fn save_filled(
    source: &str,
    board_id: &str,
    report: &Path,
    output: &Path,
) -> Result<KiCadRunStatus, Failure> {
    if fs::symlink_metadata(output).is_ok() {
        return Err(Failure::Usage(format!(
            "fill output already exists: {}",
            output.display()
        )));
    }
    let source = prospective_output_path(Path::new(source))?;
    let output = prospective_output_path(output)?;
    let report = prospective_output_path(report)?;
    if output.starts_with(&source) || report.starts_with(&source) || source.starts_with(&report) {
        return Err(Failure::Usage(
            "put the fill output and report outside the input project".into(),
        ));
    }
    if report.starts_with(&output) || output.starts_with(&report) {
        return Err(Failure::Usage("put the DRC report beside the fill output directory, such as refilled.json beside filled/".into()));
    }
    check_kicad(false)?;
    for path in [&report, &output] {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                Failure::Usage(format!("failed to create {}: {error}", parent.display()))
            })?;
        }
    }
    let result = pcb_kicad::save_filled_kicad_board(&source, board_id, &report, &output)
        .map_err(classify)?;
    let counts = &result.saved_as_stored;
    println!(
        "saved fill: {} opens before refill, {} opens on disk, {} errors, {} schematic parity findings{}; project {}",
        result.source_as_stored.opens,
        counts.opens,
        counts.errors,
        counts.schematic_parity,
        if result.schematic_parity_checked {
            ""
        } else {
            " (no schematic)"
        },
        output.display(),
    );
    if let Some(proof) = &result.connectivity_proof {
        println!(
            "native connectivity agrees across refill and reopen: {} copper objects, {} components, {} nets",
            proof.copper_objects, proof.components, proof.nets
        );
    }
    println!("reports: {}", output.join("fill-handoff.json").display());
    Ok(result.status)
}

/// KiCad's DRC of a board, through the DRC cache: complete when KiCad
/// finds no error that counts and no unconnected item.
fn drc(arguments: Vec<String>) -> Result<KiCadRunStatus, Failure> {
    let refill = !arguments.iter().any(|argument| argument == "--no-refill");
    let localize = arguments
        .iter()
        .any(|argument| argument == "--localize-zone-widths");
    let arguments = arguments
        .into_iter()
        .filter(|argument| !matches!(argument.as_str(), "--no-refill" | "--localize-zone-widths"))
        .collect();
    let (positional, values) = split_flags(
        arguments,
        &["--input", "--stitch-zone-island", "--save-filled"],
        DRC_USAGE,
    )?;
    positional_count(&positional, 3..=3, DRC_USAGE)?;
    let (mut source, board_id) = (&positional[0], &positional[1]);
    let mut report = PathBuf::from(&positional[2]);
    check_project(source, board_id)?;
    let stitch = values.get("--stitch-zone-island");
    if localize {
        if !refill || !values.is_empty() {
            return Err(Failure::Usage(
                "use --localize-zone-widths on its own to compare stored and refilled copper"
                    .into(),
            ));
        }
        let project = prospective_output_path(Path::new(source))?;
        report = prospective_output_path(&report)?;
        if report.starts_with(&project) || project.starts_with(&report) {
            return Err(Failure::Usage(
                "put the zone width diagnostic report outside the source project".into(),
            ));
        }
    }
    if let Some(output) = values.get("--save-filled") {
        if !refill || stitch.is_some() || values.contains_key("--input") {
            return Err(Failure::Usage("run --save-filled by itself. It refills the board and compares native findings with the source project".into()));
        }
        return save_filled(source, board_id, &report, Path::new(output));
    }
    if let Some(out) = stitch {
        if !refill {
            return Err(Failure::Usage(
                "--stitch-zone-island requires zone refill".into(),
            ));
        }
        if fs::symlink_metadata(out).is_ok() {
            return Err(Failure::Usage(format!(
                "repair output already exists: {out}"
            )));
        }
    }
    let input_project = values.get("--input");
    if let Some(input) = input_project {
        check_project(input, board_id)?;
    }
    if let Some(out) = stitch {
        check_stitch_paths(
            Path::new(out),
            &report,
            board_id,
            std::iter::once(source.as_str()).chain(input_project.map(String::as_str)),
        )?;
    }
    check_kicad(false)?;
    if let Some(parent) = report
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|error| {
            Failure::Usage(format!("failed to create {}: {error}", parent.display()))
        })?;
    }
    let mut value = if localize {
        let board = Path::new(source).join(format!("{board_id}.kicad_pcb"));
        let (mut drc, localized) =
            pcb_kicad::localize_zone_widths(&board).map_err(Failure::Internal)?;
        let found = localized
            .findings
            .iter()
            .filter(|row| row.location.is_some())
            .count();
        let geometry = if localized.geometry_fill == Some("private_refill") {
            " on a private refill"
        } else if localized.native_geometry_reads > 0 {
            " on saved copper"
        } else {
            ""
        };
        println!(
            "zone widths: {found}/{} localized{geometry}",
            localized.findings.len()
        );
        if let Some(reason) = &localized.reason {
            println!("zone widths: {reason}");
        }
        drc["zone_width_localization"] = serde_json::to_value(localized)
            .map_err(|error| Failure::Internal(error.to_string()))?;
        let mut json = serde_json::to_vec_pretty(&drc)
            .map_err(|error| Failure::Internal(error.to_string()))?;
        json.push(b'\n');
        pcb_kicad::write_atomically(&report, &json).map_err(Failure::Internal)?;
        drc
    } else {
        pcb_kicad::kicad_drc_report(Path::new(source), board_id, &report, refill)
            .map_err(Failure::Internal)?
    };
    if let Some(out) = stitch {
        let repair =
            pcb_kicad::stitch_zone_island(Path::new(source), board_id, &value, Path::new(out))
                .map_err(Failure::Internal)?;
        println!(
            "zone island repair: {}",
            serde_json::to_string(&repair).map_err(|error| Failure::Internal(error.to_string()))?
        );
        if repair.kept {
            source = out;
            report = Path::new(out).join("drc.json");
            value = read_json(&path_text(&report))?;
        }
    }
    let tally = pcb_kicad::tally_drc(&value);
    let cache = pcb_kicad::drc_cache_counts();
    let errors: Vec<String> = tally
        .errors
        .iter()
        .map(|(kind, count)| format!("{kind} {count}"))
        .collect();
    println!(
        "drc: {} errors ({}), {} unconnected items, {} warnings, {} cosmetic; drc_cache: {{calls {}, hits {}}}; report {}",
        tally.errors.values().sum::<usize>(),
        errors.join(", "),
        tally.unconnected,
        tally.warnings,
        tally.cosmetic,
        cache.calls,
        cache.hits,
        path_text(&report)
    );
    // The project's rules the router does not keep: KiCad checks them, so
    // their findings are for the designer (board-router.json's rules has
    // every rule read).
    // A footprint's own findings (its pads at its own holes) and the
    // input's that are not electrical (`input_own`; without --input the
    // board judged on its own, every footprint, zone and board graphic its
    // own, never a track or a via; with it, only the parts kept where that
    // board has them and its zones and graphics left unchanged) are said
    // apart and do not fail the check, as in a layout's verdict and the fab
    // release.
    let read = |directory: &str| {
        let path = Path::new(directory).join(format!("{board_id}.kicad_pcb"));
        fs::read_to_string(&path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))
    };
    let input = read(source).and_then(|board| match input_project {
        None => pcb_kicad::board_items(&board),
        Some(input) => pcb_kicad::input_items(&read(input)?, &board),
    });
    if let Err(error) = &input {
        eprintln!(
            "warning: the board's own findings are not told apart ({}): every finding counts",
            error.lines().next().unwrap_or("")
        );
    }
    let apart = |which: pcb_kicad::Apart| -> Vec<&pcb_kicad::DrcFinding> {
        tally
            .findings
            .iter()
            .filter(|finding| {
                pcb_kicad::unreferenced_apart(finding, input.as_ref().ok()) == Some(which)
            })
            .collect()
    };
    let own = apart(pcb_kicad::Apart::Footprint);
    let board_own = apart(pcb_kicad::Apart::Input);
    let dru = path_text(&Path::new(source).join(format!("{board_id}.kicad_dru")));
    // The order of a verdict's next_actions: what to act on first, the
    // input's own observations (the footprints' and the board's own
    // findings, which do not fail the check) last, tagged.
    let observations: Vec<String> = pcb_kicad::footprint_findings_note(&own, &dru)
        .into_iter()
        .chain(pcb_kicad::input_findings_note(
            &board_own,
            &dru,
            input_project.is_none(),
        ))
        .collect();
    let failed =
        tally.errors.values().sum::<usize>() > own.len() + board_own.len() || tally.unconnected > 0;
    let mut actions = Vec::new();
    actions.extend(pcb_kicad::unassigned_mask_action(&tally.findings));
    actions.extend(tally.unassigned_fill_action());
    if stitch.is_none() && refill {
        actions.extend(tally.single_layer_island_action.clone());
        actions.extend(pcb_kicad::zone_island_action(&value).map(str::to_string));
    }
    if failed
        && let Ok(config) = pcb_kicad::resolve_project_rules(
            &Path::new(source).join(format!("{board_id}.kicad_pro")),
            &Path::new(source).join(format!("{board_id}.kicad_pcb")),
        )
        && let Some(project) = config.project_rules
        && !project.unmodelled.is_empty()
    {
        actions.push(format!(
            "the project has rules the router does not keep, which KiCad checks: {}",
            project.unmodelled.join("; ")
        ));
    }
    for line in pcb_kicad::with_input_observations(actions, &observations) {
        println!("next: {line}");
    }
    Ok(if !failed {
        KiCadRunStatus::Complete
    } else {
        KiCadRunStatus::Incomplete
    })
}

const INSPECT_USAGE: &str = "usage: pcb-maker inspect-routing-run <run-dir|saved.kicad_pcb> [--net <net>] [--terminal <REF:PAD>] [--reference <project-dir>] [--escape-placement <placed-project-dir>] [--native-check] [--near-paths] [--near-constraints <file.json>] [--output <file.json>]";

/// Why a run left nets open, from its saved outputs
/// (`pcb_kicad::inspect_routing_run`): complete when the report was
/// written. `--native-check` runs KiCad's DRC first when the run has no
/// drc.json (into `native-check-drc.json` beside the board).
fn inspect_run(arguments: Vec<String>) -> Result<KiCadRunStatus, Failure> {
    let native_check = arguments
        .iter()
        .any(|argument| argument == "--native-check");
    let near_paths = arguments.iter().any(|argument| argument == "--near-paths");
    let arguments = arguments
        .into_iter()
        .filter(|argument| argument != "--native-check" && argument != "--near-paths")
        .collect();
    let (positional, values) = split_flags(
        arguments,
        &[
            "--net",
            "--terminal",
            "--reference",
            "--escape-placement",
            "--near-constraints",
            "--output",
        ],
        INSPECT_USAGE,
    )?;
    positional_count(&positional, 1..=1, INSPECT_USAGE)?;
    let run = PathBuf::from(&positional[0]);
    let near_constraints = values.get("--near-constraints").map(PathBuf::from);
    if let Some(path) = &near_constraints {
        if !near_paths {
            return Err(Failure::Usage(
                "--near-constraints requires --near-paths".into(),
            ));
        }
        if !path.is_file() {
            return Err(Failure::Usage(format!(
                "near constraints {} are missing",
                path.display()
            )));
        }
    }
    if near_paths && run.extension().and_then(|extension| extension.to_str()) == Some("kicad_pcb") {
        if !run.is_file() {
            return Err(Failure::Usage(format!(
                "saved PCB {} does not exist",
                run.display()
            )));
        }
        if native_check
            || ["--terminal", "--reference", "--escape-placement"]
                .iter()
                .any(|flag| values.contains_key(*flag))
        {
            return Err(Failure::Usage("saved PCB near-path inspection accepts --net and --near-constraints; give a routed run directory for route diagnosis".into()));
        }
        let paths = pcb_kicad::inspect_saved_near_paths(
            &run,
            near_constraints.as_deref(),
            values.get("--net").map(String::as_str),
        );
        let diagnosis =
            serde_json::json!({"schema_version": 1, "units": "mm", "near_paths": paths});
        if let Some(path) = values.get("--output") {
            pcb_kicad::write_atomically(Path::new(path), format!("{diagnosis:#}\n").as_bytes())
                .map_err(|error| Failure::Usage(format!("failed to write {path}: {error}")))?;
            println!("wrote near-path evidence to {path}");
        } else {
            println!("{diagnosis:#}");
        }
        return Ok(KiCadRunStatus::Complete);
    }
    if !run.is_dir() {
        return Err(Failure::Usage(format!(
            "run directory {} does not exist",
            run.display()
        )));
    }
    if values.contains_key("--net") && values.contains_key("--terminal") {
        return Err(Failure::Usage(format!(
            "give --net or --terminal, not both; {INSPECT_USAGE}"
        )));
    }
    if values.contains_key("--escape-placement") && !values.contains_key("--terminal") {
        return Err(Failure::Usage(
            "--escape-placement requires --terminal REF:PAD".into(),
        ));
    }
    let files = pcb_kicad::run_files(&run).map_err(classify)?;
    let mut drc = None;
    if native_check && !files.directory.join("drc.json").is_file() {
        check_kicad(false)?;
        let report = files.directory.join("native-check-drc.json");
        pcb_kicad::kicad_drc_report(&files.directory, &files.board_id, &report, true)
            .map_err(Failure::Internal)?;
        drc = Some(report);
    }
    let inspection = pcb_kicad::KiCadRunInspection {
        run,
        net: values.get("--net").cloned(),
        terminal: values.get("--terminal").cloned(),
        reference: values.get("--reference").map(PathBuf::from),
        escape_placement: values.get("--escape-placement").map(PathBuf::from),
        near_paths,
        near_constraints,
        drc,
    };
    let diagnosis = pcb_kicad::inspect_routing_run(&inspection).map_err(classify)?;
    let text = serde_json::to_string_pretty(&diagnosis)
        .map_err(|error| Failure::Internal(error.to_string()))?;
    match values.get("--output") {
        Some(path) => {
            pcb_kicad::write_atomically(Path::new(path), format!("{text}\n").as_bytes())
                .map_err(|error| Failure::Usage(format!("failed to write {path}: {error}")))?;
            println!(
                "inspected {} net{} of {}: {}",
                diagnosis.nets.len(),
                if diagnosis.nets.len() == 1 { "" } else { "s" },
                files.board_id,
                path_text(Path::new(path))
            );
        }
        None => println!("{text}"),
    }
    Ok(KiCadRunStatus::Complete)
}

const FAB_USAGE: &str = "usage: pcb-maker fab-kicad-board <project-dir> <board-id> <out-dir> [--allow-findings] [--layers <L1,L2,...>] [--profile kicad|jlcpcb] [--no-step] [--input <project-dir>]";

/// A DRC-gated fab release (`pcb_kicad::fab_kicad_board`): complete when
/// the fab files were written, incomplete (nothing exported) when KiCad's
/// DRC finds copper errors or unconnected items and `--allow-findings` is
/// not given.
fn fab(arguments: Vec<String>) -> Result<KiCadRunStatus, Failure> {
    let command = "fab-kicad-board";
    let switch = |name: &str| arguments.iter().any(|argument| argument == name);
    let (allow_findings, no_step) = (switch("--allow-findings"), switch("--no-step"));
    let arguments = arguments
        .into_iter()
        .filter(|argument| argument != "--allow-findings" && argument != "--no-step")
        .collect();
    let (positional, values) =
        split_flags(arguments, &["--layers", "--profile", "--input"], FAB_USAGE)?;
    positional_count(&positional, 3..=3, FAB_USAGE)?;
    let (source, board_id, output) = (&positional[0], &positional[1], &positional[2]);
    check_project(source, board_id)?;
    let input_project = values.get("--input").map(PathBuf::from);
    if let Some(input) = &input_project {
        check_project(&path_text(input), board_id)?;
    }
    check_fresh_output(output)?;
    let profile = match values.get("--profile").map(String::as_str) {
        None | Some("kicad") => pcb_kicad::KiCadFabProfile::Kicad,
        Some("jlcpcb") => pcb_kicad::KiCadFabProfile::Jlcpcb,
        Some(other) => {
            return Err(Failure::Usage(format!(
                "unknown --profile {other}: kicad or jlcpcb; {FAB_USAGE}"
            )));
        }
    };
    let layers = match values.get("--layers") {
        None => None,
        Some(list) => {
            let layers: Vec<String> = list
                .split(',')
                .map(str::trim)
                .filter(|layer| !layer.is_empty())
                .map(str::to_string)
                .collect();
            if layers.is_empty() {
                return Err(Failure::Usage(format!(
                    "--layers needs layer names; {FAB_USAGE}"
                )));
            }
            Some(layers)
        }
    };
    let options = pcb_kicad::KiCadFabOptions {
        allow_findings,
        layers,
        profile,
        no_step,
        input: input_project.clone(),
    };
    let output = Path::new(output);
    let outputs = BTreeMap::from([
        ("project".to_string(), path_text(Path::new(source))),
        (
            "board".to_string(),
            path_text(&Path::new(source).join(format!("{board_id}.kicad_pcb"))),
        ),
        ("drc".to_string(), path_text(&output.join("drc.json"))),
        ("erc".to_string(), path_text(&output.join("erc.json"))),
        (
            "report".to_string(),
            path_text(&output.join("fab-manifest.json")),
        ),
        (
            "zip".to_string(),
            path_text(&output.join(format!("{board_id}-gerbers.zip"))),
        ),
        (
            "verdict".to_string(),
            path_text(&output.join("verdict.json")),
        ),
    ]);
    set_target(command, output.join("fab-manifest.json"), outputs.clone());
    check_kicad(false)?;
    fs::create_dir_all(output).map_err(|error| {
        Failure::Internal(format!("failed to create {}: {error}", output.display()))
    })?;
    let started = Instant::now();
    let manifest = pcb_kicad::fab_kicad_board(Path::new(source), board_id, output, &options)
        .map_err(classify)?;
    let tally = pcb_kicad::load_drc_tally(&output.join("drc.json")).map_err(Failure::Internal)?;
    let mut facts = KiCadRunFacts::new(command);
    facts.native = Some(tally);
    // The input's items as the release gate takes them (--input, else the
    // board judged on its own).
    let read = |directory: &Path| {
        fs::read_to_string(directory.join(format!("{board_id}.kicad_pcb")))
            .map_err(|error| error.to_string())
    };
    facts.input = read(Path::new(source))
        .and_then(|board| match &input_project {
            None => pcb_kicad::board_items(&board),
            Some(input) => pcb_kicad::input_items(&read(input)?, &board),
        })
        .ok();
    facts.outputs = existing(&outputs);
    facts.wall_seconds = started.elapsed().as_secs_f64();
    let mut verdict = pcb_kicad::judge(&facts);
    let drc = &manifest.drc;
    let status = if manifest.released {
        KiCadRunStatus::Complete
    } else {
        KiCadRunStatus::Incomplete
    };
    verdict.status = status;
    verdict.exit_code = status.exit_code();
    // With the board it was laid out from: the findings between parts the
    // layout kept where the input has them are not the layout's to move
    // (as its verdict says).
    let (kept, kept_count) = if !manifest.released && input_project.is_some() {
        pcb_kicad::kept_parts_actions(&facts)
    } else {
        (Vec::new(), 0)
    };
    let lines = pcb_kicad::fab_verdict_lines(&manifest, kept, kept_count, input_project.is_some());
    verdict.reason = lines.reason;
    verdict.next_actions = lines.next_actions;
    verdict.input_observations = lines.input_observations;
    for skipped in &manifest.skipped {
        println!("skipped: {skipped}");
    }
    if let Some(erc) = &manifest.erc {
        println!(
            "erc: {} errors, {} warnings (counted, not blocking)",
            erc.errors, erc.warnings
        );
    }
    let report = output.join("fab-manifest.json");
    write_verdict(output, None, &verdict).map_err(Failure::Internal)?;
    // The release in the verdict too: what an agent reads first.
    let path = output.join("verdict.json");
    let mut value: serde_json::Value = fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .ok_or_else(|| Failure::Internal(format!("failed to read {}", path.display())))?;
    value["fab_release"] = serde_json::json!({
        "released": manifest.released,
        "findings_allowed": drc.findings_allowed,
        "profile": manifest.profile,
        "kicad_version": manifest.kicad_version,
        "files": manifest.files.len(),
        "zip": manifest.zip,
        "manifest": path_text(&report),
        "skipped": manifest.skipped,
    });
    fs::write(
        &path,
        serde_json::to_string_pretty(&value)
            .map_err(|error| Failure::Internal(error.to_string()))?,
    )
    .map_err(|error| Failure::Internal(format!("failed to write {}: {error}", path.display())))?;
    announce(&verdict, &path);
    Ok(status)
}

/// The commands that only read a project and write files: the arguments
/// are checked here (exit 2), the work is the command's own (an error
/// there exits 4).
fn checked_simple(command: &str, arguments: &[String]) -> Result<(), Failure> {
    let usage = || {
        Failure::Usage(format!(
            "{}; see pcb-maker {command} --help",
            usage_line(command)
        ))
    };
    let count = |range: std::ops::RangeInclusive<usize>| {
        if range.contains(&arguments.len()) {
            Ok(())
        } else {
            Err(usage())
        }
    };
    match command {
        "describe-kicad-board" => {
            // --constraints-template <file>: a new file, never one that
            // exists (it would be an agent's own constraints).
            let template = arguments
                .iter()
                .position(|argument| argument == "--constraints-template");
            let positional: Vec<&String> = match template {
                Some(at) => {
                    let Some(file) = arguments.get(at + 1) else {
                        return Err(usage());
                    };
                    if Path::new(file).exists() {
                        return Err(Failure::Usage(format!(
                            "--constraints-template {file}: the file exists (the template is written to a new file; an agent's constraints are never overwritten)"
                        )));
                    }
                    arguments
                        .iter()
                        .enumerate()
                        .filter(|(index, _)| *index != at && *index != at + 1)
                        .map(|(_, argument)| argument)
                        .collect()
                }
                None => arguments.iter().collect(),
            };
            if positional.len() != 2 {
                return Err(usage());
            }
            check_project(positional[0], positional[1])
        }
        "score-kicad-board" => {
            let parsed =
                crate::kicad_commands::board_score_arguments(arguments).map_err(Failure::Usage)?;
            check_project(parsed.directory, parsed.board_id)?;
            if let Some(reference) = parsed.reference {
                check_project(reference, parsed.board_id)?;
            }
            Ok(())
        }
        "export-kicad-fab" => {
            count(3..=3)?;
            check_project(&arguments[0], &arguments[1])
        }
        "repair-kicad-silkscreen" => {
            count(3..=3)?;
            check_project(&arguments[0], &arguments[1])?;
            check_fresh_output(&arguments[2])
        }
        "swap-kicad-pins" => {
            count(4..=5)?;
            check_project(&arguments[0], &arguments[1])?;
            check_fresh_output(&arguments[2])?;
            pcb_kicad::read_pin_swap_spec(Path::new(&arguments[3])).map_err(Failure::Usage)?;
            if let Some(path) = arguments.get(4) {
                read_json::<pcb_kicad::KiCadPinSwapConfig>(path)?;
            }
            Ok(())
        }
        "strip-kicad-tracks" => {
            count(2..=4)?;
            if arguments.len() > 2 && (arguments[2] != "--keep-nets" || arguments.len() != 4) {
                return Err(Failure::Usage(
                    "strip-kicad-tracks takes only --keep-nets <net>[,<net>...] after the two boards"
                        .into(),
                ));
            }
            check_file(&arguments[0])
        }
        "strip-kicad-copper" => {
            count(2..=3)?;
            check_file(&arguments[0])
        }
        "import-kicad-netlist" => {
            // Positional arguments, not the options' values.
            let mut positional = 0;
            let mut iterator = arguments.iter();
            while let Some(argument) = iterator.next() {
                if matches!(
                    argument.as_str(),
                    "--layers"
                        | "--footprints-from"
                        | "--stackup-from"
                        | "--mechanical-from"
                        | "--mechanical-footprint"
                ) {
                    iterator.next();
                } else if !argument.starts_with("--") {
                    positional += 1;
                }
            }
            if positional != 3 {
                return Err(usage());
            }
            for option in ["--footprints-from", "--stackup-from", "--mechanical-from"] {
                if let Some(index) = arguments.iter().position(|argument| argument == option) {
                    check_file(arguments.get(index + 1).ok_or_else(usage)?)?;
                }
            }
            check_file(&arguments[0])?;
            // A schematic needs kicad-cli for its netlist; a netlist's
            // import goes without (no pad check).
            check_kicad(!arguments[0].ends_with(".kicad_sch"))?;
            Ok(())
        }
        _ => Ok(()),
    }
}

/// How the message `repair-kicad-silkscreen` ends with when findings
/// remain starts (the report's path and the next step follow).
pub const SILKSCREEN_INCOMPLETE: &str =
    "silkscreen repair remains incomplete; retained native report and rendering";
/// The message `import-kicad-netlist` ends with when footprints are missing.
pub const IMPORT_INCOMPLETE: &str = "footprints are missing from the board (the report's missing): add their libraries to the schematic project's fp-lib-table (or put the .pretty folders in the output directory or the schematic's folder, or give a board that has them with --footprints-from; for KiCad's own libraries outside the standard install folders, set KICAD10_FOOTPRINT_DIR to its footprints folder) and import again into the same directory (constraints.json and layout.json are kept)";
pub const IMPORT_MECHANICAL_INCOMPLETE: &str = "mechanical transfer is incomplete. See mechanical_from.conflicting_locks and excluded_board_objects in the import report. Resolve the footprint identity or specify each omitted physical feature and its intended electrical connections before layout.";

/// Runs an agent command; `None` when `command` is not one.
/// Diagnostic commands that were removed, with what replaces them: an
/// agent that read of them (an older guide, the 2026-10-18 remapper
/// trial) gets the replacement, not "unknown command".
pub fn removed(command: &str) -> Option<String> {
    let what = match command {
        "inspect-kicad-terminal-access"
        | "inspect-kicad-routing-cut"
        | "inspect-kicad-route-obstructions" => {
            "it inspected the research grid router and read only an outline of one gr_rect or one loop of gr_lines"
        }
        _ => return None,
    };
    Some(format!(
        "{command} was removed on 2026-10-21 ({what}); `pcb-maker inspect-routing-run <run-dir> --terminal REF:PAD` (or --net <net>) names the pad's net, its groups of pads, the rules and KiCad's pairs on any outline the router reads (lines, arcs, rectangles, polygons, circles, curves); no access probe or cut exists (its access and cause say unknown)"
    ))
}

fn repaired_board_check_command(output: &str, board_id: &str, stored_only: bool) -> String {
    let project = Path::new(output);
    let name = project.file_name().unwrap_or_default().to_string_lossy();
    let sibling = |suffix| {
        shell_argument(&path_text(
            &project.with_file_name(format!("{name}{suffix}")),
        ))
    };
    let project = shell_argument(output);
    let board_id = shell_argument(board_id);
    if stored_only {
        let report = sibling("-stored-drc.json");
        format!("pcb-maker drc-kicad-board {project} {board_id} {report} --no-refill")
    } else {
        let report = sibling("-filled-drc.json");
        let filled = sibling("-filled");
        format!("pcb-maker drc-kicad-board {project} {board_id} {report} --save-filled {filled}")
    }
}

fn repair_opens(arguments: &[String]) -> Result<KiCadRunStatus, Failure> {
    let [source, board_id, candidate, output, options @ ..] = arguments else {
        return Err(Failure::Usage(usage_line("repair-kicad-opens").into()));
    };
    if candidate == "--relocate" {
        let (input, net, generated) = match options {
            [donor, flag, net] if flag == "--net" && !net.trim().is_empty() => (donor, net, false),
            [provenance, placed, flag, net]
                if provenance == "--generated-from"
                    && flag == "--net"
                    && !net.trim().is_empty() =>
            {
                (placed, net, true)
            }
            _ => return Err(Failure::Usage(usage_line("repair-kicad-opens").into())),
        };
        check_project(source, board_id)?;
        check_fresh_output(output)?;
        check_kicad(false)?;
        let report = if generated {
            check_project(input, board_id)?;
            pcb_kicad::repair_kicad_generated_open(
                Path::new(source),
                board_id,
                Path::new(input),
                Path::new(output),
                net,
            )
        } else {
            check_file(input)?;
            pcb_kicad::repair_kicad_relocated_open(
                Path::new(source),
                board_id,
                Path::new(input),
                Path::new(output),
                net,
            )
        }
        .map_err(classify)?;
        println!(
            "{}",
            serde_json::to_string_pretty(&report)
                .map_err(|error| Failure::Internal(error.to_string()))?
        );
        return Ok(if report.status == "improved" {
            KiCadRunStatus::Complete
        } else {
            KiCadRunStatus::Incomplete
        });
    }
    if matches!(candidate.as_str(), "--rule-area" | "--local-pour") {
        let [flag, pad] = options else {
            return Err(Failure::Usage(usage_line("repair-kicad-opens").into()));
        };
        if flag != "--pad" {
            return Err(Failure::Usage(usage_line("repair-kicad-opens").into()));
        }
        check_project(source, board_id)?;
        check_fresh_output(output)?;
        check_kicad(false)?;
        let (improved, json) = if candidate == "--rule-area" {
            let router =
                crate::board_router_config(None, source, board_id).map_err(Failure::Usage)?;
            let report = pcb_kicad::repair_kicad_rule_area_open(
                Path::new(source),
                board_id,
                pad,
                Path::new(output),
                &router,
            )
            .map_err(classify)?;
            (
                report.status == "improved",
                serde_json::to_string_pretty(&report),
            )
        } else {
            let report = pcb_kicad::repair_kicad_local_pour(
                Path::new(source),
                board_id,
                Path::new(output),
                pad,
            )
            .map_err(classify)?;
            (
                report.status == "improved",
                serde_json::to_string_pretty(&report),
            )
        };
        println!(
            "{}",
            json.map_err(|error| Failure::Internal(error.to_string()))?
        );
        return Ok(if improved {
            KiCadRunStatus::Complete
        } else {
            KiCadRunStatus::Incomplete
        });
    }
    if candidate == "--plane-escape" {
        let [placed, config] = options else {
            return Err(Failure::Usage(usage_line("repair-kicad-opens").into()));
        };
        check_project(source, board_id)?;
        check_project(placed, board_id)?;
        check_fresh_output(output)?;
        let mut config =
            crate::board_router_config(Some(config), source, board_id).map_err(Failure::Usage)?;
        config.diff_pairs_by_need = true;
        check_kicad(config.skip_native_verification)?;
        calibrate_work_rate(Some(&config))?;
        let report = pcb_kicad::repair_kicad_plane_escape(
            Path::new(source),
            Path::new(placed),
            board_id,
            Path::new(output),
            &config,
        )
        .map_err(classify)?;
        println!(
            "{}",
            serde_json::to_string_pretty(&report)
                .map_err(|error| Failure::Internal(error.to_string()))?
        );
        if report.status == "accepted" {
            eprintln!(
                "verified refilled opens {} to {}; {} raw native errors remain. To verify and save the filled board, run: {}",
                report.opens_before,
                report.opens_after,
                report.native_raw_errors_after.unwrap_or_default(),
                repaired_board_check_command(output, board_id, false),
            );
        }
        return Ok(if report.status == "accepted" {
            KiCadRunStatus::Complete
        } else {
            KiCadRunStatus::Incomplete
        });
    }
    let local = candidate == "--local";
    let mut only_nets = Vec::new();
    for option in options.chunks(2) {
        let [flag, net] = option else {
            return Err(Failure::Usage(usage_line("repair-kicad-opens").into()));
        };
        if flag != "--net" || net.trim().is_empty() {
            return Err(Failure::Usage(usage_line("repair-kicad-opens").into()));
        }
        only_nets.push(net.clone());
    }
    check_project(source, board_id)?;
    check_fresh_output(output)?;
    let report = if local {
        pcb_kicad::repair_kicad_opens_locally(
            Path::new(source),
            board_id,
            Path::new(output),
            &only_nets,
        )
    } else {
        check_file(candidate)?;
        pcb_kicad::repair_kicad_opens_on_nets(
            Path::new(source),
            board_id,
            Path::new(candidate),
            Path::new(output),
            &only_nets,
        )
    }
    .map_err(classify)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&report)
            .map_err(|error| Failure::Internal(error.to_string()))?
    );
    if report.added_copper > 0 {
        if report.native_edge_clearance_ignored {
            eprintln!(
                "KiCad skipped copper_edge_clearance under the project rules. To verify the added copper's distance to Edge.Cuts, enable this check on a project copy and inspect the findings on the added tracks and vias."
            );
        }
        if report.drc_refilled_zones {
            let stored_only = matches!(
                pcb_kicad::project_has_fillable_zones(Path::new(output), board_id),
                Ok(false)
            );
            let command = repaired_board_check_command(output, board_id, stored_only);
            if stored_only {
                eprintln!(
                    "KiCad's open count went from {} to {}. The repaired PCB has no fillable copper zones. Check it as stored with {command}",
                    report.opens_before, report.opens_after,
                );
            } else {
                eprintln!(
                    "With zones refilled, KiCad's open count went from {} to {}. Save and check the filled PCB with {command}",
                    report.opens_before, report.opens_after,
                );
            }
        }
        Ok(KiCadRunStatus::Complete)
    } else {
        Ok(KiCadRunStatus::Incomplete)
    }
}

fn repair_dangling(arguments: &[String]) -> Result<KiCadRunStatus, Failure> {
    let (source, board_id, output, placed) = match arguments {
        [source, board_id, output] => (source, board_id, output, None),
        [source, board_id, output, flag, placed] if flag == "--generated-from" => {
            (source, board_id, output, Some(placed))
        }
        _ => return Err(Failure::Usage(usage_line("repair-kicad-dangling").into())),
    };
    check_project(source, board_id)?;
    check_fresh_output(output)?;
    let (report, removed) = if let Some(placed) = placed {
        check_project(placed, board_id)?;
        let report = pcb_kicad::repair_kicad_generated_dangling(
            Path::new(source),
            board_id,
            Path::new(placed),
            Path::new(output),
        )
        .map_err(classify)?;
        (
            serde_json::to_string_pretty(&report),
            report.removed_track.is_some(),
        )
    } else {
        let report =
            pcb_kicad::repair_kicad_dangling(Path::new(source), board_id, Path::new(output))
                .map_err(classify)?;
        (
            serde_json::to_string_pretty(&report),
            report.removed_track.is_some(),
        )
    };
    println!(
        "{}",
        report.map_err(|error| Failure::Internal(error.to_string()))?
    );
    Ok(if removed {
        KiCadRunStatus::Complete
    } else {
        KiCadRunStatus::Incomplete
    })
}

fn shorten_pad_path(arguments: &[String]) -> Result<KiCadRunStatus, Failure> {
    let command = "shorten-kicad-pad-path";
    let decoupling = arguments
        .last()
        .is_some_and(|flag| flag == "--decoupling-shortcut");
    let arguments = &arguments[..arguments.len() - usize::from(decoupling)];
    let [source, board_id, output, flag, from, to] = arguments else {
        return Err(Failure::Usage(usage_line(command).into()));
    };
    if flag != "--pads" {
        return Err(Failure::Usage(usage_line(command).into()));
    }
    check_project(source, board_id)?;
    check_fresh_output(output)?;
    check_kicad(true)?;
    let shorten = if decoupling {
        pcb_kicad::shorten_kicad_decoupling_path
    } else {
        pcb_kicad::shorten_kicad_pad_path
    };
    let report =
        shorten(Path::new(source), board_id, Path::new(output), [from, to]).map_err(classify)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&report)
            .map_err(|error| Failure::Internal(error.to_string()))?
    );
    Ok(if report.status == "shortened" {
        KiCadRunStatus::Complete
    } else {
        KiCadRunStatus::Incomplete
    })
}

fn add_parallel_feed(arguments: &[String]) -> Result<KiCadRunStatus, Failure> {
    let usage = || Failure::Usage(usage_line("add-kicad-parallel-feed").into());
    let [
        source,
        board_id,
        output,
        pads,
        from,
        to,
        layer_flag,
        layer,
        width_flag,
        width,
    ] = arguments
    else {
        return Err(usage());
    };
    if pads != "--pads" || layer_flag != "--layer" || width_flag != "--width-mm" {
        return Err(usage());
    }
    let width = width.parse().map_err(|_| usage())?;
    check_project(source, board_id)?;
    check_fresh_output(output)?;
    check_kicad(true)?;
    let report = pcb_kicad::add_kicad_parallel_feed(
        Path::new(source),
        board_id,
        Path::new(output),
        [from, to],
        layer,
        width,
    )
    .map_err(classify)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&report)
            .map_err(|error| Failure::Internal(error.to_string()))?
    );
    Ok(if report.status == "added" {
        KiCadRunStatus::Complete
    } else {
        KiCadRunStatus::Incomplete
    })
}

fn repair_via_short(arguments: &[String]) -> Result<KiCadRunStatus, Failure> {
    let [source, board_id, placed, output] = arguments else {
        return Err(Failure::Usage(usage_line("repair-kicad-via-short").into()));
    };
    check_project(source, board_id)?;
    check_project(placed, board_id)?;
    check_fresh_output(output)?;
    check_kicad(true)?;
    let report = pcb_kicad::repair_kicad_via_short(
        Path::new(source),
        board_id,
        Path::new(placed),
        Path::new(output),
        None,
    )
    .map_err(classify)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&report)
            .map_err(|error| Failure::Internal(error.to_string()))?
    );
    Ok(if report.status == "improved" {
        KiCadRunStatus::Complete
    } else {
        KiCadRunStatus::Incomplete
    })
}

fn repair_necks(arguments: Vec<String>) -> Result<KiCadRunStatus, Failure> {
    let usage = || Failure::Usage(usage_line("repair-kicad-necks").into());
    let [source, board_id, options @ ..] = arguments.as_slice() else {
        return Err(usage());
    };
    check_project(source, board_id)?;
    if let [flag, output] = options
        && flag == "--preserve-hatch"
    {
        check_fresh_output(output)?;
        check_kicad(false)?;
        let report = pcb_kicad::repair_kicad_unchanged_hatch_widths(
            Path::new(source),
            board_id,
            Path::new(output),
        )
        .map_err(classify)?;
        println!(
            "{}",
            serde_json::to_string_pretty(&report)
                .map_err(|error| Failure::Internal(error.to_string()))?
        );
        return Ok(if report.published_improvement() {
            KiCadRunStatus::Complete
        } else {
            KiCadRunStatus::Incomplete
        });
    }
    if let [flag, fraction, output] = options
        && flag == "--retain-clipped-hatch"
    {
        let fraction = fraction.parse().map_err(|_| usage())?;
        check_fresh_output(output)?;
        let config =
            crate::lab_commands::board_router_config(None, source, board_id).map_err(classify)?;
        let report = pcb_kicad::repair_kicad_hatched_necks(
            Path::new(source),
            board_id,
            Path::new(output),
            fraction,
            &config,
        )
        .map_err(classify)?;
        println!(
            "{}",
            serde_json::to_string_pretty(&report)
                .map_err(|error| Failure::Internal(error.to_string()))?
        );
        return Ok(if report.published_improvement() {
            KiCadRunStatus::Complete
        } else {
            KiCadRunStatus::Incomplete
        });
    }
    match options {
        [] => {}
        [config] if !config.starts_with("--") => {}
        [flag, output] if flag == "--widen-supplemental" => check_fresh_output(output)?,
        _ => return Err(usage()),
    }
    crate::kicad_commands::cmd_repair_kicad_necks(arguments.into_iter()).map_err(classify)?;
    Ok(KiCadRunStatus::Complete)
}

pub fn run(command: &str, arguments: Vec<String>) -> Option<Result<KiCadRunStatus, Failure>> {
    // The commands that route or run KiCad's check reserve their memory in
    // the machine's ledger (`pcb_kicad::ledger`). A route and a layout hold
    // theirs for minutes; the others (a placement reserves only for its
    // check, the checks themselves) for seconds: their requests are short
    // and never wait behind a drain (`ledger::short_allows`;
    // PCB_MAKER_LEDGER_SHORT overrides).
    if matches!(
        command,
        "route-kicad-board"
            | "place-kicad-board"
            | "drc-kicad-board"
            | "repair-kicad-opens"
            | "repair-kicad-dangling"
            | "repair-kicad-necks"
            | "repair-kicad-via-short"
            | "shorten-kicad-pad-path"
            | "add-kicad-parallel-feed"
    ) || (command == "layout-kicad-board"
        && !arguments
            .iter()
            .any(|argument| argument == "--check-sub-boards"))
        || (command == "inspect-routing-run"
            && arguments
                .iter()
                .any(|argument| argument == "--native-check"))
    {
        let checks_only = !matches!(command, "route-kicad-board" | "layout-kicad-board");
        pcb_kicad::ledger::arm(&ledger_label(command, &arguments), checks_only);
        // A command whose memory is its KiCad checks holds one from before
        // its work (since 2026-10-27); a route's and a layout's checks raise
        // their reservation when they come.
        if checks_only {
            pcb_kicad::ledger::reserve_check_up_front();
        }
    }
    Some(match command {
        "route-kicad-board" => route(arguments),
        "layout-kicad-board" => layout(arguments),
        "layout-kicad-schematic" => schematic_layout::run(arguments),
        "compare-kicad-pour-policies" => pour_policies::compare(arguments),
        "place-kicad-board" => place(arguments),
        "drc-kicad-board" => drc(arguments),
        "inspect-routing-run" => inspect_run(arguments),
        "repair-kicad-opens" => repair_opens(&arguments),
        "repair-kicad-dangling" => repair_dangling(&arguments),
        "repair-kicad-necks" => repair_necks(arguments),
        "repair-kicad-via-short" => repair_via_short(&arguments),
        "shorten-kicad-pad-path" => shorten_pad_path(&arguments),
        "add-kicad-parallel-feed" => add_parallel_feed(&arguments),
        "fab-kicad-board" => fab(arguments),
        "describe-kicad-board"
        | "score-kicad-board"
        | "export-kicad-fab"
        | "repair-kicad-silkscreen"
        | "swap-kicad-pins"
        | "strip-kicad-tracks"
        | "strip-kicad-copper"
        | "import-kicad-netlist" => checked_simple(command, &arguments).and_then(|()| {
            let mut all = vec![command.to_string()];
            all.extend(arguments);
            match crate::run(all) {
                Ok(()) => Ok(KiCadRunStatus::Complete),
                Err(error)
                    if error.starts_with(SILKSCREEN_INCOMPLETE)
                        || matches!(
                            error.as_str(),
                            IMPORT_INCOMPLETE | IMPORT_MECHANICAL_INCOMPLETE
                        ) =>
                {
                    eprintln!("incomplete: {error}");
                    Ok(KiCadRunStatus::Incomplete)
                }
                Err(error) if error.starts_with("usage:") => Err(Failure::Usage(error)),
                Err(error) => Err(classify(error)),
            }
        }),
        _ => return None,
    })
}

/// The label of this run's reservation: the command, the board and where
/// its output goes.
pub(crate) fn ledger_label(command: &str, arguments: &[String]) -> String {
    let mut words = vec![command.to_string()];
    words.extend(
        arguments
            .iter()
            .filter(|argument| !argument.starts_with('-'))
            .skip(1)
            .take(2)
            .cloned(),
    );
    words.join(" ")
}

pub const EXIT_CODES: &str = "\
Exit codes of the commands for agents (the JSON report of a route, layout or placement carries the
same as verdict.status and verdict.exit_code):
  0  complete: the run finished and the result passes: KiCad reports no unconnected item and no
     DRC error that the reference board does not have at the same place (warnings and silkscreen,
     text and library findings do not count). KiCad also finds no missing schematic footprint,
     missing PCB pad for a named schematic net or verified PCB-to-schematic pad net mismatch.
     A placement or layout also places every part and keeps every constraint.
  1  incomplete: the run finished and wrote its result, but connections are open, there are new
     DRC errors or electrical parity findings, constraints are missed or parts found no place.
     The report says which and what to do.
     Also router_complete_unverified: KiCad's check was skipped (PCB_SKIP_NATIVE,
     skip_native_verification) and only the router's own count finds nothing open
  2  usage or input error: bad arguments, a missing or invalid input file or config, unknown keys
     or parts, an output directory that exists; nothing was routed (no files are written unless the
     run had already created its output directory)
  3  incomplete and cut short by the wall clock (verdict.cut): the result depends on the machine's
     load; rerun on a less loaded machine or with a larger budget
  4  internal error or panic: one line on stderr; the report holds verdict.error when the run had
     started (its output directory may hold partial results); also when KiCad checked the result
     but its report is missing or unreadable
  A process killed by a signal (the harness's timeout, out of memory) has no verdict; its exit
  status is the signal's (128 + n in a shell).";

const ROUTE_HELP: &str = "\
Routes a placed KiCad board: every net, with the project's rules, then KiCad's ERC and DRC.

usage: pcb-maker route-kicad-board <project-dir> <board-id> <out-dir> [router-config.json|auto] [--reference <project-dir>|none] [--reuse-routes <previous.kicad_pcb>] [--compare-incumbent <project-dir>] [--strip]

  <project-dir>        the KiCad project directory holding <board-id>.kicad_pcb (and .kicad_pro).
  <board-id>           the board's file stem: myboard for myboard.kicad_pcb.
  <out-dir>            where the routed project goes; must not exist yet.
  router-config.json   router options (JSON); default auto: rules, widths, clearances, vias and net
                       classes from the project's .kicad_pro. A file with options only still takes
                       the rules from the project. Keys (docs/agents.md): seeds (default 1),
                       pours (auto|connect|tracks, default auto), add_pours (default none),
                       automatic_planes (default true), exclusive_planes (default: tried both ways),
                       net_classes (default: the project's), ladder_budget_seconds (default 1200 s
                       of work), deadline_seconds (default none: the work alone ends the run),
                       solid_starved_thermals (default true), via_in_pad (default: no via in
                       an SMD pad but an exposed pad, and a filled and capped via in a BGA's
                       pads no track leaves where the designer's board has vias in that
                       footprint's pads; a list of parts or footprints admits them; true every SMD
                       pad; false none), existing_tracks (default keep; strip as --strip),
                       retained_pad_tips (exact REF:PAD labels for eligible kept launch tracks),\n                       prepared_row_escape_pads (joint short planar launches for consecutive pads).
  --reference <dir>    the project whose board's DRC errors count as pre-existing (the same type at
                       the same place), not as the route's; default <project-dir> (the input as
                       given); none counts every error. Native comparison runs when the result
                       has errors. An explicit directory also supplies the missing-pour comparison.
  --compare-incumbent <dir>
                       compare this alternate route with an existing route of the same unchanged
                       placement and design. Set ladder_budget_seconds in the router JSON. Four
                       native checks use the remaining budget. The verdict lists new findings and
                       open-net changes on both saved and refilled boards. Both projects remain
                       available for selection.
  --strip              remove the board's own tracks and vias first and route from nothing;
                       default: kept (router config existing_tracks: keep or strip).
  --reuse-routes PCB   try retaining earlier copper on unchanged physical pad contacts.
                       KiCad checks all track findings against the current board. A rejected trial
                       routes the current board with the remaining work budget. See route_reuse
                       in board-router.json. Requires the detailed router and the keep policy.

The board's own tracks and vias are kept as they are (pre-routed nets): pads they join count as
connected, other nets keep clear of them, and the router routes only what is open, never ripping
one up (board-router.json's kept_copper counts them). Copper zones stay and are used.
<out-dir> receives the project
with the routed board, KiCad's erc.json and drc.json, and board-router.json: per-net results,
diagnostics (dead_pads, hot_spots, starved_thermals), cut, work_seconds, skipped, and verdict
(status, exit_code, reason, kicad_checked, open_connections, kicad_unconnected_items,
new_findings_strict, preexisting_findings, clean_without_subtraction, repairs, environment, outputs,
next_actions).";

const INSPECT_HELP: &str = "\
Reads a route or layout run's saved outputs and says, per net left open, what is unconnected and
under which rules: no routing, no new search (one lowering of the result board, a second of the
reference's; seconds).

usage: pcb-maker inspect-routing-run <run-dir> [--net <net>] [--terminal <REF:PAD>] [--reference <project-dir>] [--native-check] [--near-paths] [--output <file.json>]
       pcb-maker inspect-routing-run <saved.kicad_pcb> --near-paths [--near-constraints <file.json>] [--net <net>] [--output <file.json>]

  <run-dir>            route-kicad-board's <out-dir> or layout-kicad-board's (its result/): the
                       directory with board-router.json and the routed board.
  --net <net>          only this net, whatever its status; default: every net the router left
                       partial or unreachable.
  --terminal <REF:PAD> only the net of this pad, with the group the pad is in.
  --reference <dir>    the designer's project: its route of each net (layers, widths, vias,
                       whether its copper joins every pad). Default: none.
  --native-check       when the run has no drc.json (KiCad's check skipped), run KiCad's DRC first
                       (cached; written to native-check-drc.json beside the board). Default: off.
  --near-paths         measure saved planar trace paths for the project's authored near-pin entries.
                       Reports layers, transitions and intermediate pads. Requires pcbnew bindings.
  --near-constraints  read near entries from this brief. Defaults to constraints.json beside the PCB.
  --escape-placement DIR
                       with --terminal, screen a short through-layer escape on the original
                       copper-free placed project. A suggested move stays on its current face.
  --output <file>      write the JSON there; default: stdout.

Per net: components (groups of pads the result board's copper joins, pours not counted: ref,
number, uuid, at in mm, copper layers; the largest marked), counts (the router's terminals and
unconnected terminals, the groups and links missing, KiCad's unconnected pairs, explained apart),
native_pairs (each pair's items and the groups they lie in: a pad by its uuid, a track or via by
the copper it lies on), rules (track width and clearance with their sources: class, board minimum,
custom rule; the class's own value below the minimum; the run's overrides; the widths emitted),
reference (connected: true, false, or null where a zone of the net may join its groups),
dead_pads, hot_spots_near (indices into hot_spots), and access and cause: unknown (not measured).
Top level: checks (router, native: not_run|passed|failed, components), attempts (the ladder's
ledger: each attempt's pitch, pours, open, work, negotiation_stop, the selected one;
strategy_history: earlier recorded work and a separate one-rung trial where supported) and hot_spots
(each tile's box, pads with ref, number and net, the nets whose copper crosses it, those still in
conflict; links_from says where they were read). Any outline the router reads is read. Exits 0 when the report was written, 2 on
usage or input errors (no board-router.json, an unknown net or pad), 4 on internal errors.";

const LAYOUT_HELP: &str = "\
Places and routes a KiCad board: parts not locked, not netless and not fixed are placed anew, the
board is routed, parts are moved where nets fight for room, and KiCad checks the result.

usage: pcb-maker layout-kicad-board <project-dir> <board-id> <out-dir> [router-config.json|auto] [layout-config.json] [--reference <project-dir>|none] [--reuse-routes <previous.kicad_pcb>] [--placement-seed N] [--check-sub-boards]

  <project-dir>        the KiCad project directory holding <board-id>.kicad_pcb (and .kicad_pro).
  <board-id>           the board's file stem: myboard for myboard.kicad_pcb.
  <out-dir>            where the result goes; must not exist yet.
  router-config.json   router options as for route-kicad-board; default auto (the project's rules).
  layout-config.json   layout options (JSON); default none: all defaults. Keys: placer (the placer's
                       options, with constraints: a constraints.json path relative to <project-dir>;
                       default none), total_seconds (the budget in seconds of work, default 1500),
                       moves (part moves after the first route, default 40), polish_seconds (default
                       60), swappable / pin_swaps (default none), free_dimension (default none:
                       the outline as given; with {\"edge\": \"bottom\", \"min\": 34, \"max\": 60, \"step\": 2}
                       the board is laid out at several heights, the smallest clean one kept;
                       total_seconds is then the whole search's). docs/agents.md, docs/constraints.md.
  --reference <dir>    the project whose board's DRC errors count as pre-existing (the same type at
                       the same place); default none (the input of a layout is not a placed board):
                       every error counts. On a panel of different boards sharing net names, where
                       its footprints lie also says which board each part belongs to (sub_boards).
                       An explicit directory also supplies the missing-pour comparison.
  --reuse-routes PCB   try legal previous poses and matching prior copper in a bounded trial.
                       KiCad checks the result under the current project rules. If the trial
                       fails, the ordinary layout continues with the remaining work budget.
  --plane-escape-budget SECONDS
                       reserve this part of total_seconds for one protected-escape retry when
                       the normal layout leaves one eligible native open. Ordinary layout uses
                       the rest. Keeps the first board unless the retry passes native checks.
  --terminal-launch-budget SECONDS
                       reserve this part of total_seconds for one reroute with early access
                       on up to two diagnosed open pads. KiCad must verify the recovered
                       board with no opens, no parity issues and no new rule findings.
                       Set terminal_launch_pads in the layout JSON to route from short tips
                       on explicitly selected native-open pads such as U1:7 and U1:8.
  --placement-seed N   start the placement search at this seed instead of the layout config's seed.
                       Keeps the other placement settings, constraints and router settings.
  --check-sub-boards  print the configured sub-board assignment check and next_actions as JSON.
                       Checks the given outline before any size search, placement, routing or DRC.
                       <out-dir> is kept for the actual layout. Exit 1 means an assignment needs
                       review. Exit 0 means this check found no issue, or is disabled (enabled: false).

<out-dir>/result is the finished project (with drc.json and erc.json), <out-dir>/placed the placement
(placement.html shows the selected poses, board-placer.json reports constraints, hints and
utilization), and
<out-dir>/board-layout.json the report: moves, constraints, routed (as board-router.json), quality,
cut, work_seconds and verdict; <out-dir>/verdict.json the verdict alone. When parts find no legal
place the layout stops before routing (exit 1) and writes no board-layout.json: verdict.json holds
unplaced, room (utilization per side, the area missing, an outline to try) and next_actions the
hints (how full each side is, which constraint holds a part). With free_dimension, <out-dir> holds
the chosen size's layout as above, free-dimension.json (every size tried: budget, KiCad unconnected,
errors, work, chosen) and free-dimension/ (each size's project and layout); the verdict adds
free_dimension (chosen, probes, reason).";

const PLACE_HELP: &str = "\
Places a KiCad board's movable parts (no routing): for checking constraints in seconds.

usage: pcb-maker place-kicad-board <project-dir> <board-id> <out-dir> [layout-config.json|placer-config.json] [--reference <project-dir>] [--router <router-config.json|auto>] [--measure-work-rate]

  <project-dir>        the KiCad project directory holding <board-id>.kicad_pcb (and .kicad_pro).
  <board-id>           the board's file stem: myboard for myboard.kicad_pcb.
  <out-dir>            where the placed project goes; must not exist yet.
  layout-config.json   the layout config the board will be laid out with (the layout.json
                       import-kicad-netlist writes): the check places as that layout's placement
                       does (its placer object, the placement's 40 % of total_seconds, the race's
                       seeds). A file holding only the placer object works too (placer-config.json:
                       constraints, a constraints.json path relative to <project-dir> or inline;
                       tight_bodies; docs/constraints.md); default none: all defaults.
  --reference <dir>    the designer's placed project: on a panel of different boards sharing net
                       names, where its footprints lie says which board each part belongs to
                       (verdict.sub_boards); default none: the fixed parts and the nets tell.
  --router <file|auto> the router config the layout will take (default auto: the project's rules):
                       its clearances, and the escape demand's measurements route with it.
  --measure-work-rate  measure the work rates at the start, as a layout does (default: the
                       reference's 3.15e6,150e6 pinned, so that the check repeats on a loaded
                       machine; PCB_WORK_RATE pins others). A layout's verdict gives the rates it
                       ran at: PCB_WORK_RATE=<work_rate>,<placer_work_rate> repeats its placement.

A board with room places in seconds. What cannot fit is an input error at once (exit 2): a region
smaller than its part, a row longer than the outline, parts held at one edge that need more length
than it has. Parts that need more area than a side has, or more than strips of the board hold, are
a no in seconds (exit 1, overfull). A board merely crowded (a side over 90 % full) tries looser
placement levels first, up to the work budget, before it reports unplaced parts: minutes on 80
parts. verdict.json's room gives the numbers: utilization per side, the area missing, the smallest
outline and, when parts did not fit, an outline to try (suggested_outline_mm).

Environment: PCB_PLACER_WORK_SECONDS sets the placement's work budget in seconds of work (default
the layout's placement share of its budget: 600 of the default 1500; 60 gives a quicker no).

<out-dir> receives the placed board, placement.html and board-placer.json: every constraint kept or
missed, unplaced, illegal, hints, utilization, and verdict.";

const FAB_HELP: &str = "\
Releases a board to a fab, gated by KiCad's DRC: exports only when no copper error and no
unconnected item remains.

usage: pcb-maker fab-kicad-board <project-dir> <board-id> <out-dir> [--allow-findings] [--layers <L1,L2,...>] [--profile kicad|jlcpcb] [--no-step] [--input <project-dir>]

  <project-dir>     the KiCad project directory holding <board-id>.kicad_pcb (and .kicad_sch for the
                    ERC, the BOM and the schematic netlist).
  <board-id>        the board's file stem.
  <out-dir>         where the release goes; must not exist yet.
  --allow-findings  export even with copper errors or unconnected items (the verdict lists them).
  --layers L1,...   the Gerber layers; default: every copper layer, both masks, Edge.Cuts, and the
                    pastes and silkscreens something lies on.
  --profile P       kicad (default: kicad-cli's own files) or jlcpcb (every layer of JLCPCB's KiCad
                    guide, plus assembly/<id>-bom-jlcpcb.csv in Comment,Designator,Footprint,
                    JLCPCB Part # and assembly/<id>-cpl-jlcpcb.csv in Designator,Mid X,Mid Y,Layer,
                    Rotation).
  --no-step         skip the 3D model.
  --input <dir>     the project the board was laid out or routed from: only the parts still where
                    its board has them, and its zones and board graphics, are the input's; default:
                    every part, zone and board graphic is the board's own.

The DRC is drc-kicad-board's (cached, zones refilled, without the global libraries); warnings,
excluded and cosmetic findings (silkscreen, text, library) do not block, nor errors that lie
within one footprint each (its pads at its own holes: the manifest's footprint_own, with the
custom rule that accepts them in footprint_note), nor the board's own edge clearances, solder mask
bridges and courtyards with no track or via in them (input_own, input_note; never a short or a
clearance between two nets: docs/agents.md section 4). The ERC (with a schematic)
is counted, never blocking. Released, <out-dir> holds gerbers/ (Gerbers, Excellon drill files PTH
and NPTH, their Gerber maps), <board-id>-gerbers.zip (what the fab's upload takes), assembly/
(<board-id>-pos.csv: kicad-cli's position file, both sides, mm, KiCad's rotations, DNP parts out;
<board-id>-bom.csv: kicad-cli's BOM of the schematic grouped by value, footprint and the symbols'
LCSC field, else the board's footprints), netlist/ (<board-id>.d356, IPC-D-356 for the fab's
electrical test; <board-id>.net from the schematic), <board-id>.step (when kicad-cli exports it;
else listed in skipped), drc.json, erc.json and fab-manifest.json (every file with its sha256,
the DRC and ERC counts, KiCad's version, layers, skipped, without_lcsc). verdict.json carries the
verdict and fab_release. Exits 0 released, 1 refused (nothing exported; next_actions list the
findings), 2 usage or input errors, 4 internal errors (kicad-cli failed).";

/// The commands for agents: name, one line, and the detailed help (when
/// there is more to say than the usage line).
pub const AGENT_COMMANDS: [(&str, &str, &str); 22] = [
    (
        "layout-kicad-schematic",
        "imports a schematic with a supplied mechanical brief and runs ordinary layout",
        schematic_layout::HELP,
    ),
    (
        "import-kicad-netlist",
        "makes a board from a schematic or netlist, with constraints.json and layout.json",
        "usage: pcb-maker import-kicad-netlist <schematic.kicad_sch|netlist.net> <out-dir> <board-id> [--layers N] [--mechanical-from <board.kicad_pcb>] [--mechanical-footprint UUID[=NET]] [--footprints-from <board.kicad_pcb>] [--stackup-from <board.kicad_pcb>] [--near-decoupling] [--keep-footprint-clearances]\n\n  <schematic|netlist>  the KiCad schematic (kicad-cli exports its netlist) or a KiCad .net netlist.\n  <out-dir>            the project directory to write; created if missing. The board, netlist and\n                       schematic are written anew; an existing constraints.json and layout.json are\n                       kept, so a second import (after adding a missing library) keeps your edits.\n  <board-id>           the file stem of the board to write (<out-dir>/<board-id>.kicad_pcb).\n  --layers N           copper layers, an even number of 2 or more. Default: the --stackup-from\n                       board's count, else --mechanical-from, --footprints-from, or 2.\n  --mechanical-from B  transfer board-level Edge.Cuts and rule areas, and preserve poses and\n                       sides of locked footprints matching imported symbols. It also keeps\n                       locked board-only footprints with unnetted NPTHs or Edge.Cuts.\n                       Unheld parts remain movable. Existing constraints and layout are kept, as\n                       are the local PCB outline and rule areas when an outline exists.\n                       See mechanical_from in the import JSON for matches and conflicts.\n  --mechanical-footprint UUID[=NET]\n                       With --mechanical-from, fix this board-only footprint at its native pose.\n                       Repeat for each UUID. Bare UUIDs accept unnetted circular plated holes\n                       and standard same-side circular optical fiducials marked board_only.\n                       Use UUID=NET to retain plated holes already named on that schematic net. Repeat the selection when reimporting.\n  --footprints-from B  a board with these parts (the designer's, an older revision): a footprint\n                       no library has is taken from it (the symbol's own footprint, else one of\n                       that footprint name) and written to a library in <out-dir>; where B has\n                       the symbol's footprint, B's version is used over the library's. Reported\n                       in footprints_from_board, board_versions and drifted_footprints (library\n                       and board differ by more than a fifth); add entries with from_board.\n  --stackup-from B     use this board's thickness and authored fabrication stackup. Its\n                       active copper layers must agree with the stackup and any --layers N.\n                       The JSON stackup_from records the donor checksum and thickness.\n  --near-decoupling    a new constraints.json also holds the decoupling capacitors' near entries\n                       that describe-kicad-board --constraints-template writes (each at a supply pin\n                       of the IC the schematic draws it nearest to; near_entries counts them). Off\n                       by default: measured on the schematic tier (docs/benchmarks.md).\n  --keep-footprint-clearances\n                       keep supplied footprint clearance policy. KiCad still measures own-pad\n                       gaps and reports the governing rule without inserting an exception.\n\nPrints the import report (JSON) on stdout: missing (with where the footprint was looked for),\nsubstituted (another library, KiCad 9's or 10's name, an add-on's nickname), add_on_libraries\n(footprints from KiCad's Plugin and Content Manager folder), relocated_libraries (a table's folder\nfound elsewhere), unmatched_pins and unmatched_pin_details (missing PCB pads, source net and pin\ntype with a next action), footprint_clearance_policy, footprint_clearance_evidence,\nclearances, clearance_conflicts, footprint_findings (KiCad pad\nand drill errors attributed to one imported footprint, with rule evidence and a next action),\nedge_connectors, added (the parts of an existing constraints.json's add, docs/constraints.md), starter (a key matrix found in the netlist: rows,\ncolumns, switches, diodes, halves, unassigned switches; its starter constraints are written to\n<out-dir>/constraints.starter.json, rewritten on every import: copy it to constraints.json to use\nit), and the paths of constraints.json and layout.json (kept: those that existed and were kept). Next step: edit constraints.json (docs/constraints.md), then\npcb-maker layout-kicad-board <out-dir> <board-id> <layout-out-dir> auto <out-dir>/layout.json.\nExits 0 when the import completes, 1 for missing footprints or incomplete mechanical\ntransfer, 2 on usage or input errors (the schematic or netlist unreadable), 4 on internal errors.",
    ),
    (
        "describe-kicad-board",
        "prints the board's parts, pads, nets and swap candidates as JSON",
        "usage: pcb-maker describe-kicad-board <project-dir> <board-id> [--constraints-template <file>]\n\n  <project-dir>  the KiCad project directory holding <board-id>.kicad_pcb.\n  <board-id>     the board's file stem.\n  --constraints-template <file>\n                 also write starter constraints to <file> (a new file): the plug-in connectors\n                 as edge entries (the side they lie at or face on a placed board, else any;\n                 opening_outwards where the footprint shows its mouth), each decoupling\n                 capacitor near the closest supply pin of its IC, and comments saying what to\n                 decide (docs/agents.md, \"Starter constraints for a board\").\n\nPrints JSON on stdout: outline, layers, footprints with pads and nets, nets (each with its class), classes (a table: widths, clearances, nets, the custom rules' widths per layer and closed layers), class_assignment_suggestions (nets in Default matching unused project classes), custom_rules (each .kicad_dru constraint and what the router keeps of it), swap_candidates.",
    ),
    (
        "place-kicad-board",
        "places the movable parts (no routing): checks constraints in seconds",
        PLACE_HELP,
    ),
    (
        "route-kicad-board",
        "routes a placed board and has KiCad check it",
        ROUTE_HELP,
    ),
    (
        "layout-kicad-board",
        "places and routes a board and has KiCad check it",
        LAYOUT_HELP,
    ),
    (
        "compare-kicad-pour-policies",
        "compares explicit pour policies on the same layout brief",
        pour_policies::HELP,
    ),
    (
        "swap-kicad-pins",
        "reassigns interchangeable pins to shorten the ratsnest (board and schematic)",
        "usage: pcb-maker swap-kicad-pins <project-dir> <board-id> <out-dir> <pin-swaps.json> [config.json]\n\n  <project-dir>    the KiCad project directory holding <board-id>.kicad_pcb.\n  <board-id>       the board's file stem.\n  <out-dir>        where the changed project goes; must not exist yet.\n  <pin-swaps.json> which pins may be swapped (docs/pin-swap.md).\n  config.json      swap options (JSON); default none: all defaults.",
    ),
    (
        "repair-kicad-silkscreen",
        "moves silkscreen off pads and other silkscreen, then checks with KiCad",
        "usage: pcb-maker repair-kicad-silkscreen <project-dir> <board-id> <out-dir>\n\n  <project-dir>  the KiCad project directory holding <board-id>.kicad_pcb.\n  <board-id>     the board's file stem.\n  <out-dir>      where the repaired project goes; must not exist yet.\n\nExits 1 when silkscreen findings remain (the native report is kept).",
    ),
    (
        "repair-kicad-opens",
        "repairs native opens against the original project",
        "usage: pcb-maker repair-kicad-opens <routed-dir> <board-id> <candidate.kicad_pcb|--local> <out-dir> [--net <name>]...\n       pcb-maker repair-kicad-opens <routed-dir> <board-id> --relocate <out-dir> <candidate.kicad_pcb> --net <name>\n       pcb-maker repair-kicad-opens <routed-dir> <board-id> --relocate <out-dir> --generated-from <placed-dir> --net <name>\n       pcb-maker repair-kicad-opens <routed-dir> <board-id> --rule-area <out-dir> --pad <REF:PAD>\n       pcb-maker repair-kicad-opens <routed-dir> <board-id> --local-pour <out-dir> --pad <REF:PAD>\n       pcb-maker repair-kicad-opens <routed-dir> <board-id> --plane-escape <out-dir> <placed-dir> <router.json>\n\nUse an alternate route of the same placement, or pass --local to search around\nKiCad's reported opens. Repeat --net to try specific names from the native\nreport. Select both names to try an open differential pair with no routed\ncopper on either conductor. The output is a copy of the source project with\nconfirmed additions, native reports, and open-repair.json.\nRelocation checks one explicit copper edit on a private copy. Keep original\nUUIDs, footprint poses, copper dimensions and outer segment endpoints. Two\nconnected segments may move their shared bend together. Endpoints attached\nto a moved via may follow it. Up to eight unlocked straight tracks or\nthrough vias can move up to 0.25 mm, with up to 128 added segments or vias.\n--generated-from uses the placed project to identify editable generated\ncopper. It searches track bends or tries moving one filled and capped via\nup to 2 mm within its original large land to clear access on the other\nface. The full via ring stays on that land and a lead keeps its junction.\nKiCad must verify fewer opens on the requested net in the stored and refilled\nboard, with the same other findings and native connectivity. The output has\nrelocated-open-repair.json and the original PCB if the trial does not improve.\nRule-area recovery tries an explicit open pad completely covered by a board\ntrack-and-via keepout. It can reseat the footprint and up to two movable\nneighbors, add local copper and save a filled board if both native checks improve.\nThe source project is read only. See rule-area-open-repair.json in the output.\nA local pour tries one bounded zone from an open SMD pad to retained same-net\ncopper where a rule area prohibits tracks and allows fill. It saves the fill\nonly after both native checks and connectivity agree. See local-pour-repair.json in the output.\nPlane escape tries one bounded full reroute of the original copper-free placed\nboard. It protects two nearby plane pads, one identified by KiCad as open,\nand publishes only when every open closes without new native findings.\nSet ladder_budget_seconds in router.json to bound recovery work. The
comparison refills zones in memory; use drc-kicad-board --save-filled for
a verified stored-fill handoff.\nExit 0 means KiCad reported fewer opens with no new findings. Exit 1 means no\nimprovement. Exit 2 means invalid input. Exit 4 means the check or output failed.",
    ),
    (
        "repair-kicad-dangling",
        "tries one native-verified removal of a generated pad stub",
        "usage: pcb-maker repair-kicad-dangling <routed-dir> <board-id> <out-dir> [--generated-from <placed-dir>]\n\nWithout --generated-from, use board-router.json to try removing one generated pad stub named by a native dangling-track error. With --generated-from, supply the same run's placed project to check a sole generated track leaf. KiCad must confirm identical connectivity of all surviving copper and unchanged other findings both as stored and after refill. Exit 0 means the repair was verified and exit 1 means the output is a copy with the original copper. The source is never changed. Read dangling-repair.json for the native check count and time.",
    ),
    (
        "repair-kicad-via-short",
        "jogs one generated track away from a shorting via with native verification",
        "usage: pcb-maker repair-kicad-via-short <routed-dir> <board-id> <placed-dir> <out-dir>\n\nUse the matching pre-route project as placed-dir. The result is a separate project with via-detour.json and fresh stored and refilled KiCad checks. Exit 0 means the short cleared without new findings. Exit 1 keeps the original board in the copy.",
    ),
    (
        "shorten-kicad-pad-path",
        "shortens an explicit same-net pad path with native verification",
        "usage: pcb-maker shorten-kicad-pad-path <project-dir> <board-id> <out-dir> --pads <REF:PAD> <REF:PAD> [--decoupling-shortcut]\n\nShortens a supported straight-track leaf on a routed board with its current schematic. Keeps shared branches and checks the result with KiCad both as stored and with zones refilled. --decoupling-shortcut adds a measured shorter path between a named two-pin capacitor and a schematic power-in pin, on the same or opposite faces. It keeps all original tracks and uses at most one via. The output is a private copy of the source with pad-path.json and native trial reports. Exit 0 means the shorter route passed both checks. Exit 1 means the copy retains the original copper.",
    ),
    (
        "add-kicad-parallel-feed",
        "adds a separate same-net pad-to-pad feed with native verification",
        "usage: pcb-maker add-kicad-parallel-feed <routed-dir> <board-id> <out-dir> --pads <REF:PAD> <REF:PAD> --layer <copper-layer> --width-mm <width>\n\nUse a routed project with its schematic and a fresh output directory. The command searches for one via-free path, avoids taps into other copper of the selected net, and preserves the original board items. Both KiCad checks must report zero opens and no new findings, as stored and with a refill. Exit 0 means the feed passed. Exit 1 retains the original board in the copy. See parallel-feed.json for the search and native evidence.",
    ),
    (
        "repair-kicad-necks",
        "Repair narrow copper connections with KiCad verification",
        "usage: pcb-maker repair-kicad-necks <routed-dir> <board-id> [router-config.json|auto | --widen-supplemental <out-dir> | --retain-clipped-hatch <fraction> <out-dir> | --preserve-hatch <out-dir>]\n\nThe default adds copper in the source project as needed. --widen-supplemental widens recorded repair tracks in a separate project. --retain-clipped-hatch raises the minimum hatch-hole area fraction (strictly between 0 and 1) for one unsmoothed hatch that KiCad names in a width error. It tries widening recorded tracks and bridging a remaining neck. It publishes a separate project, including hatched-neck-repair.json and native evidence.  --preserve-hatch tries localized bridges and small solid copper patches while keeping all original hatch settings. Start from a verified filled PCB. In either hatch mode exit 0 means fewer width errors without added native findings and exit 1 means the copy keeps the original copper.",
    ),
    (
        "score-kicad-board",
        "prints quality measures and the estimated price as JSON",
        "usage: pcb-maker score-kicad-board <project-dir> <board-id> [--stored] [--reference <dir>]\n\n  <project-dir>  the KiCad project directory holding <board-id>.kicad_pcb.\n  <board-id>     the board's file stem.\n  --stored       report stored zone fill polygons and skip the routing lattice.\n  --reference    compare missing copper pours against the board in <dir>.\n\nPrints JSON on stdout (docs/quality.md).",
    ),
    (
        "export-kicad-fab",
        "writes Gerbers, drill files, BOM and placement files, and the price at common fabs",
        "usage: pcb-maker export-kicad-fab <project-dir> <board-id> <out-dir>\n\n  <project-dir>  the KiCad project directory holding <board-id>.kicad_pcb.\n  <board-id>     the board's file stem.\n  <out-dir>      where the fab files go; created if missing.\n\nPrints fab-export.json's content on stdout.",
    ),
    (
        "fab-kicad-board",
        "a DRC-gated fab release: Gerbers, drill, placement, BOM, netlists, STEP, manifest and zip",
        FAB_HELP,
    ),
    (
        "drc-kicad-board",
        "runs KiCad's DRC on a board (cached) and writes its JSON report",
        "usage: pcb-maker drc-kicad-board <project-dir> <board-id> <report.json> [--no-refill] [--input <project-dir>] [--stitch-zone-island <out-dir> | --save-filled <out-dir> | --localize-zone-widths]\n\n  <project-dir>  the KiCad project directory holding <board-id>.kicad_pcb.\n  <board-id>     the board's file stem.\n  <report.json>  where KiCad's DRC report (JSON, every severity) goes; its directory is created.\n  --no-refill    check the zones as stored; default: refilled first, as KiCad's fab outputs do.\n  --input <dir>  the project the board was laid out or routed from: only the parts still where\n                 its board has them, and its zones and board graphics, are the input's;\n                 default: every part, zone and board graphic is the board's own.\n  --stitch-zone-island <out-dir>  try one through via for a single open between islands of one zone.\n                 A verified improvement goes in a new project directory beside a repair report.\n  --save-filled <out-dir>  save KiCad's refilled board in a fresh copy of the project.\n                 Use separate report and output paths outside the source project. KiCad\n                 checks the saved file without refill and includes parity when a schematic\n                 exists. Exit 0 means no native errors, opens or parity findings. Exit 1\n                 delivers a verified copy with remaining findings. Exit 4 means the save\n                 or verification failed. Use this option on its own.\n  --localize-zone-widths  Locate a zone-only width error using saved copper and a private\n                 native witness check. Requires KiCad Python bindings. Confirmed positions\n                 appear in zone_width_localization. Use on its own with the report outside\n                 the source project.\n\nWithout --save-filled the board and its rules (.kicad_pro, .kicad_dru) are checked without the schematic, as the\nreference board is. The DRC cache answers a board it has seen (docs/agents.md). Exits 0 when\nKiCad finds no error that counts (warnings, excluded and cosmetic findings do not, nor errors\nthat lie within one footprint each, nor the input's own edge clearances, solder mask bridges\nand courtyards with no track or via in them, docs/agents.md section 4) and no unconnected\nitem, 1 when it does; one line on stdout counts them. Then \"next: ...\" lines, in a verdict's\norder: when it fails and the project has rules the router does not keep (creepage, lengths,\nrules on areas), a line names them; last the input's own findings, each line starting\n\"next: nothing to do (the input's): \" (the footprints' own with the custom rule that accepts\nthem, then the board's own; until 2026-11-04 the lines \"footprint: ...\" and \"input: ...\").",
    ),
    (
        "inspect-routing-run",
        "names the pads, rules and KiCad's pairs of the nets a run left open, from its saved outputs",
        INSPECT_HELP,
    ),
    (
        "strip-kicad-tracks",
        "removes tracks and vias from a board (copper zones stay)",
        "usage: pcb-maker strip-kicad-tracks <source.kicad_pcb> <output.kicad_pcb> [--keep-nets <net>[,<net>...]]\n\n  <source.kicad_pcb>  the board to read.\n  <output.kicad_pcb>  the board to write (may be the source).\n  --keep-nets N       keep the tracks, arcs and vias of these nets (KiCad wildcards, the sheet\n                      path's leading / optional: 'DVP_*'), so that route-kicad-board keeps them\n                      and routes the rest (a designer's length-matched bus).",
    ),
    (
        "strip-kicad-copper",
        "removes tracks, vias, copper zones and copper graphics from a board",
        "usage: pcb-maker strip-kicad-copper <source.kicad_pcb> <output.kicad_pcb> [report.json]\n\n  <source.kicad_pcb>  the board to read.\n  <output.kicad_pcb>  the board to write.\n  report.json         where the removal counts go; default: printed on stdout.",
    ),
];

/// The usage line of an agent command.
pub fn usage_line(command: &str) -> &'static str {
    AGENT_COMMANDS
        .iter()
        .find(|(name, ..)| *name == command)
        .and_then(|(_, _, help)| help.lines().find(|line| line.starts_with("usage:")))
        .unwrap_or("")
}

/// The detailed help of an agent command.
pub fn help(command: &str) -> Option<String> {
    AGENT_COMMANDS
        .iter()
        .find(|(name, ..)| *name == command)
        .map(|(_, _, help)| {
            if command == "repair-kicad-opens" {
                help.to_string()
            } else {
                format!("{help}\n\n{EXIT_CODES}")
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_repair_advice_uses_the_stored_check_only_for_boards_without_pours() {
        let path = "/tmp/out/repair result";
        let id = "example board";
        let stored = repaired_board_check_command(path, id, true);
        assert!(stored.ends_with("'/tmp/out/repair result-stored-drc.json' --no-refill"));
        let filled = repaired_board_check_command(path, id, false);
        assert!(filled.ends_with(
            "'/tmp/out/repair result-filled-drc.json' --save-filled '/tmp/out/repair result-filled'"
        ));
    }

    #[test]
    fn explicit_pad_near_reports_strict_misses_and_retries_only_movable_sources() {
        let statuses =
            serde_json::from_value::<Vec<pcb_kicad::KiCadConstraintStatus>>(serde_json::json!([
                {"kind":"near","part":"C2","other":"U2","satisfied":true,"violation_mm":0.0,
                 "pad_near":{"pad":"1","pin_of":"U2:22","distance_mm":6.005814,"max_mm":6.0,
                             "overage_mm":0.005814,"satisfied":false,"refinable":true}},
                {"kind":"near","part":"C3","other":"U5","satisfied":false,"violation_mm":0.176,
                 "pad_near":{"pad":"1","pin_of":"U5:16","distance_mm":6.176026,"max_mm":6.0,
                             "overage_mm":0.176026,"satisfied":false,"refinable":false}},
                {"kind":"near","part":"C4","other":"U4","satisfied":true,"violation_mm":0.0},
                {"kind":"near","part":"C5","other":"U1","satisfied":false,"violation_mm":0.19},
                {"kind":"back","part":"C6","satisfied":true,"violation_mm":0.0},
                {"kind":"pad_near_objective","part":"C1","other":"U1","satisfied":true,"violation_mm":0.0,
                 "pad_near":{"pad":"1","pin_of":"U1:3","distance_mm":5.271,"measured":true,"refinable":true}}
            ]))
            .unwrap();
        let mut facts = KiCadRunFacts::new("layout-kicad-board");
        note_placement_constraints(&mut facts, &statuses, false);
        assert_eq!(
            facts.constraints_missed,
            [
                "near C2:1 to U2:22 (6.005814 mm, limit 6 mm, overage 0.005814 mm)",
                "near C3:1 to U5:16 (6.176026 mm, limit 6 mm, overage 0.176026 mm)",
                "near C5 U1 (0.19 mm)",
            ]
        );
        assert_eq!(facts.strict_pad_near_misses, 2);
        assert_eq!(facts.refinable_pad_near_misses, ["C2:1 to U2:22"]);
        assert_eq!(
            facts
                .decoupling_constrained
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["C1", "C2", "C3", "C4", "C5"]
        );
        note_placement_constraints(&mut facts, &statuses, true);
        assert_eq!(facts.strict_pad_near_misses, 2);
        assert!(facts.refinable_pad_near_misses.is_empty());
    }

    #[test]
    fn the_placers_overlap_warning_is_not_repeated_after_the_input_checks() {
        let input = vec![
            "fixed parts J1 and R31 overlap (pads on *.Cu): the placer keeps both ...".to_string(),
        ];
        let placed = serde_json::json!({"constraint_warnings": [
            "fixed parts J1 and R31 overlap",
            "fixed parts J1 and M1 overlap"
        ]});
        assert_eq!(
            constraint_warnings(input.clone(), &placed),
            [
                input[0].clone(),
                "fixed parts J1 and M1 overlap".to_string()
            ]
        );
    }

    #[test]
    fn every_agent_command_has_a_usage_line() {
        for (name, ..) in AGENT_COMMANDS {
            assert!(
                usage_line(name).starts_with(&format!("usage: pcb-maker {name} ")),
                "{name}"
            );
        }
    }

    #[test]
    fn flags_are_split_from_positional_arguments() {
        let arguments = ["a", "--reference", "r", "b"]
            .iter()
            .map(|text| text.to_string())
            .collect();
        let Ok((positional, values)) = split_flags(arguments, &["--reference"], "usage") else {
            panic!()
        };
        assert_eq!(positional, vec!["a", "b"]);
        assert_eq!(values.get("--reference").map(String::as_str), Some("r"));
        assert!(matches!(
            split_flags(vec!["--bogus".into()], &["--reference"], "usage"),
            Err(Failure::Usage(_))
        ));
        assert!(matches!(
            split_flags(vec!["--reference".into()], &["--reference"], "usage"),
            Err(Failure::Usage(_))
        ));
    }

    #[test]
    fn a_placement_retry_starts_outside_the_seed_pool() {
        assert_eq!(alternate_placement_seed(7, 12), 101);
        assert_eq!(alternate_placement_seed(100, 12), 112);
        assert_eq!(alternate_placement_seed(u64::MAX - 1, 4), 2);
    }

    #[test]
    fn a_placement_retry_shell_command_preserves_arguments() {
        let positional = [
            "original project",
            "board's id",
            "layout result",
            "router settings.json",
            "layout's settings.json",
        ]
        .map(str::to_string);
        let output = Path::new(&positional[2]);
        let reference = "reference's project";
        let reuse = "previous board's layout.kicad_pcb";
        let flags = BTreeMap::from([
            ("--reference".into(), reference.into()),
            ("--reuse-routes".into(), reuse.into()),
            ("--plane-escape-budget".into(), "90".into()),
        ]);
        let action = placement_retry_action(&positional, &flags, output, 101).unwrap();
        let command = action.split_once(": ").unwrap().1;
        let shell = std::process::Command::new("sh")
            .args(["-c", &format!("set -- {command}; printf '%s\\n' \"$@\"")])
            .output()
            .unwrap();
        assert!(shell.status.success());
        assert_eq!(
            String::from_utf8(shell.stdout)
                .unwrap()
                .lines()
                .collect::<Vec<_>>(),
            [
                "pcb-maker",
                "layout-kicad-board",
                &path_text(Path::new(&positional[0])),
                &positional[1],
                &path_text(Path::new("layout result-placement-101")),
                &path_text(Path::new(&positional[3])),
                &path_text(Path::new(&positional[4])),
                "--reference",
                &path_text(Path::new(reference)),
                "--reuse-routes",
                &path_text(Path::new(reuse)),
                "--plane-escape-budget",
                "90",
                "--placement-seed",
                "101",
            ]
        );
        let terminal_flags = BTreeMap::from([
            ("--reference".into(), "none".into()),
            ("--terminal-launch-budget".into(), "190".into()),
        ]);
        let action = placement_retry_action(&positional, &terminal_flags, output, 101).unwrap();
        assert!(
            action
                .ends_with(" --reference none --terminal-launch-budget '190' --placement-seed 101")
        );
    }
}
