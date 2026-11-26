// Copyright (C) 2026 Florian Loitsch.

//! Check an explicit copper relocation against the original KiCad project.

use crate::fill_handoff::layout::{
    FillPublication, copy_fill_payload, has_fillable_zone, project_inputs,
};
use pcb_kicad_check::native_report;
use pcb_kicad_file::board_stats::board_statistics;
use pcb_kicad_file::fs_util::{
    check_board_id, copy_directory_tree, temporary_next_to, write_atomically, write_typed_json,
};
use pcb_kicad_file::geometry::point_segment_distance_squared;
use pcb_kicad_file::kicad_nets::{footprint_uuid, item_net, net_codes};
use pcb_kicad_file::kicad_sexpr::{Expr, encode, form_atom, form_f64, form_xy, parse};
use pcb_kicad_route::open_repair::{
    GeneratedBranchWork, OpenConnectivityProof, TrackContact, eligible_open_partition,
    finding_types_at_cap, in_repair_copy, refine_generated_contact, report_capped,
    verify_native_partition_merges,
};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    fs,
    path::Path,
    time::{Duration, Instant},
};

mod generated;
pub(crate) mod width_witness;

pub use generated::GeneratedPadViaRetry;
pub(crate) use generated::editable as editable_generated_tracks;

const MAX_RELOCATIONS: usize = 8;
const MAX_ADDITIONS: usize = 128;
const MAX_OFFSET_MM: f64 = 0.25;
const NATIVE_DEADLINE: Duration = Duration::from_secs(150);
/// Source reports, fill and candidate reports, bounded width replay and
/// both connectivity partitions in each of the two native modes.
pub(crate) const GENERATED_PAD_VIA_CHECKS: usize = 13;
pub(crate) const MIN_GENERATED_PAD_VIA_SEARCH: f64 = 4.0;

type Point = [f64; 2];

#[derive(Clone, Copy, PartialEq)]
enum Geometry<'a> {
    Segment {
        ends: [Point; 2],
        layer: &'a str,
        width: f64,
    },
    Via {
        at: Point,
        diameter: f64,
        drill: f64,
    },
}

#[derive(Clone, Copy)]
struct Copper<'a> {
    net: &'a str,
    geometry: Geometry<'a>,
}

struct Relocation {
    pcb: Expr,
    changed: Vec<String>,
    target_additions: Vec<Expr>,
    other_additions: BTreeMap<String, String>,
    moved_filled_pad_via: Option<String>,
}

struct Materialized {
    text: String,
    fill: Option<FillPublication>,
}

