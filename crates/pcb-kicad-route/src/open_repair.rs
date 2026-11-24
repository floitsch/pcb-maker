// Copyright (C) 2026 Florian Loitsch.

//! Verify additive copper on the original project.

use super::*;
use pcb_kicad_file::kicad_sexpr::{form_xy, unescape};
use pcb_kicad_file::outline_pieces::{
    ACROSS_KEY, CAPPED_KEY, NativePanelCounts, NativePanelScope, Pieces,
};
use serde_json::Value;
use std::collections::VecDeque;

mod connectivity;
mod contact;
mod local;
mod pad_bridge;
mod paired;
mod panel_witness;
mod plane_escape;
mod plane_landing;
mod silk_cap;
mod teardrop;
mod terminal_launch;
mod width_replay;
mod zone_open;
pub use connectivity::OpenConnectivityProof;
pub use connectivity::{eligible_open_partition, verify_native_partition_merges};
pub use contact::{ContactRefinement, TrackContact, refine_generated_contact};
pub use local::{
    GeneratedBranchWork, LocalSearch, lower_for_local, propose_direct_pad_path,
    propose_generated_branch, same_as_standalone,
};
pub use pad_bridge::SameFootprintPadBridge;
pub use panel_witness::PanelWitnessReplacement;
pub(crate) use plane_escape::legacy_escape_segment;
pub use plane_escape::{
    PlaneEscapeRepair, ProtectedEscape, has_plane_escape_open, repair_kicad_plane_escape,
};
use silk_cap::{ImplicitSilkRules, SilkCap};
pub(crate) use terminal_launch::plan_row_escapes;
pub use terminal_launch::{
    PreparedRowEscape, PreparedRowEscapeAdvice, TerminalLaunchAdvice, TerminalLaunchRetry,
    advise_terminal_launches, prepared_row_escape_advice, retry_terminal_launches,
    valid_terminal_launch_pads,
};
pub use width_replay::OpenWidthReplay;
pub use zone_open::{ZoneOpenPad, locate_zone_open_pads};

const MAX_TRIALS: usize = 4;
const MAX_NET_ITEMS: usize = 256;
pub(crate) const KICAD_UNCONNECTED_CAP: usize = 499;
pub(crate) const KICAD_FINDING_TYPE_FLOOR: usize = 199;
const FINDING_KEYS: [&str; 4] = [
    "violations",
    "schematic_parity",
    "unconnected_items",
    ACROSS_KEY,
];
const PERSIST_COPPER_IDS: &str = "copy the source project to a fresh directory, run `kicad-cli pcb upgrade --force <copied-board.kicad_pcb>` on that copy, and retry the same repair with the copied project";

/// Local search and fresh native checks charged by the run that requested
/// the repair. An automatic repair tries the open nets one after the other,
/// the shortest native witness first, while the work covers a search and a
/// check for each (at most `MAX_TRIALS`). It charges the baseline,
/// candidate and any follow-up proof checks to the available work.
pub struct LayoutRepairBudget<'a> {
    pub router: &'a KiCadBoardRouterConfig,
    pub available_seconds: f64,
    pub check_seconds: f64,
    pub native_checks: usize,
    pub search_work_seconds: f64,
    /// Extra saved-fill proof to leave available when trying a branch.
    pub branch_fill_reserve_checks: Option<usize>,
}

impl LayoutRepairBudget<'_> {
    pub fn work_seconds(&self) -> f64 {
        self.native_checks as f64 * self.check_seconds + self.search_work_seconds
    }

    fn search_limit(&self) -> f64 {
        self.search_limit_after(0)
    }

    /// The search work left with `pending` proposals waiting for their
    /// native checks beside the next one's.
    fn search_limit_after(&self, pending: usize) -> f64 {
        let checks_needed = self.native_checks.max(1) + 1 + pending;
        (self.available_seconds
            - self.search_work_seconds
            - checks_needed as f64 * self.check_seconds)
            .min(20.0)
    }

    fn branch_search_limit(&self) -> f64 {
        // The branch still needs its DRC and two native component censuses.
        self.branch_fill_reserve_checks.map_or(0.0, |reserve| {
            (self.available_seconds
                - self.work_seconds()
                - (3 + reserve) as f64 * self.check_seconds)
                .min(20.0)
        })
    }

    fn can_check_branch(&self, checks: usize) -> bool {
        self.branch_fill_reserve_checks
            .is_some_and(|reserve| self.can_check_trials(checks + reserve))
    }

    fn can_check_trial(&self) -> bool {
        self.can_check_trials(1)
    }

    fn can_check_trials(&self, checks: usize) -> bool {
        self.can_check_trials_after(checks, 0.0)
    }

    fn can_check_trials_after(&self, checks: usize, search_work: f64) -> bool {
        self.work_seconds() + search_work + checks as f64 * self.check_seconds
            <= self.available_seconds
            && self
                .router
                .deadline
                .is_none_or(|deadline| Instant::now() < deadline)
    }
}

#[derive(Serialize)]
pub struct OpenRepair {
    pub status: &'static str,
    pub schematic_parity_checked: bool,
    /// Whether KiCad refilled zones in memory for the before and after counts.
    pub drc_refilled_zones: bool,
    /// The project's native DRC omitted copper-to-board-edge checks.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub native_edge_clearance_ignored: bool,
    pub opens_before: usize,
    pub opens_after: usize,
    pub added_copper: usize,
    pub identified_open_nets: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub requested_nets: Vec<String>,
    /// Requested names that cannot select a unique, identified native open.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unmatched_requested_nets: Vec<String>,
    pub trials: Vec<OpenRepairTrial>,
    pub note: Option<&'static str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub capped_schematic_parity_types: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub capped_native_violation_types: Vec<String>,
    /// Library nicknames and warning counts in KiCad's truncated footprint report.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub reported_missing_footprint_libraries: BTreeMap<String, usize>,
    /// Truncated warnings proved invariant across an accepted copper addition.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub verified_invariant_native_caps: Vec<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub panel_before: Option<NativePanelCounts>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub panel_after: Option<NativePanelCounts>,
}

impl OpenRepair {
    /// Every native open participated in the ordinary local search, which
    /// finished its full search schedule without finding any candidate copper.
    pub fn completed_without_local_path(&self) -> bool {
        self.status == "no_improvement"
            && self.added_copper == 0
            && self.opens_before > 0
            && self.opens_before == self.opens_after
            && self.note.is_none()
            && self.requested_nets.is_empty()
            && self.identified_open_nets.len() == self.trials.len()
            && self.trials.iter().all(|trial| {
                trial.status == "no_local_path"
                    && trial.proposed_copper == 0
                    && trial
                        .local_search
                        .as_ref()
                        .is_some_and(|search| search.completed)
            })
            && self
                .trials
                .iter()
                .filter_map(|trial| trial.local_search.as_ref())
                .map(|search| search.native_opens)
                .sum::<usize>()
                == self.opens_before
    }
}

#[derive(Serialize)]
pub struct OpenRepairTrial {
    pub net: String,
    pub proposed_copper: usize,
    pub status: &'static str,
    pub native_report: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact_refinement: Option<ContactRefinement>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_identity: Option<NativeIdentityEvidence>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connectivity_proof: Option<OpenConnectivityProof>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connectivity_proof_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width_witness_replay: Option<OpenWidthReplay>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_search: Option<LocalSearch>,
    /// A native zone witness resolved to its isolated physical pad.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zone_open_pad: Option<ZoneOpenPad>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub teardrop_stored_reports: Option<[String; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preset_via_complementary_reports: Option<[String; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paired_search: Option<PairedSearch>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub panel_piece: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trial_panel: Option<NativePanelCounts>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub panel_witness_replacements: Vec<PanelWitnessReplacement>,
}

#[derive(Serialize)]
pub struct PairedSearch {
    pub nets: [String; 2],
    pub members: [PairedMember; 2],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unmatched_end: Option<board_router::KiCadPairEndpointSpacing>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_action: Option<&'static str>,
    /// Router measurements of the proposed copper. KiCad checks the rules.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub geometry: Option<board_router::KiCadDiffPair>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stored_report: Option<String>,
}

