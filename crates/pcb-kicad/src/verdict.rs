// Copyright (C) 2026 Toit contributors.

//! The verdict of a command run, for agents: whether the result is
//! complete, and the process's exit code that says so.
//!
//! The rule: KiCad's own DRC on the result decides. A result is complete
//! when KiCad reports no unconnected item and no error beyond the ones the
//! reference board (the designer's, or the input of a route) already has
//! at the same place: the same type and the same items (by uuid, moved
//! together) or items at the same positions (`split_findings_strict`). A
//! type the reference has at KiCad's cap (`KICAD_FINDING_CAP`) is
//! subtracted only at the same place; a finding between items of one
//! footprint is the reference's (`footprint_own`). Warnings, excluded findings and cosmetic types (silkscreen,
//! text, library metadata, missing courtyards, isolated copper) do not
//! count. A layout or placement must also keep its placement constraints
//! and place every part. Without KiCad's check (it was skipped) the
//! router's own count can only say `router_complete_unverified` (exit 1);
//! a check that ran but left no readable report is an internal error.

use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
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

/// One violation of KiCad's report with its items.
pub fn drc_finding(finding: &Value) -> DrcFinding {
    let item_descriptions: Vec<String> = finding["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| item["description"].as_str().map(str::to_string))
        .collect();
    DrcFinding {
        kind: finding["type"].as_str().unwrap_or("").to_string(),
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
    }
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
            tally.findings.push(drc_finding(finding));
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
/// A `copper_edge_clearance` is at the same place by its copper item
/// alone: which piece of the outline KiCad names beside it varies (the
/// nearest of several within the clearance; Batch_1's GND ring along a
/// card's edge). run.py's strict rule 4.
pub fn same_place(ours: &DrcFinding, theirs: &DrcFinding) -> bool {
    if ours.kind != theirs.kind || ours.items.is_empty() {
        return false;
    }
    if ours.kind == "copper_edge_clearance" {
        let copper = |finding: &DrcFinding| {
            let mut copper = finding.clone();
            let kept: Vec<bool> = (0..finding.items.len())
                .map(|index| {
                    !finding
                        .item_descriptions
                        .get(index)
                        .is_some_and(|description| description.ends_with("on Edge.Cuts"))
                })
                .collect();
            let mut keep = kept.iter();
            copper.items.retain(|_| *keep.next().unwrap_or(&true));
            let mut keep = kept.iter();
            copper
                .item_descriptions
                .retain(|_| *keep.next().unwrap_or(&true));
            copper
        };
        let (mine, reference) = (copper(ours), copper(theirs));
        if !mine.items.is_empty()
            && !reference.items.is_empty()
            && (mine.items.len(), reference.items.len()) != (ours.items.len(), theirs.items.len())
        {
            return same_place(&mine, &reference);
        }
    }
    if ours.items.len() != theirs.items.len() {
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

/// The footprint a finding's item belongs to, from KiCad's description:
/// the reference it ends in ("PTH pad 1 [GND] of M15", "Segment [<no
/// net>] of AE1 on F.Cu"); `None` for a track, a via, a zone or a board
/// graphic.
fn item_footprint(description: &str) -> Option<&str> {
    let mut text = description;
    if let Some(index) = text.rfind(" on ") {
        let layer = &text[index + 4..];
        if !layer.is_empty() && !layer.contains(char::is_whitespace) {
            text = &text[..index];
        }
    }
    let index = text.rfind("of ")?;
    if index > 0 && !text[..index].ends_with(char::is_whitespace) {
        return None;
    }
    let reference = &text[index + 3..];
    (!reference.is_empty() && !reference.contains(char::is_whitespace)).then_some(reference)
}

/// Whether a finding is between items of one footprint only (its pads and
/// graphics: a mounting hole's ring of overlapping pads, an antenna's
/// copper at its feed pad): the footprint's geometry and its pads' nets,
/// which neither a placement nor a route changes, so the reference has it
/// too, whether KiCad's capped report names it or not. run.py's
/// `footprint_own` (strict rule 4).
pub fn footprint_own(finding: &DrcFinding) -> bool {
    if finding.items.len() < 2 || finding.item_descriptions.len() != finding.items.len() {
        return false;
    }
    let mut owners = finding
        .item_descriptions
        .iter()
        .map(|description| item_footprint(description));
    let Some(Some(first)) = owners.next() else {
        return false;
    };
    owners.all(|owner| owner == Some(first))
}

/// Whether a finding of a result judged without a reference board is its
/// footprint's own: between items of one footprint (`footprint_own`), and
/// not a short, or a pad's own ring, hole or padstack (`pad_own`). A
/// library footprint whose pads lie closer to its own holes than the
/// project's rule (the schematic trial's USB4105: GND pads 0.19 mm from
/// its locating holes against 0.25 mm) fails every layout from a schematic
/// otherwise, and no placement or route changes it; a short within one
/// footprint is the schematic's nets on its pads, which stays judged.
pub fn footprint_own_unreferenced(finding: &DrcFinding) -> bool {
    (finding.kind != "shorting_items" && footprint_own(finding)) || pad_own(finding)
}

/// Why a finding of a result judged without a reference does not count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Apart {
    /// The footprint's own (`footprint_own_unreferenced`).
    Footprint,
    /// The input's (`input_own`).
    Input,
}

/// Whether a finding of a result judged without a reference is set apart,
/// and why: the footprint's own first, then the input's where the input is
/// known; `None`: it counts.
pub fn unreferenced_apart(finding: &DrcFinding, input: Option<&KiCadInputItems>) -> Option<Apart> {
    if footprint_own_unreferenced(finding) {
        Some(Apart::Footprint)
    } else if input.is_some_and(|input| input_own(finding, input)) {
        Some(Apart::Input)
    } else {
        None
    }
}

/// The finding types KiCad reports on one pad alone that are the pad's own
/// geometry against a board-wide minimum: its annular ring
/// (`annular_width`), its hole (`drill_out_of_range`) and its padstack
/// (`padstack`: a hole that leaves no copper, copper of an SMD pad on both
/// sides, a negative local clearance).
const PAD_OWN_KINDS: [&str; 3] = ["annular_width", "drill_out_of_range", "padstack"];

/// Whether a finding is one footprint pad's own (`PAD_OWN_KINDS`, its one
/// item a pad of a footprint: "PTH pad 45 [GND] of U3", "Pad [<no net>] of
/// FID1 on all copper layers"): the pad's size, hole and layers as the
/// footprint draws them, which neither a placement nor a route changes. The
/// nightly sweep of 2026-10-27 (main e7a12a8): 156 such findings on 18 of
/// 109 layouts (the RP2040's thermal-via pads at 0.10 mm under a 0.13 mm
/// minimum on 0xCB-1337, the 7805's 0.0875 mm on the mackerel boards, a
/// connector's 0.225 mm under ErgoSNM's 0.3 mm rule, fiducials with copper
/// on both sides on hackrf), each on the designer's board too, and none on
/// a via. A via's finding names no footprint and stays judged.
pub fn pad_own(finding: &DrcFinding) -> bool {
    PAD_OWN_KINDS.contains(&finding.kind.as_str())
        && finding.items.len() == 1
        && finding
            .item_descriptions
            .first()
            .is_some_and(|description| {
                is_pad_item(description) && pad_footprint(description).is_some()
            })
}

/// Whether an item of KiCad's report is a pad ("Pad 3 [GND] of J2 on
/// F.Cu", "PTH pad 1 [VCC] of SW4", "NPTH pad of SW4"; run.py's PAD_ITEM).
fn is_pad_item(description: &str) -> bool {
    let rest = description
        .strip_prefix("NPTH ")
        .or_else(|| description.strip_prefix("PTH "))
        .unwrap_or(description);
    rest.strip_prefix("Pad")
        .or_else(|| rest.strip_prefix("pad"))
        .is_some_and(|tail| tail.is_empty() || tail.starts_with(' '))
}

/// The footprint a finding's item belongs to (`item_footprint`), also for a
/// pad KiCad names on every copper layer ("Pad [<no net>] of FID1 on all
/// copper layers"), whose layer is more than one word.
fn pad_footprint(description: &str) -> Option<&str> {
    item_footprint(description).or_else(|| {
        description
            .strip_suffix(" on all copper layers")
            .and_then(item_footprint)
    })
}

/// What of a result board is the input's, for judging a result without a
/// reference board (`input_own`): the items the run kept as the input had
/// them, by their uuids, which KiCad's report names.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct KiCadInputItems {
    /// Every footprint the run left where the input had it (its position,
    /// rotation and side), with everything in it (pads, graphics, its own
    /// zones), and the input's zones and board graphics (`gr_*`, on any
    /// layer: Edge.Cuts, Margin, copper) that the result holds unchanged
    /// (zone fills aside). Never a track or a via, nor a zone the run added.
    pub uuids: BTreeSet<String>,
    /// The references whose every footprint was kept (not blank): an item
    /// KiCad names by its footprint ("Pad 2 [GND] of J2 on F.Cu") where its
    /// uuid is not the board's (a pad of a board that stores none).
    pub kept_references: BTreeSet<String>,
    /// Footprints kept, and footprints on the result.
    pub kept_parts: usize,
    pub parts: usize,
}

/// `input_items` of a board judged on its own (`drc-kicad-board`,
/// `fab-kicad-board`): every footprint, zone and board graphic is the
/// board's.
pub fn board_items(board: &str) -> Result<KiCadInputItems, String> {
    let board = crate::parse(board)?;
    Ok(input_items_of(&board, &board))
}

/// What of `result` (a board's text) is the input's (`input`'s text):
/// `KiCadInputItems`.
pub fn input_items(input: &str, result: &str) -> Result<KiCadInputItems, String> {
    Ok(input_items_of(
        &crate::parse(input)?,
        &crate::parse(result)?,
    ))
}

/// The board graphics' heads (`gr_line`, `gr_arc`, ...): a board's own
/// drawn items, on any layer.
fn is_board_graphic(head: &str) -> bool {
    head.starts_with("gr_")
}

/// A footprint's identity across the input and the result: its uuid, else
/// its reference.
fn footprint_key(footprint: &crate::Expr) -> Option<String> {
    crate::footprint_uuid(footprint)
        .map(|uuid| format!("uuid {uuid}"))
        .or_else(|| crate::footprint_reference(footprint).map(|name| format!("reference {name}")))
}

/// A footprint's pose: position, rotation (degrees, in [0, 360)) and side.
fn footprint_pose(footprint: &crate::Expr) -> Option<([f64; 3], String)> {
    let at = crate::form_at(footprint).ok()?;
    let layer = crate::form_atom(footprint, "layer", 1)?.to_string();
    Some(([at[0], at[1], at[2].rem_euclid(360.0)], layer))
}

fn same_pose(a: &([f64; 3], String), b: &([f64; 3], String)) -> bool {
    let turn = (a.0[2] - b.0[2]).abs();
    a.1 == b.1
        && (a.0[0] - b.0[0]).abs() <= 1.0e-4
        && (a.0[1] - b.0[1]).abs() <= 1.0e-4
        && turn.min(360.0 - turn) <= 1.0e-3
}

/// An item's own uuid (`uuid`, or KiCad 5's `tstamp`).
fn item_uuid(item: &crate::Expr) -> Option<String> {
    item.children()
        .iter()
        .find_map(|child| match child.head()? {
            "uuid" | "tstamp" => Some(child.children().get(1)?.atom()?.to_string()),
            _ => None,
        })
}

/// A zone or board graphic as compared between the input and the result:
/// its text without its fill, which KiCad's check refills.
fn unfilled_text(item: &crate::Expr) -> String {
    let crate::Expr::List(children) = item else {
        return crate::encode_canonical(item);
    };
    crate::encode_canonical(&crate::Expr::List(
        children
            .iter()
            .filter(|child| {
                !matches!(
                    child.head(),
                    Some("filled_polygon" | "fill_segments" | "filled_areas_thickness")
                )
            })
            .cloned()
            .collect(),
    ))
}

/// Every uuid in `item`, its own and its children's at any depth.
fn uuids_within(item: &crate::Expr, uuids: &mut BTreeSet<String>) {
    let mut stack = vec![item];
    while let Some(node) = stack.pop() {
        if matches!(node.head(), Some("uuid" | "tstamp"))
            && let Some(uuid) = node.children().get(1).and_then(crate::Expr::atom)
        {
            uuids.insert(uuid.to_string());
        }
        stack.extend(
            node.children()
                .iter()
                .filter(|child| matches!(child, crate::Expr::List(_))),
        );
    }
}

pub(crate) fn input_items_of(input: &crate::Expr, result: &crate::Expr) -> KiCadInputItems {
    let mut poses: BTreeMap<String, Vec<([f64; 3], String)>> = BTreeMap::new();
    let mut texts: BTreeMap<String, String> = BTreeMap::new();
    for item in input.children() {
        match item.head() {
            Some("footprint" | "module") => {
                if let (Some(key), Some(pose)) = (footprint_key(item), footprint_pose(item)) {
                    poses.entry(key).or_default().push(pose);
                }
            }
            Some(head) if head == "zone" || is_board_graphic(head) => {
                if let Some(uuid) = item_uuid(item) {
                    texts.insert(uuid, unfilled_text(item));
                }
            }
            _ => {}
        }
    }
    let mut items = KiCadInputItems::default();
    // Per reference, whether every footprint of it was kept.
    let mut references: BTreeMap<String, bool> = BTreeMap::new();
    for item in result.children() {
        match item.head() {
            Some("footprint" | "module") => {
                items.parts += 1;
                let kept = footprint_key(item)
                    .and_then(|key| poses.get(&key))
                    .zip(footprint_pose(item))
                    .is_some_and(|(inputs, pose)| {
                        inputs.iter().any(|input| same_pose(input, &pose))
                    });
                if kept {
                    items.kept_parts += 1;
                    uuids_within(item, &mut items.uuids);
                }
                if let Some(reference) = crate::footprint_reference(item)
                    .filter(|reference| !reference.trim().is_empty())
                {
                    *references.entry(reference).or_insert(true) &= kept;
                }
            }
            Some(head) if head == "zone" || is_board_graphic(head) => {
                if let Some(uuid) = item_uuid(item)
                    && texts.get(&uuid) == Some(&unfilled_text(item))
                {
                    items.uuids.insert(uuid);
                }
            }
            _ => {}
        }
    }
    items.kept_references = references
        .into_iter()
        .filter_map(|(reference, kept)| kept.then_some(reference))
        .collect();
    items
}

/// The finding types a result judged without a reference may leave to the
/// input (`input_own`): none of them is electrical between two nets. A
/// board edge too close to copper (`copper_edge_clearance`), a solder mask
/// opening over copper of another net (`solder_mask_bridge`: solder may
/// bridge there, the copper does not touch), and courtyards and holes in
/// them (`courtyards_overlap`, `pth_inside_courtyard`,
/// `npth_inside_courtyard`, `malformed_courtyard`). Never excused this way:
/// `shorting_items`, `clearance`, `hole_clearance`, `hole_to_hole`,
/// `items_not_allowed`, `connection_width`, `track_width`, unconnected
/// items, or any other type: a short or a clearance between two nets'
/// copper refuses a release whoever drew it.
pub const INPUT_KINDS: [&str; 6] = [
    "copper_edge_clearance",
    "solder_mask_bridge",
    "courtyards_overlap",
    "pth_inside_courtyard",
    "npth_inside_courtyard",
    "malformed_courtyard",
];

/// The footprint an item of KiCad's report names: a pad's or graphic's
/// (`pad_footprint`) or a footprint itself ("Footprint H4", a courtyard's
/// item).
fn named_footprint(description: &str) -> Option<&str> {
    pad_footprint(description).or_else(|| {
        description
            .strip_prefix("Footprint ")
            .filter(|reference| !reference.is_empty() && !reference.contains(char::is_whitespace))
    })
}

/// Whether a finding of a result judged without a reference board is the
/// input's: of a type in `INPUT_KINDS`, and every item of it the input's
/// (`KiCadInputItems`): a pad, graphic or courtyard of a footprint the run
/// kept where the input had it, the board edge, the input's board graphics
/// or its zones. One item the run wrote (a track, a via, a pour it added,
/// a part it placed, an outline it changed) and the finding is the run's.
/// The nightly sweep of 2026-10-27: of 113 copper edge findings on 109
/// layouts, 96 were fixed parts' pads and 16 Batch_1's own GND ring at
/// its edge; of the solder mask bridges not within one footprint, the
/// fiducials of Explorer against its own zones and the stacked switch
/// footprints of 0xcb-static. A finding of osprey's antenna (a part the
/// layout placed) at the edge and vaio_re's five bridges (a moved part's
/// pads) stay the run's: the sweep's reference rows found them the run's.
pub fn input_own(finding: &DrcFinding, input: &KiCadInputItems) -> bool {
    if !INPUT_KINDS.contains(&finding.kind.as_str()) || finding.items.is_empty() {
        return false;
    }
    let aligned = finding.item_descriptions.len() == finding.items.len();
    finding.items.iter().enumerate().all(|(index, item)| {
        item.uuid
            .as_ref()
            .is_some_and(|uuid| input.uuids.contains(uuid))
            || (aligned
                && named_footprint(&finding.item_descriptions[index])
                    .is_some_and(|reference| input.kept_references.contains(reference)))
    })
}

/// Whether an item of KiCad's report belongs to a footprint, also one
/// whose reference is blank ("PTH pad S3 [GND] of   "): its description
/// goes on past its net with " of ", or it is a pad.
fn of_a_footprint(description: &str) -> bool {
    let tail = description
        .rfind(']')
        .map_or(description, |end| &description[end + 1..]);
    is_pad_item(description) || tail.starts_with(" of ") || tail.contains(" of ")
}

/// A footprint reference a custom rule can name as it is: no quote,
/// wildcard or space (`memberOfFootprint` reads `*` and `?` as wildcards).
fn rule_name_fits(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '+' | '/'))
}

