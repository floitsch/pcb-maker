// Copyright (C) 2026 Toit contributors.

//! The verdict of a command run, for agents: whether the result is
//! complete, and the process's exit code that says so.
//!
//! The rule is the benchmarks' pass rule (`benchmarks/agent-tasks/run.py`,
//! `benchmarks/corpus/run.py`): KiCad's own DRC on the result decides. A
//! result is complete when KiCad reports no unconnected item and no error
//! beyond the ones the reference board (the designer's, or the input of a
//! route) already has, counted per type. Warnings, excluded findings and
//! cosmetic types (silkscreen, text, library metadata, missing courtyards,
//! isolated copper) do not count. A layout or placement must also keep its
//! placement constraints and place every part. Without KiCad's check (it
//! was skipped), the router's own count decides.

use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The run finished and its result is complete.
pub const EXIT_COMPLETE: u8 = 0;
/// The run finished; the result is written but incomplete (connections
/// open, new DRC errors, constraints missed, parts unplaced).
pub const EXIT_INCOMPLETE: u8 = 1;
/// Bad arguments or input (missing or unreadable files, invalid JSON,
/// unknown keys or parts, an output directory that exists).
pub const EXIT_USAGE: u8 = 2;
/// The result is incomplete and the wall clock, not the work, cut the run
/// short: it depends on the machine's load.
pub const EXIT_CUT: u8 = 3;
/// An internal error or a panic.
pub const EXIT_INTERNAL: u8 = 4;

/// DRC finding types that do not count against a result, as the
/// benchmarks' `COSMETIC` pattern.
const COSMETIC_PREFIXES: [&str; 7] = [
    "silk_",
    "text_",
    "lib_footprint",
    "footprint_type_mismatch",
    "missing_courtyard",
    "isolated_copper",
    "nonmirrored_text",
];

pub fn is_cosmetic(kind: &str) -> bool {
    COSMETIC_PREFIXES
        .iter()
        .any(|prefix| kind.starts_with(prefix))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KiCadRunStatus {
    Complete,
    Incomplete,
    Cut,
    UsageError,
    InternalError,
}

impl KiCadRunStatus {
    pub fn exit_code(self) -> u8 {
        match self {
            Self::Complete => EXIT_COMPLETE,
            Self::Incomplete => EXIT_INCOMPLETE,
            Self::UsageError => EXIT_USAGE,
            Self::Cut => EXIT_CUT,
            Self::InternalError => EXIT_INTERNAL,
        }
    }
}

/// A KiCad DRC report, counted as the pass rule counts it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DrcTally {
    /// Error-severity findings by type, excluded and cosmetic ones left
    /// out.
    pub errors: BTreeMap<String, usize>,
    pub cosmetic: usize,
    /// Warning-severity findings that are not cosmetic.
    pub warnings: usize,
    pub unconnected: usize,
    pub schematic_parity: usize,
    /// The schematic parity findings by type (`net_conflict`,
    /// `missing_footprint`, ...).
    pub schematic_parity_types: BTreeMap<String, usize>,
    /// Every parity finding is a symbol pin the footprint has no pad for
    /// (`no_connect_parity_only`).
    pub schematic_parity_pins_only: bool,
}

pub fn tally_drc(report: &Value) -> DrcTally {
    let mut tally = DrcTally::default();
    for finding in report["violations"].as_array().into_iter().flatten() {
        let kind = finding["type"].as_str().unwrap_or("");
        if finding["excluded"].as_bool() == Some(true) {
            continue;
        }
        if is_cosmetic(kind) {
            tally.cosmetic += 1;
        } else if finding["severity"].as_str() == Some("error") {
            *tally.errors.entry(kind.to_string()).or_default() += 1;
        } else {
            tally.warnings += 1;
        }
    }
    tally.unconnected = report["unconnected_items"].as_array().map_or(0, Vec::len);
    tally.schematic_parity = report["schematic_parity"].as_array().map_or(0, Vec::len);
    for finding in report["schematic_parity"].as_array().into_iter().flatten() {
        *tally
            .schematic_parity_types
            .entry(finding["type"].as_str().unwrap_or("unknown").to_string())
            .or_default() += 1;
    }
    tally.schematic_parity_pins_only = no_connect_parity_only(report).is_some();
    tally
}

pub fn read_drc_tally(path: &Path) -> Option<DrcTally> {
    let text = fs::read_to_string(path).ok()?;
    Some(tally_drc(&serde_json::from_str(&text).ok()?))
}