#[derive(Serialize)]
pub struct PairedMember {
    pub net: String,
    pub native_open: bool,
    pub retained_copper: usize,
    pub pads: Vec<PairedPad>,
}

#[derive(Serialize)]
pub struct PairedPad {
    pub reference: String,
    pub pad: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
}

#[derive(Serialize)]
pub struct NativeIdentityEvidence {
    pub source_copper_without_ids: usize,
    pub unmatched_native_findings: usize,
    pub trial_native_opens: usize,
    pub action: &'static str,
}

fn read_board(path: &Path) -> Result<(String, Expr), String> {
    let source = fs::read_to_string(path)
        .map_err(|error| exit::input_error(format!("{}: {error}", path.display())))?;
    let board = parse(&source)
        .map_err(|error| exit::input_error(format!("{}: {error}", path.display())))?;
    if board.head() != Some("kicad_pcb") || !source.trim_end().ends_with(')') {
        return Err(exit::input_error(format!(
            "{} is not a KiCad board",
            path.display()
        )));
    }
    Ok((source, board))
}

pub(crate) fn item_id(item: &Expr) -> Option<String> {
    form_atom(item, "uuid", 1)
        .or_else(|| form_atom(item, "tstamp", 1))
        .filter(|id| !id.is_empty())
        .map(str::to_ascii_lowercase)
}

fn route_item(item: &Expr) -> bool {
    matches!(item.head(), Some("segment" | "arc" | "via"))
}

fn route_geometry(item: &Expr) -> String {
    let mut parts = item.children().to_vec();
    parts.retain(|form| !matches!(form.head(), Some("uuid" | "tstamp" | "net")));
    encode(&Expr::List(parts))
}

fn source_net_field(codes: &BTreeMap<String, String>, net: &str) -> Expr {
    let field = codes
        .iter()
        .find(|(_, name)| name.as_str() == net)
        .map(|(code, _)| code.clone())
        // item_net returns the spelling from the file, already escaped.
        .unwrap_or_else(|| format!("\"{net}\""));
    Expr::List(vec![Expr::Atom("net".into()), Expr::Atom(field)])
}

fn item_ids(pcb: &Expr) -> BTreeSet<String> {
    let mut occupied = BTreeSet::new();
    let mut pending = vec![pcb];
    while let Some(form) = pending.pop() {
        occupied.extend(item_id(form));
        pending.extend(form.children());
    }
    occupied
}

fn copper_by_net(pcb: &Expr, candidate: &Expr) -> BTreeMap<String, Vec<Expr>> {
    let source_codes = net_codes(pcb);
    let candidate_codes = net_codes(candidate);
    let mut occupied = item_ids(pcb);
    let mut geometry: BTreeSet<_> = pcb
        .children()
        .iter()
        .filter(|item| route_item(item))
        .filter_map(|item| Some((item_net(item, &source_codes)?, route_geometry(item))))
        .collect();
    let mut proposals = BTreeMap::<String, Vec<Expr>>::new();
    for item in candidate.children().iter().filter(|item| route_item(item)) {
        let Some(net) = item_net(item, &candidate_codes).filter(|net| !net.is_empty()) else {
            continue;
        };
        let Some(id) = item_id(item) else { continue };
        if !occupied.insert(id) || !geometry.insert((net, route_geometry(item))) {
            continue;
        }
        let mut parts = item.children().to_vec();
        let net_field = source_net_field(&source_codes, net);
        for part in &mut parts {
            if part.head() == Some("net") {
                *part = net_field.clone();
            }
        }
        proposals
            .entry(net.into())
            .or_default()
            .push(Expr::List(parts));
    }
    proposals
}

fn open_nets(pcb: &Expr, native: &Value) -> BTreeSet<String> {
    native_opens(pcb, native, &net_codes(pcb))
        .into_iter()
        .map(|(net, _)| net.to_owned())
        .collect()
}

fn requested_open_nets(
    pcb: &Expr,
    open: &BTreeSet<String>,
    requested: &[String],
) -> (BTreeSet<String>, Vec<String>) {
    let codes = net_codes(pcb);
    let names: BTreeSet<_> = pcb
        .children()
        .iter()
        .flat_map(|item| std::iter::once(item).chain(item.children()))
        .filter_map(|item| item_net(item, &codes))
        .filter(|name| !name.is_empty())
        .chain(codes.values().map(String::as_str))
        .chain(open.iter().map(String::as_str))
        .map(|raw| (raw, pcb_kicad_file::outline_pieces::unescaped(raw)))
        .collect();
    let mut selected = BTreeSet::new();
    let mut unmatched = Vec::new();
    for query in requested {
        let mut matches = names
            .iter()
            .filter(|(raw, displayed)| *raw == query || displayed == query);
        if let Some((name, _)) = matches.next()
            && matches.next().is_none()
            && open.contains(*name)
        {
            selected.insert((*name).to_string());
        } else {
            unmatched.push(query.clone());
        }
    }
    (selected, unmatched)
}

pub(crate) fn native_opens<'a>(
    pcb: &'a Expr,
    native: &'a Value,
    codes: &'a BTreeMap<String, String>,
) -> Vec<(&'a str, &'a [Value])> {
    native_open_witnesses(pcb, native, codes, false)
}

fn native_open_witnesses<'a>(
    pcb: &'a Expr,
    native: &'a Value,
    codes: &'a BTreeMap<String, String>,
    teardrops: bool,
) -> Vec<(&'a str, &'a [Value])> {
    let mut ids = BTreeMap::new();
    for item in pcb.children().iter().flat_map(|item| {
        std::iter::once(item).chain(
            item.children()
                .iter()
                .filter(|child| child.head() == Some("pad")),
        )
    }) {
        if let Some(id) = item_id(item) {
            let net = item_net(item, codes).filter(|net| !net.is_empty());
            ids.entry(id).and_modify(|old| *old = None).or_insert(net);
        }
    }
    native["unconnected_items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|finding| {
            let items = finding["items"].as_array()?;
            let ident = |item: &Value| Some(item["uuid"].as_str()?.to_ascii_lowercase());
            let net_for = |item: &Value| ids.get(&ident(item)?).copied().flatten();
            let net = items.iter().find_map(net_for)?;
            let transient = items.len() == 2
                && teardrops
                && items
                    .iter()
                    .filter(|item| net_for(item) == Some(net))
                    .count()
                    == 1
                && items.iter().any(|item| {
                    ident(item).is_some_and(|id| !ids.contains_key(&id))
                        && teardrop::witness(item, net).is_some()
                });
            (items.len() >= 2
                && !net.trim_start_matches('/').starts_with("unconnected-(")
                && (transient || items.iter().all(|item| net_for(item) == Some(net))))
            .then_some((net, items.as_slice()))
        })
        .collect()
}

type Findings = BTreeMap<String, usize>;

pub(crate) fn finding_counts(report: &Value, key: &str) -> Result<Findings, String> {
    let mut counts = BTreeMap::new();
    if matches!(key, ACROSS_KEY | "schematic_parity") && report.get(key).is_none() {
        return Ok(counts);
    }
    for finding in report[key]
        .as_array()
        .ok_or_else(|| format!("invalid native DRC report: missing array {key}"))?
    {
        let mut finding = finding.clone();
        if let Some(items) = finding["items"].as_array_mut() {
            items.sort_by_cached_key(Value::to_string);
        }
        *counts.entry(finding.to_string()).or_default() += 1;
    }
    Ok(counts)
}

