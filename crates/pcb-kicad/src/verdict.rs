// Copyright (C) 2026 Toit contributors.

//! The verdict of a command run, for agents: whether the result is
//! complete, and the process's exit code that says so.
//!
//! The rule: KiCad's own DRC on the result decides. A result is complete
//! when KiCad reports no unconnected item and no error beyond the ones the
//! reference board (the designer's, or the input of a route) already has
//! at the same place: the same type and the same items (by uuid, moved
//! together) or items at the same positions (`split_findings_strict`). A
//! type the reference has at KiCad's cap (`KICAD_FINDING_CAP`) is not
//! subtracted. Warnings, excluded findings and cosmetic types (silkscreen,
//! text, library metadata, missing courtyards, isolated copper) do not
//! count. A layout or placement must also keep its placement constraints
//! and place every part. Without KiCad's check (it was skipped) the
//! router's own count can only say `router_complete_unverified` (exit 1);
//! a check that ran but left no readable report is an internal error.

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
    /// The router's own count finds nothing open, but KiCad did not check
    /// the board (`PCB_SKIP_NATIVE`, `skip_native_verification`): no claim
    /// that it passes (exit 1).
    RouterCompleteUnverified,
    Cut,
    UsageError,
    InternalError,
}

impl KiCadRunStatus {
    pub fn exit_code(self) -> u8 {
        match self {
            Self::Complete => EXIT_COMPLETE,
            Self::Incomplete | Self::RouterCompleteUnverified => EXIT_INCOMPLETE,
            Self::UsageError => EXIT_USAGE,
            Self::Cut => EXIT_CUT,
            Self::InternalError => EXIT_INTERNAL,
        }
    }
}

/// KiCad's DRC reports at most this many findings of one type (its error
/// limit; a type spread over several of KiCad's tests can reach a few
/// hundred more, as 499 clearances): a count at or above it is a floor,
/// not a count.
pub const KICAD_FINDING_CAP: usize = 199;

/// How far (mm) an item of a finding may lie from the reference's item for
/// the two findings to be at the same place.
pub const SAME_PLACE_MM: f64 = 0.05;

/// One item a DRC finding names: its uuid and its position.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DrcItem {
    pub uuid: Option<String>,
    pub at: Option<[f64; 2]>,
    /// A zone (KiCad's item description).
    pub zone: bool,
    /// A zone KiCad places at (0, 0) whatever the problem's place (a
    /// `connection_width` on a pour), where the finding gives no position
    /// of its own: `at` says nothing about where the problem is.
    pub unlocated: bool,
}

impl DrcItem {
    /// Where the item is, `None` where KiCad does not say.
    pub fn located(&self) -> Option<[f64; 2]> {
        if self.unlocated { None } else { self.at }
    }
}

/// A DRC item from KiCad's report: a zone at (0, 0) takes the finding's
/// own position where the report gives one (KiCad 10's gives none), else
/// it is `unlocated` (run.py's counting_findings and zone_origin).
fn drc_item(item: &Value, finding: &Value) -> DrcItem {
    let position = |value: &Value| {
        value["x"]
            .as_f64()
            .zip(value["y"].as_f64())
            .map(|(x, y)| [x, y])
    };
    let mut at = position(&item["pos"]);
    let zone = item["description"]
        .as_str()
        .is_some_and(|description| description.starts_with("Zone"));
    let zone_origin = zone && at == Some([0.0, 0.0]);
    let mut unlocated = false;
    if zone_origin {
        match position(&finding["pos"]) {
            Some(own) => at = Some(own),
            None => unlocated = true,
        }
    }
    DrcItem {
        uuid: item["uuid"].as_str().map(str::to_string),
        at,
        zone,
        unlocated,
    }
}

/// One finding that counts (error severity, not excluded, not cosmetic),
/// with its items, for matching against the reference's findings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DrcFinding {
    pub kind: String,
    pub items: Vec<DrcItem>,
    /// KiCad's description (the rule and the values: "Clearance violation
    /// (netclass 'Default' clearance 0.2000 mm; actual 0.1500 mm)").
    pub description: String,
    /// The nets of its items (`[GND]` in their descriptions), in order.
    pub nets: Vec<String>,
    /// Its items' descriptions ("Pad 2 [GND] of C1 on F.Cu").
    pub item_descriptions: Vec<String>,
}

/// A KiCad DRC report, counted as the pass rule counts it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DrcTally {
    /// Error-severity findings by type, excluded and cosmetic ones left
    /// out.
    pub errors: BTreeMap<String, usize>,
    /// The same findings one by one, with their items.
    pub findings: Vec<DrcFinding>,
    pub cosmetic: usize,
    /// Warning-severity findings that are not cosmetic.
    pub warnings: usize,
    pub unconnected: usize,
    /// The nets of KiCad's unconnected items, with how many.
    pub unconnected_nets: BTreeMap<String, usize>,
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
            let item_descriptions: Vec<String> = finding["items"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|item| item["description"].as_str().map(str::to_string))
                .collect();
            tally.findings.push(DrcFinding {
                kind: kind.to_string(),
                description: finding["description"].as_str().unwrap_or("").to_string(),
                nets: item_descriptions
                    .iter()
                    .filter_map(|description| item_net(description))
                    .collect(),
                item_descriptions,
                items: finding["items"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|item| drc_item(item, finding))
                    .collect(),
            });
        } else {
            tally.warnings += 1;
        }
    }
    tally.unconnected = report["unconnected_items"].as_array().map_or(0, Vec::len);
    for finding in report["unconnected_items"].as_array().into_iter().flatten() {
        // Both items of an unconnected pair are on the one net.
        if let Some(net) = finding["items"]
            .as_array()
            .into_iter()
            .flatten()
            .find_map(|item| item["description"].as_str().and_then(item_net))
        {
            *tally.unconnected_nets.entry(net).or_default() += 1;
        }
    }
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

/// The net a DRC item's description names: `Pad 2 [GND] of C1 on F.Cu`
/// gives `GND`; `None` without one or for `<no net>`.
fn item_net(description: &str) -> Option<String> {
    let start = description.find('[')? + 1;
    let rest = &description[start..];
    let end = rest.find("] ").or_else(|| rest.rfind(']'))?;
    let net = &rest[..end];
    (!net.is_empty() && net != "<no net>").then(|| net.to_string())
}