/// The errors per type of the reference board (the findings that were
/// there before the run), or why they are unknown.
#[derive(Clone, Debug, PartialEq)]
pub enum KiCadReference {
    /// No reference: every error counts as the run's.
    None,
    Findings {
        from: PathBuf,
        errors: BTreeMap<String, usize>,
    },
    /// KiCad could not check the reference; every error counts.
    Failed { from: PathBuf, error: String },
}

/// KiCad's DRC errors by type on `project`'s board, counted as the
/// benchmarks count the designer's board: zones refilled, errors only. The
/// report is kept at `report` (see `kicad_drc_report`).
pub fn reference_findings(
    project: &Path,
    board_id: &str,
    report: &Path,
) -> Result<BTreeMap<String, usize>, String> {
    Ok(tally_drc(&kicad_drc_report(project, board_id, report, true)?).errors)
}

/// KiCad's DRC of `project`'s board, every severity, zones refilled when
/// `refill_zones`, through the DRC cache; the report is written to
/// `report` and returned. The board and its rules (`.kicad_pro`,
/// `.kicad_dru`) are copied next to the report for the check, so that the
/// check sees only them (no schematic parity) and the cache key is the
/// same wherever the project lies.
pub fn kicad_drc_report(
    project: &Path,
    board_id: &str,
    report: &Path,
    refill_zones: bool,
) -> Result<Value, String> {
    let scratch = report.with_extension("project");
    if scratch.exists() {
        fs::remove_dir_all(&scratch)
            .map_err(|error| format!("failed to remove {}: {error}", scratch.display()))?;
    }
    fs::create_dir_all(&scratch)
        .map_err(|error| format!("failed to create {}: {error}", scratch.display()))?;
    for extension in ["kicad_pcb", "kicad_pro", "kicad_dru"] {
        let file = project.join(format!("{board_id}.{extension}"));
        if file.exists() {
            fs::copy(&file, scratch.join(format!("{board_id}.{extension}")))
                .map_err(|error| format!("failed to copy {}: {error}", file.display()))?;
        }
    }
    // All severities (the report check wants them); callers count errors.
    let mut arguments = vec!["pcb", "drc", "--format", "json", "--severity-all"];
    if refill_zones {
        arguments.push("--refill-zones");
    }
    arguments.push("--output");
    let checked = crate::native_report::run_kicad_report(
        &arguments,
        report,
        &scratch.join(format!("{board_id}.kicad_pcb")),
    );
    let _ = fs::remove_dir_all(&scratch);
    checked?;
    let text = fs::read_to_string(report)
        .map_err(|error| format!("failed to read {}: {error}", report.display()))?;
    serde_json::from_str(&text)
        .map_err(|error| format!("failed to parse {}: {error}", report.display()))
}

/// Splits a result's errors into the ones beyond the reference's (new)
/// and the ones the reference accounts for, per type.
pub fn split_findings(
    errors: &BTreeMap<String, usize>,
    reference: &BTreeMap<String, usize>,
) -> (BTreeMap<String, usize>, BTreeMap<String, usize>) {
    let mut new = BTreeMap::new();
    let mut preexisting = BTreeMap::new();
    for (kind, &count) in errors {
        let allowed = reference.get(kind).copied().unwrap_or(0).min(count);
        if allowed > 0 {
            preexisting.insert(kind.clone(), allowed);
        }
        if count > allowed {
            new.insert(kind.clone(), count - allowed);
        }
    }
    (new, preexisting)
}

/// What a finished run produced, as the verdict needs it.
#[derive(Clone, Debug)]
pub struct KiCadRunFacts {
    pub command: String,
    /// KiCad's DRC on the result; `None` when it did not run.
    pub native: Option<DrcTally>,
    pub reference: KiCadReference,
    pub routable_connections: usize,
    pub routed_connections: usize,
    pub unconnected_terminals: usize,
    pub internal_violations: usize,
    pub starved_thermals: usize,
    pub dead_pads: usize,
    pub hot_spots: usize,
    pub constraints_missed: Vec<String>,
    pub unplaced: Vec<String>,
    pub placement_hints: Vec<String>,
    pub cut: Vec<String>,
    pub work_seconds: Option<f64>,
    /// The part of `work_seconds` that is KiCad's checks (a layout's).
    pub kicad_check_work_seconds: Option<f64>,
    pub wall_seconds: f64,
    pub outputs: BTreeMap<String, String>,
    pub skipped: Vec<String>,
    /// Whether the run routed (a placement alone has no copper to judge).
    pub routes: bool,
    /// The placement's room (`placement_room`), when there was one.
    pub room: Option<crate::placement_room::KiCadRoom>,
}