/// KiCad's findings that neither added copper nor a part's pose can change:
/// a footprint compared with its library, and a text's height and stroke.
/// KiCad caps each finding type on its own (DRC_ENGINE's per-type limits),
/// so their cap hides no finding a repair could cause. The checks run
/// without the user's library tables (`kicad_config.rs`), which makes every
/// board of 199 library footprints or more reach the library findings' cap:
/// on the night of 2026-11-24 it refused the layout's open repair on 17
/// sweep boards and the single-net closes of DSP-ADAU1452 and
/// poly_kybd_split72_right, whose trials KiCad found closed and otherwise
/// unchanged. The text findings reach theirs on jetson-nano and link.
pub(crate) const COPPER_INDEPENDENT_TYPES: [&str; 4] = [
    "lib_footprint_issues",
    "lib_footprint_mismatch",
    "text_height",
    "text_thickness",
];

fn copper_independent(finding: &Value) -> bool {
    finding["type"]
        .as_str()
        .is_some_and(|kind| COPPER_INDEPENDENT_TYPES.contains(&kind))
}

/// Whether KiCad's report may hide a finding a repair could change: a
/// report KiCad marked capped, the unconnected items at their limit, or a
/// violation type at its cap that copper can change.
pub fn report_capped(report: &Value) -> bool {
    report[CAPPED_KEY] == true
        || report["unconnected_items"]
            .as_array()
            .is_some_and(|items| items.len() >= KICAD_UNCONNECTED_CAP)
        || !copper_finding_types_at_cap(report).is_empty()
}

/// The violation types at KiCad's cap less the copper-independent ones.
pub fn copper_finding_types_at_cap(report: &Value) -> Vec<&str> {
    finding_types_at_cap(report, "violations")
        .into_iter()
        .filter(|kind| !COPPER_INDEPENDENT_TYPES.contains(kind))
        .collect()
}

/// Native finding types that may have reached KiCad's report cap.
pub fn finding_types_at_cap<'a>(report: &'a Value, key: &str) -> Vec<&'a str> {
    let mut counts = BTreeMap::<&str, usize>::new();
    for finding in report[key].as_array().into_iter().flatten() {
        *counts
            .entry(finding["type"].as_str().unwrap_or_default())
            .or_default() += 1;
    }
    // KiCad 10.0's DRC_ENGINE::RunTests uses the extended limit for
    // clearance and unconnected items. Treat other versions and types
    // conservatively when the exact limit is unknown.
    let known_extended = key == "violations"
        && report["kicad_version"]
            .as_str()
            .is_some_and(|version| version.starts_with("10.0."));
    counts
        .into_iter()
        .filter_map(|(kind, count)| {
            let limit = if known_extended && kind == "clearance" {
                KICAD_UNCONNECTED_CAP
            } else {
                KICAD_FINDING_TYPE_FLOOR
            };
            (count >= limit).then_some(kind)
        })
        .collect()
}

/// Whether the candidate has a finding more often than the baseline. The
/// copper-independent findings do not count: copper does not change them,
/// and where KiCad caps them two checks may list different ones.
fn adds_findings(baseline: &Value, candidate: &Value, key: &str) -> Result<bool, String> {
    let counts = |report: &Value| -> Result<Findings, String> {
        if key != "violations"
            || !report[key]
                .as_array()
                .is_some_and(|all| all.iter().any(copper_independent))
        {
            return finding_counts(report, key);
        }
        let mut copper = report.clone();
        if let Some(findings) = copper[key].as_array_mut() {
            findings.retain(|finding| !copper_independent(finding));
        }
        finding_counts(&copper, key)
    };
    let before = counts(baseline)?;
    let after = counts(candidate)?;
    Ok(after
        .iter()
        .any(|(finding, count)| before.get(finding).unwrap_or(&0) < count))
}

pub(crate) fn reported_missing_libraries(report: &Value) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for finding in report["violations"].as_array().into_iter().flatten() {
        if finding["type"] == "lib_footprint_issues"
            && finding["severity"] == "warning"
            && finding["excluded"] != true
            && let Some(name) = finding["description"].as_str().and_then(|text| {
                text.strip_prefix(
                    "The current configuration does not include the footprint library '",
                )?
                .strip_suffix('\'')
            })
            && !name.is_empty()
        {
            *counts.entry(name.to_string()).or_default() += 1;
        }
    }
    counts
}

pub fn preserved(baseline: &Value, candidate: &Value) -> Result<bool, String> {
    Ok(!report_capped(candidate) && preserved_findings(baseline, candidate)?)
}

fn preserved_findings(baseline: &Value, candidate: &Value) -> Result<bool, String> {
    preserved_findings_except(baseline, candidate, None)
}