#[derive(Serialize)]
pub struct RelocatedOpenRepair {
    pub status: &'static str,
    pub net: String,
    pub reason: Option<String>,
    pub proposed_relocations: Vec<String>,
    pub proposed_additions: usize,
    pub added_copper: usize,
    pub stored_opens_before: usize,
    pub stored_opens_after: usize,
    pub refilled_opens_before: usize,
    pub refilled_opens_after: usize,
    pub stored_connectivity: Option<OpenConnectivityProof>,
    pub refilled_connectivity: Option<OpenConnectivityProof>,
    #[serde(skip_serializing_if = "Option::is_none")]
    automatic_search: Option<generated::AutomaticSearch>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    contact_refinement: Vec<TrackContact>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resolved_via_centering_warnings: Option<[usize; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reproduced_width_warning: Option<[bool; 2]>,
    pub schematic_parity_checked: bool,
    pub native_checks: usize,
    pub search_work_seconds: f64,
    pub searches: u64,
    pub expanded_nodes: u64,
    pub wall_seconds: f64,
    #[serde(skip)]
    pub(crate) fill_publication: Option<FillPublication>,
}

/// Every native operation and every local route uses the same reservation.
/// Work remains available to the caller even when a private trial fails.
#[derive(Default)]
pub(crate) struct RelocationBudget {
    available: Option<f64>,
    deadline: Option<Instant>,
    check_seconds: f64,
    native_checks: Cell<usize>,
    branch: RefCell<GeneratedBranchWork>,
}

impl RelocationBudget {
    pub(crate) fn for_layout(
        available: f64,
        check_seconds: f64,
        deadline: Option<Instant>,
    ) -> Self {
        Self {
            available: Some(available),
            deadline,
            check_seconds,
            branch: RefCell::new(GeneratedBranchWork {
                limit_seconds: Some(available - check_seconds * GENERATED_PAD_VIA_CHECKS as f64),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    pub(crate) fn native_checks(&self) -> usize {
        self.native_checks.get()
    }

    pub(crate) fn search_work(&self) -> f64 {
        self.branch.borrow().work_seconds
    }

    fn charge_native(&self) -> Result<(), String> {
        let calls = self.native_checks.get() + 1;
        let work = calls as f64 * self.check_seconds + self.search_work();
        if self.available.is_some_and(|available| work > available) {
            return Err("insufficient remaining work for native relocation verification".into());
        }
        self.native_checks.set(calls);
        Ok(())
    }
}

fn invalid(message: impl Into<String>) -> String {
    crate::input_error(message.into())
}

fn route(item: &Expr) -> bool {
    matches!(item.head(), Some("segment" | "arc" | "via"))
}

fn uuid(item: &Expr) -> Result<String, String> {
    footprint_uuid(item)
        .filter(|id| !id.is_empty())
        .map(|id| id.to_ascii_lowercase())
        .ok_or_else(|| invalid("all source and candidate tracks and vias need stable KiCad UUIDs"))
}

fn index(pcb: &Expr) -> Result<BTreeMap<String, &Expr>, String> {
    let mut ids = BTreeMap::new();
    for item in pcb.children().iter().filter(|item| route(item)) {
        if ids.insert(uuid(item)?, item).is_some() {
            return Err(invalid("ambiguous copper UUID in source or candidate"));
        }
    }
    Ok(ids)
}

fn ordinary<'a>(item: &'a Expr, codes: &'a BTreeMap<String, String>) -> Result<Copper<'a>, String> {
    let fail =
        || invalid("relocation requires ordinary unlocked straight segments and through vias");
    let (fields, geometry) = match item.head() {
        Some("segment") => {
            let ends = [form_xy(item, "start")?, form_xy(item, "end")?];
            let width = form_f64(item, "width", 1)?;
            let layer = form_atom(item, "layer", 1).ok_or_else(fail)?;
            if width <= 0.0 || ends[0] == ends[1] {
                return Err(fail());
            }
            (
                &["start", "end", "width", "layer", "net"][..],
                Geometry::Segment { ends, layer, width },
            )
        }
        Some("via") => {
            let diameter = form_f64(item, "size", 1)?;
            let drill = form_f64(item, "drill", 1)?;
            let through = form_atom(item, "layers", 1) == Some("F.Cu")
                && form_atom(item, "layers", 2) == Some("B.Cu");
            if !through || diameter <= drill || drill <= 0.0 {
                return Err(fail());
            }
            (
                &["at", "size", "drill", "layers", "net"][..],
                Geometry::Via {
                    at: form_xy(item, "at")?,
                    diameter,
                    drill,
                },
            )
        }
        _ => return Err(fail()),
    };
    let identity = if item.child("uuid").is_some() {
        "uuid"
    } else {
        "tstamp"
    };
    if item.children().len() != fields.len() + 2
        || fields.iter().chain([&identity]).any(|field| {
            let values = match *field {
                "start" | "end" | "at" | "layers" => 2,
                _ => 1,
            };
            item.child(field)
                .is_none_or(|child| child.children().len() != values + 1)
        })
    {
        return Err(fail());
    }
    let net = item_net(item, codes)
        .filter(|net| !net.is_empty())
        .ok_or_else(fail)?;
    Ok(Copper { net, geometry })
}

fn near_segment(point: Point, ends: [Point; 2]) -> bool {
    point_segment_distance_squared(point, ends[0], ends[1]) <= MAX_OFFSET_MM.powi(2)
}

fn within_relocation(old: Copper<'_>, new: Copper<'_>) -> bool {
    if old.net != new.net {
        return false;
    }
    match (old.geometry, new.geometry) {
        (
            Geometry::Segment {
                ends: original,
                layer,
                width,
            },
            Geometry::Segment {
                ends,
                layer: next_layer,
                width: next_width,
            },
        ) => {
            layer == next_layer
                && width == next_width
                && ends.iter().all(|&p| near_segment(p, original))
        }
        (
            Geometry::Via {
                at: original,
                diameter,
                drill,
            },
            Geometry::Via {
                at,
                diameter: next_diameter,
                drill: next_drill,
            },
        ) => {
            diameter == next_diameter
                && drill == next_drill
                && point_segment_distance_squared(at, original, original) <= MAX_OFFSET_MM.powi(2)
        }
        _ => false,
    }
}

/// A bend shared by exactly two relocated segments may move with both.
/// Their outer endpoints still go through the ordinary anchor checks.
fn moved_bend(
    endpoint: Point,
    net: &str,
    layer: &str,
    width: f64,
    changed: &[(Copper<'_>, Copper<'_>)],
) -> bool {
    let relocated = |&(old, new): &(Copper<'_>, Copper<'_>)| {
        let Geometry::Segment {
            ends: [a, b],
            layer: old_layer,
            width: old_width,
        } = old.geometry
        else {
            return None;
        };
        let Geometry::Segment {
            ends: [mut x, mut y],
            ..
        } = new.geometry
        else {
            return None;
        };
        if old.net != net || old_layer != layer || old_width != width || ![a, b].contains(&endpoint)
        {
            return None;
        }
        let distance = |a: Point, b: Point| point_segment_distance_squared(a, b, b);
        let direct = distance(a, x) + distance(b, y);
        let reversed = distance(a, y) + distance(b, x);
        if (direct - reversed).abs() < 1.0e-12 {
            return Some(None);
        }
        if reversed < direct {
            std::mem::swap(&mut x, &mut y);
        }
        Some(Some(if endpoint == a { x } else { y }))
    };
    let mut incident = changed.iter().filter_map(relocated);
    matches!((incident.next(), incident.next(), incident.next()),
        (Some(Some(a)), Some(Some(b)), None) if a == b)
}

fn endpoints_preserved(changed: &[(Copper<'_>, Copper<'_>)], supporting: &[Copper<'_>]) -> bool {
    for &(old, _) in changed {
        let Geometry::Segment { ends, layer, width } = old.geometry else {
            continue;
        };
        for endpoint in ends {
            let kept = supporting.iter().any(|piece| {
                piece.net == old.net
                    && matches!(piece.geometry, Geometry::Segment {
                        ends, layer: candidate_layer, width: candidate_width,
                    } if candidate_layer == layer && candidate_width == width && ends.contains(&endpoint))
            });
            let moves_with_via = changed.iter().any(|(via, replacement)| {
                via.net == old.net
                    && matches!((via.geometry, replacement.geometry), (
                        Geometry::Via { at, diameter, .. }, Geometry::Via { at: next, .. }
                    ) if point_segment_distance_squared(endpoint, at, at) <= ((diameter + width) / 2.0).powi(2)
                       && supporting.iter().any(|piece| piece.net == old.net &&
                          matches!(piece.geometry, Geometry::Segment { ends, layer: next_layer, width: next_width }
                              if layer == next_layer && width == next_width && ends.contains(&next))))
            });
            if !kept && !moves_with_via && !moved_bend(endpoint, old.net, layer, width, changed) {
                return false;
            }
        }
    }
    true
}

fn original_net(item: &Expr, net: &str, codes: &BTreeMap<String, String>) -> Expr {
    let reference = codes
        .iter()
        .find(|(_, name)| name.as_str() == net)
        .map_or_else(|| format!("\"{net}\""), |(code, _)| code.clone());
    let mut fields = item.children().to_vec();
    if let Some(net) = fields.iter_mut().find(|part| part.head() == Some("net")) {
        *net = Expr::List(vec![Expr::Atom("net".into()), Expr::Atom(reference)]);
    }
    Expr::List(fields)
}

fn plan(original: &Expr, candidate: &Expr, target: &str) -> Result<Relocation, String> {
    if original.head() != Some("kicad_pcb")
        || candidate.head() != Some("kicad_pcb")
        || board_statistics(original)?.component_placement_sha256
            != board_statistics(candidate)?.component_placement_sha256
    {
        return Err(invalid(
            "relocation candidate must use the source footprint placement",
        ));
    }
    let before = index(original)?;
    let after = index(candidate)?;
    let before_codes = net_codes(original);
    let after_codes = net_codes(candidate);
    let mut replacements = BTreeMap::new();
    let mut changes = Vec::new();
    for (id, &old) in &before {
        let &new = after
            .get(id)
            .ok_or_else(|| invalid(format!("candidate removed {id}")))?;
        if old == new {
            continue;
        }
        let prior = ordinary(old, &before_codes)?;
        let next = ordinary(new, &after_codes)?;
        if !within_relocation(prior, next) {
            return Err(invalid(format!(
                "{id} changes net, layers or dimensions, or moves more than {MAX_OFFSET_MM} mm"
            )));
        }
        if prior.geometry != next.geometry {
            replacements.insert(id.clone(), original_net(new, next.net, &before_codes));
            changes.push((prior, next));
        }
    }
    if changes.is_empty()
        || changes.len() > MAX_RELOCATIONS
        || after.len() - before.len() > MAX_ADDITIONS
    {
        return Err(invalid(format!(
            "relocation needs 1–{MAX_RELOCATIONS} changed original items and at most {MAX_ADDITIONS} added items"
        )));
    }
    let mut target_additions = Vec::new();
    let mut other_additions = BTreeMap::new();
    let mut additional_items = Vec::new();
    let mut supporting: Vec<_> = changes
        .iter()
        .map(|(_, replacement)| *replacement)
        .collect();
    for (id, &item) in &after {
        if before.contains_key(id) {
            continue;
        }
        let added = ordinary(item, &after_codes)?;
        let source_item = original_net(item, added.net, &before_codes);
        if added.net == target {
            target_additions.push(source_item.clone());
        } else {
            if !matches!(added.geometry, Geometry::Segment { .. })
                || !changes
                    .iter()
                    .any(|(original, _)| within_relocation(*original, added))
            {
                return Err(invalid(format!(
                    "added {id} outside the target net's route or an original segment's relocation"
                )));
            }
            other_additions.insert(id.clone(), added.net.to_owned());
        }
        supporting.push(added);
        additional_items.push(source_item);
    }
    if target_additions.is_empty() || !endpoints_preserved(&changes, &supporting) {
        return Err(invalid(
            "candidate must add target copper and preserve the original segment anchors and junctions",
        ));
    }
    let pcb = original
        .children()
        .iter()
        .map(|item| {
            footprint_uuid(item)
                .and_then(|id| replacements.get(&id.to_ascii_lowercase()))
                .filter(|_| route(item))
                .unwrap_or(item)
                .clone()
        })
        .chain(additional_items)
        .collect();
    Ok(Relocation {
        pcb: Expr::List(pcb),
        changed: replacements.into_keys().collect(),
        target_additions,
        other_additions,
        moved_filled_pad_via: None,
    })
}

fn read(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))
}

fn open_count(report: &Value) -> Result<usize, String> {
    report["unconnected_items"]
        .as_array()
        .map(Vec::len)
        .ok_or("native report is missing its list of opens".into())
}

/// Why KiCad's `reports` are no evidence for a relocation: a finding type
/// that copper can change is at KiCad's cap, so a check may hide a finding
/// the relocation causes. The library and text findings at their cap are
/// none (`COPPER_INDEPENDENT_TYPES`, as in the open repair: `report_capped`).
pub(crate) fn native_evidence_limit(reports: &[&Value]) -> Option<String> {
    reports.iter().any(|report| {
        report_capped(report) || !finding_types_at_cap(report, "schematic_parity").is_empty()
    }).then(|| "KiCad's native findings reached the report cap. Review the native DRC before retrying relocation.".into())
}

/// Centering warnings can disappear when the moved generated via gets a
/// junction lead. Every surviving warning and all hard findings stay exact.
fn without_resolved_via_warnings<'a>(
    before: &'a Value,
    after: &Value,
    moved_via: Option<&str>,
) -> (std::borrow::Cow<'a, Value>, usize) {
    let (Some(id), Some(trial)) = (moved_via, after["violations"].as_array()) else {
        return (std::borrow::Cow::Borrowed(before), 0);
    };
    if report_capped(before) {
        return (std::borrow::Cow::Borrowed(before), 0);
    }
    let mut comparison = before.clone();
    let Some(findings) = comparison["violations"].as_array_mut() else {
        return (std::borrow::Cow::Borrowed(before), 0);
    };
    let start = findings.len();
    findings.retain(|finding| {
        finding["type"] != "track_not_centered_on_via"
            || finding["severity"] != "warning"
            || finding["excluded"] == true
            || !finding["items"].as_array().is_some_and(|items| {
                items.iter().any(|item| {
                    item["uuid"]
                        .as_str()
                        .is_some_and(|uuid| uuid.eq_ignore_ascii_case(id))
                })
            })
            || trial.contains(finding)
    });
    let resolved = start - findings.len();
    (std::borrow::Cow::Owned(comparison), resolved)
}

fn prove_relocation(
    before: Vec<(String, Vec<String>)>,
    mut after: Vec<(String, Vec<String>)>,
    target: &str,
    reduction: usize,
    relocation: &Relocation,
) -> Result<OpenConnectivityProof, String> {
    let mut auxiliaries = relocation.other_additions.clone();
    if before
        .iter()
        .flat_map(|(_, members)| members)
        .any(|id| auxiliaries.contains_key(id))
    {
        return Err("supporting copper duplicates an existing native item".into());
    }
    // Trace pieces replacing a shortened original may extend its native
    // component. Remove exactly these new IDs before proving the old partition.
    for (net, members) in &mut after {
        let mut retained = Vec::with_capacity(members.len());
        for id in std::mem::take(members) {
            match auxiliaries.remove(&id) {
                Some(expected) if *net != expected => {
                    return Err("supporting copper changed net".into());
                }
                Some(_) => {}
                None => retained.push(id),
            }
        }
        if retained.is_empty() {
            return Err("supporting copper formed an isolated component".into());
        }
        *members = retained;
    }
    if !auxiliaries.is_empty() {
        return Err("the native candidate is missing supporting copper identities".into());
    }
    let mut proof = verify_native_partition_merges(
        before,
        after,
        &[target.to_owned()],
        reduction,
        &relocation.target_additions,
    )?;
    proof.added_copper_objects += relocation.other_additions.len();
    Ok(proof)
}

/// Checks an explicit copper edit and publishes it only when proved to close
/// an open on the requested net without changing other native components.
pub fn repair_kicad_relocated_open(
    source: &Path,
    board_id: &str,
    candidate: &Path,
    output: &Path,
    net: &str,
) -> Result<RelocatedOpenRepair, String> {
    let donor = parse(&read(candidate).map_err(invalid)?).map_err(invalid)?;
    repair_relocation(
        source,
        board_id,
        output,
        net,
        Input::Candidate(donor),
        &RelocationBudget::default(),
    )
}

/// Search editable generated bends, relative to the pre-route placed PCB.
pub fn repair_kicad_generated_open(
    source: &Path,
    board_id: &str,
    placed: &Path,
    output: &Path,
    net: &str,
) -> Result<RelocatedOpenRepair, String> {
    repair_generated(
        source,
        board_id,
        placed,
        output,
        net,
        &RelocationBudget::default(),
    )
}

pub(crate) fn repair_generated(
    source: &Path,
    board_id: &str,
    placed: &Path,
    output: &Path,
    net: &str,
    budget: &RelocationBudget,
) -> Result<RelocatedOpenRepair, String> {
    check_board_id(board_id).map_err(invalid)?;
    same_routing_rules(source, placed, board_id)?;
    let placed = parse(&read(&placed.join(format!("{board_id}.kicad_pcb"))).map_err(invalid)?)
        .map_err(invalid)?;
    repair_relocation(
        source,
        board_id,
        output,
        net,
        Input::Generated(placed),
        budget,
    )
}

pub(crate) fn same_routing_rules(
    source: &Path,
    placed: &Path,
    board_id: &str,
) -> Result<(), String> {
    for extension in ["kicad_pro", "kicad_dru"] {
        let file = format!("{board_id}.{extension}");
        let a = source.join(&file);
        let b = placed.join(&file);
        if a.is_file() != b.is_file() || (a.is_file() && read(&a)? != read(&b)?) {
            return Err(invalid(format!(
                "the placed and routed projects differ in {file}"
            )));
        }
    }
    Ok(())
}

/// Read the layout's native opens and original placed board to suggest the
/// explicit generated-copper repair. Routing and acceptance still happen
/// only when the suggested command is run.
pub fn suggest_generated_pad_via_repairs(
    source: &Path,
    board_id: &str,
    placed: &Path,
    output: &Path,
) -> Vec<GeneratedPadViaRetry> {
    let read_candidates = || -> Option<Vec<_>> {
        check_board_id(board_id).ok()?;
        let native = crate::read_json(&source.join("drc.json")).ok()?;
        if !crate::run_diagnosis::native_report_matches_board(&native, board_id)
            || native["unconnected_items"].as_array()?.is_empty()
        {
            return None;
        }
        same_routing_rules(source, placed, board_id).ok()?;
        let file = format!("{board_id}.kicad_pcb");
        let result = parse(&read(&source.join(&file)).ok()?).ok()?;
        let placement = parse(&read(&placed.join(file)).ok()?).ok()?;
        let mut candidates = generated::pad_via_retries(
            &result, &placement, &native, source, board_id, placed, output,
        );
        let limitation = native_evidence_limit(&[&native]);
        for candidate in &mut candidates {
            candidate.native_evidence_limit.clone_from(&limitation);
        }
        Some(candidates)
    };
    read_candidates().unwrap_or_default()
}

enum Input {
    Candidate(Expr),
    Generated(Expr),
}

fn repair_relocation(
    source: &Path,
    board_id: &str,
    output: &Path,
    net: &str,
    input: Input,
    budget: &RelocationBudget,
) -> Result<RelocatedOpenRepair, String> {
    let started = Instant::now();
    check_board_id(board_id).map_err(invalid)?;
    in_repair_copy(source, output, |stage| {
        let filename = format!("{board_id}.kicad_pcb");
        let incumbent = stage.join(&filename);
        let original = read(&incumbent)?;
        let source_board = parse(&original)?;
        let explicit = match &input {
            Input::Candidate(donor) => Some(plan(&source_board, donor, net)?),
            Input::Generated(_) => None,
        };
        let project = project_inputs(stage)?;
        let parity = incumbent.with_extension("kicad_sch").is_file()
            || incumbent.with_extension("sch").is_file();
        let deadline = started + NATIVE_DEADLINE;
        let deadline = budget
            .deadline
            .map_or(deadline, |other| other.min(deadline));
        let expired = || Instant::now() >= deadline;
        let work = temporary_next_to(stage);
        let width_source = temporary_next_to(stage);
        let check = |board: &Path, name: &str, refill| {
            budget.charge_native()?;
            native_report::check_kept_route_input(board, &stage.join(name), parity, refill, expired)
        };
        native_report::remove_stale_board_run_reports(stage)?;
        let before_stored = check(&incumbent, "drc-before-relocation-stored.json", false)?;
        let before_refilled = check(&incumbent, "drc-before-relocation-refilled.json", true)?;
        if read(&incumbent)? != original || project_inputs(stage)? != project {
            return Err("native validation changed the source PCB or project".into());
        }
        let capped = native_evidence_limit(&[&before_stored, &before_refilled]);
        let (plans, automatic_search) = match input {
            _ if capped.is_some() => (Vec::new(), None),
            Input::Candidate(_) => (explicit.into_iter().collect(), None),
            Input::Generated(placed) => {
                let (plans, report) = generated::propose(
                    &source_board,
                    &placed,
                    &incumbent,
                    &before_refilled,
                    net,
                    generated::SearchMode {
                        deadline,
                        pad_via_only: budget.available.is_some(),
                        work: &mut budget.branch.borrow_mut(),
                    },
                )?;
                (plans, Some(report))
            }
        };
        let mut report = RelocatedOpenRepair {
            status: "unchanged",
            net: net.into(),
            reason: capped.or_else(|| {
                plans.is_empty().then(|| {
                    "no compatible generated copper relocation in the bounded search".into()
                })
            }),
            proposed_relocations: Vec::new(),
            proposed_additions: 0,
            added_copper: 0,
            stored_opens_before: open_count(&before_stored)?,
            stored_opens_after: open_count(&before_stored)?,
            refilled_opens_before: open_count(&before_refilled)?,
            refilled_opens_after: open_count(&before_refilled)?,
            stored_connectivity: None,
            refilled_connectivity: None,
            automatic_search,
            contact_refinement: Vec::new(),
            resolved_via_centering_warnings: None,
            reproduced_width_warning: None,
            schematic_parity_checked: parity,
            native_checks: 0,
            search_work_seconds: 0.0,
            searches: 0,
            expanded_nodes: 0,
            wall_seconds: 0.0,
            fill_publication: None,
        };
        let attempt = (|| -> Result<(), String> {
            if plans.is_empty() {
                return Ok(());
            }
            copy_directory_tree(stage, &work)?;
            let trial = work.join(&filename);
            let materialize = |relocation: &mut Relocation| -> Result<Materialized, String> {
                let started_fill = Instant::now();
                let text = encode(&relocation.pcb);
                write_atomically(&trial, text.as_bytes())?;
                if has_fillable_zone(&relocation.pcb) {
                    budget.charge_native()?;
                    native_report::save_kept_route_fill(
                        &trial,
                        &work.join("relocation-fill.json"),
                        parity,
                        expired,
                    )?;
                    // Accept KiCad's fill payload on the original project objects only.
                    let (filled_zones, filled_polygons) =
                        copy_fill_payload(&mut relocation.pcb, &parse(&read(&trial)?)?)?;
                    let saved = encode(&relocation.pcb);
                    write_atomically(&trial, saved.as_bytes())?;
                    return Ok(Materialized {
                        fill: Some(FillPublication {
                            status: "saved",
                            source_board_sha256: Some(format!(
                                "{:x}",
                                Sha256::digest(text.as_bytes())
                            )),
                            saved_board_sha256: Some(format!(
                                "{:x}",
                                Sha256::digest(saved.as_bytes())
                            )),
                            filled_zones,
                            filled_polygons,
                            native_checks: 3,
                            // Included in the relocation's check tally.
                            native_checks_charged: 3,
                            connectivity_checks: 4,
                            refill_source: Some("verified_relocation"),
                            wall_seconds: started_fill.elapsed().as_secs_f64(),
                            ..FillPublication::default()
                        }),
                        text: saved,
                    });
                }
                Ok(Materialized { text, fill: None })
            };
            for mut relocation in plans {
                report.proposed_relocations.clone_from(&relocation.changed);
                report.proposed_additions =
                    relocation.target_additions.len() + relocation.other_additions.len();
                let mut materialized = materialize(&mut relocation)?;
                let mut after_stored = check(&trial, "drc-relocation-trial-stored.json", false)?;
                let mut after_refilled = check(&trial, "drc-relocation-trial-refilled.json", true)?;
                report.contact_refinement.clear();
                if report.automatic_search.is_some()
                    && relocation.moved_filled_pad_via.is_none()
                    && open_count(&after_refilled)? < open_count(&before_refilled)?
                    && let Some(refined) = refine_generated_contact(
                        &source_board,
                        &relocation.target_additions,
                        net,
                        &before_refilled,
                        &after_refilled,
                    )?
                {
                    let old = relocation
                        .target_additions
                        .iter()
                        .map(uuid)
                        .collect::<Result<std::collections::BTreeSet<_>, _>>()?;
                    let mut candidate = relocation.pcb.clone();
                    let Expr::List(items) = &mut candidate else {
                        unreachable!()
                    };
                    items.retain(|item| {
                        !route(item) || !uuid(item).is_ok_and(|id| old.contains(&id))
                    });
                    items.extend(refined.objects);
                    relocation = plan(&source_board, &candidate, net)?;
                    materialized = materialize(&mut relocation)?;
                    report.proposed_additions =
                        relocation.target_additions.len() + relocation.other_additions.len();
                    report.contact_refinement = refined.contacts;
                    after_stored = check(&trial, "drc-relocation-trial-stored.json", false)?;
                    after_refilled = check(&trial, "drc-relocation-trial-refilled.json", true)?;
                }
                let prove = |before: &Value, after: &Value, refill| {
                    if let Some(limit) = native_evidence_limit(&[after]) {
                        return Err(limit);
                    }
                    let reduction = open_count(before)?.saturating_sub(open_count(after)?);
                    let (comparison, warnings) = without_resolved_via_warnings(
                        before,
                        after,
                        relocation.moved_filled_pad_via.as_deref(),
                    );
                    let mut reproduced = false;
                    let strict_match =
                        reduction > 0 && eligible_open_partition(&comparison, after)?;
                    if reduction > 0
                        && !strict_match
                        && let Some(moved) = &relocation.moved_filled_pad_via
                    {
                        reproduced = width_witness::Proof {
                            source: &source_board,
                            source_project: stage,
                            filename: &filename,
                            scratch: &width_source,
                            target: net,
                            moved,
                            try_third_order: false,
                        }
                        .replay(
                            &relocation.pcb,
                            before,
                            &comparison,
                            after,
                            refill,
                            check,
                        )?;
                    }
                    if !strict_match && !reproduced {
                        return Err(
                            "the trial did not reduce opens with the same native findings".into(),
                        );
                    }
                    let version = before["kicad_version"]
                        .as_str()
                        .ok_or("missing KiCad version")?;
                    budget.charge_native()?;
                    let a = native_report::connectivity_partition(
                        &incumbent, version, refill, expired,
                    )?;
                    budget.charge_native()?;
                    let b =
                        native_report::connectivity_partition(&trial, version, refill, expired)?;
                    Ok((
                        prove_relocation(a, b, net, reduction, &relocation)?,
                        warnings,
                        reproduced,
                    ))
                };
                let (
                    (stored, stored_warnings, stored_width),
                    (refilled, refilled_warnings, refilled_width),
                ) = match prove(&before_stored, &after_stored, false).and_then(|stored| {
                    Ok((stored, prove(&before_refilled, &after_refilled, true)?))
                }) {
                    Ok(proofs) => proofs,
                    Err(reason) => {
                        report.reason = Some(reason);
                        continue;
                    }
                };
                if expired() {
                    return Err("the repair deadline expired during native verification".into());
                }
                if read(&incumbent)? != original || read(&trial)? != materialized.text {
                    return Err("native verification changed a PCB under test".into());
                }
                if project_inputs(stage)? != project || project_inputs(&work)? != project {
                    return Err("native verification changed the project inputs".into());
                }
                write_atomically(&incumbent, materialized.text.as_bytes())?;
                write_typed_json(&stage.join("drc.json"), &after_stored)?;
                write_typed_json(&stage.join("drc-refilled.json"), &after_refilled)?;
                if let Some(fill) = &mut materialized.fill {
                    let mut counts = after_stored.clone();
                    if !parity && counts.get("schematic_parity").is_none() {
                        counts["schematic_parity"] = serde_json::json!([]);
                    }
                    fill.saved_as_stored = Some(crate::fill_handoff::counts(&counts));
                    write_typed_json(&stage.join("drc-saved-fill.json"), &after_stored)?;
                }
                report.fill_publication = materialized.fill;
                report.status = "improved";
                report.reason = None;
                report.added_copper = report.proposed_additions;
                report.stored_opens_after = open_count(&after_stored)?;
                report.refilled_opens_after = open_count(&after_refilled)?;
                report.stored_connectivity = Some(stored);
                report.refilled_connectivity = Some(refilled);
                if relocation.moved_filled_pad_via.is_some() {
                    report.resolved_via_centering_warnings =
                        Some([stored_warnings, refilled_warnings]);
                }
                if stored_width || refilled_width {
                    report.reproduced_width_warning = Some([stored_width, refilled_width]);
                }
                break;
            }
            Ok(())
        })();
        for scratch in [&work, &width_source] {
            if scratch.is_dir() {
                fs::remove_dir_all(scratch).map_err(|error| error.to_string())?;
            }
        }
        attempt?;
        if report.status == "unchanged" {
            write_typed_json(&stage.join("drc.json"), &before_stored)?;
            write_typed_json(&stage.join("drc-refilled.json"), &before_refilled)?;
        }
        report.wall_seconds = started.elapsed().as_secs_f64();
        report.native_checks = budget.native_checks();
        let branch = budget.branch.borrow();
        report.search_work_seconds = branch.work_seconds;
        report.searches = branch.searches;
        report.expanded_nodes = branch.expanded_nodes;
        write_typed_json(&stage.join("relocated-open-repair.json"), &report)?;
        Ok(report)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_native_charge_cannot_spend_reserved_work_twice() {
        let budget = RelocationBudget::for_layout(45.0, 3.0, None);
        budget.branch.borrow_mut().work_seconds = 4.0;
        for _ in 0..GENERATED_PAD_VIA_CHECKS {
            budget.charge_native().unwrap();
        }
        assert!(budget.charge_native().is_err());
        assert_eq!(budget.native_checks(), GENERATED_PAD_VIA_CHECKS);
    }

    #[test]
    fn only_a_capped_finding_copper_can_change_limits_the_evidence() {
        let capped = |kind: &str| {
            serde_json::json!({
                "violations": vec![serde_json::json!({"type": kind}); 199],
                "unconnected_items": [],
            })
        };
        // The library and text findings at their cap hide nothing a
        // relocation could cause.
        for kind in [
            "lib_footprint_issues",
            "lib_footprint_mismatch",
            "text_height",
            "text_thickness",
        ] {
            assert_eq!(native_evidence_limit(&[&capped(kind)]), None, "{kind}");
        }
        let control = serde_json::json!({"violations": [], "unconnected_items": []});
        assert_eq!(native_evidence_limit(&[&control]), None);
        assert!(
            native_evidence_limit(&[&control, &capped("solder_mask_bridge")])
                .unwrap()
                .contains("report cap")
        );
    }

    #[test]
    fn only_disappearing_active_centering_warnings_at_the_moved_via_can_be_removed() {
        let warning = |kind, severity, uuid| {
            serde_json::json!({
                "type": kind, "severity": severity, "items": [{"uuid": uuid}]
            })
        };
        let prior = warning("track_not_centered_on_via", "warning", "via");
        let unrelated = warning("track_not_centered_on_via", "warning", "other");
        let hard = warning("track_not_centered_on_via", "error", "via");
        let other_kind = warning("connection_width", "warning", "via");
        let before = serde_json::json!({"violations": [prior, unrelated, hard, other_kind]});
        let after = serde_json::json!({"violations": []});
        let (normalized, count) = without_resolved_via_warnings(&before, &after, Some("via"));
        assert_eq!(count, 1);
        assert_eq!(
            normalized["violations"],
            serde_json::json!([unrelated, hard, other_kind])
        );
        let (_, unchanged) = without_resolved_via_warnings(&before, &before, Some("via"));
        assert_eq!(unchanged, 0);
    }

    #[test]
    fn moved_trace_bends_must_keep_the_same_joint() {
        let piece = |a, b| Copper {
            net: "CLK",
            geometry: Geometry::Segment {
                ends: [a, b],
                layer: "F.Cu",
                width: 0.127,
            },
        };
        let [left, bend, right] = [[1.0, 1.0], [1.5, 1.5], [2.0, 1.5]];
        let moved = [1.5, 1.45];
        let first = (piece(left, bend), piece(left, moved));
        let other = (piece(bend, right), piece(right, moved));
        assert!(endpoints_preserved(&[first, other], &[first.1, other.1]));
        let split = (other.0, piece(right, [1.5, 1.44]));
        assert!(!endpoints_preserved(&[first, split], &[first.1, split.1]));
        let moved_left = (first.0, piece([1.0, 1.05], moved));
        assert!(!endpoints_preserved(
            &[moved_left, other],
            &[moved_left.1, other.1]
        ));
    }

    #[test]
    fn supporting_trace_must_extend_an_existing_component_without_merging_others() {
        let make = |components: &[(&str, &[&str])]| {
            components
                .iter()
                .map(|(net, items)| {
                    (
                        net.to_string(),
                        items.iter().map(ToString::to_string).collect(),
                    )
                })
                .collect::<Vec<_>>()
        };
        let before = make(&[
            ("DATA", &["d1"]),
            ("DATA", &["d2"]),
            ("CLK", &["c1", "c2"]),
            ("CLK", &["c3"]),
        ]);
        let edit = Relocation {
            pcb: parse("(kicad_pcb)").unwrap(),
            changed: vec!["c1".into()],
            target_additions: vec![parse("(segment (uuid d3))").unwrap()],
            other_additions: BTreeMap::from([("c4".into(), "CLK".into())]),
            moved_filled_pad_via: None,
        };
        let data = ("DATA", &["d1", "d2", "d3"][..]);
        let accepted = make(&[data, ("CLK", &["c1", "c2", "c4"]), ("CLK", &["c3"])]);
        assert_eq!(
            prove_relocation(before.clone(), accepted, "DATA", 1, &edit)
                .unwrap()
                .added_copper_objects,
            2
        );
        for rejected in [
            make(&[data, ("CLK", &["c1", "c2", "c3", "c4"])]),
            make(&[
                data,
                ("CLK", &["c1", "c2"]),
                ("CLK", &["c3"]),
                ("CLK", &["c4"]),
            ]),
            make(&[data, ("CLK", &["c1", "c3", "c4"]), ("CLK", &["c2"])]),
        ] {
            assert!(prove_relocation(before.clone(), rejected, "DATA", 1, &edit).is_err());
        }
    }
}