impl KiCadRunFacts {
    pub fn new(command: &str) -> Self {
        Self {
            command: command.into(),
            native: None,
            reference: KiCadReference::None,
            routable_connections: 0,
            routed_connections: 0,
            unconnected_terminals: 0,
            internal_violations: 0,
            starved_thermals: 0,
            dead_pads: 0,
            hot_spots: 0,
            constraints_missed: Vec::new(),
            unplaced: Vec::new(),
            placement_hints: Vec::new(),
            cut: Vec::new(),
            work_seconds: None,
            kicad_check_work_seconds: None,
            wall_seconds: 0.0,
            outputs: BTreeMap::new(),
            skipped: Vec::new(),
            routes: true,
            room: None,
        }
    }

    /// Whether the reference board would change the verdict: the result
    /// has errors that count.
    pub fn needs_reference(&self) -> bool {
        self.native
            .as_ref()
            .is_some_and(|native| !native.errors.is_empty())
    }
}

/// The verdict a run's report carries (`verdict` in board-router.json,
/// board-layout.json and board-placer.json) and its exit code.
#[derive(Clone, Debug, Serialize)]
pub struct KiCadVerdict {
    pub status: KiCadRunStatus,
    pub exit_code: u8,
    /// One line: what the status rests on.
    pub reason: String,
    pub command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Connections (nets with two pads or more) the router did not finish,
    /// by its own count.
    pub open_connections: usize,
    /// Pads the router did not reach, by its own count.
    pub unconnected_terminals: usize,
    /// KiCad's unconnected items on the result (`None`: KiCad did not
    /// check it). This, not the router's count, decides.
    pub kicad_unconnected_items: Option<usize>,
    /// KiCad errors per type beyond the reference's: the run's own.
    pub new_findings: BTreeMap<String, usize>,
    /// KiCad errors per type the reference board has as well (the
    /// designer's own findings); they do not count.
    pub preexisting_findings: BTreeMap<String, usize>,
    /// The project whose board the pre-existing findings come from, or
    /// `null` when there was none or none was needed.
    pub preexisting_from: Option<String>,
    /// Silkscreen, text and library findings, which do not count.
    pub cosmetic_findings: usize,
    /// Other warning-severity findings, which do not count.
    pub warnings: usize,
    pub schematic_parity_issues: usize,
    /// `schematic_parity_issues` by KiCad's type.
    pub schematic_parity_by_type: BTreeMap<String, usize>,
    /// Clearance violations the router's own exact check found.
    pub internal_violations: usize,
    /// Pads KiCad finds short of thermal spokes (counted in
    /// `new_findings` as `starved_thermal` too).
    pub starved_thermals: usize,
    pub constraints_missed: Vec<String>,
    pub unplaced: Vec<String>,
    /// What the wall clock cut short rather than the work (`deadline`,
    /// `negotiation guard`, `budget guard`, `ladder`, `placement`,
    /// `layout`): the result depends on the machine's load.
    pub cut: Vec<String>,
    /// The work done, in seconds of an idle machine (what the budgets
    /// count).
    pub work_seconds: Option<f64>,
    /// The part of `work_seconds` that is KiCad's checks inside a layout:
    /// each counts a cost modelled from the board, the same whether the
    /// DRC cache answered it or not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kicad_check_work_seconds: Option<f64>,
    pub wall_seconds: f64,
    /// Where the results are: `project`, `board`, `drc`, `report`, ...
    pub outputs: BTreeMap<String, String>,
    /// Steps that did not run while the board was incomplete, with why.
    pub skipped: Vec<String>,
    /// What to do next, if anything; factual.
    pub next_actions: Vec<String>,
    /// The room on the board as numbers: utilization per side, the area
    /// missing, the smallest outline that holds the parts, and an outline
    /// to try when parts found no place (`null` for a route).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub room: Option<crate::placement_room::KiCadRoom>,
}