pub fn read_drc_tally(path: &Path) -> Option<DrcTally> {
    load_drc_tally(path).ok()
}

/// `read_drc_tally`, saying why a report cannot be read.
pub fn load_drc_tally(path: &Path) -> Result<DrcTally, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let report: Value = serde_json::from_str(&text)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))?;
    if !report["violations"].is_array() {
        return Err(format!("{} holds no DRC violations list", path.display()));
    }
    Ok(tally_drc(&report))
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
        /// The findings one by one, for the strict rule.
        findings: Vec<DrcFinding>,
    },
    /// KiCad could not check the reference; every error counts.
    Failed { from: PathBuf, error: String },
}

/// KiCad's DRC errors on `project`'s board, counted as the benchmarks
/// count the designer's board: zones refilled, errors only; by type and
/// one by one. The report is kept at `report` (see `kicad_drc_report`).
pub fn reference_findings(
    project: &Path,
    board_id: &str,
    report: &Path,
) -> Result<(BTreeMap<String, usize>, Vec<DrcFinding>), String> {
    let tally = tally_drc(&kicad_drc_report(project, board_id, report, true)?);
    Ok((tally.errors, tally.findings))
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
/// and the ones the reference accounts for, per type: the old rule, which
/// hides a new error of a type the reference has anywhere on the board
/// (kept as `new_findings` for comparisons; `split_findings_strict`
/// decides).
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

fn distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// Whether `ours` is the reference's finding `theirs` at the same place:
/// the same type and either the same items (every uuid on both sides, the
/// same set) moved together (the distances between them kept, so a part's
/// own finding goes with the part), or items at the same positions, each
/// within `SAME_PLACE_MM`. A finding of one item has no distances to keep:
/// it is at the same place where its item is within `SAME_PLACE_MM` of
/// theirs (`DrcItem::located`: never a zone KiCad places at (0, 0), as for
/// a `connection_width` on a pour, where any of ours matched any of the
/// designer's on that pour by its uuid; review 2026-10-21, H2), or where it
/// is the same item, not a zone, with the same description word for word,
/// KiCad's measured values included (a pad's annular ring, a footprint's
/// courtyard: a property of the item, which moved with its part).
/// run.py's strict rule 3.
pub fn same_place(ours: &DrcFinding, theirs: &DrcFinding) -> bool {
    if ours.kind != theirs.kind || ours.items.is_empty() || ours.items.len() != theirs.items.len() {
        return false;
    }
    if let ([mine], [reference]) = (ours.items.as_slice(), theirs.items.as_slice()) {
        if let (Some(a), Some(b)) = (mine.located(), reference.located())
            && distance(a, b) <= SAME_PLACE_MM
        {
            return true;
        }
        return !mine.zone
            && mine.uuid.is_some()
            && mine.uuid == reference.uuid
            && !ours.description.is_empty()
            && ours.description == theirs.description;
    }
    let uuids = |finding: &DrcFinding| -> Option<Vec<String>> {
        let mut uuids: Vec<String> = finding
            .items
            .iter()
            .map(|item| item.uuid.clone())
            .collect::<Option<_>>()?;
        uuids.sort();
        Some(uuids)
    };
    if let (Some(mine), Some(reference)) = (uuids(ours), uuids(theirs))
        && mine == reference
    {
        let at = |finding: &DrcFinding, uuid: &str| {
            finding
                .items
                .iter()
                .find(|item| item.uuid.as_deref() == Some(uuid))
                .and_then(|item| item.at)
        };
        let rigid = mine.iter().enumerate().all(|(index, first)| {
            mine[index + 1..].iter().all(|second| {
                match (
                    at(ours, first),
                    at(ours, second),
                    at(theirs, first),
                    at(theirs, second),
                ) {
                    (Some(a), Some(b), Some(c), Some(d)) => {
                        (distance(a, b) - distance(c, d)).abs() <= SAME_PLACE_MM
                    }
                    _ => true,
                }
            })
        });
        if rigid {
            return true;
        }
    }
    let mut used = vec![false; theirs.items.len()];
    ours.items.iter().all(|item| {
        let Some(at) = item.at else {
            return false;
        };
        let found = theirs.items.iter().enumerate().position(|(index, other)| {
            !used[index]
                && other
                    .at
                    .is_some_and(|other| distance(at, other) <= SAME_PLACE_MM)
        });
        if let Some(index) = found {
            used[index] = true;
        }
        found.is_some()
    })
}

/// A result's errors split by the strict rule.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StrictSplit {
    /// Per type, the findings no reference finding is at the same place
    /// as (`same_place`), and every finding of a type in `unknown`.
    pub new: BTreeMap<String, usize>,
    /// Per type, the findings the reference has at the same place.
    pub preexisting: BTreeMap<String, usize>,
    /// Types of the result that the reference has at KiCad's cap
    /// (`KICAD_FINDING_CAP`): whether they were there before is unknown,
    /// so none is subtracted.
    pub unknown: Vec<String>,
    /// The run's own findings (`new`), as indices into the result's.
    pub new_indices: Vec<usize>,
}

/// Splits a result's findings into the reference's (the same type at the
/// same place, each reference finding answering for one) and the run's own.
pub fn split_findings_strict(ours: &[DrcFinding], reference: &[DrcFinding]) -> StrictSplit {
    let mut reference_counts: BTreeMap<&str, usize> = BTreeMap::new();
    for finding in reference {
        *reference_counts.entry(finding.kind.as_str()).or_default() += 1;
    }
    let capped = |kind: &str| {
        reference_counts
            .get(kind)
            .is_some_and(|count| *count >= KICAD_FINDING_CAP)
    };
    let mut split = StrictSplit::default();
    let mut used = vec![false; reference.len()];
    for (number, finding) in ours.iter().enumerate() {
        let kind = finding.kind.as_str();
        if capped(kind) {
            if !split.unknown.iter().any(|known| known == kind) {
                split.unknown.push(kind.to_string());
            }
            *split.new.entry(kind.to_string()).or_default() += 1;
            split.new_indices.push(number);
            continue;
        }
        let matched = reference
            .iter()
            .enumerate()
            .position(|(index, theirs)| !used[index] && same_place(finding, theirs));
        if let Some(index) = matched {
            used[index] = true;
            *split.preexisting.entry(kind.to_string()).or_default() += 1;
        } else {
            *split.new.entry(kind.to_string()).or_default() += 1;
            split.new_indices.push(number);
        }
    }
    split
}