fn preserved_findings_except(
    baseline: &Value,
    candidate: &Value,
    except: Option<&str>,
) -> Result<bool, String> {
    if !same_check_policy(baseline, candidate) {
        return Ok(false);
    }
    for key in FINDING_KEYS {
        if Some(key) != except && adds_findings(baseline, candidate, key)? {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(crate) fn same_check_policy(baseline: &Value, candidate: &Value) -> bool {
    ["kicad_version", "included_severities", "ignored_checks"]
        .into_iter()
        .all(|field| baseline.get(field) == candidate.get(field))
}

/// Explain a refusal when the only unmatched native findings refer to
/// anonymous routed copper at the coordinates of a legacy source item.
/// This comparison is used after the strict acceptance check has failed.
fn legacy_copper_identity(
    pcb: &Expr,
    additions: &[Expr],
    baseline: &Value,
    candidate: &Value,
) -> Result<Option<NativeIdentityEvidence>, String> {
    if baseline["coordinate_units"] != "mm" || candidate["coordinate_units"] != "mm" {
        return Ok(None);
    }
    // KiCad stores coordinates in integer nanometres. Older writers can
    // emit more digits than survive in the native report.
    let native_point = |x: f64, y: f64| ((x * 1e6).round() as i64, (y * 1e6).round() as i64);
    let mut anchors = BTreeSet::new();
    let mut source_copper_without_ids = 0;
    for item in pcb
        .children()
        .iter()
        .filter(|item| route_item(item) && item_id(item).is_none())
    {
        let (kind, fields): (_, &[_]) = if item.head() == Some("via") {
            ("Via", &["at"])
        } else {
            ("Track", &["start", "mid", "end"])
        };
        source_copper_without_ids += 1;
        for field in fields {
            if let Ok([x, y]) = form_xy(item, field) {
                anchors.insert((kind, native_point(x, y)));
            }
        }
    }
    if anchors.is_empty() {
        return Ok(None);
    }
    let mut stable = item_ids(pcb);
    stable.extend(additions.iter().filter_map(item_id));
    let without_ephemeral_ids = |report: &Value| {
        let mut projected = report.clone();
        for key in FINDING_KEYS {
            for finding in projected
                .get_mut(key)
                .and_then(Value::as_array_mut)
                .into_iter()
                .flatten()
            {
                for item in finding
                    .get_mut("items")
                    .and_then(Value::as_array_mut)
                    .into_iter()
                    .flatten()
                {
                    let Some(uuid) = item["uuid"].as_str().filter(|id| !id.is_empty()) else {
                        continue;
                    };
                    let kind = item["description"]
                        .as_str()
                        .and_then(|desc| desc.split_once(" ["))
                        .map(|(kind, _)| kind);
                    let position = item["pos"]["x"].as_f64().zip(item["pos"]["y"].as_f64());
                    if let Some((kind, (x, y))) = kind.zip(position)
                        && anchors.contains(&(kind, native_point(x, y)))
                        && !stable.contains(&uuid.to_ascii_lowercase())
                        && let Some(item) = item.as_object_mut()
                    {
                        item.remove("uuid");
                    }
                }
            }
        }
        projected
    };
    if !preserved(
        &without_ephemeral_ids(baseline),
        &without_ephemeral_ids(candidate),
    )? {
        return Ok(None);
    }
    let mut unmatched_native_findings = 0;
    for key in FINDING_KEYS {
        let before = finding_counts(baseline, key)?;
        for (finding, count) in finding_counts(candidate, key)? {
            unmatched_native_findings +=
                count.saturating_sub(before.get(&finding).copied().unwrap_or(0));
        }
    }
    Ok(
        (unmatched_native_findings != 0).then_some(NativeIdentityEvidence {
            source_copper_without_ids,
            unmatched_native_findings,
            trial_native_opens: candidate["unconnected_items"].as_array().unwrap().len(),
            action: PERSIST_COPPER_IDS,
        }),
    )
}

pub(crate) fn native_check(
    board: &Path,
    path: &Path,
    parity: bool,
    refill: bool,
) -> Result<Value, String> {
    let mut args = vec![
        "pcb",
        "drc",
        "--format",
        "json",
        "--all-track-errors",
        "--severity-all",
    ];
    if parity {
        args.push("--schematic-parity");
    }
    if refill {
        args.push("--refill-zones");
    }
    args.push("--output");
    native_report::run_kicad_report(&args, path, board)?;
    let report = native_report::read_kicad_report(path)?;
    if parity && !report["schematic_parity"].is_array() {
        return Err("native DRC report is missing schematic parity".into());
    }
    finding_counts(&report, "violations")?;
    finding_counts(&report, "unconnected_items")?;
    Ok(report)
}

pub(crate) fn write_additions(path: &Path, source: &str, additions: &[Expr]) -> Result<(), String> {
    let close = source.trim_end().len() - 1;
    let mut text = source[..close].to_owned();
    for item in additions {
        text.push('\n');
        text.push_str(&encode(item));
    }
    text.push_str(&source[close..]);
    fs::write(path, text).map_err(|error| error.to_string())
}

/// Publishes a copy of the input project with only native-verified, same-net
/// additions, or its original board when none improve the native report.
pub fn repair_kicad_opens(
    source: &Path,
    board_id: &str,
    candidate: &Path,
    output: &Path,
) -> Result<OpenRepair, String> {
    repair_kicad_opens_on_nets(source, board_id, candidate, output, &[])
}

/// Tries donor copper only for the requested native open nets.
/// An empty selection includes every identified open net.
pub fn repair_kicad_opens_on_nets(
    source: &Path,
    board_id: &str,
    candidate: &Path,
    output: &Path,
    only_nets: &[String],
) -> Result<OpenRepair, String> {
    check_board_id(board_id).map_err(exit::input_error)?;
    let filename = format!("{board_id}.kicad_pcb");
    let (original, pcb) = read_board(&source.join(&filename))?;
    let (_, candidate) = read_board(candidate)?;
    let before_placement = board_statistics(&pcb).map_err(exit::input_error)?;
    let candidate_placement = board_statistics(&candidate).map_err(exit::input_error)?;
    if before_placement.component_placement_sha256 != candidate_placement.component_placement_sha256
    {
        return Err(exit::input_error(
            "candidate footprint placement differs from the routed source; reroute with existing_tracks set to keep",
        ));
    }
    in_repair_copy(source, output, |stage| {
        repair_in_copy(
            stage,
            board_id,
            &stage.join(filename),
            &original,
            &pcb,
            Method::Candidate(&candidate, only_nets),
            None,
        )
    })
}

/// Searches the source project's kept copper for local connections and
/// publishes only additions that pass the same gate as an alternate route.
pub fn repair_kicad_opens_locally(
    source: &Path,
    board_id: &str,
    output: &Path,
    only_nets: &[String],
) -> Result<OpenRepair, String> {
    check_board_id(board_id).map_err(exit::input_error)?;
    let filename = format!("{board_id}.kicad_pcb");
    let (original, pcb) = read_board(&source.join(&filename))?;
    in_repair_copy(source, output, |stage| {
        let board = stage.join(filename);
        repair_in_copy(
            stage,
            board_id,
            &board,
            &original,
            &pcb,
            Method::Local(only_nets),
            None,
        )
    })
}

/// The same additive operation as manual repair, limited to one eligible net
/// under the layout's active routing settings, work budget and wall guard.
pub fn repair_layout_opens(
    source: &Path,
    board_id: &str,
    output: &Path,
    budget: &mut LayoutRepairBudget<'_>,
) -> Result<OpenRepair, String> {
    repair_layout_opens_on_nets(source, board_id, output, budget, &[])
}

fn repair_layout_opens_on_nets(
    source: &Path,
    board_id: &str,
    output: &Path,
    budget: &mut LayoutRepairBudget<'_>,
    nets: &[String],
) -> Result<OpenRepair, String> {
    if budget.search_limit() <= 0.0 || !budget.can_check_trial() {
        return Err("remaining layout work does not cover local repair checks".into());
    }
    check_board_id(board_id).map_err(exit::input_error)?;
    let filename = format!("{board_id}.kicad_pcb");
    let (original, pcb) = read_board(&source.join(&filename))?;
    in_repair_copy(source, output, |stage| {
        repair_in_copy(
            stage,
            board_id,
            &stage.join(filename),
            &original,
            &pcb,
            Method::Local(nets),
            Some(budget),
        )
    })
}

/// Runs one verified repair in a private project and publishes it as a unit.
pub fn in_repair_copy<T>(
    source: &Path,
    output: &Path,
    repair: impl FnOnce(&Path) -> Result<T, String>,
) -> Result<T, String> {
    let parent = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let source_directory =
        fs::canonicalize(source).map_err(|error| exit::input_error(error.to_string()))?;
    let output_parent =
        fs::canonicalize(parent).map_err(|error| exit::input_error(error.to_string()))?;
    if output_parent.starts_with(source_directory) {
        return Err(exit::input_error(
            "place the repair output outside the source project",
        ));
    }
    if fs::symlink_metadata(output).is_ok() {
        return Err(exit::input_error(format!(
            "output directory {} already exists",
            output.display()
        )));
    }
    let stage = temporary_next_to(output);
    let result = (|| {
        copy_directory_tree(source, &stage)?;
        let report = repair(&stage)?;
        fs::create_dir(output)
            .map_err(|error| format!("failed to reserve {}: {error}", output.display()))?;
        if let Err(error) = fs::rename(&stage, output) {
            let _ = fs::remove_dir(output);
            return Err(format!("failed to publish {}: {error}", output.display()));
        }
        Ok(report)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&stage);
    }
    result
}

enum Method<'a> {
    Candidate(&'a Expr, &'a [String]),
    Local(&'a [String]),
}

fn repair_in_copy(
    directory: &Path,
    board_id: &str,
    board: &Path,
    original: &str,
    pcb: &Expr,
    method: Method<'_>,
    mut budget: Option<&mut LayoutRepairBudget<'_>>,
) -> Result<OpenRepair, String> {
    if budget.is_none() {
        native_report::remove_stale_board_run_reports(directory)?;
    }
    let baseline_path = directory.join("drc-before-open-repair.json");
    let parity = directory.join(format!("{board_id}.kicad_sch")).is_file();
    let refill = pad_bridge::has_fills(pcb);
    if let Some(budget) = &mut budget {
        budget.native_checks += 1;
    }
    let implicit_silk_rules = match method {
        Method::Local(_) => ImplicitSilkRules::read(board),
        Method::Candidate(..) => None,
    };
    let mut native = native_check(board, &baseline_path, parity, refill)?;
    if fs::read_to_string(board).map_err(|error| error.to_string())? != original {
        return Err("KiCad's pre-check modified the board; additive verification requires the original board to be checked as written".into());
    }
    let count = |report: &Value| report["unconnected_items"].as_array().unwrap().len();
    let pieces = match method {
        Method::Local(_) => Pieces::of_board(pcb).map_err(exit::input_error)?,
        Method::Candidate(..) => None,
    };
    let mut panel = pieces
        .as_ref()
        .map(|pieces| NativePanelScope::new(pcb, pieces));
    let panel_before = panel.as_mut().and_then(|scope| scope.counts(&native));
    let requested = match method {
        Method::Candidate(_, only_nets) | Method::Local(only_nets) => only_nets,
    };
    let all_open = if matches!(method, Method::Local(_))
        && budget.is_none()
        && !requested.is_empty()
        && pieces.is_none()
    {
        native_open_witnesses(pcb, &native, &net_codes(pcb), true)
            .into_iter()
            .map(|(net, _)| net.to_owned())
            .collect()
    } else {
        open_nets(pcb, &native)
    };
    let (nets, unmatched_requested_nets) = if requested.is_empty() {
        (all_open.clone(), Vec::new())
    } else {
        requested_open_nets(pcb, &all_open, requested)
    };
    let mut report = OpenRepair {
        status: "no_improvement",
        schematic_parity_checked: parity,
        drc_refilled_zones: refill,
        native_edge_clearance_ignored: native["ignored_checks"].as_array().is_some_and(|checks| {
            checks
                .iter()
                .any(|check| check["key"] == "copper_edge_clearance")
        }),
        opens_before: count(&native),
        opens_after: count(&native),
        added_copper: 0,
        identified_open_nets: all_open.into_iter().collect(),
        requested_nets: requested.to_vec(),
        unmatched_requested_nets,
        trials: Vec::new(),
        note: None,
        capped_schematic_parity_types: finding_types_at_cap(&native, "schematic_parity")
            .into_iter()
            .map(str::to_owned)
            .collect(),
        capped_native_violation_types: finding_types_at_cap(&native, "violations")
            .into_iter()
            .map(str::to_owned)
            .collect(),
        reported_missing_footprint_libraries: BTreeMap::new(),
        verified_invariant_native_caps: Vec::new(),
        panel_after: panel_before.clone(),
        panel_before,
    };
    let mut kept = Vec::new();
    let mut kept_path = baseline_path;
    let silk_cap = implicit_silk_rules
        .as_ref()
        .filter(|rules| {
            copper_finding_types_at_cap(&native) == ["silk_overlap"]
                && ImplicitSilkRules::read(board).as_ref() == Some(*rules)
        })
        .and_then(|_| SilkCap::prove(pcb, &native));
    let capped = (report_capped(&native) && silk_cap.is_none())
        || (budget.is_some() && !report.capped_schematic_parity_types.is_empty());
    if capped {
        report.note =
            Some("KiCad capped its baseline report; compare a complete report before repairing");
        // The library findings at their cap do not cap the report
        // (`COPPER_INDEPENDENT_TYPES`): another type does, and restoring the
        // libraries would not lift it. Their counts are reported still.
        if report
            .capped_native_violation_types
            .iter()
            .any(|kind| kind == "lib_footprint_issues")
        {
            report.reported_missing_footprint_libraries = reported_missing_libraries(&native);
        }
    } else if report.opens_before == 0 {
        report.note = Some("the native baseline has no unconnected items");
    }
    if report.opens_before > 0
        && (!capped || (budget.is_none() && matches!(method, Method::Local(_))))
    {
        if !report.requested_nets.is_empty() && nets.is_empty() {
            report.note.get_or_insert(
                "no requested net has an unambiguous native open with matching endpoint UUIDs",
            );
        }
        let proposals = match method {
            Method::Candidate(candidate, _) => copper_by_net(pcb, candidate)
                .into_iter()
                .map(|(net, objects)| (net, local::Proposal::candidate(objects)))
                .collect(),
            Method::Local(requested) => {
                let mut ordinary = nets.clone();
                let mut proposals = Vec::new();
                if budget.is_none()
                    && pieces.is_none()
                    && !report_capped(&native)
                    && report.capped_schematic_parity_types.is_empty()
                    && let Some(([net, mate], proposal)) =
                        paired::propose(pcb, board, &native, &nets)?
                {
                    ordinary.remove(&net);
                    ordinary.remove(&mate);
                    proposals.push((net, proposal));
                }
                proposals.extend(local::propose(
                    pcb,
                    board,
                    &native,
                    &ordinary,
                    pieces.as_ref(),
                    if !requested.is_empty() && budget.is_none() {
                        local::SearchScope::Named
                    } else {
                        local::SearchScope::Ordinary
                    },
                    budget.as_deref_mut(),
                )?);
                proposals
            }
        };
        let mut proposals: VecDeque<_> = proposals
            .into_iter()
            .filter(|(net, _)| nets.contains(net))
            .take(MAX_TRIALS)
            .collect();
        let mut zone_open_pads = BTreeMap::new();
        if !capped
            && budget.is_none()
            && pieces.is_none()
            && matches!(method, Method::Local(requested) if !requested.is_empty())
        {
            let names = NetNames::of(pcb);
            let selected: Vec<_> = proposals
                .iter()
                .filter(|(_, proposal)| proposal.status == "native_endpoints_unavailable")
                .map(|(net, _)| names.normalize(net).to_owned())
                .collect();
            if !selected.is_empty() {
                let limit = Instant::now() + local::LOCAL_WALL;
                zone_open_pads = locate_zone_open_pads(board, pcb, &native, &selected, || {
                    Instant::now() >= limit
                })
                .unwrap_or_default()
                .into_iter()
                .map(|pad| (pad.net.clone(), pad))
                .collect();
            }
        }
        let mut branch_pending =
            budget.is_some() && pieces.is_none() && matches!(method, Method::Local([]));
        loop {
            let Some((net, proposal)) = proposals.pop_front() else {
                if !std::mem::take(&mut branch_pending) {
                    break;
                }
                let Some(budget) = budget.as_deref_mut() else {
                    break;
                };
                if budget.branch_search_limit() <= 0.0 || !budget.can_check_branch(3) {
                    break;
                }
                let large = local::large_open_nets(pcb, &native);
                if large.is_empty() {
                    break;
                }
                // The preceding trial may have replaced some open witnesses
                // with newly added copper. Route with that exact incumbent.
                write_additions(board, original, &kept)?;
                let (_, current) = read_board(board)?;
                let lowering = board_router::lowering_work(&current);
                if budget.branch_search_limit() <= lowering {
                    break;
                }
                budget.search_work_seconds += lowering;
                proposals = local::propose(
                    &current,
                    board,
                    &native,
                    &large,
                    None,
                    local::SearchScope::LargeNet,
                    Some(budget),
                )?
                .into();
                continue;
            };
            let objects = proposal.objects;
            let needs_branch_proof = budget.is_some()
                && proposal
                    .search
                    .as_ref()
                    .is_some_and(|search| !search.attempted_branches.is_empty());
            let pad_bridge = proposal
                .search
                .as_ref()
                .is_some_and(|search| search.same_footprint_pad_bridge.is_some());
            let panel_piece = pieces
                .as_ref()
                .and_then(|pieces| pieces.added_routing_piece(&objects));
            let zone_open_pad = zone_open_pads.remove(unescape(&net).as_ref());
            let mut trial = OpenRepairTrial {
                net,
                proposed_copper: objects.len(),
                status: proposal.status,
                native_report: None,
                contact_refinement: None,
                native_identity: None,
                connectivity_proof: None,
                connectivity_proof_error: None,
                width_witness_replay: None,
                local_search: proposal.search,
                zone_open_pad,
                teardrop_stored_reports: None,
                preset_via_complementary_reports: None,
                paired_search: proposal.paired,
                panel_piece: panel_piece.map(|piece| piece + 1),
                trial_panel: None,
                panel_witness_replacements: Vec::new(),
            };
            if objects.len() > MAX_NET_ITEMS {
                trial.status = "copper_limit";
            } else if !objects.is_empty() && pieces.is_some() && trial.panel_piece.is_none() {
                trial.status = "candidate_not_confined_to_one_panel_piece";
            } else if !objects.is_empty()
                && budget.as_ref().is_some_and(|budget| {
                    if pad_bridge {
                        !budget.can_check_trials(pad_bridge::checks_needed(refill))
                    } else if needs_branch_proof {
                        !budget.can_check_branch(3)
                    } else {
                        !budget.can_check_trial()
                    }
                })
            {
                trial.status = "local_layout_budget_exhausted";
            } else if !objects.is_empty() {
                let deadline = budget.as_ref().and_then(|budget| budget.router.deadline);
                let check = |report: &Path, refill| {
                    if pad_bridge {
                        native_report::check_kept_route_input(board, report, parity, refill, || {
                            deadline.is_some_and(|limit| Instant::now() >= limit)
                        })
                    } else {
                        native_check(board, report, parity, refill)
                    }
                };
                let filename = format!("drc-open-trial-{}.json", report.trials.len() + 1);
                let mut path = directory.join(&filename);
                trial.native_report = Some(filename);
                let previous = kept.len();
                let preset = trial
                    .local_search
                    .as_ref()
                    .is_some_and(|search| search.preset_via_mm.is_some());
                let plane_landing = trial
                    .local_search
                    .as_ref()
                    .is_some_and(|search| search.plane_landing.is_some());
                let complementary = if pad_bridge && refill {
                    Some("same-footprint-pad-bridge-stored")
                } else if preset {
                    Some(if refill {
                        "preset-via-stored"
                    } else {
                        "preset-via-refilled"
                    })
                } else if refill
                    && !plane_landing
                    && trial
                        .local_search
                        .as_ref()
                        .is_some_and(|search| !search.teardrop_contacts.is_empty())
                {
                    Some("teardrop-stored")
                } else {
                    None
                };
                let complementary_before = if let Some(name) = complementary {
                    let file = format!("drc-{name}-before-{}.json", report.trials.len() + 1);
                    write_additions(board, original, &kept)?;
                    let expected = fs::read(board).map_err(|error| error.to_string())?;
                    if pad_bridge && let Some(budget) = &mut budget {
                        budget.native_checks += 1;
                    }
                    let baseline = check(&directory.join(&file), !refill)?;
                    if fs::read(board).map_err(|error| error.to_string())? != expected {
                        return Err("KiCad's complementary pre-check modified the board".into());
                    }
                    Some((name, file, baseline))
                } else {
                    None
                };
                let stored_baseline = if refill && trial.paired_search.is_some() {
                    write_additions(board, original, &kept)?;
                    let expected = fs::read(board).map_err(|error| error.to_string())?;
                    let baseline = native_check(
                        board,
                        &directory.join("drc-before-paired-repair-stored.json"),
                        parity,
                        false,
                    )?;
                    if fs::read(board).map_err(|error| error.to_string())? != expected {
                        return Err("KiCad's stored pre-check modified the board".into());
                    }
                    Some(baseline)
                } else {
                    None
                };
                kept.extend(objects);
                write_additions(board, original, &kept)?;
                let mut written = fs::read(board).map_err(|error| error.to_string())?;
                if let Some(budget) = &mut budget {
                    budget.native_checks += 1;
                }
                let mut next = check(&path, refill)?;
                if matches!(method, Method::Local(_))
                    && trial.paired_search.is_none()
                    && !pad_bridge
                    && !plane_landing
                    && pieces.is_none()
                    && !capped
                    && count(&next) < count(&native)
                    && budget.as_ref().is_none_or(|budget| {
                        if needs_branch_proof {
                            budget.can_check_branch(3)
                        } else {
                            budget.can_check_trial()
                        }
                    })
                    && written == fs::read(board).map_err(|error| error.to_string())?
                    && let Some(contact::Adjusted { objects, contacts }) = contact::refine(
                        pcb,
                        &kept[..previous],
                        &kept[previous..],
                        &trial.net,
                        &native,
                        &next,
                    )?
                {
                    let filename = format!("drc-open-contact-{}.json", report.trials.len() + 1);
                    path = directory.join(&filename);
                    trial.contact_refinement = Some(ContactRefinement {
                        original_report: trial
                            .native_report
                            .replace(filename)
                            .expect("original trial report"),
                        contacts,
                    });
                    trial.proposed_copper = objects.len();
                    kept.truncate(previous);
                    kept.extend(objects);
                    write_additions(board, original, &kept)?;
                    written = fs::read(board).map_err(|error| error.to_string())?;
                    if let Some(budget) = &mut budget {
                        budget.native_checks += 1;
                    }
                    next = native_check(board, &path, parity, refill)?;
                }
                trial.trial_panel = panel.as_mut().and_then(|scope| scope.counts(&next));
                let proved_static_cap = silk_cap.as_ref().is_some_and(|proof| {
                    ImplicitSilkRules::read(board) == implicit_silk_rules
                        && proof.preserved(&next, &kept)
                });
                trial.status = if written != fs::read(board).map_err(|error| error.to_string())? {
                    "native_check_changed_board"
                } else if capped {
                    "native_baseline_report_capped"
                } else if ((report_capped(&next) || silk_cap.is_some()) && !proved_static_cap)
                    || (budget.is_some()
                        && !finding_types_at_cap(&next, "schematic_parity").is_empty())
                {
                    "native_trial_report_capped"
                } else if let Some(landing) = trial
                    .local_search
                    .as_mut()
                    .and_then(|search| search.plane_landing.as_mut())
                {
                    let native_name = unescape(&trial.net);
                    let proof = connectivity::Trial {
                        board,
                        original,
                        additions: &kept,
                        previous,
                        target: &native_name,
                        refill,
                    };
                    match proof.prove_plane_landing(&native, &next, landing) {
                        Ok((proof, islands)) => {
                            landing.verified_filled_islands = Some(islands);
                            trial.connectivity_proof = Some(proof);
                            "accepted"
                        }
                        Err(error) => {
                            trial.connectivity_proof_error = Some(error);
                            "plane_landing_without_native_proof"
                        }
                    }
                } else if let Some(piece) = panel_piece
                    && report
                        .panel_after
                        .as_ref()
                        .zip(trial.trial_panel.as_ref())
                        .is_none_or(|(before, after)| !before.reduces_opens_on_piece(after, piece))
                {
                    "native_panel_piece_not_improved"
                } else if let Some((pieces, piece)) = pieces.as_ref().zip(panel_piece)
                    && adds_findings(&native, &next, ACROSS_KEY)?
                {
                    let witness = panel_witness::PanelTrial {
                        pcb,
                        pieces,
                        piece,
                        previous: &kept[..previous],
                        added: &kept[previous..],
                    };
                    if !preserved_findings_except(&native, &next, Some(ACROSS_KEY))? {
                        "new_or_incomparable_native_findings"
                    } else if let Some(proof) = witness.prove(&native, &next) {
                        trial.panel_witness_replacements = proof;
                        "accepted"
                    } else {
                        "native_panel_cross_gap_witness_changed"
                    }
                } else if let Some(pair) = &mut trial.paired_search {
                    if !paired::findings_preserved(&native, &next)? {
                        "coupled_new_or_incomparable_native_findings"
                    } else if !paired::closes_both(&native, &next, pcb, pair) {
                        "coupled_native_witnesses_not_closed"
                    } else if let Some(baseline) = stored_baseline {
                        let file = "drc-paired-repair-stored.json";
                        let stored = native_check(board, &directory.join(file), parity, false)?;
                        pair.stored_report = Some(file.into());
                        if fs::read(board).map_err(|error| error.to_string())? != written {
                            "native_check_changed_board"
                        } else if !paired::findings_preserved(&baseline, &stored)? {
                            "coupled_new_or_incomparable_stored_findings"
                        } else if paired::closes_both(&baseline, &stored, pcb, pair) {
                            "accepted"
                        } else {
                            "coupled_stored_witnesses_not_closed"
                        }
                    } else {
                        "accepted"
                    }
                } else if let Some(bridge) = trial
                    .local_search
                    .as_ref()
                    .and_then(|search| search.same_footprint_pad_bridge.as_ref())
                {
                    if !bridge.closes(&native, &next)? {
                        "same_footprint_pad_bridge_did_not_close_exact_open"
                    } else {
                        let native_name = unescape(&trial.net);
                        let proof = connectivity::Trial {
                            board,
                            original,
                            additions: &kept,
                            previous,
                            target: &native_name,
                            refill,
                        };
                        match proof.prove(
                            &native,
                            1,
                            &mut budget,
                            Some(connectivity::ExpectedWitnesses::PadBridge(bridge)),
                        ) {
                            Ok(proof) => {
                                trial.connectivity_proof = Some(proof);
                                "accepted"
                            }
                            Err(error) => {
                                trial.connectivity_proof_error = Some(error);
                                "same_footprint_pad_bridge_unverified_partition"
                            }
                        }
                    }
                } else if needs_branch_proof && count(&next) >= count(&native) {
                    "open_still_reported"
                } else if needs_branch_proof || !preserved_findings(&native, &next)? {
                    if matches!(method, Method::Local(_))
                        && pieces.is_none()
                        && count(&native) > count(&next)
                    {
                        let native_name = unescape(&trial.net);
                        let proof = connectivity::Trial {
                            board,
                            original,
                            additions: &kept,
                            previous,
                            target: &native_name,
                            refill,
                        };
                        let partition = if eligible_open_partition(&native, &next)? {
                            Some(proof.prove(
                                &native,
                                count(&native) - count(&next),
                                &mut budget,
                                None,
                            ))
                        } else if !needs_branch_proof
                            && let Some(replayed) = proof.replay_width(
                                &native,
                                &next,
                                parity,
                                report.trials.len() + 1,
                                &mut budget,
                            )?
                        {
                            trial.width_witness_replay = Some(replayed.report);
                            Some(replayed.proof)
                        } else {
                            None
                        };
                        if let Some(partition) = partition {
                            match partition {
                                Ok(proof) => trial.connectivity_proof = Some(proof),
                                Err(error) => trial.connectivity_proof_error = Some(error),
                            }
                        }
                    }
                    if trial.connectivity_proof.is_some() {
                        "accepted"
                    } else {
                        trial.native_identity = legacy_copper_identity(pcb, &kept, &native, &next)?;
                        if trial.native_identity.is_some() {
                            "unstable_native_copper_ids"
                        } else if trial.width_witness_replay.is_some() {
                            "unverified_native_width_witness_change"
                        } else {
                            "new_or_incomparable_native_findings"
                        }
                    }
                } else if count(&next) >= count(&native) {
                    "open_still_reported"
                } else {
                    "accepted"
                };
                if trial.status == "accepted"
                    && let Some((name, before_file, baseline)) = complementary_before
                {
                    let file = format!("drc-{name}-after-{}.json", report.trials.len() + 1);
                    if pad_bridge && let Some(budget) = &mut budget {
                        budget.native_checks += 1;
                    }
                    let checked = check(&directory.join(&file), !refill)?;
                    let reports = Some([before_file, file]);
                    let mut exact = true;
                    if let Some(bridge) = trial
                        .local_search
                        .as_mut()
                        .and_then(|search| search.same_footprint_pad_bridge.as_mut())
                    {
                        bridge.stored_reports = reports;
                        exact = bridge.closes(&baseline, &checked)?;
                        if !exact
                            && count(&baseline).checked_sub(count(&checked)) == Some(1)
                            && bridge.can_compare_partitions(&baseline, &checked)?
                        {
                            let native_name = unescape(&trial.net);
                            let proof = connectivity::Trial {
                                board,
                                original,
                                additions: &kept,
                                previous,
                                target: &native_name,
                                refill: !refill,
                            };
                            match proof.prove(
                                &baseline,
                                1,
                                &mut budget,
                                Some(connectivity::ExpectedWitnesses::PadBridge(bridge)),
                            ) {
                                Ok(proof) => {
                                    bridge.stored_connectivity_proof = Some(proof);
                                    exact = true;
                                }
                                Err(error) => trial.connectivity_proof_error = Some(error),
                            }
                        }
                    } else if preset {
                        trial.preset_via_complementary_reports = reports;
                    } else {
                        trial.teardrop_stored_reports = reports;
                    }
                    if fs::read(board).map_err(|error| error.to_string())? != written {
                        trial.status = "native_check_changed_board";
                    } else if !exact {
                        trial.status = "same_footprint_pad_bridge_did_not_close_exact_stored_open";
                    } else if !pad_bridge
                        && (report_capped(&baseline)
                            || [&baseline, &checked].into_iter().any(|report| {
                                !finding_types_at_cap(report, "schematic_parity").is_empty()
                            })
                            || !preserved(&baseline, &checked)?)
                    {
                        trial.status = if preset {
                            "preset_via_new_or_incomparable_findings"
                        } else {
                            "teardrop_new_or_incomparable_stored_findings"
                        };
                    } else if count(&checked) >= count(&baseline) {
                        trial.status = if preset {
                            "preset_via_open_still_reported"
                        } else {
                            "teardrop_stored_open_still_reported"
                        };
                    }
                }
                if trial.status == "accepted" {
                    native = next;
                    kept_path = path;
                    report.panel_after = trial.trial_panel.clone();
                    if proved_static_cap && report.verified_invariant_native_caps.is_empty() {
                        report.verified_invariant_native_caps.push("silk_overlap");
                    }
                } else {
                    if let Some(pair) = &mut trial.paired_search {
                        pair.reason = Some(trial.status);
                        pair.next_action = Some(
                            "Review the recorded KiCad trial with the original project rules before changing the pair's copper.",
                        );
                    }
                    kept.truncate(previous);
                }
            }
            report.trials.push(trial);
        }
        if report.trials.is_empty() && report.note.is_none() {
            report.note = Some(
                "no repair proposal matches a native open by endpoint UUIDs and exact net name",
            );
        }
    }
    if kept.is_empty() {
        fs::write(board, original).map_err(|error| error.to_string())?;
    } else {
        write_additions(board, original, &kept)?;
        report.status = "improved";
        report.opens_after = count(&native);
        report.added_copper = kept.len();
    }
    fs::copy(kept_path, directory.join("drc.json")).map_err(|error| error.to_string())?;
    write_typed_json(&directory.join("open-repair.json"), &report)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn budgeted_branch_leaves_its_native_proof_and_saved_fill_reserve() {
        let router = KiCadBoardRouterConfig::default();
        let mut budget = LayoutRepairBudget {
            router: &router,
            available_seconds: 28.0,
            check_seconds: 3.0,
            native_checks: 4,
            search_work_seconds: 0.0,
            branch_fill_reserve_checks: Some(2),
        };
        assert_eq!(budget.branch_search_limit(), 1.0);
        // Each proposal waiting for its check leaves the next search less.
        assert_eq!(budget.search_limit(), 28.0 - 5.0 * 3.0);
        assert_eq!(budget.search_limit_after(2), 28.0 - 7.0 * 3.0);
        assert!(budget.can_check_branch(3));
        budget.search_work_seconds += 2.0;
        assert!(budget.can_check_trial());
        assert!(!budget.can_check_branch(3));
    }

    #[test]
    fn missing_library_counts_name_only_reported_active_warnings() {
        let warning = json!({
            "type":"lib_footprint_issues", "severity":"warning",
            "description":"The current configuration does not include the footprint library 'Board Parts'"
        });
        let mut excluded = warning.clone();
        excluded["excluded"] = json!(true);
        let mut other = warning.clone();
        other["description"] = json!("Footprint differs from library copy");
        let report = json!({"violations":[warning, excluded, other, warning]});
        assert_eq!(
            reported_missing_libraries(&report),
            BTreeMap::from([("Board Parts".into(), 2)])
        );
    }

    #[test]
    fn proposals_never_replace_source_copper_and_keep_exact_net_names() {
        let original = parse(
            r#"(kicad_pcb
                (net 2 "/SCL") (net 3 "SCL")
                (segment (start 1 1) (end 2 2) (width 0.2) (net 2) (uuid "kept"))
                (via (at 8 8) (size 0.6) (net 3) (uuid "other")))"#,
        )
        .unwrap();
        let alternative = parse(
            r#"(kicad_pcb
                (segment (start 1 1) (end 2 3) (width 0.2) (net "/SCL") (uuid "kept"))
                (segment (start 1 1) (end 2 2) (width 0.2) (net "/SCL") (uuid "duplicate"))
                (segment (start 2 2) (end 4 4) (width 0.2) (net "/SCL") (uuid "new"))
                (segment (start 4 4) (end 5 5) (width 0.2) (net "SCL") (uuid "other-net")))"#,
        )
        .unwrap();
        let proposals = copper_by_net(&original, &alternative);
        assert_eq!(proposals["/SCL"].len(), 1);
        assert_eq!(form_atom(&proposals["/SCL"][0], "uuid", 1), Some("new"));
        assert_eq!(form_atom(&proposals["/SCL"][0], "net", 1), Some("2"));
        assert_eq!(form_atom(&proposals["SCL"][0], "net", 1), Some("3"));

        let unconnected =
            |a, b| json!({"unconnected_items": [{"items": [{"uuid": a}, {"uuid": b}]}]});
        assert!(open_nets(&original, &unconnected("kept", "other")).is_empty());
        assert!(open_nets(&original, &unconnected("kept", "unidentified")).is_empty());
    }

    #[test]
    fn requested_opens_accept_native_spellings_only_when_unambiguous() {
        let board = parse(
            r#"(kicad_pcb
                (net 1 "M2B_PWM_L\\") (net 2 "GND") (net 3 "CLOSED")
                (net 4 "/RAIL{slash}2") (net 5 "/RAIL/2"))"#,
        )
        .unwrap();
        let open = [r"M2B_PWM_L\\", "GND", "/RAIL{slash}2"]
            .map(str::to_owned)
            .into();
        let select = |queries: &[&str]| {
            requested_open_nets(
                &board,
                &open,
                &queries.iter().map(|n| n.to_string()).collect::<Vec<_>>(),
            )
        };
        let (selected, unmatched) = select(&[r"M2B_PWM_L\", "GND", "unknown", "CLOSED", "/RAIL/2"]);
        assert_eq!(
            selected,
            [r"M2B_PWM_L\\".to_string(), "GND".to_string()].into()
        );
        assert_eq!(unmatched, ["unknown", "CLOSED", "/RAIL/2"]);
        assert_eq!(select(&[r"M2B_PWM_L\\", "/RAIL{slash}2"]).0.len(), 2);
    }

    #[test]
    fn closing_an_open_cannot_hide_a_new_finding_or_a_new_open() {
        let finding =
            |uuid| json!({"type": "clearance", "severity": "warning", "items": [{"uuid": uuid}]});
        let original =
            json!({"violations": [finding("old")], "unconnected_items": [finding("missing")]});
        let mut trial = json!({"violations": [finding("old")], "unconnected_items": []});
        assert!(preserved(&original, &trial).unwrap());
        trial["violations"] = json!([finding("old"), finding("old")]);
        assert!(!preserved(&original, &trial).unwrap());
        trial["violations"] = json!([finding("new")]);
        assert!(!preserved(&original, &trial).unwrap());
        trial["violations"] = original["violations"].clone();
        trial["unconnected_items"] = json!([finding("different location")]);
        assert!(!preserved(&original, &trial).unwrap());
        trial["unconnected_items"] = json!([]);
        trial["schematic_parity"] = json!([finding("new parity")]);
        assert!(!preserved(&original, &trial).unwrap());
    }

    #[test]
    fn unstable_legacy_copper_ids_explain_a_refusal_without_loosening_it() {
        let pcb =
            parse("(kicad_pcb (segment (start 2.00000012 3) (end 2.00000012 3) (width 0.25)))")
                .unwrap();
        let finding = |uuid, x| {
            json!({"type": "track_dangling", "severity": "warning", "description": "Track has unconnected end",
                   "items": [{"uuid": uuid, "description": "Track [GND] on F.Cu, length 0.0000 mm",
                              "pos": {"x": x, "y": 3}}]})
        };
        let baseline = json!({"coordinate_units": "mm", "violations": [finding("before", 2)],
                              "unconnected_items": [{"items": [{"uuid": "unconnected-pad"}]}]});
        let mut trial = json!({"coordinate_units": "mm", "violations": [finding("after", 2)],
                               "unconnected_items": []});
        assert!(!preserved(&baseline, &trial).unwrap());
        let evidence = legacy_copper_identity(&pcb, &[], &baseline, &trial)
            .unwrap()
            .unwrap();
        assert_eq!(evidence.source_copper_without_ids, 1);
        assert_eq!(evidence.unmatched_native_findings, 1);
        assert_eq!(evidence.trial_native_opens, 0);

        let added = parse("(segment (start 2 3) (end 2 4) (uuid \"after\"))").unwrap();
        assert!(
            legacy_copper_identity(&pcb, &[added], &baseline, &trial)
                .unwrap()
                .is_none()
        );
        trial["violations"][0]["severity"] = json!("error");
        assert!(
            legacy_copper_identity(&pcb, &[], &baseline, &trial)
                .unwrap()
                .is_none()
        );
        trial["violations"] = json!([finding("after", 4)]);
        assert!(
            legacy_copper_identity(&pcb, &[], &baseline, &trial)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn drc_caps_refuse_trials_and_inherited_parity_caps_are_reported() {
        let violations = vec![json!({"type": "track_width"}); 199];
        let parity = vec![json!({"type": "net_conflict"}); 199];
        let native = json!({"violations": violations, "unconnected_items": []});
        assert!(report_capped(&native));
        let native = json!({"violations": [], "unconnected_items": [], "schematic_parity": parity});
        assert!(!report_capped(&native));
        assert_eq!(
            finding_types_at_cap(&native, "schematic_parity"),
            ["net_conflict"]
        );
        assert!(preserved(&native, &native).unwrap());
    }

    #[test]
    fn library_findings_at_their_cap_neither_cap_nor_block_a_trial() {
        // The library-free checks list every library footprint once: from
        // 199 on, KiCad's cap (DSP-ADAU1452 and poly_kybd_split72_right on
        // the designer tier of 2026-11-24).
        let library = |reference: usize| {
            json!({
                "type": "lib_footprint_issues", "severity": "warning",
                "description": "The current configuration does not include the footprint library 'Capacitor_SMD'",
                "items": [{"description": format!("Footprint C{reference}"), "uuid": format!("{reference}")}]
            })
        };
        let open = json!({"type": "unconnected_items", "items": [{"uuid": "a"}, {"uuid": "b"}]});
        let baseline = json!({
            "kicad_version": "10.0.8",
            "violations": (0..199).map(library).collect::<Vec<_>>(),
            "unconnected_items": [open]
        });
        assert_eq!(
            finding_types_at_cap(&baseline, "violations"),
            ["lib_footprint_issues"]
        );
        assert!(copper_finding_types_at_cap(&baseline).is_empty());
        assert!(!report_capped(&baseline));
        // Another subset of the capped findings is no new finding.
        let mut trial = baseline.clone();
        trial["violations"] = json!((1..200).map(library).collect::<Vec<_>>());
        trial["unconnected_items"] = json!([]);
        assert!(preserved(&baseline, &trial).unwrap());
        // Every other finding still counts, and another type's cap still caps.
        trial["violations"]
            .as_array_mut()
            .unwrap()
            .push(json!({"type": "clearance", "severity": "error", "items": []}));
        assert!(!preserved(&baseline, &trial).unwrap());
        let mut silk = baseline.clone();
        silk["violations"]
            .as_array_mut()
            .unwrap()
            .extend(vec![json!({"type": "silk_overlap"}); 199]);
        assert!(report_capped(&silk));
        assert_eq!(copper_finding_types_at_cap(&silk), ["silk_overlap"]);
        // A text's height at its cap is copper-independent too.
        let mut text = baseline.clone();
        text["violations"]
            .as_array_mut()
            .unwrap()
            .extend(vec![json!({"type": "text_height"}); 199]);
        assert!(!report_capped(&text));
    }

    #[test]
    fn kicad_10_0_uses_the_extended_clearance_cap() {
        let mut native = json!({
            "kicad_version": "10.0.8",
            "violations": vec![json!({"type": "clearance"}); 498],
            "unconnected_items": []
        });
        assert!(!report_capped(&native));
        native["violations"]
            .as_array_mut()
            .unwrap()
            .push(json!({"type": "clearance"}));
        assert!(report_capped(&native));
        native["violations"].as_array_mut().unwrap().pop();
        native["kicad_version"] = json!("11.0.0");
        assert!(report_capped(&native));
    }
}