fn counts(map: &BTreeMap<String, usize>) -> String {
    map.iter()
        .map(|(kind, count)| format!("{kind} {count}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The status of a finished run, and the one-line reason.
pub fn decide(
    facts: &KiCadRunFacts,
    new_findings: &BTreeMap<String, usize>,
) -> (KiCadRunStatus, String) {
    let mut problems = Vec::new();
    if !facts.unplaced.is_empty() {
        problems.push(format!(
            "{} parts without a legal place ({})",
            facts.unplaced.len(),
            facts.unplaced.join(", ")
        ));
    }
    if facts.routes {
        match &facts.native {
            Some(native) => {
                if native.unconnected > 0 {
                    problems.push(format!("{} unconnected items in KiCad", native.unconnected));
                }
                if !new_findings.is_empty() {
                    problems.push(format!("new DRC errors: {}", counts(new_findings)));
                }
            }
            None => {
                let open = facts
                    .routable_connections
                    .saturating_sub(facts.routed_connections);
                if open > 0 || facts.unconnected_terminals > 0 {
                    problems.push(format!(
                        "{open} connections open, {} pads unreached (router's count; KiCad did not check)",
                        facts.unconnected_terminals
                    ));
                }
                if facts.internal_violations > 0 {
                    problems.push(format!(
                        "{} clearance violations (router's check)",
                        facts.internal_violations
                    ));
                }
            }
        }
    }
    if !facts.constraints_missed.is_empty() {
        problems.push(format!(
            "constraints missed: {}",
            facts.constraints_missed.join(", ")
        ));
    }
    if problems.is_empty() {
        let reason = if !facts.routes {
            "complete: every part placed, every constraint kept".to_string()
        } else if facts.native.is_some() {
            format!(
                "complete: {}/{} connections, KiCad finds no unconnected item and no error beyond the reference's",
                facts.routed_connections, facts.routable_connections
            )
        } else {
            format!(
                "complete by the router's count ({}/{} connections); KiCad did not check it",
                facts.routed_connections, facts.routable_connections
            )
        };
        return (KiCadRunStatus::Complete, reason);
    }
    let problems = problems.join("; ");
    if facts.cut.is_empty() {
        (
            KiCadRunStatus::Incomplete,
            format!("incomplete: {problems}"),
        )
    } else {
        (
            KiCadRunStatus::Cut,
            format!(
                "incomplete and cut short by the wall clock ({}): {problems}",
                facts.cut.join(", ")
            ),
        )
    }
}

fn next_actions(
    facts: &KiCadRunFacts,
    status: KiCadRunStatus,
    new_findings: &BTreeMap<String, usize>,
) -> Vec<String> {
    let mut actions = Vec::new();
    if !facts.unplaced.is_empty() {
        actions.push(format!(
            "give the unplaced parts room: a larger outline, a looser region or edge constraint, or the other side ({})",
            if facts.placement_hints.is_empty() { "see hints in board-placer.json".to_string() } else { facts.placement_hints.join("; ") }
        ));
        if let Some(room) = &facts.room
            && let Some(size) = room.suggested_outline_mm
        {
            actions.push(room_action(room, size));
        }
    }
    if !facts.cut.is_empty() && status != KiCadRunStatus::Complete {
        actions.push(format!(
            "the wall clock cut the run ({}): rerun on a less loaded machine, or with a larger budget (total_seconds in the layout config, ladder_budget_seconds in the router config)",
            facts.cut.join(", ")
        ));
    }
    let open = facts
        .native
        .as_ref()
        .map_or(facts.unconnected_terminals, |native| native.unconnected);
    if facts.routes && open > 0 {
        let mut where_ = Vec::new();
        if facts.hot_spots > 0 {
            where_.push(format!(
                "diagnostics.hot_spots ({}) says where nets fought for room",
                facts.hot_spots
            ));
        }
        if facts.dead_pads > 0 {
            where_.push(format!(
                "diagnostics.dead_pads ({}) lists pads no track of their class can enter",
                facts.dead_pads
            ));
        }
        actions.push(format!(
            "connections are open: {}; make room there (move parts apart, enlarge the outline, add layers) or narrow the net classes",
            if where_.is_empty() { "see diagnostics in the report".to_string() } else { where_.join(", ") }
        ));
    }
    if facts.starved_thermals > 0 && new_findings.contains_key("starved_thermal") {
        actions.push(format!(
            "{} pads are short of thermal spokes (diagnostics.starved_thermals has advice for each); \"solid_starved_thermals\": true in the router config connects them solid",
            facts.starved_thermals
        ));
    }
    let other: BTreeMap<String, usize> = new_findings
        .iter()
        .filter(|(kind, _)| kind.as_str() != "starved_thermal")
        .map(|(kind, count)| (kind.clone(), *count))
        .collect();
    if !other.is_empty() {
        actions.push(format!(
            "new DRC errors ({}): see {}",
            counts(&other),
            facts.outputs.get("drc").map_or("drc.json", String::as_str)
        ));
    }
    if !facts.constraints_missed.is_empty() {
        actions.push("constraints missed (constraints[] has violation_mm for each): relax them or give the parts room".into());
    }
    if let KiCadReference::Failed { error, .. } = &facts.reference {
        actions.push(format!(
            "the reference board could not be checked ({error}): every DRC error counted as new"
        ));
    }
    if let Some(native) = facts
        .native
        .as_ref()
        .filter(|native| native.schematic_parity > 0)
    {
        actions.push(if native.schematic_parity_pins_only {
            format!(
                "nothing to do for the {} schematic parity findings: each is a symbol pin the footprint has no pad for (net_conflict \"No pad found for pin\"), a no-connect pin on KiCad's unconnected-(...) net when import-kicad-netlist listed it in unmatched_pins",
                native.schematic_parity
            )
        } else {
            format!(
                "schematic parity findings ({}; counted, not judged): see {}",
                counts(&native.schematic_parity_types),
                facts.outputs.get("drc").map_or("drc.json", String::as_str)
            )
        });
    }
    if facts.routes && facts.native.is_none() {
        actions.push("KiCad's check did not run (PCB_SKIP_NATIVE or skip_native_verification): the verdict is the router's own count".into());
    }
    if status != KiCadRunStatus::Complete {
        actions.extend(facts.skipped.iter().cloned());
    }
    actions
}

/// The next action an outline suggestion makes: the size to try, and why.
pub fn room_action(room: &crate::placement_room::KiCadRoom, size: [f64; 2]) -> String {
    let full: Vec<String> = ["front", "back"]
        .iter()
        .zip(room.utilization.iter().zip(room.missing_mm2))
        .filter(|(_, (_, missing))| *missing > 0.0)
        .map(|(side, (share, missing))| {
            format!(
                "the {side} side is {:.0} % full, {missing:.0} mm2 short of {:.0} %",
                share * 100.0,
                room.target_utilization * 100.0
            )
        })
        .collect();
    let mut why = full;
    why.extend(room.limits.iter().cloned());
    format!(
        "try an outline of {:.1} x {:.1} mm (room.suggested_outline_mm; now {:.1} x {:.1}){}",
        size[0],
        size[1],
        room.outline_mm[0],
        room.outline_mm[1],
        if why.is_empty() {
            String::new()
        } else {
            format!(": {}", why.join("; "))
        }
    )
}

/// The parity findings that are only the schematic's no-connect pins
/// (KiCad's `unconnected-(...)` nets): pins the footprint has no pad for,
/// harmless; `None` when the report holds others too or none.
pub fn no_connect_parity_only(report: &Value) -> Option<usize> {
    let findings = report["schematic_parity"].as_array()?;
    let harmless = |finding: &Value| {
        finding["type"].as_str() == Some("net_conflict")
            && finding["description"]
                .as_str()
                .is_some_and(|text| text.contains("No pad found for pin"))
    };
    (!findings.is_empty() && findings.iter().all(harmless)).then_some(findings.len())
}

/// The verdict of a finished run.
pub fn judge(facts: &KiCadRunFacts) -> KiCadVerdict {
    let errors = facts
        .native
        .as_ref()
        .map(|native| native.errors.clone())
        .unwrap_or_default();
    let (new_findings, preexisting_findings, preexisting_from) = match &facts.reference {
        KiCadReference::Findings {
            from,
            errors: reference,
        } => {
            let (new, preexisting) = split_findings(&errors, reference);
            (new, preexisting, Some(from.display().to_string()))
        }
        KiCadReference::None | KiCadReference::Failed { .. } => (errors, BTreeMap::new(), None),
    };
    let (status, reason) = decide(facts, &new_findings);
    KiCadVerdict {
        status,
        exit_code: status.exit_code(),
        reason,
        command: facts.command.clone(),
        error: None,
        open_connections: facts
            .routable_connections
            .saturating_sub(facts.routed_connections),
        unconnected_terminals: facts.unconnected_terminals,
        kicad_unconnected_items: facts.native.as_ref().map(|native| native.unconnected),
        next_actions: next_actions(facts, status, &new_findings),
        new_findings,
        preexisting_findings,
        preexisting_from,
        cosmetic_findings: facts.native.as_ref().map_or(0, |native| native.cosmetic),
        warnings: facts.native.as_ref().map_or(0, |native| native.warnings),
        schematic_parity_issues: facts
            .native
            .as_ref()
            .map_or(0, |native| native.schematic_parity),
        schematic_parity_by_type: facts
            .native
            .as_ref()
            .map(|native| native.schematic_parity_types.clone())
            .unwrap_or_default(),
        internal_violations: facts.internal_violations,
        starved_thermals: facts.starved_thermals,
        constraints_missed: facts.constraints_missed.clone(),
        unplaced: facts.unplaced.clone(),
        cut: facts.cut.clone(),
        work_seconds: facts.work_seconds,
        kicad_check_work_seconds: facts.kicad_check_work_seconds,
        wall_seconds: facts.wall_seconds,
        outputs: facts.outputs.clone(),
        skipped: facts.skipped.clone(),
        room: facts.room.clone(),
    }
}

/// The verdict of a run that ended in an error (`status` a usage or an
/// internal error).
pub fn error_verdict(
    command: &str,
    status: KiCadRunStatus,
    error: &str,
    wall_seconds: f64,
    outputs: BTreeMap<String, String>,
) -> KiCadVerdict {
    let mut facts = KiCadRunFacts::new(command);
    facts.wall_seconds = wall_seconds;
    facts.outputs = outputs;
    let mut verdict = judge(&facts);
    verdict.status = status;
    verdict.exit_code = status.exit_code();
    verdict.reason = format!(
        "{}: {}",
        if status == KiCadRunStatus::UsageError {
            "usage or input error"
        } else {
            "internal error"
        },
        error.lines().next().unwrap_or("")
    );
    verdict.error = Some(error.to_string());
    verdict.next_actions = if status == KiCadRunStatus::UsageError {
        vec!["fix the arguments or input files the error names; nothing was routed".into()]
    } else {
        vec!["an internal error: the output directory may hold partial results; report the error with the input".into()]
    };
    verdict
}

/// The router's facts for a route's verdict (`directory`: the routed
/// project, with KiCad's `drc.json`).
pub fn route_facts(
    command: &str,
    result: &crate::KiCadBoardRouterResult,
    directory: &Path,
) -> KiCadRunFacts {
    let mut facts = KiCadRunFacts::new(command);
    facts.native = result
        .native
        .as_ref()
        .and_then(|_| read_drc_tally(&directory.join("drc.json")));
    facts.routable_connections = result.routable_connections;
    facts.routed_connections = result.routed_connections;
    facts.unconnected_terminals = result.unconnected_terminals;
    facts.internal_violations = result.internal_violations.len();
    facts.starved_thermals = result.diagnostics.starved_thermals.len();
    facts.dead_pads = result.diagnostics.dead_pads.len();
    facts.hot_spots = result.diagnostics.hot_spots.len();
    facts.cut = result.cut.clone();
    facts.skipped = result.skipped.clone();
    facts
}

/// The kinds of wall-clock cut this process saw (`pcb_router`'s
/// `WALL_CUTS`).
pub fn wall_cuts() -> Vec<String> {
    pcb_router::router::wall_cuts()
}

/// Errors marked as the input's fault (`input_error`), for the exit code.
static INPUT_ERRORS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Marks `message` as an error in the input (an unknown part in the
/// constraints, a board that does not parse): the command exits with
/// `EXIT_USAGE`, not as an internal error.
pub fn input_error(message: impl Into<String>) -> String {
    let message = message.into();
    if let Ok(mut errors) = INPUT_ERRORS.lock() {
        errors.push(message.clone());
    }
    message
}

/// Whether `error` is, or wraps, a message marked by `input_error`.
pub fn is_input_error(error: &str) -> bool {
    INPUT_ERRORS
        .lock()
        .map(|errors| {
            errors
                .iter()
                .any(|message| !message.is_empty() && error.contains(message.as_str()))
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn map(entries: &[(&str, usize)]) -> BTreeMap<String, usize> {
        entries
            .iter()
            .map(|(kind, count)| (kind.to_string(), *count))
            .collect()
    }

    fn routed(native: DrcTally) -> KiCadRunFacts {
        let mut facts = KiCadRunFacts::new("route-kicad-board");
        facts.routable_connections = 54;
        facts.routed_connections = 54;
        facts.native = Some(native);
        facts
    }

    #[test]
    fn the_drc_is_counted_as_the_benchmarks_count_it() {
        let report = json!({
            "violations": [
                {"type": "clearance", "severity": "error"},
                {"type": "clearance", "severity": "error"},
                {"type": "clearance", "severity": "error", "excluded": true},
                {"type": "silk_overlap", "severity": "error"},
                {"type": "lib_footprint_issues", "severity": "warning"},
                {"type": "via_dangling", "severity": "warning"},
                {"type": "courtyards_overlap", "severity": "error"}
            ],
            "unconnected_items": [{}, {}],
            "schematic_parity": [{}]
        });
        let tally = tally_drc(&report);
        assert_eq!(
            tally.errors,
            map(&[("clearance", 2), ("courtyards_overlap", 1)])
        );
        assert_eq!(tally.cosmetic, 2);
        assert_eq!(tally.warnings, 1);
        assert_eq!(tally.unconnected, 2);
        assert_eq!(tally.schematic_parity, 1);
    }

    #[test]
    fn findings_beyond_the_references_are_new() {
        let (new, preexisting) = split_findings(
            &map(&[("clearance", 3), ("courtyards_overlap", 1)]),
            &map(&[
                ("clearance", 1),
                ("courtyards_overlap", 2),
                ("hole_clearance", 4),
            ]),
        );
        assert_eq!(new, map(&[("clearance", 2)]));
        assert_eq!(
            preexisting,
            map(&[("clearance", 1), ("courtyards_overlap", 1)])
        );
    }

    #[test]
    fn a_designer_finding_does_not_make_a_route_incomplete() {
        // USB_Cable_Tester in route mode: 0 unconnected, one courtyard
        // overlap that the designer's board has too (exit 1 before).
        let mut facts = routed(DrcTally {
            errors: map(&[("courtyards_overlap", 1)]),
            ..Default::default()
        });
        assert!(facts.needs_reference());
        assert_eq!(judge(&facts).exit_code, EXIT_INCOMPLETE);
        facts.reference = KiCadReference::Findings {
            from: "designer".into(),
            errors: map(&[("courtyards_overlap", 2), ("clearance", 6)]),
        };
        let verdict = judge(&facts);
        assert_eq!(verdict.status, KiCadRunStatus::Complete);
        assert_eq!(verdict.exit_code, EXIT_COMPLETE);
        assert!(verdict.new_findings.is_empty());
        assert_eq!(
            verdict.preexisting_findings,
            map(&[("courtyards_overlap", 1)])
        );
        assert!(verdict.next_actions.is_empty());
    }

    #[test]
    fn kicad_decides_over_the_routers_count() {
        // A pad the pour's fill joins: the router counts it open, KiCad not.
        let mut facts = routed(DrcTally::default());
        facts.routed_connections = 53;
        facts.unconnected_terminals = 1;
        assert_eq!(judge(&facts).exit_code, EXIT_COMPLETE);
        // And the other way round.
        let mut facts = routed(DrcTally {
            unconnected: 2,
            ..Default::default()
        });
        facts.hot_spots = 3;
        let verdict = judge(&facts);
        assert_eq!(verdict.exit_code, EXIT_INCOMPLETE);
        assert_eq!(verdict.kicad_unconnected_items, Some(2));
        assert!(
            verdict
                .next_actions
                .iter()
                .any(|action| action.contains("hot_spots"))
        );
    }

    #[test]
    fn without_kicads_check_the_routers_count_decides() {
        let mut facts = routed(DrcTally::default());
        facts.native = None;
        assert_eq!(judge(&facts).exit_code, EXIT_COMPLETE);
        facts.internal_violations = 1;
        assert_eq!(judge(&facts).exit_code, EXIT_INCOMPLETE);
        facts.internal_violations = 0;
        facts.unconnected_terminals = 2;
        assert_eq!(judge(&facts).exit_code, EXIT_INCOMPLETE);
    }

    #[test]
    fn a_cut_matters_only_to_an_incomplete_result() {
        let mut facts = routed(DrcTally::default());
        facts.cut = vec!["deadline".into()];
        assert_eq!(judge(&facts).exit_code, EXIT_COMPLETE);
        facts.native = Some(DrcTally {
            unconnected: 4,
            ..Default::default()
        });
        facts.skipped = vec![
            "the narrow-signal step (signals at 0.15 mm) did not run: the deadline was reached"
                .into(),
        ];
        let verdict = judge(&facts);
        assert_eq!(verdict.status, KiCadRunStatus::Cut);
        assert_eq!(verdict.exit_code, EXIT_CUT);
        assert!(
            verdict
                .next_actions
                .iter()
                .any(|action| action.contains("narrow-signal"))
        );
    }

    #[test]
    fn placements_and_layouts_answer_for_their_constraints() {
        let mut facts = routed(DrcTally::default());
        facts.command = "layout-kicad-board".into();
        facts.constraints_missed = vec!["near C1".into()];
        assert_eq!(judge(&facts).exit_code, EXIT_INCOMPLETE);
        let mut facts = KiCadRunFacts::new("place-kicad-board");
        facts.routes = false;
        assert_eq!(judge(&facts).exit_code, EXIT_COMPLETE);
        facts.unplaced = vec!["U1".into()];
        facts.placement_hints = vec!["front side 92 % full".into()];
        let verdict = judge(&facts);
        assert_eq!(verdict.exit_code, EXIT_INCOMPLETE);
        assert!(verdict.next_actions[0].contains("92 %"));
    }

    #[test]
    fn a_failed_reference_counts_every_error() {
        let mut facts = routed(DrcTally {
            errors: map(&[("clearance", 1)]),
            ..Default::default()
        });
        facts.reference = KiCadReference::Failed {
            from: "designer".into(),
            error: "kicad-cli failed".into(),
        };
        let verdict = judge(&facts);
        assert_eq!(verdict.exit_code, EXIT_INCOMPLETE);
        assert_eq!(verdict.new_findings, map(&[("clearance", 1)]));
        assert!(
            verdict
                .next_actions
                .iter()
                .any(|action| action.contains("could not be checked"))
        );
    }

    #[test]
    fn errors_have_their_own_codes() {
        let verdict = error_verdict(
            "route-kicad-board",
            KiCadRunStatus::UsageError,
            "failed to read x.json: no such file",
            0.1,
            BTreeMap::new(),
        );
        assert_eq!(verdict.exit_code, EXIT_USAGE);
        assert_eq!(
            verdict.error.as_deref(),
            Some("failed to read x.json: no such file")
        );
        let verdict = error_verdict(
            "route-kicad-board",
            KiCadRunStatus::InternalError,
            "panic: index out of bounds",
            3.0,
            BTreeMap::new(),
        );
        assert_eq!(verdict.exit_code, EXIT_INTERNAL);
        let codes: Vec<u8> = [
            KiCadRunStatus::Complete,
            KiCadRunStatus::Incomplete,
            KiCadRunStatus::UsageError,
            KiCadRunStatus::Cut,
            KiCadRunStatus::InternalError,
        ]
        .iter()
        .map(|status| status.exit_code())
        .collect();
        assert_eq!(codes, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn marked_input_errors_are_recognised_when_wrapped() {
        let message = input_error("constraint names unknown part \"U99\"");
        assert!(is_input_error(&format!("placement: {message}")));
        assert!(!is_input_error("kicad-cli failed with exit status 1"));
    }

    #[test]
    fn parity_findings_are_counted_by_type_and_no_connect_pins_need_nothing() {
        let pin =
            json!({"type": "net_conflict", "description": "No pad found for pin 31 in symbol U1."});
        let report = json!({"violations": [], "unconnected_items": [], "schematic_parity": [pin.clone(), pin.clone()]});
        let tally = tally_drc(&report);
        assert_eq!(tally.schematic_parity_types, map(&[("net_conflict", 2)]));
        assert!(tally.schematic_parity_pins_only);
        let verdict = judge(&routed(tally));
        assert_eq!(
            verdict.schematic_parity_by_type,
            map(&[("net_conflict", 2)])
        );
        assert!(
            verdict
                .next_actions
                .iter()
                .any(|action| action
                    .starts_with("nothing to do for the 2 schematic parity findings"))
        );
        // A footprint missing is something to look at.
        let report = json!({"schematic_parity": [pin, {"type": "missing_footprint", "description": "Missing footprint R3"}]});
        let tally = tally_drc(&report);
        assert!(!tally.schematic_parity_pins_only);
        let verdict = judge(&routed(tally));
        assert!(verdict.next_actions.iter().any(|action| {
            action.starts_with("schematic parity findings (missing_footprint 1, net_conflict 1")
        }));
    }
}