/// What the run repaired to satisfy KiCad's DRC, beyond routing: tracks
/// that change no connection.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct KiCadRepairs {
    /// A pad's own straight track into its pour island (the kept
    /// attempt's `island_stubs` in board-router.json; off unless
    /// `PCB_ROUTER_ISLAND_STUBS` is set); `null`: not counted (a run
    /// without a routed board).
    pub island_stubs: Option<usize>,
    /// Tracks the connection-width repair kept (`neck_repair`), and their
    /// length.
    pub neck_tracks: usize,
    pub neck_track_mm: f64,
    /// The kept board's pour mode (`pours` of board-router.json:
    /// `connect`, `tracks`, `none`, ...), and whether it routed the pour
    /// nets as tracks.
    pub pour_mode: String,
    pub pours_as_tracks: bool,
}

impl KiCadRepairs {
    pub fn of(result: &crate::KiCadBoardRouterResult) -> Self {
        let neck = result.neck_repair.as_ref().filter(|repair| repair.kept);
        Self {
            island_stubs: Some(result.island_stubs),
            neck_tracks: neck.map_or(0, |repair| repair.tracks),
            neck_track_mm: neck.map_or(0.0, |repair| repair.track_mm),
            pour_mode: result.pours.clone(),
            pours_as_tracks: result.pours == "tracks" || result.pours.contains("as tracks"),
        }
    }
}

/// Environment variables that change what a run does or reports (the
/// router's seed and switches, KiCad's check and its repairs, the budgets'
/// guards), echoed into the verdict when set.
pub const RESULT_ENVIRONMENT: [&str; 15] = [
    "PCB_ROUTER_SEED",
    "PCB_ROUTER_SEEDS",
    "PCB_ROUTER_ISLAND_STUBS",
    "PCB_ROUTER_NO_WALL_GUARD",
    "PCB_SKIP_NATIVE",
    "PCB_MIN_CONNECTION_FLOOR",
    "PCB_KICAD_NO_NECK_REPAIR",
    "PCB_KICAD_TIMEOUT",
    "PCB_MAKER_KICAD_USER_LIBS",
    "PCB_PLACER_WORK_SECONDS",
    "PCB_NARROW_RESERVE",
    "PCB_PROBES_SIDE_BY_SIDE",
    "PCB_LADDER_STATS",
    "PCB_WORK_RATE",
    "PCB_MAKER_TEST_PANIC_ATTEMPT",
];