/// What a result's input findings (`input_own`) are and how KiCad's own DRC
/// accepts them, for `dru` (the board's `.kicad_dru`): per type, a custom
/// rule that sets their severity to ignore where one does (KiCad 10.0.7,
/// checked on the sweep's boards 2026-10-27: an edge clearance by its
/// footprints, or by the net of the board's own graphics; courtyards by
/// their pairs of footprints, either order), else an exclusion (no custom
/// rule of KiCad's changes a solder mask bridge: `bridged_mask`,
/// `solder_mask_expansion` and `clearance` with `(severity ignore)` left
/// Explorer's three as they were). `None` without such findings.
pub fn input_findings_note(findings: &[&DrcFinding], dru: &str) -> Option<String> {
    if findings.is_empty() {
        return None;
    }
    let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
    // Edge clearance: the footprints of the copper, and the nets of the
    // board's own copper graphics.
    let mut edge_parts: BTreeSet<String> = BTreeSet::new();
    let mut edge_nets: BTreeSet<String> = BTreeSet::new();
    let mut courtyard_pairs: BTreeSet<(String, String)> = BTreeSet::new();
    let mut excluded: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for finding in findings {
        *kinds.entry(finding.kind.as_str()).or_default() += 1;
        let parts: Vec<&str> = finding
            .item_descriptions
            .iter()
            .filter_map(|description| named_footprint(description))
            .collect();
        let mut exclude = || {
            let what = finding
                .item_descriptions
                .iter()
                .take(2)
                .cloned()
                .collect::<Vec<_>>()
                .join(" and ");
            excluded.entry(finding.kind.clone()).or_default().push(what);
        };
        match finding.kind.as_str() {
            "copper_edge_clearance" => {
                // The copper item: the one not on the outline.
                let copper: Vec<&String> = finding
                    .item_descriptions
                    .iter()
                    .filter(|description| {
                        !description.ends_with(" on Edge.Cuts")
                            && !description.ends_with(" on Margin")
                    })
                    .collect();
                match copper.as_slice() {
                    [description] => match named_footprint(description) {
                        Some(part) if rule_name_fits(part) => {
                            edge_parts.insert(part.to_string());
                        }
                        // A footprint's item whose reference is blank.
                        _ if of_a_footprint(description) => exclude(),
                        Some(_) => exclude(),
                        None => match item_net(description) {
                            Some(net) if !net.contains('\'') && !net.contains('"') => {
                                edge_nets.insert(net);
                            }
                            _ => exclude(),
                        },
                    },
                    _ => exclude(),
                }
            }
            "courtyards_overlap" => match parts.as_slice() {
                [a, b] if rule_name_fits(a) && rule_name_fits(b) => {
                    let (a, b) = if a <= b { (a, b) } else { (b, a) };
                    courtyard_pairs.insert((a.to_string(), b.to_string()));
                }
                _ => exclude(),
            },
            _ => exclude(),
        }
    }
    let mut rules = Vec::new();
    if !edge_parts.is_empty() {
        rules.push(format!(
            "(rule \"input edge_clearance\" (constraint edge_clearance (min 0mm)) (severity ignore) (condition \"{}\"))",
            edge_parts
                .iter()
                .map(|part| format!("A.memberOfFootprint('{part}')"))
                .collect::<Vec<_>>()
                .join(" || ")
        ));
    }
    for net in &edge_nets {
        rules.push(format!(
            "(rule \"input {net} graphics edge_clearance\" (constraint edge_clearance (min 0mm)) (severity ignore) (condition \"A.Type == 'Graphic' && A.NetName == '{net}' && !A.memberOfFootprint('*')\"))"
        ));
    }
    if !courtyard_pairs.is_empty() {
        rules.push(format!(
            "(rule \"input courtyards\" (constraint courtyard_clearance (min 0mm)) (severity ignore) (condition \"{}\"))",
            courtyard_pairs
                .iter()
                .map(|(a, b)| format!("(A.Reference == '{a}' && B.Reference == '{b}')"))
                .collect::<Vec<_>>()
                .join(" || ")
        ));
    }
    let mut accept = Vec::new();
    if !rules.is_empty() {
        accept.push(format!(
            "add to {dru} (a new file starts with (version 1)): {}",
            rules.join(" ")
        ));
    }
    if !excluded.is_empty() {
        let shown = 6;
        let listed: Vec<String> = excluded
            .iter()
            .map(|(kind, items)| {
                let mut named: Vec<String> = items.iter().take(shown).cloned().collect();
                if items.len() > shown {
                    named.push(format!("{} more", items.len() - shown));
                }
                format!("{kind} {} ({})", items.len(), named.join("; "))
            })
            .collect();
        accept.push(format!(
            "exclude in KiCad's DRC dialog (Exclude this violation: no custom rule changes a solder mask bridge or names a part without a plain reference): {}",
            listed.join(", ")
        ));
    }
    Some(format!(
        "{} DRC findings are the input's ({}): every item of each is a part kept where the input has it, the board edge, the input's own board graphics or zones, none a track, via, pour or part of this run, and none is electrical between two nets (input_findings), so they are not judged and fab-kicad-board releases with them; the designer's board as --reference judges them by place instead. KiCad's own DRC still reports them; to accept them there, {}",
        findings.len(),
        counts(
            &kinds
                .iter()
                .map(|(kind, count)| (kind.to_string(), *count))
                .collect()
        ),
        accept.join("; and ")
    ))
}

/// The constraint of KiCad's custom rules that a finding type is checked
/// by, where a rule for one footprint can set it.
fn custom_rule_constraint(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "clearance" => "clearance",
        "hole_clearance" => "hole_clearance",
        "hole_to_hole" => "hole_to_hole",
        "copper_edge_clearance" => "edge_clearance",
        "annular_width" => "annular_width",
        "drill_out_of_range" => "hole_size",
        "connection_width" => "connection_width",
        _ => return None,
    })
}

/// The value KiCad measured, from its description ("...; actual 0.1944
/// mm)").
fn actual_mm(description: &str) -> Option<f64> {
    let rest = &description[description.rfind("actual ")? + 7..];
    rest.split_whitespace().next()?.parse().ok()
}

/// The findings of one footprint and type, for `footprint_findings_note`.
struct OwnGroup {
    count: usize,
    /// The smallest value KiCad measured.
    smallest: Option<f64>,
    /// Whether every finding is of one item (a pad's own: `pad_own`).
    single: bool,
    /// Whether one is over a maximum (a hole too large).
    over_max: bool,
    /// Whether KiCad held one to the board's minimum ("board minimum hole
    /// clearance 0.2500 mm"): a custom rule below it does not take.
    board_minimum: bool,
}

/// The board setup minimum (`board.design_settings.rules` in the
/// .kicad_pro) KiCad names "board minimum" for a finding type.
fn board_minimum_setting(kind: &str) -> &'static str {
    match kind {
        "hole_clearance" => "min_hole_clearance",
        "clearance" => "min_clearance",
        "hole_to_hole" => "min_hole_to_hole",
        "copper_edge_clearance" => "min_copper_edge_clearance",
        _ => "the board setup's minimum",
    }
}

