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
        Ok((errors, findings)) => KiCadReference::Findings {
            from: project.to_path_buf(),
            errors,
            findings,
        },
        Err(error) => KiCadReference::Failed {
            from: project.to_path_buf(),
            error: error.lines().next().unwrap_or("").to_string(),
        },
    };
}

fn missed_constraints(constraints: &[pcb_kicad::KiCadConstraintStatus]) -> Vec<String> {
    constraints
        .iter()
        .filter(|status| !status.satisfied)
        .map(|status| match &status.other {
            Some(other) => format!(
                "{} {} {} ({:.2} mm)",
                status.kind, status.part, other, status.violation_mm
            ),
            None => format!(
                "{} {} ({:.2} mm)",
                status.kind, status.part, status.violation_mm
            ),
        })
        .collect()
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

const ROUTE_USAGE: &str = "usage: pcb-maker route-kicad-board <project-dir> <board-id> <out-dir> [router-config.json|auto] [--reference <project-dir>|none] [--strip]";
const LAYOUT_USAGE: &str = "usage: pcb-maker layout-kicad-board <project-dir> <board-id> <out-dir> [router-config.json|auto] [layout-config.json] [--reference <project-dir>|none]";
/// place-kicad-board's default work budget: the layout's placement share
/// of its default budget.
const PLACE_WORK_SECONDS: f64 = 600.0;

const PLACE_USAGE: &str =
    "usage: pcb-maker place-kicad-board <project-dir> <board-id> <out-dir> [placer-config.json]";

fn route(arguments: Vec<String>) -> Result<KiCadRunStatus, Failure> {
    let command = "route-kicad-board";
    // --strip: the board's own tracks and vias go first (default: kept).
    let strip = arguments.iter().any(|argument| argument == "--strip");
    let arguments = arguments
        .into_iter()
        .filter(|argument| argument != "--strip")
        .collect();
    let (positional, values) = split_flags(arguments, &["--reference"], ROUTE_USAGE)?;
    positional_count(&positional, 3..=4, ROUTE_USAGE)?;
    let (source, board_id, output) = (&positional[0], &positional[1], &positional[2]);
    check_project(source, board_id)?;
    check_fresh_output(output)?;
    let reference = reference_option(&values, Some(source), board_id)?;
    let mut config =
        crate::board_router_config(positional.get(3).map(String::as_str), source, board_id)
            .map_err(Failure::Usage)?;
    if strip {
        config.existing_tracks = pcb_kicad::KiCadExistingTracks::Strip;
    }
    let output = Path::new(output);
    let outputs = BTreeMap::from([
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
    let report = output.join("board-router.json");
    set_target(command, report.clone(), outputs.clone());
    let kicad_warning = check_kicad(config.skip_native_verification)?;
    calibrate_work_rate(Some(&config))?;
    set_target(command, report.clone(), outputs.clone());
    // Test hook: a panic once the run has started (tests/cli_exit_codes.rs).
    if std::env::var_os("PCB_MAKER_TEST_PANIC").is_some() {
        panic!("PCB_MAKER_TEST_PANIC is set");
    }
    let started = Instant::now();
    let result = pcb_kicad::route_kicad_board(Path::new(source), board_id, output, &config)
        .map_err(classify)?;
    println!(
        "routed {}/{} connections ({} unconnected terminals), pours {}, {} vias, {:.1} mm, {} iterations; lowering {:.2}s routing {:.2}s internal check {:.2}s ({} violations) native {:.2}s complete={}",
        result.routed_connections,
        result.routable_connections,
        result.unconnected_terminals,
        result.pours,
        result.vias,
        result.length_mm,
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
    facts.work_seconds = Some(result.work_seconds);
    if let Some(accounting) = &result.wall_accounting {
        let charged = &accounting.charged;
        facts.pour_work_seconds = Some(charged.pour_seconds);
        facts.lattice_work_seconds = Some(charged.lattice_seconds + charged.lowering_seconds);
    }
    facts.outputs = existing(&outputs);
    facts.input_warnings.extend(kicad_warning);
    check_reference(
        &mut facts,
        reference.as_deref(),
        board_id,
        &output.join("reference-drc.json"),
    );
    facts.wall_seconds = started.elapsed().as_secs_f64();
    let verdict = pcb_kicad::judge(&facts);
    write_verdict(output, Some(&report), &verdict).map_err(Failure::Internal)?;
    announce(&verdict, &output.join("verdict.json"));
    Ok(verdict.status)
}

fn print_layout(result: &pcb_kicad::KiCadBoardLayoutResult) -> Result<(), String> {
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
    println!(
        "final: routed {}/{} connections, {} vias, {:.1} mm, {}",
        result.routed.routed_connections,
        result.routed.routable_connections,
        result.routed.vias,
        result.routed.length_mm,
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
    let missed = missed_constraints(&result.constraints);
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
                "decoupling: capacitors a median {} mm from a supply pin, {} within 3 mm",
                decoupling["capacitor_median_mm"], decoupling["capacitors_within_3mm"]
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
                println!(
                    "{label} {}: {} mm from its pins",
                    name.as_str().unwrap_or("?"),
                    distance
                );
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
) -> Result<Vec<String>, Failure> {
    if let Some(pcb_kicad::KiCadConstraintsSource::File(path)) = &placer.constraints {
        pcb_kicad::read_placement_constraints(&Path::new(source).join(path))
            .map_err(Failure::Usage)?;
    }
    let warnings = pcb_kicad::placement_input_warnings(Path::new(source), board_id, placer)
        .map_err(Failure::Usage)?;
    for warning in &warnings {
        eprintln!("warning: {warning}");
    }
    Ok(warnings)
}

/// The placement's facts from its report (`board-placer.json`).
fn placement_facts(command: &str, report: &serde_json::Value) -> KiCadRunFacts {
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
    facts.unplaced = strings("unplaced");
    for part in strings("illegal") {
        if !facts.unplaced.contains(&part) {
            facts.unplaced.push(part);
        }
    }
    facts.placement_hints = strings("hints");
    let constraints: Vec<pcb_kicad::KiCadConstraintStatus> =
        serde_json::from_value(report["constraints"].clone()).unwrap_or_default();
    facts.constraints_missed = missed_constraints(&constraints);
    facts.work_seconds = report["work_seconds"].as_f64();
    facts.cut = pcb_kicad::wall_cuts();
    facts.room = serde_json::from_value(report["room"].clone()).ok();
    facts
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
        if !warnings.iter().any(|known| known == warning) {
            warnings.push(warning.to_string());
        }
    }
    warnings
}

fn layout(arguments: Vec<String>) -> Result<KiCadRunStatus, Failure> {
    let command = "layout-kicad-board";
    let (positional, values) = split_flags(arguments, &["--reference"], LAYOUT_USAGE)?;
    positional_count(&positional, 3..=5, LAYOUT_USAGE)?;
    let (source, board_id, output) = (&positional[0], &positional[1], &positional[2]);
    check_project(source, board_id)?;
    check_fresh_output(output)?;
    let reference = reference_option(&values, None, board_id)?;
    let router =
        crate::board_router_config(positional.get(3).map(String::as_str), source, board_id)
            .map_err(Failure::Usage)?;
    let config: pcb_kicad::KiCadBoardLayoutConfig = match positional.get(4) {
        Some(path) => read_json(path)?,
        None => Default::default(),
    };
    let mut input_warnings = check_constraints(source, board_id, &config.placer)?;
    set_target(
        command,
        Path::new(output).join("board-layout.json"),
        BTreeMap::from([(
            "verdict".to_string(),
            path_text(&Path::new(output).join("verdict.json")),
        )]),
    );
    input_warnings.extend(check_kicad(router.skip_native_verification)?);
    // A free dimension: the layout at several sizes (src/agent_cli/free_dimension.rs).
    if let Some(free) = &config.free_dimension {
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
            let mut facts = placement_facts(command, &placed);
            facts.input_warnings = constraint_warnings(input_warnings, &placed);
            facts.outputs = existing(&outputs);
            facts.wall_seconds = started.elapsed().as_secs_f64();
            let mut verdict = pcb_kicad::judge(&facts);
            verdict.error = Some(error);
            // No board-layout.json: there is no layout (scripts take its
            // absence for a layout that did not finish).
            write_verdict(output, None, &verdict).map_err(Failure::Internal)?;
            announce(&verdict, &output.join("verdict.json"));
            return Ok(verdict.status);
        }
    };
    print_layout(&result).map_err(Failure::Internal)?;
    let mut facts = pcb_kicad::route_facts(command, &result.routed, &result_directory)
        .map_err(Failure::Internal)?;
    facts.constraints_missed = missed_constraints(&result.constraints);
    facts.board_mark_overlaps = result.labels.overlaps.clone();
    // The placement's room (the parts and the outline the layout kept)
    // and what it warned of.
    let placed = fs::read_to_string(output.join("placed/board-placer.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .unwrap_or_default();
    facts.room = serde_json::from_value(placed["room"].clone()).ok();
    facts.input_warnings = constraint_warnings(input_warnings, &placed);
    facts.cut = result.cut.clone();
    facts.work_seconds = Some(result.work_seconds + result.routed.work_seconds);
    facts.kicad_check_work_seconds = Some(result.kicad_check_work_seconds);
    facts.outputs = existing(&outputs);
    check_reference(
        &mut facts,
        reference.as_deref(),
        board_id,
        &output.join("reference-drc.json"),
    );
    facts.wall_seconds = started.elapsed().as_secs_f64();
    let verdict = pcb_kicad::judge(&facts);
    write_verdict(output, Some(&report), &verdict).map_err(Failure::Internal)?;
    announce(&verdict, &output.join("verdict.json"));
    Ok(verdict.status)
}

fn place(arguments: Vec<String>) -> Result<KiCadRunStatus, Failure> {
    let command = "place-kicad-board";
    let (positional, _) = split_flags(arguments, &[], PLACE_USAGE)?;
    positional_count(&positional, 3..=4, PLACE_USAGE)?;
    let (source, board_id, output) = (&positional[0], &positional[1], &positional[2]);
    check_project(source, board_id)?;
    check_fresh_output(output)?;
    let mut config: pcb_kicad::KiCadBoardPlacerConfig = match positional.get(3) {
        // A layout config (import-kicad-netlist's layout.json) works too:
        // its placer object is the placement's config.
        Some(path) => match read_json::<serde_json::Value>(path)? {
            serde_json::Value::Object(mut layout) if layout.contains_key("placer") => {
                serde_json::from_value(layout.remove("placer").unwrap_or_default()).map_err(
                    |error| Failure::Usage(format!("failed to parse {path}: placer: {error}")),
                )?
            }
            value => serde_json::from_value(value)
                .map_err(|error| Failure::Usage(format!("failed to parse {path}: {error}")))?,
        },
        None => Default::default(),
    };
    let mut input_warnings = check_constraints(source, board_id, &config)?;
    // A placement needs no KiCad, but what comes after it does.
    input_warnings.extend(check_kicad(true)?);
    // The layout's placement budget by default (40 % of its 1500 s of
    // work), so that the check places as the layout would and ends: a
    // board without room ran the placer's relaxation levels for over 12
    // minutes without one. PCB_PLACER_WORK_SECONDS sets another.
    config.work_seconds = Some(
        std::env::var("PCB_PLACER_WORK_SECONDS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(PLACE_WORK_SECONDS),
    );
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
    calibrate_work_rate(None)?;
    let report = output.join("board-placer.json");
    set_target(command, report.clone(), outputs.clone());
    let started = Instant::now();
    let result = pcb_kicad::place_kicad_board(Path::new(source), board_id, output, &config)
        .map_err(classify)?;
    println!(
        "placed {} of {} components ({} nets): wirelength source {:.0} mm, global {:.0}, legal {:.0}, final {:.0}; {} global iterations, overflow {:.3}; unplaced {:?}, illegal {:?}; {:.2}s",
        result.movable,
        result.components,
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
    let mut facts = placement_facts(command, &placed);
    facts.input_warnings = constraint_warnings(input_warnings, &placed);
    facts.outputs = existing(&outputs);
    facts.wall_seconds = started.elapsed().as_secs_f64();
    let verdict = pcb_kicad::judge(&facts);
    write_verdict(output, Some(&report), &verdict).map_err(Failure::Internal)?;
    announce(&verdict, &output.join("verdict.json"));
    Ok(verdict.status)
}

const DRC_USAGE: &str =
    "usage: pcb-maker drc-kicad-board <project-dir> <board-id> <report.json> [--no-refill]";

/// KiCad's DRC of a board, through the DRC cache: complete when KiCad
/// finds no error that counts and no unconnected item.
fn drc(arguments: Vec<String>) -> Result<KiCadRunStatus, Failure> {
    let refill = !arguments.iter().any(|argument| argument == "--no-refill");
    let arguments = arguments
        .into_iter()
        .filter(|argument| argument != "--no-refill")
        .collect();
    let (positional, _) = split_flags(arguments, &[], DRC_USAGE)?;
    positional_count(&positional, 3..=3, DRC_USAGE)?;
    let (source, board_id, report) = (&positional[0], &positional[1], Path::new(&positional[2]));
    check_project(source, board_id)?;
    check_kicad(false)?;
    if let Some(parent) = report
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|error| {
            Failure::Usage(format!("failed to create {}: {error}", parent.display()))
        })?;
    }
    let value = pcb_kicad::kicad_drc_report(Path::new(source), board_id, report, refill)
        .map_err(Failure::Internal)?;
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
        path_text(report)
    );
    // The project's rules the router does not keep: KiCad checks them, so
    // their findings are for the designer (board-router.json's rules has
    // every rule read).
    let failed = !tally.errors.is_empty() || tally.unconnected > 0;
    if failed
        && let Ok(config) = pcb_kicad::resolve_project_rules(
            &Path::new(source).join(format!("{board_id}.kicad_pro")),
            &Path::new(source).join(format!("{board_id}.kicad_pcb")),
        )
        && let Some(project) = config.project_rules
        && !project.unmodelled.is_empty()
    {
        println!(
            "next: the project has rules the router does not keep, which KiCad checks: {}",
            project.unmodelled.join("; ")
        );
    }
    Ok(if !failed {
        KiCadRunStatus::Complete
    } else {
        KiCadRunStatus::Incomplete
    })
}

const FAB_USAGE: &str = "usage: pcb-maker fab-kicad-board <project-dir> <board-id> <out-dir> [--allow-findings] [--layers <L1,L2,...>] [--profile kicad|jlcpcb] [--no-step]";

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
    let (positional, values) = split_flags(arguments, &["--layers", "--profile"], FAB_USAGE)?;
    positional_count(&positional, 3..=3, FAB_USAGE)?;
    let (source, board_id, output) = (&positional[0], &positional[1], &positional[2]);
    check_project(source, board_id)?;
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
    facts.outputs = existing(&outputs);
    facts.wall_seconds = started.elapsed().as_secs_f64();
    let mut verdict = pcb_kicad::judge(&facts);
    let drc = &manifest.drc;
    let errors: usize = drc.errors.values().sum();
    let counts = format!(
        "KiCad's DRC: {errors} errors{}, {} unconnected items, {} warnings, {} cosmetic",
        if drc.errors.is_empty() {
            String::new()
        } else {
            format!(
                " ({})",
                drc.errors
                    .iter()
                    .map(|(kind, count)| format!("{kind} {count}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        },
        drc.unconnected_items,
        drc.warnings,
        drc.cosmetic
    );
    let status = if manifest.released {
        KiCadRunStatus::Complete
    } else {
        KiCadRunStatus::Incomplete
    };
    verdict.status = status;
    verdict.exit_code = status.exit_code();
    verdict.next_actions.clear();
    if manifest.released {
        verdict.reason = format!(
            "released{}: {} files, {}; {counts}",
            if drc.findings_allowed {
                " with findings (--allow-findings)"
            } else {
                ""
            },
            manifest.files.len(),
            manifest.zip.as_deref().unwrap_or("no zip")
        );
        if drc.findings_allowed {
            verdict.next_actions.push(format!(
                "the release went ahead with KiCad's findings (--allow-findings): {counts}; make sure the fab accepts them"
            ));
            verdict
                .next_actions
                .extend(manifest.findings.iter().cloned());
        }
        if !manifest.without_lcsc.is_empty() {
            let shown = manifest.without_lcsc.len().min(10);
            verdict.next_actions.push(format!(
                "{} parts have no LCSC part number, which JLCPCB's assembly needs (an LCSC field on the symbol, then Update PCB from Schematic): {}{} (fab-manifest.json's without_lcsc lists them)",
                manifest.without_lcsc.len(),
                manifest.without_lcsc[..shown].join(", "),
                if shown < manifest.without_lcsc.len() { ", ..." } else { "" }
            ));
        }
        verdict.next_actions.push(
            "check the rotations of polarised parts in the fab's placement preview: library orientations differ between KiCad and assemblers".into(),
        );
    } else {
        verdict.reason = format!("refused: {counts}; nothing was exported");
        verdict.next_actions.push(
            "fix the findings (route the open connections, move what is too close) and release again; --allow-findings exports anyway"
                .into(),
        );
        verdict
            .next_actions
            .extend(manifest.findings.iter().cloned());
    }
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
        "describe-kicad-board" | "score-kicad-board" => {
            count(2..=2)?;
            check_project(&arguments[0], &arguments[1])
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
            count(2..=2)?;
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
                if matches!(argument.as_str(), "--layers" | "--footprints-from") {
                    iterator.next();
                } else if !argument.starts_with("--") {
                    positional += 1;
                }
            }
            if positional != 3 {
                return Err(usage());
            }
            if let Some(index) = arguments
                .iter()
                .position(|argument| argument == "--footprints-from")
                && let Some(board) = arguments.get(index + 1)
            {
                check_file(board)?;
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
pub const IMPORT_INCOMPLETE: &str = "footprints are missing from the board (the report's missing): add their libraries to the project's fp-lib-table (or put the .pretty folders in the project, or give a board that has them with --footprints-from; for KiCad's own libraries outside the standard install folders, set KICAD10_FOOTPRINT_DIR to its footprints folder) and import again into the same directory (constraints.json and layout.json are kept)";

/// Runs an agent command; `None` when `command` is not one.
pub fn run(command: &str, arguments: Vec<String>) -> Option<Result<KiCadRunStatus, Failure>> {
    Some(match command {
        "route-kicad-board" => route(arguments),
        "layout-kicad-board" => layout(arguments),
        "place-kicad-board" => place(arguments),
        "drc-kicad-board" => drc(arguments),
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
                    if error.starts_with(SILKSCREEN_INCOMPLETE) || error == IMPORT_INCOMPLETE =>
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

pub const EXIT_CODES: &str = "\
Exit codes of the commands for agents (the JSON report of a route, layout or placement carries the
same as verdict.status and verdict.exit_code):
  0  complete: the run finished and the result passes: KiCad reports no unconnected item and no
     DRC error that the reference board does not have at the same place (warnings and silkscreen,
     text and library findings do not count); a placement or layout also places every part and
     keeps every constraint
  1  incomplete: the run finished and wrote its result, but connections are open, there are new
     DRC errors, constraints are missed or parts found no place; the report says which and what to do.
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

usage: pcb-maker route-kicad-board <project-dir> <board-id> <out-dir> [router-config.json|auto] [--reference <project-dir>|none] [--strip]

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
                       solid_starved_thermals (default true), via_in_pad (default false: no
                       via in an SMD pad but an exposed pad; true for filled and capped vias),
                       existing_tracks (default keep; strip as --strip).
  --reference <dir>    the project whose board's DRC errors count as pre-existing (the same type at
                       the same place), not as the route's; default <project-dir> (the input as
                       given); none counts every error. Checked only when the result has errors
                       (one more KiCad DRC).
  --strip              remove the board's own tracks and vias first and route from nothing;
                       default: kept (router config existing_tracks: keep or strip).

The board's own tracks and vias are kept as they are (pre-routed nets): pads they join count as
connected, other nets keep clear of them, and the router routes only what is open, never ripping
one up (board-router.json's kept_copper counts them). Copper zones stay and are used.
<out-dir> receives the project
with the routed board, KiCad's erc.json and drc.json, and board-router.json: per-net results,
diagnostics (dead_pads, hot_spots, starved_thermals), cut, work_seconds, skipped, and verdict
(status, exit_code, reason, kicad_checked, open_connections, kicad_unconnected_items,
new_findings_strict, preexisting_findings, clean_without_subtraction, repairs, environment, outputs,
next_actions).";

const LAYOUT_HELP: &str = "\
Places and routes a KiCad board: parts not locked, not netless and not fixed are placed anew, the
board is routed, parts are moved where nets fight for room, and KiCad checks the result.

usage: pcb-maker layout-kicad-board <project-dir> <board-id> <out-dir> [router-config.json|auto] [layout-config.json] [--reference <project-dir>|none]

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
                       every error counts.

<out-dir>/result is the finished project (with drc.json and erc.json), <out-dir>/placed the placement
(placement.html animates it, board-placer.json reports constraints, hints and utilization), and
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

usage: pcb-maker place-kicad-board <project-dir> <board-id> <out-dir> [placer-config.json]

  <project-dir>        the KiCad project directory holding <board-id>.kicad_pcb (and .kicad_pro).
  <board-id>           the board's file stem: myboard for myboard.kicad_pcb.
  <out-dir>            where the placed project goes; must not exist yet.
  placer-config.json   the placer options (the placer object of a layout config); default none: all
                       defaults, no constraints. Keys: constraints (a constraints.json path relative
                       to <project-dir>, or inline; default none), placement_seeds (default 3),
                       tight_bodies (default: as the project's DRC allows). docs/constraints.md.
                       A layout config (the layout.json import-kicad-netlist writes) works as well:
                       its placer object is used.

A board with room places in seconds. What cannot fit is an input error at once (exit 2): a region
smaller than its part, a row longer than the outline, parts held at one edge that need more length
than it has. Parts that need more area than a side has, or more than strips of the board hold, are
a no in seconds (exit 1, overfull). A board merely crowded (a side over 90 % full) tries looser
placement levels first, up to the work budget, before it reports unplaced parts: minutes on 80
parts. verdict.json's room gives the numbers: utilization per side, the area missing, the smallest
outline and, when parts did not fit, an outline to try (suggested_outline_mm).

Environment: PCB_PLACER_WORK_SECONDS sets the placement's work budget in seconds of work (default
600, the layout's placement share of its default budget; 60 gives a quicker no).

<out-dir> receives the placed board, placement.html and board-placer.json: every constraint kept or
missed, unplaced, illegal, hints, utilization, and verdict.";

const FAB_HELP: &str = "\
Releases a board to a fab, gated by KiCad's DRC: exports only when no copper error and no
unconnected item remains.

usage: pcb-maker fab-kicad-board <project-dir> <board-id> <out-dir> [--allow-findings] [--layers <L1,L2,...>] [--profile kicad|jlcpcb] [--no-step]

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

The DRC is drc-kicad-board's (cached, zones refilled, without the global libraries); warnings,
excluded and cosmetic findings (silkscreen, text, library) do not block. The ERC (with a schematic)
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
pub const AGENT_COMMANDS: [(&str, &str, &str); 13] = [
    (
        "import-kicad-netlist",
        "makes a board from a schematic or netlist, with constraints.json and layout.json",
        "usage: pcb-maker import-kicad-netlist <schematic.kicad_sch|netlist.net> <out-dir> <board-id> [--layers N] [--footprints-from <board.kicad_pcb>]\n\n  <schematic|netlist>  the KiCad schematic (kicad-cli exports its netlist) or a KiCad .net netlist.\n  <out-dir>            the project directory to write; created if missing. The board, netlist and\n                       schematic are written anew; an existing constraints.json and layout.json are\n                       kept, so a second import (after adding a missing library) keeps your edits.\n  <board-id>           the file stem of the board to write (<out-dir>/<board-id>.kicad_pcb).\n  --layers N           copper layers, an even number of 2 or more; default 2.\n  --footprints-from B  a board with these parts (the designer's, an older revision): a footprint\n                       no library has is taken from it (the symbol's own footprint, else one of\n                       that footprint name) and written to a library in <out-dir>; where B has\n                       the symbol's footprint, B's version is used over the library's. Reported\n                       in footprints_from_board, board_versions and drifted_footprints (library\n                       and board differ by more than a fifth); add entries with from_board.\n\nPrints the import report (JSON) on stdout: missing (with where the footprint was looked for),\nsubstituted, unmatched_pins, clearances, edge_connectors, added (the parts of an existing\nconstraints.json's add, docs/constraints.md), starter (a key matrix found in the netlist: rows,\ncolumns, switches, diodes, halves, unassigned switches; its starter constraints are written to\n<out-dir>/constraints.starter.json, rewritten on every import: copy it to constraints.json to use\nit), and the paths of constraints.json and layout.json. Next step: edit constraints.json (docs/constraints.md), then\npcb-maker layout-kicad-board <out-dir> <board-id> <layout-out-dir> auto <out-dir>/layout.json.\nExits 0 when every footprint was found, 1 when footprints are missing (the board leaves them\nout), 2 on usage or input errors (the schematic or netlist unreadable), 4 on internal errors.",
    ),
    (
        "describe-kicad-board",
        "prints the board's parts, pads, nets and swap candidates as JSON",
        "usage: pcb-maker describe-kicad-board <project-dir> <board-id>\n\n  <project-dir>  the KiCad project directory holding <board-id>.kicad_pcb.\n  <board-id>     the board's file stem.\n\nPrints JSON on stdout: outline, layers, footprints with pads and nets, nets, swap_candidates.",
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
        "score-kicad-board",
        "prints quality measures and the estimated price as JSON",
        "usage: pcb-maker score-kicad-board <project-dir> <board-id>\n\n  <project-dir>  the KiCad project directory holding <board-id>.kicad_pcb.\n  <board-id>     the board's file stem.\n\nPrints JSON on stdout (docs/quality.md).",
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
        "usage: pcb-maker drc-kicad-board <project-dir> <board-id> <report.json> [--no-refill]\n\n  <project-dir>  the KiCad project directory holding <board-id>.kicad_pcb.\n  <board-id>     the board's file stem.\n  <report.json>  where KiCad's DRC report (JSON, every severity) goes; its directory is created.\n  --no-refill    check the zones as stored; default: refilled first, as KiCad's fab outputs do.\n\nThe board and its rules (.kicad_pro, .kicad_dru) are checked without the schematic, as the\nreference board is. The DRC cache answers a board it has seen (docs/agents.md). Exits 0 when\nKiCad finds no error that counts (warnings, excluded and cosmetic findings do not) and no\nunconnected item, 1 when it does; one line on stdout counts them, and when it does and the
project has rules the router does not keep (creepage, lengths, rules on areas), a line
\"next: ...\" names them.",
    ),
    (
        "strip-kicad-tracks",
        "removes tracks and vias from a board (copper zones stay)",
        "usage: pcb-maker strip-kicad-tracks <source.kicad_pcb> <output.kicad_pcb>\n\n  <source.kicad_pcb>  the board to read.\n  <output.kicad_pcb>  the board to write (may be the source).",
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
        .map(|(_, _, help)| format!("{help}\n\n{EXIT_CODES}"))
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