/// The variables of `RESULT_ENVIRONMENT` that are set, with their values.
pub fn result_environment() -> BTreeMap<String, String> {
    RESULT_ENVIRONMENT
        .iter()
        .filter_map(|name| {
            std::env::var_os(name).map(|value| (name.to_string(), value.to_string_lossy().into()))
        })
        .collect()
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
    /// Route mode: the parts of `work_seconds` that are pour analyses and
    /// fill probes, and lattice builds and lowerings (the rest is search).
    pub pour_work_seconds: Option<f64>,
    pub lattice_work_seconds: Option<f64>,
    pub wall_seconds: f64,
    pub outputs: BTreeMap<String, String>,
    pub skipped: Vec<String>,
    /// Attempts of the router's ladder that panicked, each with its
    /// message (`attempts_failed` in board-router.json).
    pub attempts_failed: Vec<String>,
    /// Whether the run routed (a placement alone has no copper to judge).
    pub routes: bool,
    /// The placement's room (`placement_room`), when there was one.
    pub room: Option<crate::placement_room::KiCadRoom>,
    /// What the run repaired for KiCad's DRC (a route's or a layout's).
    pub repairs: Option<KiCadRepairs>,
    /// The project's routing rules the router does not keep
    /// (`KiCadProjectRulesReport::unmodelled`).
    pub unmodelled_rules: Vec<String>,
    /// What the input check found before the run (a keepout over a fixed
    /// part: `keepout_warnings`); next_actions repeat them.
    pub input_warnings: Vec<String>,
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
            pour_work_seconds: None,
            lattice_work_seconds: None,
            wall_seconds: 0.0,
            outputs: BTreeMap::new(),
            skipped: Vec::new(),
            attempts_failed: Vec::new(),
            routes: true,
            room: None,
            repairs: None,
            unmodelled_rules: Vec::new(),
            input_warnings: Vec::new(),
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
    /// Whether KiCad's DRC checked the result (false with
    /// `PCB_SKIP_NATIVE` or `skip_native_verification`, and for a
    /// placement).
    pub kicad_checked: bool,
    /// KiCad's unconnected items on the result (`None`: KiCad did not
    /// check it). This, not the router's count, decides.
    pub kicad_unconnected_items: Option<usize>,
    /// KiCad errors per type that the reference board does not have at the
    /// same place (`split_findings_strict`): the run's own. They decide.
    pub new_findings_strict: BTreeMap<String, usize>,
    /// KiCad errors per type beyond the reference's count of that type
    /// anywhere on the board: the old rule, kept for comparisons (it hides
    /// a new error of a type the reference has).
    pub new_findings: BTreeMap<String, usize>,
    /// KiCad errors per type the reference board has at the same place
    /// (the designer's own findings); they do not count.
    pub preexisting_findings: BTreeMap<String, usize>,
    /// Types the reference has at KiCad's cap (199 findings or more of the
    /// type): not subtracted, since its true count is unknown.
    pub preexisting_unknown: Vec<String>,
    /// KiCad checked the result and finds no unconnected item and no error
    /// that counts, with nothing subtracted.
    pub clean_without_subtraction: bool,
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
    /// Route mode: the part of `work_seconds` that is the pour analyses
    /// and fill probes (every phase's: the negotiation's pour refresh, the
    /// finishing phases' pour checks, the manual spokes), modelled from
    /// their counts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pour_work_seconds: Option<f64>,
    /// Route mode: the part of `work_seconds` that is building the
    /// routers' lattices and lowering the board, modelled from their
    /// counts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lattice_work_seconds: Option<f64>,
    pub wall_seconds: f64,
    /// `work_seconds` over `wall_seconds`: about 1 where the work rates
    /// fit the machine (they are measured at the start unless pinned),
    /// lower where the wall clock ran ahead of the work (load, KiCad's
    /// checks beyond their model, a pinned rate faster than the machine).
    pub work_wall_ratio: Option<f64>,
    /// The search expansions a second of work stood for (`work_rate`).
    pub work_rate: f64,
    /// The placement units a second of work stood for.
    pub placer_work_rate: f64,
    /// Where the rates came from: `measured` (the start-up calibration),
    /// `PCB_WORK_RATE`, `config` (the router settings `work_rate`,
    /// `placer_work_rate`) or `reference` (none calibrated).
    pub work_rate_source: String,
    /// What the start-up calibration measured, also beside a pinned rate.
    pub work_rate_measured: Option<f64>,
    pub placer_work_rate_measured: Option<f64>,
    /// How many times the wall-clock guards stretched for pinned rates
    /// faster than the machine measured (`KiCadWorkRate::wall_stretch`; 1
    /// with the rates measured).
    pub wall_stretch: f64,
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
    /// What the run repaired for KiCad's DRC (`KiCadRepairs`; absent for a
    /// placement and an error).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repairs: Option<KiCadRepairs>,
    /// The result-changing environment variables that were set
    /// (`RESULT_ENVIRONMENT`), with their values.
    pub environment: BTreeMap<String, String>,
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
        if facts.routes && facts.native.is_none() {
            return (
                KiCadRunStatus::RouterCompleteUnverified,
                format!(
                    "router_complete_unverified: the router's count finds {}/{} connections and no clearance violation, but KiCad did not check the board (PCB_SKIP_NATIVE or skip_native_verification)",
                    facts.routed_connections, facts.routable_connections
                ),
            );
        }
        let reason = if !facts.routes {
            "complete: every part placed, every constraint kept".to_string()
        } else {
            format!(
                "complete: {}/{} connections, KiCad finds no unconnected item and no error beyond the reference's at the same place",
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
    own: &[&DrcFinding],
    unknown: &[String],
) -> Vec<String> {
    let mut actions = facts.input_warnings.clone();
    if !facts.attempts_failed.is_empty() {
        actions.push(format!(
            "{} attempts of the router's ladder failed (a panic; attempts_failed in board-router.json has each message): the board is the best of the other attempts; report the panic ({})",
            facts.attempts_failed.len(),
            facts.attempts_failed.join("; ")
        ));
    }
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
        if let Some(native) = facts
            .native
            .as_ref()
            .filter(|native| !native.unconnected_nets.is_empty())
        {
            where_.push(format!(
                "KiCad's unconnected items are on {}",
                top_counts(&native.unconnected_nets, 8)
            ));
        }
        actions.push(format!(
            "connections are open: {}; make room there (move parts apart, enlarge the outline, add layers) or narrow the net classes",
            if where_.is_empty() { "the router's count; drc.json lists KiCad's unconnected items".to_string() } else { where_.join(", ") }
        ));
    }
    let thermal_action = facts.starved_thermals > 0 && new_findings.contains_key("starved_thermal");
    if thermal_action {
        actions.push(format!(
            "{} pads are short of thermal spokes (diagnostics.starved_thermals has advice for each); \"solid_starved_thermals\" (on unless false in the router config) connects them solid",
            facts.starved_thermals
        ));
    }
    let other: BTreeMap<String, usize> = new_findings
        .iter()
        .filter(|(kind, _)| kind.as_str() != "starved_thermal")
        .map(|(kind, count)| (kind.clone(), *count))
        .collect();
    if !other.is_empty() || (!thermal_action && new_findings.contains_key("starved_thermal")) {
        let drc = facts.outputs.get("drc").map_or("drc.json", String::as_str);
        actions.push(format!(
            "new DRC errors ({}): see {drc}",
            counts(new_findings)
        ));
        // The largest types, each with what to do.
        let mut kinds: Vec<(&String, &usize)> = new_findings
            .iter()
            .filter(|(kind, _)| !(thermal_action && kind.as_str() == "starved_thermal"))
            .collect();
        kinds.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        for (kind, count) in kinds.into_iter().take(FINDING_ACTIONS) {
            let findings: Vec<&DrcFinding> = own
                .iter()
                .copied()
                .filter(|finding| &finding.kind == kind)
                .collect();
            actions.push(finding_action(kind, *count, &findings));
        }
    }
    if !facts.constraints_missed.is_empty() {
        actions.push("constraints missed (constraints[] has violation_mm for each): relax them or give the parts room".into());
    }
    if !unknown.is_empty() {
        actions.push(format!(
            "the reference board has {} at KiCad's cap of {KICAD_FINDING_CAP} findings: none of them is subtracted (preexisting_unknown)",
            unknown.join(", ")
        ));
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
    if facts.routes && status != KiCadRunStatus::Complete && !facts.unmodelled_rules.is_empty() {
        actions.push(unmodelled_rules_action(&facts.unmodelled_rules));
    }
    if facts.routes && facts.native.is_none() {
        actions.push("KiCad's check did not run (PCB_SKIP_NATIVE or skip_native_verification): the router's own count is no verdict; run KiCad's DRC (pcb-maker drc-kicad-board) before using the board".into());
    }
    if status != KiCadRunStatus::Complete {
        actions.extend(facts.skipped.iter().cloned());
    }
    actions
}

/// The finding types the verdict gives an action of their own, the
/// largest first.
const FINDING_ACTIONS: usize = 4;

/// `map`'s largest entries as `name (count)`, at most `limit` and how many
/// more.
fn top_counts(map: &BTreeMap<String, usize>, limit: usize) -> String {
    let mut entries: Vec<(&String, &usize)> = map.iter().collect();
    entries.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    let mut text: Vec<String> = entries
        .iter()
        .take(limit)
        .map(|(name, count)| format!("{name} ({count})"))
        .collect();
    if entries.len() > limit {
        text.push(format!("{} more", entries.len() - limit));
    }
    text.join(", ")
}

/// One finding type's next action: what KiCad found (the rule, the nets
/// or the items of the type's findings) and the setting or step that
/// addresses it, where there is one.
fn finding_action(kind: &str, count: usize, findings: &[&DrcFinding]) -> String {
    // The rule as KiCad states it: the first parenthesis of the first
    // finding's description ("Clearance violation (netclass 'Default'
    // clearance 0.2000 mm; actual 0.1500 mm)"; a layer may follow in a
    // second one).
    let rule = findings.first().and_then(|finding| {
        let text = &finding.description;
        let start = text.find('(')? + 1;
        let mut depth = 1;
        let end = text[start..].char_indices().find_map(|(index, c)| {
            match c {
                '(' => depth += 1,
                ')' => depth -= 1,
                _ => {}
            }
            (depth == 0).then_some(start + index)
        })?;
        let rule = text[start..end].trim();
        (!rule.is_empty()).then(|| rule.to_string())
    });
    let mut nets: BTreeMap<String, usize> = BTreeMap::new();
    for finding in findings {
        let mut names = finding.nets.clone();
        names.dedup();
        if !names.is_empty() {
            *nets.entry(names.join(" and ")).or_default() += 1;
        }
    }
    let mut what = Vec::new();
    if let Some(rule) = rule {
        what.push(rule);
    }
    if !nets.is_empty() {
        what.push(format!("nets {}", top_counts(&nets, 4)));
    } else if let Some(finding) = findings.first() {
        let items: Vec<&str> = finding
            .item_descriptions
            .iter()
            .take(2)
            .map(String::as_str)
            .collect();
        if !items.is_empty() {
            what.push(format!("first: {}", items.join("; ")));
        }
    }
    let step = match kind {
        "clearance" | "shorting_items" => {
            "move those nets' parts apart, or check whether the rule is one the router does not keep (rules.project.unmodelled in the report)"
        }
        "hole_clearance" | "hole_to_hole" => {
            "move the parts apart; the project's min_hole_clearance and min_hole_to_hole (Board Setup) set the rule"
        }
        "copper_edge_clearance" => {
            "for a part at an edge, its edge constraint's max_mm or flush (constraints.md, edge); otherwise the project's min_copper_edge_clearance"
        }
        "items_not_allowed" => {
            "an item lies in a rule area (a keepout): move the keepout or the part, or let the rule area allow it"
        }
        "solder_mask_bridge" => {
            "copper of two nets under one mask opening: the project's solder_mask_to_copper_clearance and the solder_mask_bridge severity (Board Setup) decide, and the router keeps them"
        }
        "courtyards_overlap" => {
            "placer.tight_bodies false keeps bodies to their courtyards; or give the parts room"
        }
        "starved_thermal" => {
            "\"solid_starved_thermals\": true in the router config connects the pads solid"
        }
        "connection_width" | "track_width" => {
            "the project's min_connection and the classes' widths (Board Setup) set the rule"
        }
        _ => "see drc.json",
    };
    if what.is_empty() {
        format!("{kind} {count}: {step}")
    } else {
        format!("{kind} {count} ({}): {step}", what.join("; "))
    }
}

/// The next action for a project with rules the router does not keep.
fn unmodelled_rules_action(rules: &[String]) -> String {
    format!(
        "the project has rules the router does not keep, which KiCad checks: their findings are for the designer to resolve by hand or by moving parts: {}",
        rules.join("; ")
    )
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
    let rate = crate::work_rate::work_rate();
    let errors = facts
        .native
        .as_ref()
        .map(|native| native.errors.clone())
        .unwrap_or_default();
    let findings = facts
        .native
        .as_ref()
        .map(|native| native.findings.clone())
        .unwrap_or_default();
    let (new_findings, strict, preexisting_from) = match &facts.reference {
        KiCadReference::Findings {
            from,
            errors: reference,
            findings: reference_findings,
        } => (
            split_findings(&errors, reference).0,
            split_findings_strict(&findings, reference_findings),
            Some(from.display().to_string()),
        ),
        KiCadReference::None | KiCadReference::Failed { .. } => (
            errors.clone(),
            StrictSplit {
                new: errors,
                new_indices: (0..findings.len()).collect(),
                ..Default::default()
            },
            None,
        ),
    };
    let own: Vec<&DrcFinding> = strict
        .new_indices
        .iter()
        .filter_map(|&index| findings.get(index))
        .collect();
    let (status, reason) = decide(facts, &strict.new);
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
        kicad_checked: facts.native.is_some(),
        kicad_unconnected_items: facts.native.as_ref().map(|native| native.unconnected),
        next_actions: next_actions(facts, status, &strict.new, &own, &strict.unknown),
        clean_without_subtraction: facts
            .native
            .as_ref()
            .is_some_and(|native| native.unconnected == 0 && native.errors.is_empty()),
        new_findings_strict: strict.new,
        new_findings,
        preexisting_findings: strict.preexisting,
        preexisting_unknown: strict.unknown,
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
        pour_work_seconds: facts.pour_work_seconds,
        lattice_work_seconds: facts.lattice_work_seconds,
        wall_seconds: facts.wall_seconds,
        work_wall_ratio: work_wall_ratio(facts.work_seconds, facts.wall_seconds),
        work_rate: rate.expansions_per_second,
        placer_work_rate: rate.placer_work_per_second,
        work_rate_source: rate.source.clone(),
        work_rate_measured: rate
            .measured
            .as_ref()
            .map(|measured| measured.expansions_per_second),
        placer_work_rate_measured: rate
            .measured
            .as_ref()
            .map(|measured| measured.placer_work_per_second),
        wall_stretch: rate.wall_stretch(!facts.command.starts_with("route")),
        outputs: facts.outputs.clone(),
        skipped: facts.skipped.clone(),
        room: facts.room.clone(),
        repairs: facts.repairs.clone(),
        environment: result_environment(),
    }
}

/// Work over wall clock (`None` without work or wall).
pub(crate) fn work_wall_ratio(work_seconds: Option<f64>, wall_seconds: f64) -> Option<f64> {
    work_seconds
        .filter(|_| wall_seconds > 0.0)
        .map(|work| work / wall_seconds)
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
    verdict.next_actions = if error.contains(crate::KICAD_REQUIREMENT) {
        vec![format!(
            "{}; then run again (into the same output directory: it holds only this verdict)",
            crate::KICAD_REQUIREMENT
        )]
    } else if error.contains(crate::native_report::KICAD_TIMEOUT_MESSAGE) {
        vec!["kicad-cli did not finish within its limit: set PCB_KICAD_TIMEOUT to more seconds (default 900) and run again into a new output directory".into()]
    } else if status == KiCadRunStatus::UsageError {
        let mut actions = noted_input_actions();
        actions.push("fix the arguments or input files the error names; nothing was routed".into());
        actions
    } else {
        vec!["an internal error: the output directory may hold partial results; report the error with the input".into()]
    };
    verdict
}

/// The router's facts for a route's verdict (`directory`: the routed
/// project, with KiCad's `drc.json`). When KiCad checked the board, its
/// report must be there: a missing or unreadable one is an error (the
/// verdict would otherwise rest on the router's count while saying KiCad
/// had judged).
pub fn route_facts(
    command: &str,
    result: &crate::KiCadBoardRouterResult,
    directory: &Path,
) -> Result<KiCadRunFacts, String> {
    let mut facts = KiCadRunFacts::new(command);
    if result.native.is_some() {
        let report = directory.join("drc.json");
        facts.native = Some(load_drc_tally(&report).map_err(|error| {
            format!(
                "KiCad checked the board but its report is missing or unreadable ({error}); no verdict without it"
            )
        })?);
    }
    facts.repairs = Some(KiCadRepairs::of(result));
    facts.routable_connections = result.routable_connections;
    facts.routed_connections = result.routed_connections;
    facts.unconnected_terminals = result.unconnected_terminals;
    facts.internal_violations = result.internal_violations.len();
    facts.starved_thermals = result.diagnostics.starved_thermals.len();
    facts.dead_pads = result.diagnostics.dead_pads.len();
    facts.hot_spots = result.diagnostics.hot_spots.len();
    facts.cut = result.cut.clone();
    facts.skipped = result.skipped.clone();
    facts.unmodelled_rules = result.rules.project.unmodelled.clone();
    facts.attempts_failed = result.attempts_failed.clone();
    // What the board's reading left out (copper on a layer the stack
    // lacks): the next_actions lead with it.
    facts.input_warnings = result.warnings.clone();
    Ok(facts)
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

/// What the input's error asks of the agent, noted where the error is
/// found (`note_input_action`), for the verdict's next_actions.
static INPUT_ACTIONS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Notes an action that fixes an input error about to be returned (the
/// outline's extent and the pitch of a lattice that does not fit): an
/// input error's verdict (`error_verdict`) lists it first.
pub fn note_input_action(action: impl Into<String>) {
    if let Ok(mut actions) = INPUT_ACTIONS.lock() {
        let action = action.into();
        if !actions.contains(&action) {
            actions.push(action);
        }
    }
}

/// The actions noted for input errors so far (`note_input_action`).
pub fn noted_input_actions() -> Vec<String> {
    INPUT_ACTIONS
        .lock()
        .map(|actions| actions.clone())
        .unwrap_or_default()
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

    fn finding(kind: &str, items: &[(&str, f64, f64)]) -> serde_json::Value {
        json!({"type": kind, "severity": "error", "description": kind,
               "items": items.iter().map(|(uuid, x, y)| json!({"description": "item", "uuid": uuid, "pos": {"x": x, "y": y}})).collect::<Vec<_>>()})
    }

    fn report(violations: Vec<serde_json::Value>) -> serde_json::Value {
        json!({"violations": violations, "unconnected_items": [], "schematic_parity": []})
    }

    fn reference(violations: Vec<serde_json::Value>) -> KiCadReference {
        let tally = tally_drc(&report(violations));
        KiCadReference::Findings {
            from: "designer".into(),
            errors: tally.errors,
            findings: tally.findings,
        }
    }

    #[test]
    fn a_designer_finding_does_not_make_a_route_incomplete() {
        // USB_Cable_Tester in route mode: 0 unconnected, one courtyard
        // overlap that the designer's board has too (exit 1 before).
        let overlap = finding("courtyards_overlap", &[("a", 1.0, 2.0), ("b", 3.0, 2.0)]);
        let mut facts = routed(tally_drc(&report(vec![overlap.clone()])));
        assert!(facts.needs_reference());
        assert_eq!(judge(&facts).exit_code, EXIT_INCOMPLETE);
        facts.reference = reference(vec![
            overlap,
            finding("courtyards_overlap", &[("c", 9.0, 9.0), ("d", 9.5, 9.0)]),
            finding("clearance", &[("e", 5.0, 5.0)]),
        ]);
        let verdict = judge(&facts);
        assert_eq!(verdict.status, KiCadRunStatus::Complete);
        assert_eq!(verdict.exit_code, EXIT_COMPLETE);
        assert!(verdict.new_findings_strict.is_empty());
        assert!(verdict.new_findings.is_empty());
        assert!(!verdict.clean_without_subtraction);
        assert_eq!(
            verdict.preexisting_findings,
            map(&[("courtyards_overlap", 1)])
        );
        assert!(verdict.next_actions.is_empty());
    }

    #[test]
    fn only_a_finding_at_the_references_place_is_preexisting() {
        // The designer has one clearance finding; the result has it and a
        // new one of the same type elsewhere: the old per-type rule hid the
        // new one, the strict rule counts it.
        let theirs = finding("clearance", &[("p1", 10.0, 10.0), ("p2", 10.4, 10.0)]);
        let elsewhere = finding("clearance", &[("t1", 30.0, 12.0), ("p9", 30.3, 12.0)]);
        let mut facts = routed(tally_drc(&report(vec![theirs.clone(), elsewhere])));
        facts.reference = reference(vec![
            theirs,
            finding("clearance", &[("p5", 50.0, 50.0), ("p6", 50.2, 50.0)]),
        ]);
        let verdict = judge(&facts);
        assert!(verdict.new_findings.is_empty(), "the old rule hides it");
        assert_eq!(verdict.new_findings_strict, map(&[("clearance", 1)]));
        assert_eq!(verdict.preexisting_findings, map(&[("clearance", 1)]));
        assert_eq!(verdict.status, KiCadRunStatus::Incomplete);
        assert!(verdict.reason.contains("clearance 1"), "{}", verdict.reason);
    }

    #[test]
    fn findings_match_by_items_moved_together_or_by_position() {
        let at = |kind: &str, items: &[(Option<&str>, f64, f64)]| DrcFinding {
            kind: kind.into(),
            description: String::new(),
            nets: Vec::new(),
            item_descriptions: Vec::new(),
            items: items
                .iter()
                .map(|(uuid, x, y)| DrcItem {
                    uuid: uuid.map(str::to_string),
                    at: Some([*x, *y]),
                    zone: false,
                    unlocated: false,
                })
                .collect(),
            ..DrcFinding::default()
        };
        // A footprint's own hole-to-hole finding moves and turns with it.
        let designer = at(
            "hole_to_hole",
            &[(Some("h1"), 85.0, 13.8), (Some("h2"), 85.0, 13.2)],
        );
        let moved = at(
            "hole_to_hole",
            &[(Some("h2"), 20.0, 5.0), (Some("h1"), 20.6, 5.0)],
        );
        assert!(same_place(&moved, &designer));
        // The same two items no longer side by side: not the same finding.
        let apart = at(
            "hole_to_hole",
            &[(Some("h1"), 20.0, 5.0), (Some("h2"), 22.0, 5.0)],
        );
        assert!(!same_place(&apart, &designer));
        // New items (our tracks) at the designer's positions match there.
        let ours = at(
            "hole_to_hole",
            &[(Some("x1"), 85.02, 13.8), (None, 85.0, 13.23)],
        );
        assert!(same_place(&ours, &designer));
        let off = at(
            "hole_to_hole",
            &[(Some("x1"), 85.2, 13.8), (None, 85.0, 13.2)],
        );
        assert!(!same_place(&off, &designer));
        // Another type never matches.
        let clearance = at(
            "clearance",
            &[(Some("h1"), 85.0, 13.8), (Some("h2"), 85.0, 13.2)],
        );
        assert!(!same_place(&clearance, &designer));
        // Each reference finding answers for one of ours.
        let split = split_findings_strict(&[designer.clone(), designer.clone()], &[designer]);
        assert_eq!(split.new, map(&[("hole_to_hole", 1)]));
        assert_eq!(split.preexisting, map(&[("hole_to_hole", 1)]));
    }

    /// KiCad's connection_width on a pour: the zone item at (0, 0).
    fn zone_neck(actual: f64, own: Option<(f64, f64)>) -> serde_json::Value {
        let mut finding = json!({"type": "connection_width", "severity": "error",
            "description": format!("Copper connection too narrow (board setup constraints minimum connection width 0.1270 mm; actual {actual:.4} mm) (F.Cu)"),
            "items": [{"description": "Zone [GND] on F.Cu and B.Cu, priority 0",
                       "uuid": "8951654c-dd26-460d-9f97-f89e9425e93d", "pos": {"x": 0.0, "y": 0.0}}]});
        if let Some((x, y)) = own {
            finding["pos"] = json!({"x": x, "y": y});
        }
        finding
    }

    #[test]
    fn a_one_item_finding_is_the_designers_only_where_its_item_is() {
        // USB_Keypad (designer-x1315b): our one connection_width on the GND
        // pour, the stripped designer board 45 on the same zone, all at
        // (0, 0): the uuid alone matched them until 2026-10-22.
        let ours = tally_drc(&report(vec![zone_neck(0.1098, None)])).findings;
        let designer: Vec<serde_json::Value> = (0..45)
            .map(|index| zone_neck(0.05 + f64::from(index) / 1000.0, None))
            .collect();
        let theirs = tally_drc(&report(designer)).findings;
        assert!(ours[0].items[0].unlocated);
        let split = split_findings_strict(&ours, &theirs);
        assert_eq!(split.new, map(&[("connection_width", 1)]));
        assert!(split.preexisting.is_empty());
        // A finding with a position of its own is placed there.
        let placed = |x: f64| tally_drc(&report(vec![zone_neck(0.1, Some((x, 30.0)))])).findings;
        assert!(same_place(&placed(20.0)[0], &placed(20.03)[0]));
        assert!(!same_place(&placed(20.0)[0], &placed(25.0)[0]));
        // A pad's one-item finding: by its place, not by its uuid.
        let pad = |uuid: &str, x: f64| {
            tally_drc(&report(vec![finding(
                "starved_thermal",
                &[(uuid, x, 10.0)],
            )]))
            .findings
        };
        assert!(same_place(&pad("p1", 10.02)[0], &pad("p1", 10.0)[0]));
        assert!(same_place(&pad("q7", 10.04)[0], &pad("p1", 10.0)[0]));
        // The same pad elsewhere with the same finding word for word (a
        // pad's own annular ring, moved with its part): the designer's.
        assert!(same_place(&pad("p1", 30.0)[0], &pad("p1", 10.0)[0]));
        let mut other = pad("p1", 30.0);
        other[0].description = "starved_thermal (actual 1)".into();
        assert!(!same_place(&other[0], &pad("p1", 10.0)[0]));
        // Never a zone: the 45 above have other words, the same would not do.
        let same_words = tally_drc(&report(vec![zone_neck(0.1098, None)])).findings;
        assert!(!same_place(&ours[0], &same_words[0]));
    }

    #[test]
    fn a_type_at_kicads_cap_is_unknown_and_not_subtracted() {
        let ours = finding("hole_clearance", &[("a", 1.0, 1.0)]);
        let mut capped: Vec<serde_json::Value> = (0..KICAD_FINDING_CAP)
            .map(|index| finding("hole_clearance", &[("z", index as f64, 40.0)]))
            .collect();
        capped.push(ours.clone());
        let mut facts = routed(tally_drc(&report(vec![ours])));
        facts.reference = reference(capped);
        let verdict = judge(&facts);
        assert_eq!(
            verdict.preexisting_unknown,
            vec!["hole_clearance".to_string()]
        );
        assert_eq!(verdict.new_findings_strict, map(&[("hole_clearance", 1)]));
        assert!(verdict.new_findings.is_empty());
        assert_eq!(verdict.exit_code, EXIT_INCOMPLETE);
        assert!(
            verdict
                .next_actions
                .iter()
                .any(|action| action.contains("KiCad's cap"))
        );
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
    fn without_kicads_check_a_route_is_never_complete() {
        let mut facts = routed(DrcTally::default());
        facts.native = None;
        let verdict = judge(&facts);
        assert_eq!(verdict.status, KiCadRunStatus::RouterCompleteUnverified);
        assert_eq!(verdict.exit_code, EXIT_INCOMPLETE);
        assert!(!verdict.kicad_checked);
        assert!(!verdict.clean_without_subtraction);
        assert!(
            verdict.reason.starts_with("router_complete_unverified"),
            "{}",
            verdict.reason
        );
        assert!(!verdict.reason.contains("complete:"), "{}", verdict.reason);
        assert_eq!(
            serde_json::to_value(verdict.status).unwrap(),
            "router_complete_unverified"
        );
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
    fn input_warnings_lead_the_next_actions_whatever_the_status() {
        let mut facts = routed(DrcTally::default());
        facts.command = "layout-kicad-board".into();
        facts.input_warnings =
            vec!["keepout 1 (x 48..68, y 3.5..6) meets the courtyards of ... (J2)".into()];
        let verdict = judge(&facts);
        assert_eq!(verdict.exit_code, EXIT_COMPLETE);
        assert_eq!(verdict.next_actions, facts.input_warnings);
        facts.native = Some(DrcTally {
            unconnected: 2,
            ..Default::default()
        });
        facts.skipped = vec!["the finer lattice (0.075 mm) was not tried: projected 367 s of routing exceeds refine_budget_seconds (240 s); \"refine_budget_seconds\": 367 or more in the router config tries it".into()];
        let verdict = judge(&facts);
        assert!(verdict.next_actions[0].starts_with("keepout 1"));
        assert!(
            verdict
                .next_actions
                .iter()
                .any(|action| action.contains("\"refine_budget_seconds\": 367"))
        );
    }

    #[test]
    fn new_drc_errors_name_their_rule_nets_and_step() {
        let item = |description: &str, x: f64| json!({"description": description, "pos": {"x": x, "y": 1.0}, "uuid": format!("u{x}")});
        let report = json!({
            "violations": [
                {"type": "clearance", "severity": "error",
                 "description": "Clearance violation (netclass 'Default' clearance 0.2000 mm; actual 0.1500 mm)",
                 "items": [item("Track [GND] on F.Cu, length 1.0000 mm", 1.0), item("Pad 1 [/SDA] of U1 on F.Cu", 2.0)]},
                {"type": "clearance", "severity": "error",
                 "description": "Clearance violation (netclass 'Default' clearance 0.2000 mm; actual 0.1800 mm)",
                 "items": [item("Via [GND] on F.Cu - B.Cu", 3.0), item("Pad 2 [/SDA] of U1 on F.Cu", 4.0)]},
                {"type": "starved_thermal", "severity": "error",
                 "description": "Thermal relief connection to zone incomplete (layer F.Cu; zone min spoke count 2; actual 1)",
                 "items": [item("Zone [GND] on F.Cu", 5.0), item("Pad 29 [GND] of U1 on F.Cu", 6.0)]},
                {"type": "items_not_allowed", "severity": "error",
                 "description": "Items not allowed (Keepout)",
                 "items": [item("Rule Area [<no net>] on F.Cu", 7.0), item("Footprint R1", 8.0)]}
            ],
            "unconnected_items": [{"type": "unconnected_items", "items": [item("Pad 1 [+3V3] of C1 on F.Cu", 9.0), item("Pad 2 [+3V3] of U1 on F.Cu", 10.0)]}],
            "schematic_parity": []
        });
        let tally = tally_drc(&report);
        assert_eq!(
            tally.unconnected_nets,
            BTreeMap::from([("+3V3".to_string(), 1)])
        );
        let mut facts = routed(tally);
        facts.routed_connections = 53;
        let verdict = judge(&facts);
        let actions = verdict.next_actions.join("\n");
        assert!(
            actions.contains("KiCad's unconnected items are on +3V3 (1)"),
            "{actions}"
        );
        assert!(
            actions.contains("clearance 2 (netclass 'Default' clearance 0.2000 mm; actual 0.1500 mm; nets GND and /SDA (2)): move those nets' parts apart"),
            "{actions}"
        );
        // The router counted no starved pad: the type's own action says
        // what connects them.
        assert!(
            actions.contains("starved_thermal 1 (layer F.Cu"),
            "{actions}"
        );
        assert!(actions.contains("solid_starved_thermals"), "{actions}");
        assert!(
            actions.contains("items_not_allowed 1 (Keepout; first: Rule Area [<no net>] on F.Cu; Footprint R1): an item lies in a rule area"),
            "{actions}"
        );
        // A layer in a second parenthesis, a rule without a name.
        let finding = DrcFinding {
            kind: "connection_width".into(),
            description: "Width (board setup constraints minimum connection width 0.1270 mm; actual 0.1137 mm) (F.Cu)".into(),
            ..DrcFinding::default()
        };
        assert!(
            finding_action("connection_width", 1, &[&finding]).starts_with(
                "connection_width 1 (board setup constraints minimum connection width 0.1270 mm; actual 0.1137 mm): "
            )
        );
        let finding = DrcFinding {
            kind: "clearance".into(),
            description: "Clearance violation ( clearance 0.1270 mm; actual 0.0000 mm)".into(),
            ..DrcFinding::default()
        };
        assert!(
            finding_action("clearance", 1, &[&finding])
                .starts_with("clearance 1 (clearance 0.1270 mm; actual 0.0000 mm): ")
        );
    }

    #[test]
    fn kicad_problems_name_the_requirement_and_the_timeout_variable() {
        let missing = error_verdict(
            "route-kicad-board",
            KiCadRunStatus::UsageError,
            &format!(
                "kicad-cli version did not run or failed: {}",
                crate::KICAD_REQUIREMENT
            ),
            0.1,
            BTreeMap::new(),
        );
        assert_eq!(missing.exit_code, EXIT_USAGE);
        assert!(missing.next_actions[0].contains("PCB_KICAD_CLI"));
        assert!(missing.next_actions[0].contains("KiCad 10"));
        let slow = error_verdict(
            "layout-kicad-board",
            KiCadRunStatus::InternalError,
            "kicad-cli timed out after 900 s (PCB_KICAD_TIMEOUT sets the limit in seconds, default 900)",
            0.1,
            BTreeMap::new(),
        );
        assert!(slow.next_actions[0].contains("PCB_KICAD_TIMEOUT"));
    }

    #[test]
    fn a_failed_reference_counts_every_error() {
        let mut facts = routed(tally_drc(&report(vec![finding(
            "clearance",
            &[("a", 1.0, 1.0)],
        )])));
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
            KiCadRunStatus::RouterCompleteUnverified,
        ]
        .iter()
        .map(|status| status.exit_code())
        .collect();
        assert_eq!(codes, vec![0, 1, 2, 3, 4, 1]);
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