/// What a result's footprint findings (`footprint_own_unreferenced`) are and
/// how KiCad's own DRC accepts them: per footprint and type, a custom rule
/// for `dru` (the board's `.kicad_dru`) at the smallest value KiCad
/// measured, rounded down to 0.01 mm, or an exclusion. `None` without such
/// findings.
pub fn footprint_findings_note(findings: &[&DrcFinding], dru: &str) -> Option<String> {
    let mut groups: BTreeMap<(String, String), OwnGroup> = BTreeMap::new();
    for finding in findings {
        let Some(owner) = finding
            .item_descriptions
            .first()
            .and_then(|description| pad_footprint(description))
        else {
            continue;
        };
        let entry = groups
            .entry((owner.to_string(), finding.kind.clone()))
            .or_insert(OwnGroup {
                count: 0,
                smallest: None,
                single: true,
                over_max: false,
                board_minimum: false,
            });
        entry.count += 1;
        if let Some(actual) = actual_mm(&finding.description) {
            entry.smallest = Some(
                entry
                    .smallest
                    .map_or(actual, |smallest: f64| smallest.min(actual)),
            );
        }
        entry.single &= finding.items.len() == 1;
        entry.over_max |= finding.description.contains(" max ");
        entry.board_minimum |= finding.description.contains("(board minimum ");
    }
    if groups.is_empty() {
        return None;
    }
    let listed: Vec<String> = groups
        .iter()
        .map(|((owner, kind), group)| format!("{owner}: {kind} {}", group.count))
        .collect();
    let mut rules = Vec::new();
    let mut excluded = Vec::new();
    let mut minimums: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for ((owner, kind), group) in &groups {
        if group.board_minimum {
            // KiCad reports "board minimum" where the rule that holds
            // (here J1's own 0.1 mm clearance) is below the board setup's
            // minimum, which it applies instead: framework_mobo_lefthalf's
            // J1 (kicad-cli 10.0.7, 2026-10-27) kept its 8 findings under
            // a custom rule at 0.1 mm, also with (severity ignore), and
            // lost them with min_hole_clearance at 0.1 mm.
            minimums
                .entry(board_minimum_setting(kind))
                .or_default()
                .push(format!("{owner}'s {kind}"));
            continue;
        }
        match (custom_rule_constraint(kind), group.smallest) {
            (Some(constraint), Some(smallest)) if !group.over_max => {
                let min = ((smallest * 100.0 + 1.0e-9).floor() / 100.0).max(0.0);
                // KiCad checks a pad's own ring or hole with no item B: a
                // condition on B never holds there (kicad-cli 10.0.7 on
                // 0xCB-1337's U3, 2026-10-27).
                let condition = if group.single {
                    format!("A.memberOfFootprint('{owner}')")
                } else {
                    format!("A.memberOfFootprint('{owner}') && B.memberOfFootprint('{owner}')")
                };
                rules.push(format!(
                    "(rule \"{owner} own {kind}\" (constraint {constraint} (min {min}mm)) (condition \"{condition}\"))"
                ));
            }
            _ => excluded.push(format!("{owner}'s {kind}")),
        }
    }
    let mut accept = Vec::new();
    if !rules.is_empty() {
        accept.push(format!(
            "add to {dru} (a new file starts with (version 1)): {}",
            rules.join(" ")
        ));
    }
    if !excluded.is_empty() {
        accept.push(format!(
            "exclude {} in KiCad's DRC dialog (Exclude this violation)",
            excluded.join(", ")
        ));
    }
    for (setting, which) in &minimums {
        accept.push(format!(
            "exclude {} in KiCad's DRC dialog (Exclude this violation): KiCad holds them to the board minimum ({setting} in the .kicad_pro, Board Setup > Constraints), which no custom rule goes below, or lower {setting}, which then holds for the whole board",
            which.join(", ")
        ));
    }
    Some(format!(
        "{} DRC findings lie within one footprint each ({}): the footprint's own geometry as its library draws it, which no placement or route changes, so they are not judged (footprint_findings) and fab-kicad-board releases with them. KiCad's own DRC still reports them; to accept them there, {}",
        groups.values().map(|group| group.count).sum::<usize>(),
        listed.join(", "),
        accept.join("; or ")
    ))
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
    /// (`KICAD_FINDING_CAP`): whether a finding not at the same place was
    /// there before is unknown, so only the ones at the same place are
    /// subtracted.
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
        if footprint_own(finding) {
            *split.preexisting.entry(kind.to_string()).or_default() += 1;
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
            if capped(kind) && !split.unknown.iter().any(|known| known == kind) {
                split.unknown.push(kind.to_string());
            }
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
pub const RESULT_ENVIRONMENT: [&str; 18] = [
    "PCB_ROUTER_SEED",
    "PCB_ROUTER_DIFF_PAIRS",
    "PCB_ROUTER_LENGTH_MATCHING",
    "PCB_ROUTER_PIN_ROW_COST",
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

/// The binary's build, `<commit> <date>` (`set_build_version`); the
/// verdict's `environment` names it under `BUILD_VERSION_KEY`.
static BUILD_VERSION: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// The key of the build in the verdict's `environment` (lower case: it is
/// not an environment variable).
pub const BUILD_VERSION_KEY: &str = "pcb_maker_version";

/// Records the binary's build for the verdicts (the executable's `main`
/// calls it once; library callers that do not leave it out).
pub fn set_build_version(version: &str) {
    let _ = BUILD_VERSION.set(version.to_string());
}

/// The variables of `RESULT_ENVIRONMENT` that are set, with their values,
/// and the binary's build (`set_build_version`) when it was recorded.
pub fn result_environment() -> BTreeMap<String, String> {
    let mut environment: BTreeMap<String, String> = RESULT_ENVIRONMENT
        .iter()
        .filter_map(|name| {
            std::env::var_os(name).map(|value| (name.to_string(), value.to_string_lossy().into()))
        })
        .collect();
    if let Some(version) = BUILD_VERSION.get() {
        environment.insert(BUILD_VERSION_KEY.into(), version.clone());
    }
    environment
}

/// What a finished run produced, as the verdict needs it.
#[derive(Clone, Debug)]
pub struct KiCadRunFacts {
    pub command: String,
    /// KiCad's DRC on the result; `None` when it did not run.
    pub native: Option<DrcTally>,
    pub reference: KiCadReference,
    /// What of the result is the input's (`input_items`), for judging it
    /// without a reference (`input_own`); `None`: not known, nothing is
    /// left to the input.
    pub input: Option<KiCadInputItems>,
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
    /// Route mode with `diff_pairs`: the differential pairs found and what
    /// became of them (`KiCadDiffPairs`, less the per-pair list).
    pub diff_pairs: Option<crate::KiCadDiffPairs>,
    /// The length and skew rules' nets after the route (`KiCadNetLength`).
    pub lengths: Vec<crate::KiCadNetLength>,
    /// The project's routing rules the router does not keep
    /// (`KiCadProjectRulesReport::unmodelled`).
    pub unmodelled_rules: Vec<String>,
    /// Every constraint of the project's custom rules (.kicad_dru) and
    /// what the router made of it (`KiCadProjectRulesReport::custom_rules`).
    pub custom_rules: Vec<crate::project_rules::KiCadCustomRuleReport>,
    /// What the input check found before the run (a keepout over a fixed
    /// part: `keepout_warnings`; fixed parts on top of each other:
    /// `overlap_warnings`); next_actions repeat them.
    pub input_warnings: Vec<String>,
    /// The board's own texts and silkscreen graphics that parts lie on
    /// after a layout (`KiCadLabelReport::overlaps`): they stay where the
    /// input put them.
    pub board_mark_overlaps: Vec<String>,
    /// The footprints' fields placed after a layout (`silk_fields`).
    pub silkscreen: Option<crate::silk_fields::KiCadSilkscreenReport>,
    /// How whole the pours stayed (`KiCadBoardRouterResult::pour_quality`).
    pub pours: Vec<crate::board_router::KiCadPourQuality>,
    /// A layout's move descent, when it ran.
    pub moves: Option<crate::move_descent::KiCadMovesFact>,
    /// Where the lattice searches went and what the capacitated
    /// corridors learned (`KiCadCorridors`), when the run routed.
    pub corridors: Option<crate::KiCadCorridors>,
    /// A layout's placement race, when it ran.
    pub race: Option<crate::board_layout::KiCadRaceReport>,
    /// The router file's values below the project's minimum that the run
    /// allowed, with what became of them.
    pub rule_overrides: Vec<crate::run_diagnosis::KiCadOverrideOutcome>,
    /// The placement's bus groups (`bus_groups` on), when it had them.
    pub bus_groups: Option<crate::board_placer::KiCadBusGroupsReport>,
    /// The panel's copies the placement replicated (`panel_copies` on),
    /// when it found them.
    pub panel_copies: Option<crate::board_placer::KiCadPanelCopiesReport>,
    /// The placement's sub-boards (a panel of different boards), when it
    /// found two or more.
    pub sub_boards: Option<crate::sub_boards::KiCadSubBoardsReport>,
    /// A layout's decoupling (score-kicad-board's `decoupling` of the
    /// board written): below `DECOUPLING_SHARE` of the capacitors within
    /// 3 mm of a supply pin adds a next action.
    pub decoupling: Option<crate::quality::KiCadDecoupling>,
    /// Whether the layout's placer had `decoupling_near` on.
    pub decoupling_near: bool,
    /// The parts a `near` constraint of the layout held (its capacitors
    /// among them).
    pub decoupling_constrained: BTreeSet<String>,
}

/// The share of decoupling capacitors within 3 mm of a supply pin below
/// which a layout's verdict says so: designers put 60 % or more there
/// (the quick tier's designer boards; rp2350-dual-usb's designer 86 %).
pub const DECOUPLING_SHARE: f64 = 0.6;

/// The next action for a layout whose decoupling capacitors lie far from
/// their supply pins (`DECOUPLING_SHARE`), naming the farthest pins that a
/// capacitor could still serve: one of 1 uF or less on the pin's rail (a
/// regulator's: also a bulk one) that no `near` constraint holds
/// (`constrained`). The placer's `decoupling_near` is offered only when it
/// is off and such a capacitor exists (the schematic trial followed it on a
/// board whose every capacitor was constrained and got the same board).
fn decoupling_action(
    decoupling: &crate::quality::KiCadDecoupling,
    decoupling_near: bool,
    constrained: &BTreeSet<String>,
) -> Option<String> {
    let share = decoupling.capacitors_within_3mm?;
    if decoupling.capacitors < 3 || share >= DECOUPLING_SHARE {
        return None;
    }
    // Per far pin, the capacitors free to serve it (`None`: not known).
    let far: Vec<(&str, f64, Option<Vec<&String>>)> = decoupling
        .worst
        .iter()
        .enumerate()
        .filter_map(|(index, (pin, distance))| {
            let distance = distance.filter(|distance| *distance > 3.0)?;
            let free = decoupling.worst_capacitors.get(index).map(|serving| {
                serving
                    .iter()
                    .filter(|part| !constrained.contains(*part))
                    .collect::<Vec<_>>()
            });
            Some((pin.as_str(), distance, free))
        })
        .collect();
    let line = |pins: &[&(&str, f64, Option<Vec<&String>>)]| {
        pins.iter()
            .map(|(pin, distance, _)| format!("{pin} {distance:.1} mm"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let named: Vec<&(&str, f64, Option<Vec<&String>>)> = far
        .iter()
        .filter(|(_, _, free)| free.as_ref().is_none_or(|free| !free.is_empty()))
        .collect();
    let unnamed: Vec<&(&str, f64, Option<Vec<&String>>)> = far
        .iter()
        .filter(|(_, _, free)| free.as_ref().is_some_and(Vec::is_empty))
        .collect();
    let head = format!(
        "decoupling: {:.0} % of the decoupling capacitors lie within 3 mm of a supply pin (designers 60 % or more), a median {} mm",
        share * 100.0,
        decoupling
            .capacitor_median_mm
            .map_or("?".to_string(), |median| format!("{median:.1}")),
    );
    let mut steps = Vec::new();
    if let Some((pin, _, free)) = named.first() {
        let part = free
            .as_ref()
            .and_then(|free| free.first())
            .map_or("<capacitor>", |part| part.as_str());
        steps.push(format!(
            "give the capacitors of those pins a near constraint ({{\"part\": \"{part}\", \"pin_of\": \"{pin}\", \"max_mm\": 1.5}} in constraints.json; describe-kicad-board --constraints-template writes them for every capacitor)"
        ));
    }
    let any_free = far.is_empty()
        || far
            .iter()
            .any(|(_, _, free)| free.as_ref().is_none_or(|free| !free.is_empty()));
    if !decoupling_near && any_free {
        steps.push("turn on the placer's decoupling_near (layout.json {\"placer\": {\"decoupling_near\": true}}: every capacitor within 1 mm of the nearest supply pin of its rail, off by default)".into());
    }
    let mut text = head;
    if !named.is_empty() {
        text.push_str(&format!(
            "; supply pins farthest from a capacitor: {}",
            line(&named)
        ));
    }
    if !steps.is_empty() {
        text.push_str(&format!(": {}", steps.join(", or ")));
    }
    if !unnamed.is_empty() {
        text.push_str(&format!(
            "; {} ({}) have no capacitor of their own: every capacitor of 1 uF or less on their rails is held at another pin by a near constraint{}, so no placement brings one nearer, and one at them is a schematic change",
            if named.is_empty() {
                "the supply pins farthest from a capacitor"
            } else {
                "these other far pins"
            },
            line(&unnamed),
            if decoupling_near {
                " (and decoupling_near is on)"
            } else {
                ""
            },
        ));
    }
    Some(text)
}

impl KiCadRunFacts {
    /// The panel's sub-boards when their parts were placed mixed: more
    /// than a tenth of the movable parts belong to no sub-board, so the
    /// placement held none in its board (`sub_boards.held` false).
    pub fn boards_mixed(&self) -> Option<&crate::sub_boards::KiCadSubBoardsReport> {
        self.sub_boards
            .as_ref()
            .filter(|sub| sub.unassigned > 0 && !sub.held)
    }

    pub fn new(command: &str) -> Self {
        Self {
            command: command.into(),
            native: None,
            reference: KiCadReference::None,
            input: None,
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
            diff_pairs: None,
            lengths: Vec::new(),
            unmodelled_rules: Vec::new(),
            custom_rules: Vec::new(),
            input_warnings: Vec::new(),
            board_mark_overlaps: Vec::new(),
            silkscreen: None,
            pours: Vec::new(),
            moves: None,
            corridors: None,
            race: None,
            rule_overrides: Vec::new(),
            bus_groups: None,
            panel_copies: None,
            sub_boards: None,
            decoupling: None,
            decoupling_near: false,
            decoupling_constrained: BTreeSet::new(),
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
    /// type): only the findings at the same place are subtracted, since
    /// its true count is unknown.
    pub preexisting_unknown: Vec<String>,
    /// Without a reference board: KiCad errors per type that lie within one
    /// footprint each (its pads and holes as the library draws them; no
    /// short) or are one pad's own ring, hole or padstack (`pad_own`),
    /// which no placement or route changes
    /// (`footprint_own_unreferenced`). They do not count; next_actions says
    /// how KiCad's own DRC accepts them (a custom rule, an exclusion).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub footprint_findings: BTreeMap<String, usize>,
    /// Without a reference board: KiCad errors per type that are the
    /// input's (`input_own`): of a type that is not electrical between two
    /// nets (`INPUT_KINDS`: edge clearance, solder mask bridges,
    /// courtyards), every item a part the run kept where the input had it,
    /// the board edge, the input's board graphics or zones. They do not
    /// count; next_actions says how KiCad's own DRC accepts them.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub input_findings: BTreeMap<String, usize>,
    /// KiCad checked the result and finds no unconnected item and no error
    /// that counts, with nothing subtracted.
    pub clean_without_subtraction: bool,
    /// The project whose board the pre-existing findings come from, or
    /// `null` when there was none or none was needed.
    pub preexisting_from: Option<String>,
    /// Silkscreen, text and library findings, which do not count.
    pub cosmetic_findings: usize,
    /// A layout's footprint fields (the visible Reference and Value) on the
    /// silkscreen, placed after routing where KiCad's silkscreen checks
    /// find nothing: how many, how many moved, those without a clean spot
    /// (`blocked`, with the findings predicted there) or hidden for want of
    /// one (`silkscreen.hide_when_blocked`), and KiCad's findings they will
    /// have by type (`predicted`). The rest of `cosmetic_findings` is the
    /// footprints' and the board's own graphics.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub silkscreen: Option<crate::silk_fields::KiCadSilkscreenReport>,
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
    /// Route mode: whether differential pairs routed coupled and why
    /// (`on`, `default`: "rules", "off" or "explicit", `reason`); with them
    /// on, pairs found, coupled, partly coupled, fallen back, links,
    /// crossings, coupled and uncoupled length (`KiCadDiffPairs`; the
    /// per-pair list is in board-router.json).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff_pairs: Option<crate::KiCadDiffPairs>,
    /// The project's length and skew rules (`length_matching`): per rule
    /// and net its `length_mm` from the written board, the `target_mm`
    /// the meander aims at, the bounds KiCad holds it to (`min_mm`,
    /// `max_mm`; a skew rule's from its group's longest net), the
    /// `meander_mm` added, whether it is `within` them and, if not, a
    /// `note` why. Absent where the project has no such rule.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub lengths: Vec<crate::KiCadNetLength>,
    /// The result-changing environment variables that were set
    /// (`RESULT_ENVIRONMENT`), with their values, and the binary's build
    /// (`pcb_maker_version`: commit and build date).
    pub environment: BTreeMap<String, String>,
    /// How whole the pours stayed, per zone and layer, by the pour model on
    /// the board written: pieces, the largest piece's share of the pour's
    /// area, copper coverage, other nets' tracks cutting it
    /// (`KiCadPourQuality`). Absent without pours. It does not decide the
    /// status; a pour in many pieces adds a next action.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pours: Vec<crate::board_router::KiCadPourQuality>,
    /// A layout's move descent (`move_descent` in layout.json): moves
    /// scored, moves the router tried, moves kept, the parts moved, the
    /// open terminals before and after. Absent when it did not run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub moves: Option<crate::move_descent::KiCadMovesFact>,
    /// Where the run's lattice searches went (corridors, the capacitated
    /// corridors' band, windows, the board; the negotiation's and the hard
    /// searches' apart, with their work) and what the capacitated
    /// corridors learned (`capacitated_corridors`: misses, capacities
    /// lowered, re-plans, retries, fallbacks), summed over every router
    /// of the run (docs/router.md, "Capacitated corridors (2026-10-24)").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub corridors: Option<crate::KiCadCorridors>,
    /// A layout's placement race (with or without `race_probe`): every
    /// probed placement's open terminals, unfinished nets, nets in
    /// conflict and work, whether the 40 % gate before 2026-10-25 would
    /// have dropped it, the placement kept and the one the gate would
    /// have kept, the probes' best, whether they separated the
    /// placements and why (`probe`, `kept`, `kept_gated`, `best`,
    /// `confident`, `reason`, `probe_seconds`, `dropped`). Absent when it
    /// did not run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub race: Option<crate::board_layout::KiCadRaceReport>,
    /// The router file's values below the project's minimum that the run
    /// routed with (`allow_below_project_minimum`), each with the value
    /// requested, the project's minimum and its source, the value routed
    /// with, the track widths written on its nets (and how many are below
    /// the minimum), the value KiCad's DRC requires and KiCad's findings
    /// of that kind on its nets (`KiCadOverrideOutcome`). Absent without
    /// such values.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub rule_overrides: Vec<crate::run_diagnosis::KiCadOverrideOutcome>,
    /// Route and layout: every constraint of the project's custom rules
    /// (.kicad_dru) with what the router made of it: `rule`, `constraint`,
    /// `status` (`applied`, `approximated`, `no_effect`, `not_routing`,
    /// `unmodelled`, `skipped`) and `detail`; as board-router.json's
    /// `rules.custom_rules`. Absent without such rules.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub custom_rules: Vec<crate::project_rules::KiCadCustomRuleReport>,
    /// The placement's bus groups (`bus_groups` in the placer's
    /// configuration, off by default): how many groups moved more than
    /// one part, the banks (members, parts moved), the clusters, and the
    /// parts per group (`KiCadBusGroupsReport`). Absent when off.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bus_groups: Option<crate::board_placer::KiCadBusGroupsReport>,
    /// The panel's copies (`panel_copies` in the placer's configuration,
    /// on by default) where the placement found them: how many, the parts
    /// and movable parts per copy, how they were told apart (`net
    /// islands`, `numbered references`, `repeated references`), where
    /// their regions came from, the movable parts in no copy, how many
    /// replicated parts moved after the replication, and per copy its
    /// region, turn and offset from the first and its references in the
    /// first copy's order (the mapping: the r-th reference of each copy is
    /// the same part of the circuit). Absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub panel_copies: Option<crate::board_placer::KiCadPanelCopiesReport>,
    /// The sub-boards of a panel of different boards (`sub_boards` in the
    /// placer's configuration, on by default) where the outline falls into
    /// two pieces or more once its tabs are taken out and parts lie on two
    /// of them: how many, what told the parts' sub-boards, the nets of one
    /// name on several (one net per sub-board to the placer and the
    /// router), the movable parts without a sub-board, whether the
    /// placement held the parts in their regions, the tabs, and per
    /// sub-board its region and parts. KiCad's unconnected items across
    /// two sub-boards are not counted (`unconnected_across_sub_boards` in
    /// drc.json). Absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub_boards: Option<crate::sub_boards::KiCadSubBoardsReport>,
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
    // First: a panel whose boards' parts were placed mixed is not the
    // board as designed, whatever else holds (the OpenESC panel's place
    // check placed all 404 parts and exited 0).
    if let Some(sub) = facts.boards_mixed() {
        problems.push(format!(
            "the panel's {} boards' parts are mixed: {} movable parts belong to no sub-board (give --reference <the designer's project> or region constraints)",
            sub.count, sub.unassigned
        ));
    }
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
    footprint: &[&DrcFinding],
    input: &[&DrcFinding],
) -> Vec<String> {
    let mut actions = Vec::new();
    // First, ahead of the input's warnings: on a panel whose parts' boards
    // are unknown nothing after it is about the boards as designed (the
    // OpenESC panel's line came 21st, after its frames' marks, and before
    // an outline suggestion for a panel whose frame is fixed).
    if let Some(sub) = facts.boards_mixed() {
        actions.push(format!(
            "the board is a panel of {} different boards sharing net names, and {} movable parts belong to no sub-board that the fixed parts or the nets tell ({}): the placement put them where it would, the boards' parts mixed; give the designer's placed board with --reference <project-dir> (its places say each part's board), or hold each part in its board with region constraints, before acting on the room or the connections below",
            sub.count,
            sub.unassigned,
            sub.unassigned_parts.join(", ")
        ));
    }
    actions.extend(facts.input_warnings.iter().cloned());
    if !facts.attempts_failed.is_empty() {
        actions.push(format!(
            "{} attempts of the router's ladder failed (a panic; attempts_failed in board-router.json has each message): the board is the best of the other attempts; report the panic ({})",
            facts.attempts_failed.len(),
            facts.attempts_failed.join("; ")
        ));
    }
    if !facts.unplaced.is_empty() && facts.boards_mixed().is_some() {
        // The parts are short of room because two boards' parts share
        // one: an outline or room suggestion is no advice for a panel
        // whose frame is fixed (the OpenESC panel's layout suggested
        // 43.0 x 83.0 mm).
        actions.push(format!(
            "{} parts found no legal place with the panel's boards' parts mixed: the sub-board action above comes first; no outline or room is suggested while the parts' boards are unknown",
            facts.unplaced.len()
        ));
    } else if !facts.unplaced.is_empty() {
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
    if !facts.unplaced.is_empty()
        && let Some(action) = placement_rates_action(&facts.command, &crate::work_rate::work_rate())
    {
        actions.push(action);
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
            "connections are open: {}; `pcb-maker inspect-routing-run` on this output names each open net's pads, rules and KiCad's pairs and each hot spot's pads and nets (--reference: the designer's route of the net): read it before changing the design. Where the designer's board routes those nets on this placement under these rules (reference.connected), density is not shown to be the reason and more room, layers or narrower classes are not called for; without that evidence they are hypotheses to test (classes never below the board's minimum)",
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
            let kept = kept_parts_of(&findings, facts.input.as_ref()).map(|mut parts| {
                parts.sort_by(|a, b| crate::canonical_order::natural_cmp(a, b));
                let how = if facts.command == "route-kicad-board" {
                    "route mode keeps every part"
                } else {
                    "fixed"
                };
                format!("{}: {how}", parts.join(", "))
            });
            actions.push(finding_action(kind, *count, &findings, kept.as_deref()));
        }
    }
    // The board's .kicad_dru: beside the board written.
    let dru = facts.outputs.get("board").map_or_else(
        || "the board's .kicad_dru".to_string(),
        |board| match board.strip_suffix(".kicad_pcb") {
            Some(stem) => format!("{stem}.kicad_dru"),
            None => "the board's .kicad_dru".to_string(),
        },
    );
    actions.extend(footprint_findings_note(footprint, &dru));
    actions.extend(input_findings_note(input, &dru));
    if !facts.board_mark_overlaps.is_empty() {
        let shown = 6;
        let mut listed: Vec<String> = facts
            .board_mark_overlaps
            .iter()
            .take(shown)
            .cloned()
            .collect();
        if facts.board_mark_overlaps.len() > shown {
            listed.push(format!("{} more", facts.board_mark_overlaps.len() - shown));
        }
        actions.push(format!(
            "parts lie on {} of the board's own texts or silkscreen graphics, which stay where the input put them (labels.overlaps in board-layout.json): keep the parts off them (a keepout constraint over the text, a region, fixed positions) or move the text in the input: {}",
            facts.board_mark_overlaps.len(),
            listed.join("; ")
        ));
    }
    if let Some(silkscreen) = &facts.silkscreen
        && !silkscreen.blocked.is_empty()
    {
        let shown = 4;
        let mut listed: Vec<String> = silkscreen.blocked.iter().take(shown).cloned().collect();
        if silkscreen.blocked.len() > shown {
            listed.push(format!("{} more", silkscreen.blocked.len() - shown));
        }
        actions.push(format!(
            "{} silkscreen fields have no spot KiCad finds clean (cosmetic; silkscreen.blocked): give their parts room, hide them (layout.json \"silkscreen\": {{\"hide_when_blocked\": true}}) or move them in KiCad: {}",
            silkscreen.blocked.len(),
            listed.join("; ")
        ));
    }
    if !facts.constraints_missed.is_empty() {
        actions.push("constraints missed (constraints[] has violation_mm for each): relax them or give the parts room".into());
    }
    if !unknown.is_empty() {
        actions.push(format!(
            "the reference board has {} at KiCad's cap of {KICAD_FINDING_CAP} findings: only the findings at the same place are subtracted (preexisting_unknown)",
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
    for pour in facts.pours.iter().filter(|pour| pour.in_many_pieces()) {
        actions.push(pour_action(pour));
    }
    actions.extend(facts.decoupling.as_ref().and_then(|decoupling| {
        decoupling_action(
            decoupling,
            facts.decoupling_near,
            &facts.decoupling_constrained,
        )
    }));
    if facts.routes && facts.native.is_none() {
        actions.push("KiCad's check did not run (PCB_SKIP_NATIVE or skip_native_verification): the router's own count is no verdict; run KiCad's DRC (pcb-maker drc-kicad-board) before using the board".into());
    }
    if status != KiCadRunStatus::Complete {
        actions.extend(facts.skipped.iter().cloned());
    }
    actions
}

/// The next action for a pour in many pieces: `keep_whole` under the
/// parts whose regions on the layer hold no other net's pad
/// (`KiCadPourQuality::keep_whole_parts`), and not under parts where every
/// region would cut another net's pads off (the trial's board A,
/// 2026-10-26: keep_whole under its ICs on F.Cu opened 22 connections).
fn pour_action(pour: &crate::board_router::KiCadPourQuality) -> String {
    let advice = if pour.keep_whole_parts.is_empty() {
        format!(
            "a keep_whole region under any part of six pads or more would cover other nets' pads on {} and leave them unreachable there, so keep_whole under parts does not fit this layer (a box clear of other nets' pads may)",
            pour.layer
        )
    } else {
        format!(
            "\"keep_whole\" in its add_pours entry keeps other nets' tracks and vias out of it under named parts or a box, so its main piece stays whole there; on {layer} the regions of {parts} hold no other net's pad: {{\"net\": \"{net}\", \"layers\": [\"{layer}\"], \"keep_whole\": [{{\"parts\": [{quoted}]}}]}} (a region over another net's pad leaves that pad unreachable)",
            layer = pour.layer,
            net = pour.net,
            parts = pour.keep_whole_parts.join(", "),
            quoted = pour
                .keep_whole_parts
                .iter()
                .map(|part| format!("\"{part}\""))
                .collect::<Vec<_>>()
                .join(", "),
        )
    };
    format!(
        "the {} pour is in many pieces ({}): {advice}; more room or layers give it more",
        pour.net,
        pour.line()
    )
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
fn finding_action(
    kind: &str,
    count: usize,
    findings: &[&DrcFinding],
    kept: Option<&str>,
) -> String {
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
            "move those nets' parts apart, or check whether the rule is one the router does not keep (rules.unmodelled and rules.custom_rules in board-router.json)"
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
            "copper of two nets under one mask opening (KiCad names the opening and the first copper it found there, not the other net's item): the router keeps its tracks and vias the project's solder_mask_to_copper_clearance plus the opening's expansion from an opening of another net, and out of one over two nets; the solder_mask_bridge severity (Board Setup) decides whether it counts"
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
    // A finding between items of one footprint (a USB receptacle's pad
    // and its own locating hole) is the library's geometry: moving parts
    // does not change it.
    let step = match (own_footprints(findings), kept) {
        (Some(references), _) => format!(
            "each finding lies between items of one footprint ({}): its library geometry, which no placement or route changes; the designer's board as --reference makes it pre-existing where it has the same footprint, a custom rule for the footprint (.kicad_dru) or a KiCad exclusion accepts it, and fab-kicad-board --allow-findings releases with it",
            references.join(", ")
        ),
        // Two nets' copper of parts the run kept where the input has them
        // (framework_mobo_lefthalf's fixed R31 and R32 under J1's pads, as
        // the designer put them): "move the parts apart" is not the
        // run's to do.
        (None, Some(parts)) if ELECTRICAL_KINDS.contains(&kind) => format!(
            "every finding lies between parts kept where the input has them ({parts}), copper this run neither placed nor routed, but between two nets, so it counts and fab-kicad-board refuses it; move one of the parts in the input (or free it from fixed so the placer does), or accept it knowingly (a KiCad exclusion, fab-kicad-board --allow-findings); the designer's board as --reference makes it pre-existing where that board has it, which the release still refuses"
        ),
        (None, _)
            if kind == "solder_mask_bridge" && !footprint_mask_graphics(findings).is_empty() =>
        {
            format!(
                "{step}; but no route keeps a zone's fill out: KiCad fills a zone of another net under a footprint's mask graphic ({}), so with the footprint's pads that is a bridge whatever is routed (MokyaLora's J6, a U.FL's B.Mask rectangle over its GND pads, placed on the VSYS zone of B.Cu: the bridge stayed with every track under it removed and went with the zone); move the part off the zone of the other net, keep that zone's fill out of the opening (a rule area without pours, or the zone's outline), or allow solder mask bridges in the footprint's properties where the opening is meant to expose both",
                footprint_mask_graphics(findings).join(", ")
            )
        }
        (None, _) => step.to_string(),
    };
    if what.is_empty() {
        format!("{kind} {count}: {step}")
    } else {
        format!("{kind} {count} ({}): {step}", what.join("; "))
    }
}

/// The footprints' mask graphics among the findings' items, as KiCad names
/// them ("Rectangle of J6 on B.Mask"): a footprint's graphic, not a pad
/// or the board's own graphic, on a mask layer.
fn footprint_mask_graphics(findings: &[&DrcFinding]) -> Vec<String> {
    let mut graphics: Vec<String> = findings
        .iter()
        .flat_map(|finding| finding.item_descriptions.iter())
        .filter(|description| {
            !description.starts_with("Pad ")
                && (description.ends_with(" on F.Mask") || description.ends_with(" on B.Mask"))
                && description.contains(" of ")
        })
        .cloned()
        .collect();
    graphics.sort();
    graphics.dedup();
    graphics
}

/// The electrical findings that count between parts kept where the input
/// has them (`ELECTRICAL_KINDS`, every item a pad of such a part:
/// `kept_parts_of`), one action per type as the layout's verdict words it,
/// and how many findings they are. `fab-kicad-board --input` says them
/// instead of "move what is too close" (framework_mobo_lefthalf's fixed
/// R31 and R32 under J1's pads, the designer's places).
pub fn kept_parts_actions(facts: &KiCadRunFacts) -> (Vec<String>, usize) {
    let Some(native) = &facts.native else {
        return (Vec::new(), 0);
    };
    let mut actions = Vec::new();
    let mut total = 0;
    for kind in ELECTRICAL_KINDS {
        let findings: Vec<&DrcFinding> = native
            .findings
            .iter()
            .filter(|finding| {
                finding.kind == kind && unreferenced_apart(finding, facts.input.as_ref()).is_none()
            })
            .collect();
        if let Some(mut parts) = kept_parts_of(&findings, facts.input.as_ref()) {
            parts.sort_by(|a, b| crate::canonical_order::natural_cmp(a, b));
            let kept = format!("{}: unchanged from the input", parts.join(", "));
            actions.push(finding_action(kind, findings.len(), &findings, Some(&kept)));
            total += findings.len();
        }
    }
    (actions, total)
}

/// The finding types between two nets' copper that `kept_parts_of` speaks
/// of: never the input's to excuse (`INPUT_KINDS`).
const ELECTRICAL_KINDS: [&str; 4] = [
    "clearance",
    "hole_clearance",
    "hole_to_hole",
    "shorting_items",
];

/// The parts of findings every item of which is a pad of a part the run
/// kept where the input has it (`KiCadInputItems`), in order; `None`
/// without the input's items, for a finding of an item of no part or of a
/// part the run placed, or without findings.
fn kept_parts_of(findings: &[&DrcFinding], input: Option<&KiCadInputItems>) -> Option<Vec<String>> {
    let input = input?;
    if findings.is_empty() {
        return None;
    }
    let mut parts: Vec<String> = Vec::new();
    for finding in findings {
        if finding.item_descriptions.len() < 2
            || finding.item_descriptions.len() != finding.items.len()
        {
            return None;
        }
        for (item, description) in finding.items.iter().zip(&finding.item_descriptions) {
            let reference = named_footprint(description)?;
            let kept = item
                .uuid
                .as_ref()
                .is_some_and(|uuid| input.uuids.contains(uuid))
                || input.kept_references.contains(reference);
            if !kept {
                return None;
            }
            if !parts.iter().any(|part| part == reference) {
                parts.push(reference.to_string());
            }
        }
    }
    Some(parts)
}

/// The footprints of findings whose items all belong to one footprint
/// each ("Pad B1 [GND] of J1 on F.Cu" and "NPTH pad of J1"), in order;
/// `None` when a finding has fewer than two items or an item of no
/// footprint or of another.
fn own_footprints(findings: &[&DrcFinding]) -> Option<Vec<String>> {
    if findings.is_empty() {
        return None;
    }
    let mut references: Vec<String> = Vec::new();
    for finding in findings {
        if finding.item_descriptions.len() < 2 {
            return None;
        }
        let mut owner: Option<&str> = None;
        for item in &finding.item_descriptions {
            let reference = footprint_of(item)?;
            match owner {
                Some(other) if other != reference => return None,
                _ => owner = Some(reference),
            }
        }
        let owner = owner?.to_string();
        if !references.contains(&owner) {
            references.push(owner);
        }
    }
    Some(references)
}

/// The footprint an item of KiCad's report belongs to: the reference after
/// " of " (past the net in brackets), as in "Pad 2 [GND] of C1 on F.Cu".
fn footprint_of(item: &str) -> Option<&str> {
    let tail = item.rfind(']').map_or(item, |end| &item[end + 1..]);
    let rest = &tail[tail.find(" of ")? + 4..];
    rest.split_whitespace()
        .next()
        .filter(|reference| !reference.is_empty())
}

/// The next action for a project with rules the router does not keep.
fn unmodelled_rules_action(rules: &[String]) -> String {
    format!(
        "the project has rules the router does not keep, which KiCad checks: their findings are for the designer to resolve by hand or by moving parts (KiCad names the rule in each finding's description in drc.json, \"rule 'NAME' ...\"; a rule that only allows less than the router keeps costs nothing): {}",
        rules.join("; ")
    )
}

/// One line on the project's custom rules (`KiCadVerdict::custom_rules`):
/// how many constraints the router applied, approximated, found without
/// effect, left to KiCad (`not_routing`, `skipped`) or could not keep
/// (`unmodelled`), naming the approximated and unmodelled ones; `None`
/// without custom rules. MokyaLora (2026-10-27): its two area rules were
/// in board-router.json only, and no output a run prints said a word of
/// them.
pub fn custom_rules_line(rules: &[crate::project_rules::KiCadCustomRuleReport]) -> Option<String> {
    if rules.is_empty() {
        return None;
    }
    let order = [
        "applied",
        "approximated",
        "no_effect",
        "not_routing",
        "skipped",
        "unmodelled",
    ];
    let mut counts: Vec<String> = Vec::new();
    for status in order.iter().copied().chain(
        rules
            .iter()
            .map(|rule| rule.status.as_str())
            .filter(|status| !order.contains(status)),
    ) {
        let matching: Vec<&crate::project_rules::KiCadCustomRuleReport> =
            rules.iter().filter(|rule| rule.status == status).collect();
        if matching.is_empty()
            || counts
                .iter()
                .any(|count| count.starts_with(&format!("{status} ")))
        {
            continue;
        }
        let named = if matches!(status, "approximated" | "unmodelled") {
            format!(
                " ({})",
                matching
                    .iter()
                    .map(|rule| format!("{} {}", rule.rule, rule.constraint))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        } else {
            String::new()
        };
        counts.push(format!("{status} {}{named}", matching.len()));
    }
    Some(format!(
        "custom rules (.kicad_dru): {} constraints: {}; custom_rules in verdict.json says what each became (approximated: kept more widely than KiCad asks; unmodelled: KiCad checks it, the router does not keep it)",
        rules.len(),
        counts.join(", ")
    ))
}

/// The next action an outline suggestion makes: the size to try, and why.
/// A layout whose placement left parts without a place at the rates it
/// measured: what the place check differs in. `place-kicad-board` places
/// as the layout does (`placement_check_config`) at the reference's rates
/// unless told otherwise; at the layout's rates it repeats the layout's
/// placement (the OpenESC panel: the check placed every part, the layout,
/// at 0.84 of the reference's placer rate and with the escape demand
/// measured, left 8 without a place).
fn placement_rates_action(command: &str, rate: &crate::work_rate::KiCadWorkRate) -> Option<String> {
    if !command.starts_with("layout") || rate.source != "measured" {
        return None;
    }
    Some(format!(
        "this layout placed at the work rates measured at its start ({:.4e} expansions/s, {:.4e} placer units/s; {:.2} and {:.2} of the reference's): on a board this full the rates decide which parts find a place; place-kicad-board places as the layout does but at the reference's rates by default, and PCB_WORK_RATE={:.4e},{:.4e} repeats this placement, with place-kicad-board (the same layout config) or with the layout",
        rate.expansions_per_second,
        rate.placer_work_per_second,
        rate.expansions_per_second / pcb_router::router::REFERENCE_EXPANSIONS_PER_SECOND,
        rate.placer_work_per_second / pcb_placer::REFERENCE_WORK_PER_SECOND,
        rate.expansions_per_second,
        rate.placer_work_per_second,
    ))
}

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
        // Without a reference, a footprint's own findings
        // (`footprint_own_unreferenced`) and the input's (`input_own`) are
        // set apart; every other error counts.
        KiCadReference::None | KiCadReference::Failed { .. } => {
            let mut new = errors.clone();
            let mut new_indices = Vec::new();
            for (index, finding) in findings.iter().enumerate() {
                if unreferenced_apart(finding, facts.input.as_ref()).is_some() {
                    if let Some(count) = new.get_mut(&finding.kind) {
                        *count -= 1;
                        if *count == 0 {
                            new.remove(&finding.kind);
                        }
                    }
                } else {
                    new_indices.push(index);
                }
            }
            (
                new.clone(),
                StrictSplit {
                    new,
                    new_indices,
                    ..Default::default()
                },
                None,
            )
        }
    };
    let own: Vec<&DrcFinding> = strict
        .new_indices
        .iter()
        .filter_map(|&index| findings.get(index))
        .collect();
    let apart = |which: Apart| -> Vec<&DrcFinding> {
        match &facts.reference {
            KiCadReference::Findings { .. } => Vec::new(),
            KiCadReference::None | KiCadReference::Failed { .. } => findings
                .iter()
                .filter(|finding| unreferenced_apart(finding, facts.input.as_ref()) == Some(which))
                .collect(),
        }
    };
    let footprint = apart(Apart::Footprint);
    let input = apart(Apart::Input);
    let by_kind = |findings: &[&DrcFinding]| {
        let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
        for finding in findings {
            *kinds.entry(finding.kind.clone()).or_default() += 1;
        }
        kinds
    };
    let footprint_findings = by_kind(&footprint);
    let input_findings = by_kind(&input);
    let (status, reason) = decide(facts, &strict.new);
    let mut actions = next_actions(
        facts,
        status,
        &strict.new,
        &own,
        &strict.unknown,
        &footprint,
        &input,
    );
    // With a reference its findings do not count here, but the release
    // judges the board on its own: say which of them it refuses (an
    // electrical finding not within one footprint). framework_mobo_lefthalf
    // with --reference: "2 unconnected" and no word of the 2 clearance and
    // 4 hole clearance findings of its fixed parts that refused the
    // release.
    if matches!(facts.reference, KiCadReference::Findings { .. }) {
        let new: BTreeSet<usize> = strict.new_indices.iter().copied().collect();
        let blocking: Vec<&DrcFinding> = findings
            .iter()
            .enumerate()
            .filter(|(index, finding)| {
                !new.contains(index)
                    && ELECTRICAL_KINDS.contains(&finding.kind.as_str())
                    && !footprint_own(finding)
            })
            .map(|(_, finding)| finding)
            .collect();
        if !blocking.is_empty() {
            let parts: BTreeSet<&str> = blocking
                .iter()
                .flat_map(|finding| finding.item_descriptions.iter())
                .filter_map(|description| named_footprint(description))
                .collect();
            let mut parts: Vec<&str> = parts.into_iter().collect();
            parts.sort_by(|a, b| crate::canonical_order::natural_cmp(a, b));
            actions.push(format!(
                "{} of the findings the reference has at the same place are between two nets ({}{}): not judged here, but fab-kicad-board, which judges the board on its own, refuses them: fix them in the input, or release with --allow-findings knowingly",
                blocking.len(),
                counts(&by_kind(&blocking)),
                if parts.is_empty() {
                    String::new()
                } else {
                    format!("; parts {}", parts.join(", "))
                }
            ));
        }
    }
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
        next_actions: actions,
        footprint_findings,
        input_findings,
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
        silkscreen: facts.silkscreen.clone(),
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
        // No outline to try while a panel's parts' boards are unknown.
        room: facts.room.clone().map(|mut room| {
            if facts.boards_mixed().is_some() {
                room.suggested_outline_mm = None;
            }
            room
        }),
        repairs: facts.repairs.clone(),
        diff_pairs: facts.diff_pairs.clone(),
        lengths: facts.lengths.clone(),
        environment: result_environment(),
        pours: facts.pours.clone(),
        moves: facts.moves.clone(),
        corridors: facts.corridors.clone(),
        race: facts.race.clone(),
        bus_groups: facts.bus_groups.clone(),
        panel_copies: facts.panel_copies.clone(),
        sub_boards: facts.sub_boards.clone(),
        rule_overrides: facts.rule_overrides.clone(),
        custom_rules: facts.custom_rules.clone(),
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
    facts.corridors = Some(crate::KiCadCorridors::now());
    facts.diff_pairs = result.diff_pairs.clone().map(|mut pairs| {
        pairs.pairs.clear();
        pairs
    });
    facts.lengths = result.lengths.clone();
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
    facts.custom_rules = result.rules.project.custom_rules.clone();
    facts.attempts_failed = result.attempts_failed.clone();
    // What the board's reading left out (copper on a layer the stack
    // lacks): the next_actions lead with it.
    facts.input_warnings = result.warnings.clone();
    facts.pours = result.pour_quality.clone();
    if !result.rules.project.overrides.is_empty() {
        let drc = facts
            .native
            .is_some()
            .then(|| fs::read_to_string(directory.join("drc.json")).ok())
            .flatten()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok());
        facts.rule_overrides = crate::run_diagnosis::override_outcomes(
            &result.rules.project.overrides,
            &directory.join(format!("{}.kicad_pcb", result.board_id)),
            drc.as_ref(),
        );
    }
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
    fn the_references_electrical_findings_are_said_to_refuse_the_release() {
        let clearance = json!({"type": "clearance", "severity": "error",
               "description": "Clearance violation (footprint J1 clearance 0.1000 mm; actual 0.0867 mm)",
               "items": [{"description": "Pad 1 [A] of R31 on B.Cu", "uuid": "r31", "pos": {"x": 1.0, "y": 1.0}},
                         {"description": "PTH pad B6 [B] of J1", "uuid": "b6", "pos": {"x": 1.5, "y": 1.0}}]});
        let own = json!({"type": "hole_clearance", "severity": "error",
               "description": "Hole clearance violation (board minimum hole clearance 0.2500 mm; actual 0.1030 mm)",
               "items": [{"description": "PTH pad B1 [GND] of J1", "uuid": "b1", "pos": {"x": 2.0, "y": 1.0}},
                         {"description": "NPTH pad of J1", "uuid": "h", "pos": {"x": 2.0, "y": 2.0}}]});
        let mut facts = routed(tally_drc(&report(vec![clearance.clone(), own.clone()])));
        facts.reference = reference(vec![clearance, own]);
        let verdict = judge(&facts);
        assert_eq!(verdict.status, KiCadRunStatus::Complete);
        let action = verdict
            .next_actions
            .iter()
            .find(|action| {
                action.contains("fab-kicad-board, which judges the board on its own, refuses them")
            })
            .unwrap();
        assert!(action.starts_with("1 of the findings the reference has at the same place are between two nets (clearance 1; parts J1, R31)"), "{action}");
        // Without a reference: no such line.
        facts.reference = KiCadReference::None;
        assert!(
            !judge(&facts)
                .next_actions
                .iter()
                .any(|action| action.contains("the reference has at the same place"))
        );
    }

    #[test]
    fn the_custom_rules_are_said_in_one_line() {
        use crate::project_rules::KiCadCustomRuleReport;
        let rule = |name: &str, status: &str| KiCadCustomRuleReport {
            rule: name.into(),
            constraint: "clearance".into(),
            status: status.into(),
            detail: String::new(),
        };
        assert_eq!(custom_rules_line(&[]), None);
        let line = custom_rules_line(&[
            rule("Shrink_Clearance_In_MCU", "no_effect"),
            rule("TFT_ZONE_2W_Clearance", "approximated"),
            rule("Creepage", "unmodelled"),
        ])
        .unwrap();
        assert!(
            line.starts_with("custom rules (.kicad_dru): 3 constraints: approximated 1 (TFT_ZONE_2W_Clearance clearance), no_effect 1, unmodelled 1 (Creepage clearance); "),
            "{line}"
        );
        let mut facts = routed(DrcTally::default());
        facts.custom_rules = vec![rule("TFT_ZONE_2W_Clearance", "approximated")];
        let verdict = serde_json::to_value(judge(&facts)).unwrap();
        assert_eq!(verdict["custom_rules"][0]["status"], "approximated");
    }

    #[test]
    fn a_clearance_between_fixed_parts_is_not_the_runs_to_move() {
        // framework_mobo_lefthalf without a reference: R31 and R32 fixed
        // under J1's pads by the designer, 0.0867 mm against 0.1 mm.
        let between = |r: &str, pad: &str, net: &str| {
            json!({"type": "clearance", "severity": "error",
                   "description": "Clearance violation (footprint J1 clearance 0.1000 mm; actual 0.0867 mm)",
                   "items": [{"description": format!("Pad 1 [{net}] of {r} on B.Cu"), "uuid": format!("{r}-1"), "pos": {"x": 1.0, "y": 1.0}},
                             {"description": format!("PTH pad {pad} [/TC_D+] of J1"), "uuid": pad, "pos": {"x": 1.5, "y": 1.0}}]})
        };
        let mut facts = routed(tally_drc(&report(vec![
            between("R31", "B6", "Net-(J1-CC2)"),
            between("R32", "B7", "Net-(J1-SBU2)"),
        ])));
        facts.command = "layout-kicad-board".into();
        let kept = |references: &[&str]| KiCadInputItems {
            kept_references: references.iter().map(|r| r.to_string()).collect(),
            ..Default::default()
        };
        facts.input = Some(kept(&["J1", "R31", "R32"]));
        let verdict = judge(&facts);
        assert_eq!(verdict.new_findings_strict.get("clearance"), Some(&2));
        let action = verdict
            .next_actions
            .iter()
            .find(|action| action.starts_with("clearance 2"))
            .unwrap();
        assert!(
            action.contains("kept where the input has them (J1, R31, R32: fixed)")
                && action.contains("fab-kicad-board refuses it")
                && !action.contains("move those nets' parts apart"),
            "{action}"
        );
        // R32 placed by the run: the finding is the run's to move.
        facts.input = Some(kept(&["J1", "R31"]));
        let verdict = judge(&facts);
        let action = verdict
            .next_actions
            .iter()
            .find(|action| action.starts_with("clearance 2"))
            .unwrap();
        assert!(action.contains("move those nets' parts apart"), "{action}");
        // The fab gate with --input: the same findings, said per type.
        facts.command = "fab-kicad-board".into();
        facts.input = Some(kept(&["J1", "R31", "R32"]));
        let (actions, count) = kept_parts_actions(&facts);
        assert_eq!(count, 2);
        assert_eq!(actions.len(), 1);
        assert!(
            actions[0].starts_with("clearance 2")
                && actions[0].contains("(J1, R31, R32: unchanged from the input)")
                && !actions[0].contains("move those nets' parts apart"),
            "{actions:?}"
        );
        facts.input = Some(kept(&["J1", "R31"]));
        assert_eq!(kept_parts_actions(&facts), (Vec::new(), 0));
    }

    #[test]
    fn a_footprints_own_findings_under_the_board_minimum_get_no_custom_rule() {
        // framework_mobo_lefthalf's J1: a footprint clearance of 0.1 mm
        // below the board's 0.25 mm hole clearance, so KiCad reports the
        // board minimum, which a custom rule does not lower.
        let own = |pad: &str| {
            drc_finding(&json!({"type": "hole_clearance", "severity": "error",
                   "description": "Hole clearance violation (board minimum hole clearance 0.2500 mm; actual 0.1030 mm)",
                   "items": [{"description": format!("PTH pad {pad} [GND] of J1"), "uuid": pad, "pos": {"x": 1.0, "y": 1.0}},
                             {"description": "NPTH pad of J1", "uuid": "hole", "pos": {"x": 1.0, "y": 2.0}}]}))
        };
        let (a, b) = (own("B1"), own("B12"));
        let note = footprint_findings_note(&[&a, &b], "board.kicad_dru").unwrap();
        assert!(!note.contains("(rule "), "{note}");
        assert!(
            note.contains("exclude J1's hole_clearance in KiCad's DRC dialog (Exclude this violation): KiCad holds them to the board minimum (min_hole_clearance in the .kicad_pro"),
            "{note}"
        );
    }

    #[test]
    fn a_footprints_own_findings_do_not_fail_a_layout_without_a_reference() {
        // The schematic trial's USB-C receptacle: its GND pads 0.19 mm from
        // its own locating holes (rule 0.25 mm), no reference board.
        let own = |pad: &str, x: f64| {
            json!({"type": "hole_clearance", "severity": "error",
                   "description": "Hole clearance violation (board setup constraints hole clearance 0.2500 mm; actual 0.1944 mm)",
                   "items": [{"description": format!("Pad {pad} [GND] of J1 on F.Cu"), "uuid": pad, "pos": {"x": x, "y": 1.0}},
                             {"description": "NPTH pad of J1", "uuid": "hole", "pos": {"x": x, "y": 2.0}}]})
        };
        let mut facts = routed(tally_drc(&report(vec![own("A1", 1.0), own("B1", 5.0)])));
        facts
            .outputs
            .insert("board".into(), "out/result/board.kicad_pcb".into());
        let verdict = judge(&facts);
        assert_eq!(
            verdict.status,
            KiCadRunStatus::Complete,
            "{}",
            verdict.reason
        );
        assert!(verdict.new_findings_strict.is_empty() && verdict.new_findings.is_empty());
        assert_eq!(verdict.footprint_findings, map(&[("hole_clearance", 2)]));
        assert!(!verdict.clean_without_subtraction);
        let note = verdict
            .next_actions
            .iter()
            .find(|action| action.contains("within one footprint"))
            .expect("a line for the footprint's findings");
        assert!(
            note.contains("J1: hole_clearance 2")
                && note.contains("out/result/board.kicad_dru")
                && note.contains("(rule \"J1 own hole_clearance\" (constraint hole_clearance (min 0.19mm)) (condition \"A.memberOfFootprint('J1') && B.memberOfFootprint('J1')\"))")
                && !note.contains("move the parts apart"),
            "{note}"
        );
        // A hole of another part, and a short within one footprint, count.
        let mut apart = own("A1", 1.0);
        apart["items"][1]["description"] = "NPTH pad of H2".into();
        let short = json!({"type": "shorting_items", "severity": "error", "description": "Items shorting two nets",
            "items": [{"description": "Pad 1 [VDD] of U1 on F.Cu", "uuid": "p1", "pos": {"x": 0.0, "y": 0.0}},
                      {"description": "Pad 2 [GND] of U1 on F.Cu", "uuid": "p2", "pos": {"x": 0.1, "y": 0.0}}]});
        let verdict = judge(&routed(tally_drc(&report(vec![
            own("B1", 5.0),
            apart,
            short,
        ]))));
        assert_eq!(verdict.status, KiCadRunStatus::Incomplete);
        assert_eq!(
            verdict.new_findings_strict,
            map(&[("hole_clearance", 1), ("shorting_items", 1)])
        );
        assert_eq!(verdict.footprint_findings, map(&[("hole_clearance", 1)]));
        // With a reference, they are the reference's, as before.
        let mut facts = routed(tally_drc(&report(vec![own("A1", 1.0)])));
        facts.reference = reference(Vec::new());
        let verdict = judge(&facts);
        assert_eq!(verdict.status, KiCadRunStatus::Complete);
        assert!(verdict.footprint_findings.is_empty());
        assert_eq!(verdict.preexisting_findings, map(&[("hole_clearance", 1)]));
    }

    #[test]
    fn a_pads_own_ring_hole_or_padstack_does_not_fail_a_layout_without_a_reference() {
        // 0xCB-1337's RP2040 (U3): its thermal-via pads 45 have a 0.10 mm
        // ring under the board's 0.13 mm minimum, on the designer's board
        // too; hackrf's fiducials have copper on both sides. Laid out
        // without a reference they made a complete board exit 1.
        let one = |kind: &str, description: &str, item: &str, x: f64| {
            json!({"type": kind, "severity": "error", "description": description,
                   "items": [{"description": item, "uuid": format!("{item}{x}"), "pos": {"x": x, "y": 1.0}}]})
        };
        let ring = |x: f64| {
            one(
                "annular_width",
                "Annular width (board setup constraints min annular width 0.1300 mm; actual 0.1000 mm)",
                "PTH pad 45 [GND] of U3",
                x,
            )
        };
        let fiducial = one(
            "padstack",
            "Padstack is questionable (SMD pad has copper on both sides of the board)",
            "Pad [<no net>] of FID1 on all copper layers",
            9.0,
        );
        let hole = one(
            "drill_out_of_range",
            "Hole size out of range (board setup constraints min hole 0.3000 mm; actual 0.2400 mm)",
            "PTH pad 1 [GND] of TC1",
            7.0,
        );
        let mut facts = routed(tally_drc(&report(vec![
            ring(1.0),
            ring(2.0),
            fiducial,
            hole,
        ])));
        facts
            .outputs
            .insert("board".into(), "out/result/board.kicad_pcb".into());
        let verdict = judge(&facts);
        assert_eq!(
            verdict.status,
            KiCadRunStatus::Complete,
            "{}",
            verdict.reason
        );
        assert!(verdict.new_findings_strict.is_empty());
        assert_eq!(
            verdict.footprint_findings,
            map(&[
                ("annular_width", 2),
                ("drill_out_of_range", 1),
                ("padstack", 1)
            ])
        );
        let note = verdict
            .next_actions
            .iter()
            .find(|action| action.contains("within one footprint"))
            .expect("a line for the footprint's findings");
        // KiCad checks a pad's ring and hole with no item B: the rule's
        // condition names A alone (a condition on B never holds).
        assert!(
            note.contains("U3: annular_width 2")
                && note.contains("(rule \"U3 own annular_width\" (constraint annular_width (min 0.1mm)) (condition \"A.memberOfFootprint('U3')\"))")
                && note.contains("(rule \"TC1 own drill_out_of_range\" (constraint hole_size (min 0.24mm)) (condition \"A.memberOfFootprint('TC1')\"))")
                && note.contains("exclude FID1's padstack"),
            "{note}"
        );
        // A via's ring, a hole over a maximum, a starved thermal on one
        // pad and a ring of a pad no footprint names count, or (the
        // maximum) are excluded, not given a minimum.
        let via = one(
            "annular_width",
            "Annular width (board setup constraints min annular width 0.1300 mm; actual 0.1000 mm)",
            "Via [GND] on F.Cu - B.Cu",
            3.0,
        );
        let starved = one(
            "starved_thermal",
            "Thermal relief connection to zone incomplete (zone min spoke count 2; actual 1)",
            "Pad 2 [GND] of C1 on F.Cu",
            4.0,
        );
        let nameless = one(
            "drill_out_of_range",
            "Hole size out of range (board setup constraints min hole 0.3000 mm; actual 0.1000 mm)",
            "NPTH pad of  ",
            5.0,
        );
        let big = one(
            "drill_out_of_range",
            "Hole size out of range (board setup constraints max hole 6.3500 mm; actual 7.0000 mm)",
            "NPTH pad of H1",
            6.0,
        );
        let verdict = judge(&routed(tally_drc(&report(vec![
            via, starved, nameless, big,
        ]))));
        assert_eq!(verdict.status, KiCadRunStatus::Incomplete);
        assert_eq!(
            verdict.new_findings_strict,
            map(&[
                ("annular_width", 1),
                ("drill_out_of_range", 1),
                ("starved_thermal", 1)
            ])
        );
        assert_eq!(
            verdict.footprint_findings,
            map(&[("drill_out_of_range", 1)])
        );
        let note = verdict
            .next_actions
            .iter()
            .find(|action| action.contains("within one footprint"))
            .expect("a line for H1's hole");
        assert!(
            note.contains("exclude H1's drill_out_of_range") && !note.contains("(rule "),
            "{note}"
        );
        // With a reference, the strict rule decides (the designer's at the
        // same place), as before.
        let mut facts = routed(tally_drc(&report(vec![ring(1.0)])));
        facts.reference = reference(Vec::new());
        let verdict = judge(&facts);
        assert_eq!(verdict.new_findings_strict, map(&[("annular_width", 1)]));
        assert!(verdict.footprint_findings.is_empty());
    }

    #[test]
    fn the_inputs_items_are_the_parts_kept_and_its_zones_and_graphics_unchanged() {
        let input = r#"(kicad_pcb (version 20240108)
            (footprint "lib:A" (layer "F.Cu") (uuid "fa") (at 10 10)
              (property "Reference" "J1") (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (uuid "pa1")))
            (footprint "lib:B" (layer "F.Cu") (uuid "fb") (at 20 10 90)
              (property "Reference" "R1") (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (uuid "pb1")))
            (footprint "lib:C" (layer "F.Cu") (uuid "fc") (at 5 5)
              (property "Reference" "R2") (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (uuid "pc1")))
            (gr_line (start 0 0) (end 30 0) (layer "Edge.Cuts") (uuid "e1"))
            (gr_line (start 0 0) (end 0 30) (layer "Edge.Cuts") (uuid "e2"))
            (zone (net 1) (net_name "GND") (layer "F.Cu") (uuid "z1") (polygon (pts (xy 0 0) (xy 30 0) (xy 30 30)))))"#;
        // J1 where it was (its rotation written out), R1 moved, R2 flipped,
        // e2 changed (a new outline), z1 filled, a pour and a track added.
        let result = r#"(kicad_pcb (version 20240108)
            (footprint "lib:A" (layer "F.Cu") (uuid "fa") (at 10.0 10 360)
              (property "Reference" "J1") (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (uuid "pa1")))
            (footprint "lib:B" (layer "F.Cu") (uuid "fb") (at 25 10 90)
              (property "Reference" "R1") (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (uuid "pb1")))
            (footprint "lib:C" (layer "B.Cu") (uuid "fc") (at 5 5)
              (property "Reference" "R2") (pad "1" smd rect (at 0 0) (size 1 1) (layers "B.Cu") (uuid "pc1")))
            (gr_line (start 0 0) (end 30 0) (layer "Edge.Cuts") (uuid "e1"))
            (gr_line (start 0 0) (end 0 40) (layer "Edge.Cuts") (uuid "e2"))
            (zone (net 1) (net_name "GND") (layer "F.Cu") (uuid "z1") (polygon (pts (xy 0 0) (xy 30 0) (xy 30 30)))
              (filled_polygon (layer "F.Cu") (pts (xy 1 1) (xy 29 1) (xy 29 29))))
            (zone (net 1) (net_name "GND") (layer "B.Cu") (uuid "z2") (polygon (pts (xy 0 0) (xy 30 0) (xy 30 30))))
            (segment (start 1 1) (end 2 2) (width 0.2) (layer "F.Cu") (net 1) (uuid "s1")))"#;
        let items = input_items(input, result).unwrap();
        let set = |uuids: &[&str]| {
            uuids
                .iter()
                .map(|uuid| uuid.to_string())
                .collect::<BTreeSet<_>>()
        };
        assert_eq!(items.uuids, set(&["fa", "pa1", "e1", "z1"]));
        assert_eq!(items.kept_references, set(&["J1"]));
        assert_eq!((items.kept_parts, items.parts), (1, 3));
        // A board judged on its own: every part, zone and graphic, never a
        // track.
        let alone = board_items(result).unwrap();
        assert!(
            ["fa", "pb1", "pc1", "e2", "z2"]
                .iter()
                .all(|uuid| alone.uuids.contains(*uuid))
                && !alone.uuids.contains("s1")
        );
    }

    /// A finding with KiCad's item descriptions and uuids.
    fn input_finding(kind: &str, items: &[(&str, &str)]) -> serde_json::Value {
        json!({"type": kind, "severity": "error", "description": kind,
               "items": items.iter().enumerate().map(|(index, (description, uuid))| json!({"description": description, "uuid": uuid, "pos": {"x": index as f64, "y": 0.0}})).collect::<Vec<_>>()})
    }

    /// A layout's input: J1, J2 and FID1 kept, R1 moved; the edge e1, the
    /// board's copper ring g1 and the zone z1 the input's.
    fn kept_input() -> KiCadInputItems {
        KiCadInputItems {
            uuids: ["fj1", "j1", "j2", "fid", "e1", "g1", "z1"]
                .iter()
                .map(|uuid| uuid.to_string())
                .collect(),
            kept_references: ["J1", "J2", "FID1"]
                .iter()
                .map(|reference| reference.to_string())
                .collect(),
            kept_parts: 3,
            parts: 4,
        }
    }

    fn laid_out(findings: Vec<serde_json::Value>) -> KiCadRunFacts {
        let mut facts = routed(tally_drc(&report(findings)));
        facts.command = "layout-kicad-board".into();
        facts.input = Some(kept_input());
        facts
            .outputs
            .insert("board".into(), "out/result/board.kicad_pcb".into());
        facts
    }

    #[test]
    fn the_inputs_own_findings_do_not_fail_a_layout_without_a_reference() {
        let fixed_edge = input_finding(
            "copper_edge_clearance",
            &[
                ("Segment on Edge.Cuts", "e1"),
                ("Pad MP [GND] of J1 on F.Cu", "j1"),
            ],
        );
        let ring_edge = input_finding(
            "copper_edge_clearance",
            &[("Arc on Edge.Cuts", "e1"), ("Segment [GND] on F.Cu", "g1")],
        );
        let fiducial = input_finding(
            "solder_mask_bridge",
            &[
                ("Pad [<no net>] of FID1 on F.Cu", "fid"),
                ("Zone [GNDD] on F.Cu and B.Cu, priority 0", "z1"),
            ],
        );
        let courtyards = input_finding(
            "courtyards_overlap",
            &[("Footprint J2", "fj2"), ("Footprint J1", "fj1")],
        );
        let verdict = judge(&laid_out(vec![
            fixed_edge.clone(),
            ring_edge,
            fiducial,
            courtyards,
        ]));
        assert_eq!(
            verdict.status,
            KiCadRunStatus::Complete,
            "{}",
            verdict.reason
        );
        assert_eq!(
            verdict.input_findings,
            map(&[
                ("copper_edge_clearance", 2),
                ("courtyards_overlap", 1),
                ("solder_mask_bridge", 1)
            ])
        );
        assert!(verdict.footprint_findings.is_empty() && verdict.new_findings_strict.is_empty());
        let note = verdict
            .next_actions
            .iter()
            .find(|action| action.contains("the input's"))
            .expect("a line for the input's findings");
        assert!(
            note.contains("out/result/board.kicad_dru")
                && note.contains("(rule \"input edge_clearance\" (constraint edge_clearance (min 0mm)) (severity ignore) (condition \"A.memberOfFootprint('J1')\"))")
                && note.contains("(rule \"input GND graphics edge_clearance\" (constraint edge_clearance (min 0mm)) (severity ignore) (condition \"A.Type == 'Graphic' && A.NetName == 'GND' && !A.memberOfFootprint('*')\"))")
                && note.contains("(rule \"input courtyards\" (constraint courtyard_clearance (min 0mm)) (severity ignore) (condition \"(A.Reference == 'J1' && B.Reference == 'J2')\"))")
                && note.contains("exclude in KiCad's DRC dialog")
                && note.contains("solder_mask_bridge 1 (Pad [<no net>] of FID1 on F.Cu and Zone [GNDD]"),
            "{note}"
        );
        // A kept pad of a blank reference: no rule names it, an exclusion.
        let blank = input_finding(
            "copper_edge_clearance",
            &[
                ("Rectangle on Margin", "e1"),
                ("PTH pad S3 [GND] of   ", "j2"),
            ],
        );
        let verdict = judge(&laid_out(vec![blank]));
        assert_eq!(
            verdict.status,
            KiCadRunStatus::Complete,
            "{}",
            verdict.reason
        );
        let note = verdict
            .next_actions
            .iter()
            .find(|action| action.contains("the input's"))
            .unwrap();
        assert!(
            !note.contains("(rule")
                && note.contains("copper_edge_clearance 1 (Rectangle on Margin and PTH pad S3"),
            "{note}"
        );
        // A pad KiCad gives a uuid the board does not store: its part,
        // kept, tells.
        let mut unstored = fixed_edge;
        unstored["items"][1]["uuid"] = "random".into();
        assert_eq!(
            judge(&laid_out(vec![unstored])).status,
            KiCadRunStatus::Complete
        );
    }

    #[test]
    fn a_finding_with_anything_the_run_wrote_or_between_two_nets_counts() {
        let cases = [
            // A part the layout moved, at the edge.
            input_finding(
                "copper_edge_clearance",
                &[
                    ("Segment on Edge.Cuts", "e1"),
                    ("Pad 1 [VCC] of R1 on F.Cu", "r1"),
                ],
            ),
            // A kept pad and our track.
            input_finding(
                "solder_mask_bridge",
                &[
                    ("Pad 1 [GND] of J1 on F.Cu", "j1"),
                    ("Track [VCC] on F.Cu, length 1.0000 mm", "t1"),
                ],
            ),
            // A kept pad and a pour of ours.
            input_finding(
                "solder_mask_bridge",
                &[
                    ("Pad 1 [GND] of J1 on F.Cu", "j1"),
                    ("Zone [VCC] on B.Cu, priority 0", "ours"),
                ],
            ),
            // Two kept parts' pads shorting, and too close: electrical.
            input_finding(
                "shorting_items",
                &[
                    ("Pad 1 [GND] of J1 on F.Cu", "j1"),
                    ("Pad 1 [VCC] of J2 on F.Cu", "j2"),
                ],
            ),
            input_finding(
                "clearance",
                &[
                    ("Pad 1 [GND] of J1 on F.Cu", "j1"),
                    ("Pad 1 [VCC] of J2 on F.Cu", "j2"),
                ],
            ),
            input_finding(
                "hole_clearance",
                &[
                    ("Pad 1 [GND] of J1 on F.Cu", "j1"),
                    ("PTH pad 1 [VCC] of J2", "j2"),
                ],
            ),
        ];
        for case in cases {
            let verdict = judge(&laid_out(vec![case.clone()]));
            assert_eq!(verdict.status, KiCadRunStatus::Incomplete, "{case}");
            assert!(verdict.input_findings.is_empty(), "{case}");
            assert!(
                !verdict
                    .next_actions
                    .iter()
                    .any(|action| action.contains("the input's"))
            );
        }
        // Without the input's items (not known), nothing is the input's.
        let fixed_edge = input_finding(
            "copper_edge_clearance",
            &[
                ("Segment on Edge.Cuts", "e1"),
                ("Pad MP [GND] of J1 on F.Cu", "j1"),
            ],
        );
        let mut facts = laid_out(vec![fixed_edge.clone()]);
        facts.input = None;
        assert_eq!(judge(&facts).status, KiCadRunStatus::Incomplete);
        // With a reference nothing changes: the reference decides by place.
        let mut facts = laid_out(vec![fixed_edge.clone()]);
        facts.reference = reference(Vec::new());
        let verdict = judge(&facts);
        assert_eq!(verdict.status, KiCadRunStatus::Incomplete);
        assert!(verdict.input_findings.is_empty());
        assert_eq!(
            verdict.new_findings_strict,
            map(&[("copper_edge_clearance", 1)])
        );
        let mut facts = laid_out(vec![fixed_edge.clone()]);
        facts.reference = reference(vec![fixed_edge]);
        let verdict = judge(&facts);
        assert_eq!(verdict.status, KiCadRunStatus::Complete);
        assert!(verdict.input_findings.is_empty());
        assert_eq!(
            verdict.preexisting_findings,
            map(&[("copper_edge_clearance", 1)])
        );
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
    fn a_type_at_kicads_cap_is_subtracted_only_at_the_same_place() {
        let ours = finding("hole_clearance", &[("a", 1.0, 1.0)]);
        let elsewhere = finding("hole_clearance", &[("b", 70.0, 70.0)]);
        let mut capped: Vec<serde_json::Value> = (0..KICAD_FINDING_CAP)
            .map(|index| finding("hole_clearance", &[("z", index as f64, 40.0)]))
            .collect();
        capped.push(ours.clone());
        let mut facts = routed(tally_drc(&report(vec![ours, elsewhere])));
        facts.reference = reference(capped);
        let verdict = judge(&facts);
        assert_eq!(
            verdict.preexisting_unknown,
            vec!["hole_clearance".to_string()]
        );
        assert_eq!(verdict.new_findings_strict, map(&[("hole_clearance", 1)]));
        assert_eq!(verdict.preexisting_findings, map(&[("hole_clearance", 1)]));
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
    fn a_footprints_own_finding_is_the_references() {
        // Batch_1: a mounting hole's ring of overlapping pads, its antenna's
        // copper at its feed pad; the reference's report is at KiCad's cap
        // and names none of them.
        let between = |kind: &str, first: &str, second: &str| {
            json!({"type": kind, "severity": "error", "description": kind,
                   "items": [{"description": first, "uuid": "u1", "pos": {"x": 1.0, "y": 2.0}},
                             {"description": second, "uuid": "u2", "pos": {"x": 1.0, "y": 4.0}}]})
        };
        let ours = tally_drc(&report(vec![
            between(
                "clearance",
                "PTH pad [<no net>] of M15",
                "PTH pad [GND] of M15",
            ),
            between(
                "clearance",
                "Pad 1 [Net-(AE1-A)] of AE1 on F.Cu",
                "Segment [<no net>] of AE1 on F.Cu",
            ),
            // Two footprints, or a track: not one footprint's own.
            between(
                "clearance",
                "PTH pad [<no net>] of M15",
                "PTH pad [GND] of M16",
            ),
            between(
                "clearance",
                "Track [GND] on F.Cu, length 1.0000 mm",
                "Pad 2 [VCC] of R62 on F.Cu",
            ),
        ]))
        .findings;
        assert!(footprint_own(&ours[0]) && footprint_own(&ours[1]));
        assert!(!footprint_own(&ours[2]) && !footprint_own(&ours[3]));
        let capped: Vec<DrcFinding> = tally_drc(&report(
            (0..KICAD_FINDING_CAP)
                .map(|index| finding("clearance", &[("z", index as f64, 40.0)]))
                .collect(),
        ))
        .findings;
        let split = split_findings_strict(&ours, &capped);
        assert_eq!(split.preexisting, map(&[("clearance", 2)]));
        assert_eq!(split.new, map(&[("clearance", 2)]));
        assert_eq!(split.new_indices, vec![2, 3]);
    }

    #[test]
    fn a_copper_edge_finding_is_at_the_same_place_by_its_copper() {
        // KiCad names the nearest of several outline pieces beside the
        // designer's GND ring: another one on the stripped board.
        let edge = |outline: &str, uuid: &str, x: f64| {
            json!({"type": "copper_edge_clearance", "severity": "error",
                   "description": "Board edge clearance violation",
                   "items": [{"description": outline, "uuid": uuid, "pos": {"x": x, "y": 125.0}},
                             {"description": "Segment [GND] on F.Cu", "uuid": "ring", "pos": {"x": 17.55, "y": 125.0}}]})
        };
        let ours = tally_drc(&report(vec![edge("Arc on Edge.Cuts", "e1", 99.0)])).findings;
        let theirs = tally_drc(&report(vec![edge("Segment on Edge.Cuts", "e2", 17.0)])).findings;
        assert!(same_place(&ours[0], &theirs[0]));
        let mut moved = edge("Segment on Edge.Cuts", "e2", 17.0);
        moved["items"][1]["pos"]["x"] = json!(30.0);
        moved["items"][1]["uuid"] = json!("track");
        let moved = tally_drc(&report(vec![moved])).findings;
        assert!(!same_place(&moved[0], &theirs[0]));
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
        // The open connections' action asks for the evidence first: a
        // design change is a hypothesis unless the reference rules
        // density out (the remapper trial: no "too dense" without it).
        let open = verdict
            .next_actions
            .iter()
            .find(|action| action.starts_with("connections are open"))
            .unwrap();
        assert!(open.contains("inspect-routing-run"), "{open}");
        assert!(open.contains("hypotheses to test"), "{open}");
        assert!(!open.contains("too dense"), "{open}");
        assert!(!open.contains("enlarge the outline"), "{open}");
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
    fn a_panel_without_its_parts_boards_says_so_first() {
        let mut facts = KiCadRunFacts::new("place-kicad-board");
        facts.routes = false;
        facts.input_warnings = vec!["fixed part U3 lies on 13 of the board's texts".into()];
        facts.sub_boards = Some(crate::sub_boards::KiCadSubBoardsReport {
            count: 2,
            unassigned: 392,
            unassigned_parts: vec!["C1".into(), "C2".into()],
            ..Default::default()
        });
        let verdict = judge(&facts);
        assert!(
            verdict.next_actions[0].starts_with("the board is a panel of 2 different boards"),
            "{:?}",
            verdict.next_actions
        );
        assert!(verdict.next_actions[0].contains("--reference <project-dir>"));
        assert!(verdict.next_actions[1].starts_with("fixed part U3"));
    }

    #[test]
    fn a_placement_with_the_panels_boards_mixed_is_incomplete_without_room_advice() {
        // The OpenESC panel without a reference: every part placed, exit 0
        // until 2026-10-27; its layout then left 8 parts without a place
        // and suggested an outline for a panel whose frame is fixed.
        let mut facts = KiCadRunFacts::new("place-kicad-board");
        facts.routes = false;
        facts.sub_boards = Some(crate::sub_boards::KiCadSubBoardsReport {
            count: 2,
            unassigned: 392,
            unassigned_parts: vec!["C1".into()],
            ..Default::default()
        });
        let verdict = judge(&facts);
        assert_eq!(verdict.exit_code, EXIT_INCOMPLETE);
        assert!(
            verdict.reason.starts_with(
                "incomplete: the panel's 2 boards' parts are mixed: 392 movable parts"
            ),
            "{}",
            verdict.reason
        );
        assert!(verdict.next_actions[0].contains("--reference <project-dir>"));
        // With parts unplaced: the sub-board line leads the reason, and
        // no outline is suggested.
        facts.unplaced = vec!["Q17".into(), "CL35".into()];
        facts.placement_hints = vec!["the front side is 107 % full: enlarge the outline".into()];
        facts.room = Some(crate::placement_room::KiCadRoom {
            outline_mm: [41.6, 80.4],
            suggested_outline_mm: Some([43.0, 83.0]),
            ..Default::default()
        });
        let verdict = judge(&facts);
        assert!(
            verdict
                .reason
                .starts_with("incomplete: the panel's 2 boards"),
            "{}",
            verdict.reason
        );
        assert!(verdict.reason.contains("2 parts without a legal place"));
        assert!(
            !verdict
                .next_actions
                .iter()
                .any(|action| action.contains("try an outline") || action.contains("enlarge")),
            "{:?}",
            verdict.next_actions
        );
        assert!(
            verdict.next_actions[1]
                .starts_with("2 parts found no legal place with the panel's boards' parts mixed"),
            "{:?}",
            verdict.next_actions
        );
        assert_eq!(verdict.room.as_ref().unwrap().suggested_outline_mm, None);
        // Held in their boards (a tenth or fewer unknown): as before.
        if let Some(sub) = &mut facts.sub_boards {
            sub.unassigned = 6;
            sub.held = true;
        }
        let verdict = judge(&facts);
        assert!(verdict.reason.starts_with("incomplete: 2 parts without"));
        assert!(
            verdict
                .next_actions
                .iter()
                .any(|action| action.contains("try an outline of 43.0 x 83.0"))
        );
    }

    #[test]
    fn a_layout_without_a_place_for_its_parts_says_the_rates_it_placed_at() {
        let measured = crate::work_rate::KiCadWorkRate {
            expansions_per_second: 2.4041e6,
            placer_work_per_second: 1.2539e8,
            source: "measured".into(),
            measured: None,
        };
        let action = placement_rates_action("layout-kicad-board", &measured).unwrap();
        assert!(
            action.contains("PCB_WORK_RATE=2.4041e6,1.2539e8 repeats this placement"),
            "{action}"
        );
        assert!(
            action.contains("0.76 and 0.84 of the reference's"),
            "{action}"
        );
        // Pinned rates repeat by themselves, and a placement is the check.
        let pinned = crate::work_rate::KiCadWorkRate {
            source: "PCB_WORK_RATE".into(),
            ..measured.clone()
        };
        assert_eq!(placement_rates_action("layout-kicad-board", &pinned), None);
        assert_eq!(placement_rates_action("place-kicad-board", &measured), None);
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
    fn a_layout_with_its_decoupling_far_from_the_pins_gets_a_next_action() {
        let mut facts = routed(DrcTally::default());
        let decoupling = |share: f64| crate::quality::KiCadDecoupling {
            supply_pins: 21,
            capacitors: 14,
            median_mm: Some(7.75),
            within_3mm: Some(0.048),
            within_5mm: Some(0.333),
            worst: vec![
                ("U2:46".into(), None),
                ("U4:5".into(), Some(29.83)),
                ("U2:39".into(), Some(14.3)),
            ],
            capacitor_median_mm: Some(6.94),
            capacitors_within_3mm: Some(share),
            worst_capacitors: vec![vec![], vec!["C4".into(), "C9".into()], vec!["C5".into()]],
        };
        // The trial's rp2350-dual-usb: complete, with one action.
        facts.decoupling = Some(decoupling(0.071));
        let verdict = judge(&facts);
        assert_eq!(verdict.exit_code, EXIT_COMPLETE);
        assert_eq!(verdict.next_actions.len(), 1, "{:?}", verdict.next_actions);
        let action = &verdict.next_actions[0];
        assert!(
            action.starts_with("decoupling: 7 % of the decoupling capacitors")
                && action.contains("a median 6.9 mm")
                && action.contains("U4:5 29.8 mm, U2:39 14.3 mm")
                && action.contains("{\"part\": \"C4\", \"pin_of\": \"U4:5\", \"max_mm\": 1.5}")
                && !action.contains("U2:46")
                && action.contains("decoupling_near"),
            "{action}"
        );
        // A near constraint holds C4: C9 is still free for U4:5.
        facts.decoupling_constrained = BTreeSet::from(["C4".to_string()]);
        let action = judge(&facts).next_actions[0].clone();
        assert!(
            action.contains("\"part\": \"C9\", \"pin_of\": \"U4:5\""),
            "{action}"
        );
        // decoupling_near on: not offered again.
        facts.decoupling_near = true;
        let action = judge(&facts).next_actions[0].clone();
        assert!(
            !action.contains("turn on the placer's decoupling_near"),
            "{action}"
        );
        // The schematic trial: every capacitor of those rails held at
        // another pin. No pin to give one, no decoupling_near: said so.
        facts.decoupling_constrained = ["C4", "C5", "C9"]
            .iter()
            .map(|part| part.to_string())
            .collect();
        facts.decoupling_near = false;
        let action = judge(&facts).next_actions[0].clone();
        assert!(
            !action.contains("pin_of")
                && !action.contains("turn on the placer's decoupling_near")
                && action.contains("(U4:5 29.8 mm, U2:39 14.3 mm) have no capacitor of their own"),
            "{action}"
        );
        facts.decoupling_constrained.clear();
        // As a designer places them: nothing to say.
        facts.decoupling = Some(decoupling(0.857));
        assert!(judge(&facts).next_actions.is_empty());
    }

    #[test]
    fn a_pour_in_many_pieces_is_reported_and_gets_a_next_action() {
        let mut facts = routed(DrcTally::default());
        let pour = |pieces: usize, largest: f64| crate::board_router::KiCadPourQuality {
            net: "GND".into(),
            layer: "B.Cu".into(),
            pieces,
            largest_share: largest / 2591.0,
            coverage: 638.0 / 2591.0,
            largest_mm2: largest,
            copper_mm2: 638.0,
            area_mm2: 2591.0,
            islands_removed: 4,
            cut_mm: 1453.0,
            keep_whole_parts: vec!["U1".into()],
        };
        // Whole: in the verdict, no action.
        facts.pours = vec![pour(3, 600.0)];
        let verdict = judge(&facts);
        assert_eq!(verdict.exit_code, EXIT_COMPLETE);
        assert_eq!(verdict.pours.len(), 1);
        assert!(
            verdict.next_actions.is_empty(),
            "{:?}",
            verdict.next_actions
        );
        // 43 pieces: complete still, with the action naming keep_whole.
        facts.pours = vec![pour(43, 120.0)];
        let verdict = judge(&facts);
        assert_eq!(verdict.exit_code, EXIT_COMPLETE);
        assert_eq!(verdict.next_actions.len(), 1);
        assert!(
            verdict.next_actions[0].contains("GND on B.Cu: 43 pieces, the largest 5 % of the pour")
                && verdict.next_actions[0].contains("keep_whole"),
            "{}",
            verdict.next_actions[0]
        );
        assert!(
            verdict.next_actions[0].contains(r#""keep_whole": [{"parts": ["U1"]}]"#),
            "{}",
            verdict.next_actions[0]
        );
        // No part whose region keeps clear of other nets' pads: no
        // keep_whole under parts (the trial's board A on F.Cu).
        let mut crowded = pour(43, 120.0);
        crowded.keep_whole_parts.clear();
        facts.pours = vec![crowded];
        let action = &judge(&facts).next_actions[0];
        assert!(
            action.contains("does not fit this layer") && !action.contains("\"parts\""),
            "{action}"
        );
        // Few pieces, but the largest holds less than half the copper.
        facts.pours = vec![pour(4, 200.0)];
        assert_eq!(judge(&facts).next_actions.len(), 1);
        let json = serde_json::to_value(judge(&facts)).unwrap();
        assert_eq!(json["pours"][0]["pieces"], 4);
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
            finding_action("connection_width", 1, &[&finding], None).starts_with(
                "connection_width 1 (board setup constraints minimum connection width 0.1270 mm; actual 0.1137 mm): "
            )
        );
        let finding = DrcFinding {
            kind: "clearance".into(),
            description: "Clearance violation ( clearance 0.1270 mm; actual 0.0000 mm)".into(),
            ..DrcFinding::default()
        };
        assert!(
            finding_action("clearance", 1, &[&finding], None)
                .starts_with("clearance 1 (clearance 0.1270 mm; actual 0.0000 mm): ")
        );
        // A footprint's own pad and hole (the schematic trial's USB-C
        // receptacle): no "move the parts apart".
        let own = |pad: &str| {
            DrcFinding {
            kind: "hole_clearance".into(),
            description: "Hole clearance violation (board setup constraints hole clearance 0.2500 mm; actual 0.1944 mm)".into(),
            nets: vec!["GND".into()],
            item_descriptions: vec![format!("Pad {pad} [GND] of J1 on F.Cu"), "NPTH pad of J1".into()],
            ..DrcFinding::default()
        }
        };
        let (a1, b1) = (own("A1"), own("B1"));
        let action = finding_action("hole_clearance", 2, &[&a1, &b1], None);
        assert!(
            action.contains("between items of one footprint (J1)")
                && action.contains("--allow-findings")
                && !action.contains("move the parts apart"),
            "{action}"
        );
        let apart = DrcFinding {
            item_descriptions: vec!["Pad 1 [GND] of J1 on F.Cu".into(), "NPTH pad of H2".into()],
            ..own("A1")
        };
        assert!(
            finding_action("hole_clearance", 1, &[&apart], None).contains("move the parts apart")
        );
    }

    #[test]
    fn a_bridge_at_a_footprints_mask_graphic_names_the_zone_fill_under_it() {
        // MokyaLora (the trial's layout): KiCad names J6's rectangle and
        // the first copper it found under it, a GND track; the other net
        // is the VSYS zone's fill, which no route keeps out.
        let finding = DrcFinding {
            kind: "solder_mask_bridge".into(),
            description: "Rear solder mask aperture bridges items with different nets".into(),
            nets: vec!["GND".into()],
            item_descriptions: vec![
                "Rectangle of J6 on B.Mask".into(),
                "Track [GND] on B.Cu, length 0.5381 mm".into(),
            ],
            ..DrcFinding::default()
        };
        let action = finding_action("solder_mask_bridge", 1, &[&finding], None);
        assert!(
            action.contains("no route keeps a zone's fill out")
                && action.contains("(Rectangle of J6 on B.Mask)")
                && action.contains("move the part off the zone"),
            "{action}"
        );
        assert!(!action.contains("the router keeps them"), "{action}");
        // A pad's opening over a track: the router's rule, no zone note.
        let pad = DrcFinding {
            item_descriptions: vec![
                "Pad 2 [VBUS] of J1 on F.Cu".into(),
                "Track [GND] on F.Cu, length 1.0 mm".into(),
            ],
            ..finding.clone()
        };
        let action = finding_action("solder_mask_bridge", 1, &[&pad], None);
        assert!(
            action.contains("the router keeps its tracks and vias"),
            "{action}"
        );
        assert!(!action.contains("zone's fill out"), "{action}");
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
