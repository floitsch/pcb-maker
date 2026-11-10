// Copyright (C) 2026 Toit contributors.

//! `inspect-routing-run`: why a route or layout run left nets open, from
//! what the run saved (board-router.json, the board it wrote, KiCad's
//! drc.json) and, when given, the reference board. Nothing is routed: the
//! result board is lowered as the router lowers a board and its copper
//! joined as KiCad's connectivity joins it without pours
//! (`pcb_router::verify::copper_components`), each group of pads named
//! (reference, number, uuid, position, copper layers). Beside them: the
//! router's own counts, KiCad's unconnected pairs (reconciled with the
//! groups, the counts explained apart), the net's rules with their
//! sources, the reference's route, the ladder's attempts. What was not
//! measured says so (`access`, `cause`: unknown).

use super::*;
use crate::board_router::{LayerTable, lower};
use crate::project_rules::{KiCadProjectRulesReport, wildcard_match};
use pcb_router as core;

/// What to inspect (`inspect-routing-run`'s arguments).
#[derive(Clone, Debug, Default)]
pub struct KiCadRunInspection {
    /// The run's output directory: route-kicad-board's `<out-dir>` or
    /// layout-kicad-board's (its `result/`).
    pub run: PathBuf,
    /// Only this net (any status).
    pub net: Option<String>,
    /// Only the net of this pad, `REF:PAD`.
    pub terminal: Option<String>,
    /// The reference project directory (the designer's board).
    pub reference: Option<PathBuf>,
    /// KiCad's DRC report to read instead of the run's `drc.json` (a
    /// `--native-check` the caller ran).
    pub drc: Option<PathBuf>,
}

/// The files of a run (`run_files`).
#[derive(Clone, Debug)]
pub struct KiCadRunFiles {
    /// The directory holding board-router.json and the board.
    pub directory: PathBuf,
    pub board_id: String,
}

/// The directory of a run's board and report: `run` itself, or a
/// layout's `run/result`.
pub fn run_files(run: &Path) -> Result<KiCadRunFiles, String> {
    let directory = [run.to_path_buf(), run.join("result")]
        .into_iter()
        .find(|directory| directory.join("board-router.json").is_file())
        .ok_or_else(|| {
            crate::verdict::input_error(format!(
                "{} holds no board-router.json (nor its result/): give a route-kicad-board or layout-kicad-board output directory",
                run.display()
            ))
        })?;
    let report = read_value(&directory.join("board-router.json"))?;
    let board_id = report["board_id"]
        .as_str()
        .ok_or_else(|| {
            crate::verdict::input_error(format!(
                "{} names no board_id",
                directory.join("board-router.json").display()
            ))
        })?
        .to_string();
    check_board_id(&board_id).map_err(crate::verdict::input_error)?;
    if !directory.join(format!("{board_id}.kicad_pcb")).is_file() {
        return Err(crate::verdict::input_error(format!(
            "{} is missing: the run's board",
            directory.join(format!("{board_id}.kicad_pcb")).display()
        )));
    }
    Ok(KiCadRunFiles {
        directory,
        board_id,
    })
}

fn read_value(path: &Path) -> Result<serde_json::Value, String> {
    let text = fs::read_to_string(path).map_err(|error| {
        crate::verdict::input_error(format!("failed to read {}: {error}", path.display()))
    })?;
    serde_json::from_str(&text).map_err(|error| {
        crate::verdict::input_error(format!("failed to parse {}: {error}", path.display()))
    })
}

/// One pad of a group: its identity on every board of the run (the uuid
/// stays with a footprint's pad; reference and number name it).
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct KiCadInspectedPad {
    #[serde(rename = "ref")]
    pub reference: String,
    pub number: String,
    pub uuid: Option<String>,
    pub at: [f64; 2],
    pub layers: Vec<String>,
}

/// One connected group of a net's pads on the result board.
#[derive(Clone, Debug, Serialize)]
pub struct KiCadInspectedComponent {
    pub pads: Vec<KiCadInspectedPad>,
    /// Other terminals of the group that are no pad (a zone's island, a
    /// copper graphic), by the lowering's label.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub other: Vec<String>,
    /// Whether this is the net's largest group.
    pub largest: bool,
}

/// One unconnected pair of KiCad's DRC report.
#[derive(Clone, Debug, Serialize)]
pub struct KiCadNativePair {
    pub items: Vec<KiCadNativeItem>,
    /// The groups (indices into `components`) its items lie in; `null`
    /// for an item that is no pad of a group (a track, a via, a zone).
    pub components: Vec<Option<usize>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadNativeItem {
    pub description: String,
    pub at: [f64; 2],
    pub uuid: Option<String>,
}

/// A net's rules on the run's project, with their sources.
#[derive(Clone, Debug, Serialize, Default)]
pub struct KiCadInspectedRules {
    /// As the run's project resolves them (what KiCad's DRC holds the net
    /// to; the router's rules on a run without overrides).
    pub track_width_mm: Option<f64>,
    pub clearance_mm: Option<f64>,
    pub via_diameter_mm: Option<f64>,
    pub via_drill_mm: Option<f64>,
    /// The net's classes, the highest priority first.
    pub classes: Vec<String>,
    /// Where each value comes from: the class, the board's minimum (with
    /// its own source) or a custom rule.
    pub track_width_source: String,
    pub clearance_source: String,
    /// The class's own values where they are below the resolved ones (a
    /// class a router file wrote under `allow_below_project_minimum`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class_track_width_mm: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class_clearance_mm: Option<f64>,
    /// The board's minimums and their sources.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_minimums: Option<crate::project_rules::KiCadProjectMinimums>,
    /// The custom rules file the project has (`<id>.kicad_dru`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_rules_file: Option<String>,
    /// What the run's router file asked for below the project's, for this
    /// net (board-router.json's `rules.overrides`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub overrides: Vec<serde_json::Value>,
    /// The track widths the result board has on this net: what was
    /// emitted.
    pub emitted_track_widths_mm: Vec<f64>,
}

/// The net's copper on a board: tracks, vias, layers, widths.
#[derive(Clone, Debug, Serialize, Default, PartialEq)]
pub struct KiCadNetCopper {
    pub segments: usize,
    pub arcs: usize,
    pub vias: usize,
    pub layers: Vec<String>,
    pub track_widths_mm: Vec<f64>,
    pub length_mm: f64,
}

/// The reference board's route of a net.
#[derive(Clone, Debug, Serialize)]
pub struct KiCadReferenceRoute {
    /// Whose placement the route is on (`KiCadReferencePlacement`): "the
    /// designer's own placement" (parts lie elsewhere on this run's board:
    /// a layout's, which `connected` says nothing of) or "this placement"
    /// (every part where this run's board has it: route mode's).
    pub placement: String,
    /// Whether its copper joins every pad of the net: `true` when its
    /// tracks, arcs and vias leave one group, `false` when they leave
    /// several and the net has no zone on the reference, `null` when they
    /// leave several and a zone of the net may join them (zone fill is
    /// not modelled here; `connected_reason` says so).
    pub connected: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connected_reason: Option<String>,
    /// The groups its copper leaves without pours.
    pub components: usize,
    /// Layers on which the reference has a zone of the net.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pour_layers: Vec<String>,
    #[serde(flatten)]
    pub copper: KiCadNetCopper,
}

/// The counts of a net, each with what it counts.
#[derive(Clone, Debug, Serialize)]
pub struct KiCadNetCounts {
    /// board-router.json: the router's terminals (pads joined before
    /// routing, by kept copper or by overlap, are one) and those its
    /// search left unjoined (its state at the end, not the copper
    /// written: a net it ripped up counts every terminal).
    pub router_terminals: Option<usize>,
    pub router_unconnected_terminals: Option<usize>,
    /// The result board's copper without pours: its groups of pads and
    /// the links missing between them (groups less one).
    pub components: usize,
    pub missing_links: usize,
    /// KiCad's unconnected pairs on this net (after its zone fill); none
    /// when no DRC report was read.
    pub native_pairs: Option<usize>,
    /// How the three relate, in a sentence.
    pub explanation: String,
}

/// The diagnosis of one net.
#[derive(Clone, Debug, Serialize)]
pub struct KiCadNetDiagnosis {
    pub net: String,
    /// The router's status (board-router.json `nets[]`): `routed`,
    /// `partial`, `unreachable`, ...
    pub status: String,
    pub counts: KiCadNetCounts,
    pub components: Vec<KiCadInspectedComponent>,
    /// What the groups were measured on: the result board's copper
    /// without its pours.
    pub components_basis: String,
    /// Layers on which the net has a zone: its fill may join groups
    /// (`native_pairs` counts after the fill).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pour_layers: Vec<String>,
    /// A net with a zone: the groups left out of `components` (its fill,
    /// which this command does not model, joins them or not); listed are
    /// the groups KiCad's pairs name and the `--terminal` pad's.
    #[serde(skip_serializing_if = "is_zero_count")]
    pub components_omitted: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_pairs: Option<Vec<KiCadNativePair>>,
    pub copper: KiCadNetCopper,
    pub rules: KiCadInspectedRules,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<KiCadReferenceRoute>,
    /// The router's dead pads of this net (board-router.json).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub dead_pads: Vec<serde_json::Value>,
    /// The router's hot spots within 3 mm of a pad of this net, at most
    /// five, the most contested first: where nets fought longest near it
    /// (an observation, not a cause); `hot_spot` indexes the report's
    /// `hot_spots`, which list the tile's pads and nets.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hot_spots_near: Vec<serde_json::Value>,
    /// The pad `--terminal` named: its group.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal: Option<serde_json::Value>,
    /// Not measured by this command: no access probe or cut ran.
    pub access: serde_json::Value,
    pub cause: serde_json::Value,
}

/// Whose placement the reference's routes (`nets[].reference`) are on,
/// and for a layout what is evidence for this run's placement instead.
/// Trial 4 (2026-11-04): on a layout `reference.connected` read as the
/// designer's route "on this placement", which no run of the designer's
/// is; the layout's verdict says so since, and so does this block.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct KiCadReferencePlacement {
    /// The reference project.
    pub project: String,
    /// "this placement" (every part of this run's board where the
    /// reference has it) or "the designer's own placement".
    pub placement: String,
    /// The parts of this run's board where the reference has them (the
    /// same position, rotation and side), and its parts.
    pub parts_in_place: usize,
    pub parts: usize,
    /// The reference board's tracks and arcs, every net's. None (a task
    /// input or a netlist import, stripped or never routed: trial 5's
    /// ducktop2 right_io) makes `connected` false wherever no zone joins a
    /// net, which says nothing about whether the net routes.
    pub reference_tracks: usize,
    /// What `nets[].reference.connected` shows, in a sentence.
    pub connected_means: String,
    /// A layout's own routes of its placement (board-layout.json): the
    /// first route's open terminals, vias and length, the move descent's
    /// routed moves and open terminals before and after (`moves`), and the
    /// result's open terminals and KiCad's unconnected items. Absent for a
    /// route.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence_for_this_placement: Option<serde_json::Value>,
}

/// The phrases the layout's verdict uses (`open_evidence`).
const ON_DESIGNERS_PLACEMENT: &str = "the designer's own placement";
const ON_THIS_PLACEMENT: &str = "this placement";

/// `KiCadReferencePlacement` of a run's board (`result`) against the
/// reference's (`reference`, of `project`), with a layout's report
/// (`layout`: board-layout.json, at `layout_path`) and the run's verdict.
fn reference_placement(
    project: &str,
    reference: &Expr,
    result: &Expr,
    layout: Option<(&serde_json::Value, &Path)>,
    verdict: Option<&serde_json::Value>,
) -> KiCadReferencePlacement {
    let items = crate::verdict::input_items_of(reference, result);
    let same = items.parts > 0 && items.kept_parts == items.parts;
    let evidence = layout.map(|(report, path)| {
        let descent = &report["move_descent"];
        let moves = if descent.is_object() {
            serde_json::json!({
                "routed": descent["confirmed"],
                "kept": descent["kept"],
                "open_before": descent["open_before"],
                "open_after": descent["open_after"],
                "parts_moved": descent["parts_moved"],
            })
        } else {
            let tried = report["moves"].as_array().map_or(0, Vec::len);
            let kept = report["moves"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|entry| entry["kept"] == true)
                .count();
            serde_json::json!({"routed": tried, "kept": kept})
        };
        serde_json::json!({
            "report": path.display().to_string(),
            "first_route_unconnected_terminals": report["first_unconnected_terminals"],
            "first_route_vias": report["first_vias"],
            "first_route_length_mm": report["first_length_mm"],
            "moves": moves,
            "unconnected_terminals": report["routed"]["unconnected_terminals"],
            "kicad_unconnected_items": verdict.map_or(serde_json::Value::Null, |verdict| verdict["kicad_unconnected_items"].clone()),
        })
    });
    let reference_tracks = reference
        .children()
        .iter()
        .filter(|item| matches!(item.head(), Some("segment" | "arc")))
        .count();
    let connected_means = if reference_tracks == 0 {
        "the reference board has no tracks or arcs (an unrouted board: a task input or a stripped copy), so reference.connected is false wherever no zone joins a net and says nothing about whether it routes, on any placement; give --reference the designer's routed board".to_string()
    } else if same {
        format!(
            "the designer's route on {ON_THIS_PLACEMENT}: every part of this run's board lies where the reference has it, so where reference.connected is true the net routes under these rules on this placement, and density is not shown to be why it stayed open"
        )
    } else {
        format!(
            "the designer's route on {ON_DESIGNERS_PLACEMENT}: {} of the {} parts of this run's board lie elsewhere on the reference, so reference.connected shows that a net routes under these rules there, not on this placement, and no route of the designer's on this placement exists{}",
            items.parts - items.kept_parts,
            items.parts,
            if evidence.is_some() {
                "; the evidence for this placement is the layout's own routes (evidence_for_this_placement: its first route and its move descent)"
            } else {
                ""
            }
        )
    };
    KiCadReferencePlacement {
        project: project.to_string(),
        placement: if same {
            ON_THIS_PLACEMENT
        } else {
            ON_DESIGNERS_PLACEMENT
        }
        .to_string(),
        parts_in_place: items.kept_parts,
        parts: items.parts,
        reference_tracks,
        connected_means,
        evidence_for_this_placement: evidence,
    }
}

/// `inspect-routing-run`'s report.
#[derive(Clone, Debug, Serialize)]
pub struct KiCadRunDiagnosis {
    pub schema_version: u32,
    pub units: String,
    pub run: serde_json::Value,
    pub checks: serde_json::Value,
    /// Route mode's ladder (board-router.json `attempts`): every attempt
    /// judged and the one written; `null` for a run before the ledger.
    pub attempts: serde_json::Value,
    /// The router's hot spots (board-router.json), each with its tile's
    /// objects: pads, nets, nets still in conflict (`links_from` says
    /// whether the router recorded them or this command read them off the
    /// result board).
    pub hot_spots: Vec<serde_json::Value>,
    /// With `--reference`: whose placement its routes are on, and a
    /// layout's own evidence for this placement (`KiCadReferencePlacement`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<KiCadReferencePlacement>,
    pub nets: Vec<KiCadNetDiagnosis>,
}

fn is_zero_count(count: &usize) -> bool {
    *count == 0
}

/// A footprint pad of the board: identity and place.
struct BoardPad {
    reference: String,
    number: String,
    uuid: Option<String>,
    at: [f64; 2],
    net: Option<String>,
}

fn unquoted(text: &str) -> String {
    text.trim_matches('"').to_string()
}

fn board_pads(pcb: &Expr, names: &NetNames) -> Result<Vec<BoardPad>, String> {
    let mut pads = Vec::new();
    for footprint in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
    {
        let reference = footprint_reference(footprint)
            .map(|text| unquoted(&text))
            .unwrap_or_default();
        let footprint_at = form_at(footprint)?;
        for pad in footprint
            .children()
            .iter()
            .filter(|item| item.head() == Some("pad"))
        {
            let number = pad
                .children()
                .get(1)
                .and_then(Expr::atom)
                .map(unquoted)
                .unwrap_or_default();
            let pad_at = form_at(pad)?;
            let offset = rotate_vector([pad_at[0], pad_at[1]], -footprint_at[2]);
            pads.push(BoardPad {
                reference: reference.clone(),
                number,
                uuid: pad.children().iter().find_map(|item| match item.head()? {
                    "uuid" | "tstamp" => item.children().get(1)?.atom().map(unquoted),
                    _ => None,
                }),
                at: [footprint_at[0] + offset[0], footprint_at[1] + offset[1]],
                net: node_net(pad).map(|raw| names.normalize(raw).to_string()),
            });
        }
    }
    Ok(pads)
}

fn round(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

fn layer_names(table: &LayerTable, mask: core::LayerMask) -> Vec<String> {
    table
        .names
        .iter()
        .enumerate()
        .filter(|(index, _)| mask & (1 << index) != 0)
        .map(|(_, name)| name.clone())
        .collect()
}

/// The net's tracks, arcs and vias on a board.
fn net_copper(pcb: &Expr, names: &NetNames, net: &str) -> KiCadNetCopper {
    let codes = net_codes(pcb);
    let mut copper = KiCadNetCopper::default();
    let mut layers = BTreeSet::new();
    let mut widths = BTreeSet::new();
    for item in pcb.children() {
        let Some(head) = item.head() else { continue };
        if !matches!(head, "segment" | "arc" | "via") {
            continue;
        }
        if item_net(item, &codes).map(|raw| names.normalize(raw)) != Some(net) {
            continue;
        }
        match head {
            "via" => copper.vias += 1,
            _ => {
                if head == "segment" {
                    copper.segments += 1;
                } else {
                    copper.arcs += 1;
                }
                if let Some(layer) = form_atom(item, "layer", 1) {
                    layers.insert(unquoted(layer));
                }
                if let Ok(width) = form_f64(item, "width", 1) {
                    widths.insert((width * 1.0e6).round() as i64);
                }
                if let (Ok(start), Ok(end)) = (form_xy(item, "start"), form_xy(item, "end")) {
                    copper.length_mm += distance_squared(start, end).sqrt();
                }
            }
        }
    }
    copper.layers = layers.into_iter().collect();
    copper.track_widths_mm = widths
        .into_iter()
        .map(|width| width as f64 / 1.0e6)
        .collect();
    copper.length_mm = round(copper.length_mm);
    copper
}

/// What became of a router file's value below the project's minimum
/// that the run allowed (`allow_below_project_minimum`): asked for, held
/// to by the project, routed with, written, and what KiCad's DRC
/// requires and found (the verdict's `rule_overrides`).
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct KiCadOverrideOutcome {
    /// The router file's key (`net_classes[Narrow]`, `default_rules`, ...).
    pub source: String,
    /// `track_width` or `clearance`.
    pub quantity: String,
    /// The nets (or patterns) it applies to; empty for every net.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub nets: Vec<String>,
    pub requested_mm: f64,
    /// The project's minimum and where it comes from.
    pub project_minimum_mm: f64,
    pub project_source: String,
    /// The router's rule for those nets: the requested value, which the
    /// run allowed (the project's minimum is not raised to it).
    pub routed_with_mm: f64,
    /// Track width: the distinct widths of the tracks and arcs the result
    /// board has on those nets (every net for an override without nets),
    /// and how many of them are below the project's minimum. `null` for
    /// a clearance (nothing here measures the clearances written).
    pub emitted_track_widths_mm: Option<Vec<f64>>,
    pub emitted_below_minimum: Option<usize>,
    /// What KiCad's DRC holds the board to: the project's minimum.
    pub native_required_mm: f64,
    /// KiCad's findings of that kind (`track_width`, `clearance`) on those
    /// nets (every net for an override without nets); `null` when KiCad
    /// did not check the board.
    pub native_findings: Option<usize>,
}

fn names_net(patterns: &[String], net: &str) -> bool {
    patterns.is_empty()
        || patterns.iter().any(|pattern| {
            let pattern = pattern.trim_start_matches('/');
            wildcard_match(pattern, net) || wildcard_match(pattern, net.trim_start_matches('/'))
        })
}

/// The outcome of each override the run allowed, on the board it wrote
/// (`board`) and KiCad's report of it (`drc`, when KiCad checked).
pub fn override_outcomes(
    overrides: &[crate::project_rules::KiCadRuleOverride],
    board: &Path,
    drc: Option<&serde_json::Value>,
) -> Vec<KiCadOverrideOutcome> {
    if overrides.is_empty() {
        return Vec::new();
    }
    const EPSILON: f64 = 1.0e-9;
    // Per net, the widths of its tracks and arcs on the board written.
    let mut widths: Vec<(String, f64)> = Vec::new();
    if let Ok(pcb) = fs::read_to_string(board)
        .map_err(|error| error.to_string())
        .and_then(|text| parse(&text))
    {
        let codes = net_codes(&pcb);
        let names = NetNames::of(&pcb);
        for item in pcb.children() {
            if !matches!(item.head(), Some("segment" | "arc")) {
                continue;
            }
            if let (Some(net), Ok(width)) = (item_net(item, &codes), form_f64(item, "width", 1)) {
                widths.push((names.normalize(net).to_string(), width));
            }
        }
    }
    overrides
        .iter()
        .map(|item| {
            let width = item.quantity == "track_width";
            let mine: Vec<f64> = widths
                .iter()
                .filter(|(net, _)| names_net(&item.nets, net))
                .map(|(_, width)| *width)
                .collect();
            let distinct: BTreeSet<i64> = mine
                .iter()
                .map(|width| (width * 1.0e6).round() as i64)
                .collect();
            let kind = if width { "track_width" } else { "clearance" };
            let native_findings = drc.map(|drc| {
                drc["violations"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|finding| finding["type"].as_str() == Some(kind))
                    .filter(|finding| {
                        finding["items"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .any(|part| {
                                part["description"]
                                    .as_str()
                                    .and_then(described_net)
                                    .is_some_and(|net| names_net(&item.nets, &net))
                            })
                    })
                    .count()
            });
            KiCadOverrideOutcome {
                source: item.source.clone(),
                quantity: item.quantity.clone(),
                nets: item.nets.clone(),
                requested_mm: item.requested_mm,
                project_minimum_mm: item.project_minimum_mm,
                project_source: item.project_source.clone(),
                routed_with_mm: item.requested_mm,
                emitted_track_widths_mm: width
                    .then(|| distinct.iter().map(|width| *width as f64 / 1.0e6).collect()),
                emitted_below_minimum: width.then(|| {
                    mine.iter()
                        .filter(|width| **width < item.project_minimum_mm - EPSILON)
                        .count()
                }),
                native_required_mm: item.project_minimum_mm,
                native_findings,
            }
        })
        .collect()
}

/// A board lowered as the router lowers it, with its pads and layer
/// names, for the connectivity of its nets.
struct LoweredBoard {
    pcb: Expr,
    names: NetNames,
    board: core::Board,
    layers: LayerTable,
    pads: Vec<BoardPad>,
}

impl LoweredBoard {
    fn read(directory: &Path, board_id: &str) -> Result<Self, String> {
        let path = directory.join(format!("{board_id}.kicad_pcb"));
        let text = fs::read_to_string(&path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        let pcb = parse(&text)?;
        let config = resolve_project_rules(&directory.join(format!("{board_id}.kicad_pro")), &path)
            .unwrap_or_default();
        let board = lower(&pcb, &config, true)?.board;
        let layers = LayerTable::from_pcb(&pcb)?;
        let names = NetNames::of(&pcb);
        let pads = board_pads(&pcb, &names)?;
        Ok(Self {
            pcb,
            names,
            board,
            layers,
            pads,
        })
    }

    fn net_id(&self, net: &str) -> Option<core::NetId> {
        self.board
            .nets
            .iter()
            .position(|description| description.name == net)
            .map(|index| index as core::NetId)
    }

    /// The pad of the board an obstacle (a pad's copper) is, by its label
    /// (`REF.NUMBER`) and the nearest centre.
    fn pad(&self, obstacle: usize, anchor: [f64; 2]) -> Option<KiCadInspectedPad> {
        let label = &self.board.obstacles.get(obstacle)?.label;
        let found = self
            .pads
            .iter()
            .filter(|pad| format!("{}.{}", pad.reference, pad.number) == *label)
            .min_by(|a, b| {
                distance_squared(a.at, anchor).total_cmp(&distance_squared(b.at, anchor))
            })?;
        Some(KiCadInspectedPad {
            reference: found.reference.clone(),
            number: found.number.clone(),
            uuid: found.uuid.clone(),
            at: [round(found.at[0]), round(found.at[1])],
            layers: layer_names(&self.layers, self.board.obstacles[obstacle].layers),
        })
    }

    /// The net's groups of terminals joined by its copper without pours.
    fn components(&self, net: core::NetId) -> Vec<KiCadInspectedComponent> {
        let description = &self.board.nets[net as usize];
        let groups = core::verify::copper_components(&self.board, net, &core::NetRoute::default());
        let mut order: Vec<usize> = Vec::new();
        for group in &groups {
            if !order.contains(group) {
                order.push(*group);
            }
        }
        let mut components: Vec<KiCadInspectedComponent> = order
            .iter()
            .map(|_| KiCadInspectedComponent {
                pads: Vec::new(),
                other: Vec::new(),
                largest: false,
            })
            .collect();
        for (terminal, group) in description.terminals.iter().zip(&groups) {
            let index = order.iter().position(|known| known == group).unwrap();
            let component = &mut components[index];
            match self.pad(terminal.pad, terminal.anchor) {
                Some(pad) => component.pads.push(pad),
                None => component.other.push(terminal.label.clone()),
            }
        }
        for merged in self
            .board
            .merged_pads
            .iter()
            .filter(|merged| merged.net == net)
        {
            let Some(group) = groups.get(merged.host) else {
                continue;
            };
            let index = order.iter().position(|known| known == group).unwrap();
            if let Some(pad) = self.pad(merged.pad, merged.anchor)
                && !components[index].pads.contains(&pad)
            {
                components[index].pads.push(pad);
            }
        }
        let sizes: Vec<usize> = order
            .iter()
            .map(|group| groups.iter().filter(|known| *known == group).count())
            .collect();
        if let Some(largest) =
            (0..sizes.len()).max_by(|a, b| sizes[*a].cmp(&sizes[*b]).then(b.cmp(a)))
        {
            components[largest].largest = true;
        }
        components
    }

    /// The groups (indices into `components(net)`) that KiCad's items of
    /// the net lie in which are no pad: a track or arc at `at` on the
    /// layer given, a via at `at` (`None` for the layer). `None` where no
    /// copper of the net is there, or where it joins no pad. Each item is
    /// a probe terminal on the copper found there, so it falls into that
    /// copper's group and joins nothing else.
    fn item_components(
        &mut self,
        net: core::NetId,
        items: &[([f64; 2], Option<usize>)],
    ) -> Vec<Option<usize>> {
        const REACH_MM: f64 = 0.005;
        let real = self.board.nets[net as usize].terminals.len();
        let groups = core::verify::copper_components(&self.board, net, &core::NetRoute::default());
        let mut order: Vec<usize> = Vec::new();
        for group in &groups {
            if !order.contains(group) {
                order.push(*group);
            }
        }
        let mut probed: Vec<Option<usize>> = vec![None; items.len()];
        let mut probes = Vec::new();
        for (index, (at, layer)) in items.iter().enumerate() {
            let found = self
                .board
                .obstacles
                .iter()
                .enumerate()
                .filter(|(_, obstacle)| {
                    obstacle.net == Some(net)
                        && obstacle.kind == core::ObstacleKind::Copper
                        && crate::board_router::pad_of_label(&obstacle.label).is_none()
                        && layer.is_none_or(|layer| obstacle.layers & (1 << layer) != 0)
                })
                .map(|(obstacle, shape)| (obstacle, shape.shape.distance_to_point(*at)))
                .filter(|(_, distance)| *distance <= REACH_MM)
                .min_by(|a, b| a.1.total_cmp(&b.1));
            if let Some((obstacle, _)) = found {
                probed[index] = Some(probes.len());
                probes.push(core::Terminal {
                    anchor: *at,
                    layers: self.board.obstacles[obstacle].layers,
                    pad: obstacle,
                    contact: None,
                    label: "probe".into(),
                });
            }
        }
        if probes.is_empty() {
            return probed;
        }
        self.board.nets[net as usize].terminals.extend(probes);
        let with_probes =
            core::verify::copper_components(&self.board, net, &core::NetRoute::default());
        self.board.nets[net as usize].terminals.truncate(real);
        probed
            .into_iter()
            .map(|probe| {
                let group = with_probes[real + probe?];
                order.iter().position(|known| *known == group)
            })
            .collect()
    }

    fn pour_layers(&self, net: core::NetId) -> Vec<String> {
        let mut layers: Vec<String> = self
            .board
            .planes
            .iter()
            .filter(|plane| plane.net == net)
            .filter_map(|plane| self.layers.names.get(plane.layer).cloned())
            .collect();
        layers.sort();
        layers.dedup();
        layers
    }
}

/// The router's hot spots with their tiles' objects. A run since
/// 2026-10-25 recorded them (`pads`, `nets`, `conflicted_nets` in
/// board-router.json, as the board stood when the router stopped); for an
/// older run they are read off the result board (the copper written,
/// after the finishing steps), the tile from the hot spot's centre and
/// the lattice's pitch (16 x 16 nodes).
fn linked_hot_spots(report: &serde_json::Value, result: &LoweredBoard) -> Vec<serde_json::Value> {
    let conflicted: BTreeSet<String> = report["diagnostics"]["conflicted_nets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|net| net.as_str().map(str::to_string))
        .collect();
    let half = report["grid_pitch_mm"].as_f64().map(|pitch| pitch * 8.0);
    report["diagnostics"]["hot_spots"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|spot| {
            let mut spot = spot.clone();
            if spot.get("pads").is_some() {
                spot["links_from"] = "board-router.json: the board as it stood when the router stopped".into();
                return spot;
            }
            let at = [spot["at"][0].as_f64(), spot["at"][1].as_f64()];
            let layer = spot["layer"]
                .as_str()
                .and_then(|name| result.layers.names.iter().position(|known| known == name));
            match (at, layer, half) {
                ([Some(x), Some(y)], Some(layer), Some(half)) => {
                    let bounds = [x - half, y - half, x + half, y + half];
                    let links = crate::board_router::hot_spot_links(
                        &result.board,
                        &[],
                        layer,
                        bounds,
                        &conflicted,
                    );
                    spot["box"] = serde_json::json!(bounds.map(round));
                    if let (Some(target), Ok(serde_json::Value::Object(links))) =
                        (spot.as_object_mut(), serde_json::to_value(&links))
                    {
                        target.extend(links);
                    }
                    spot["links_from"] = "the result board (board-router.json predates hot-spot links): the copper written, after the router's finishing steps".into();
                }
                _ => {
                    spot["links_from"] = serde_json::Value::Null;
                }
            }
            spot
        })
        .collect()
}

/// The layer of a DRC item that is a track or an arc (`Track [SDA] on
/// F.Cu, length 0.2983 mm`): `Some(Some(layer))`; a via (`Via [SDA] on
/// F.Cu - B.Cu`): `Some(None)`, every layer; anything else `None`.
fn copper_item_layer(description: &str, layers: &LayerTable) -> Option<Option<usize>> {
    if description.starts_with("Via ") {
        return Some(None);
    }
    if !description.starts_with("Track ") && !description.starts_with("Arc ") {
        return None;
    }
    let after = &description[description.find("] on ")? + 5..];
    let name = after.split([',', ' ']).next()?;
    let layer = layers.names.iter().position(|known| known == name)?;
    Some(Some(layer))
}

/// The net named in a DRC item's description (`Pad 52 [QSPI_SCLK] of
/// U3 on F.Cu`), as the board spells it (KiCad writes the text: `A"B`
/// for the board's `A\"B`, `escape`).
fn described_net(description: &str) -> Option<std::borrow::Cow<'_, str>> {
    let start = description.find('[')?;
    let end = description[start..].find(']')? + start;
    Some(escape(&description[start + 1..end]))
}

/// The net's rules on the run's project with their sources.
fn inspected_rules(
    config: Option<&KiCadBoardRouterConfig>,
    net: &str,
    overrides: &[serde_json::Value],
    emitted: &KiCadNetCopper,
) -> KiCadInspectedRules {
    let mut rules = KiCadInspectedRules {
        emitted_track_widths_mm: emitted.track_widths_mm.clone(),
        ..Default::default()
    };
    rules.overrides = overrides
        .iter()
        .filter(|item| {
            let nets: Vec<&str> = item["nets"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(serde_json::Value::as_str)
                .collect();
            nets.is_empty()
                || nets.iter().any(|pattern| {
                    wildcard_match(pattern, net)
                        || wildcard_match(pattern.trim_start_matches('/'), net)
                })
        })
        .cloned()
        .collect();
    let Some(config) = config else {
        rules.track_width_source = "unknown: the run's project does not resolve".into();
        rules.clearance_source = rules.track_width_source.clone();
        return rules;
    };
    let resolved = config
        .connection_rules
        .get(net)
        .or(config.default_rules.as_ref());
    // In nanometres, as KiCad holds them (0.19999999999999998 is 0.2).
    let nanometres = |value: f64| (value * 1.0e6).round() / 1.0e6;
    rules.track_width_mm = resolved.map(|rules| nanometres(rules.trace_width_mm));
    rules.clearance_mm = resolved.map(|rules| nanometres(rules.clearance_mm));
    rules.via_diameter_mm = resolved.map(|rules| nanometres(rules.via_size_mm));
    rules.via_drill_mm = resolved.map(|rules| nanometres(rules.via_drill_mm));
    let default_report = KiCadProjectRulesReport::default();
    let report = config.project_rules.as_ref().unwrap_or(&default_report);
    rules.classes = report
        .net_classes
        .get(net)
        .cloned()
        .unwrap_or_else(|| vec!["Default".into()]);
    rules.project_minimums = report.minimums.clone();
    rules.custom_rules_file = report.custom_rules_file.clone();
    // The class's own value of a quantity (the first class stating it)
    // and the source of the resolved one.
    let source = |index: usize, resolved: Option<f64>| -> (Option<f64>, String) {
        let stated = rules.classes.iter().find_map(|class| {
            report
                .class_values
                .get(class)
                .and_then(|values| values[index])
                .map(|value| (class.clone(), value))
        });
        let Some(resolved) = resolved else {
            return (None, "unknown".into());
        };
        let (minimum, minimum_source) =
            report
                .minimums
                .as_ref()
                .map_or((0.0, String::new()), |minimums| {
                    if index == 0 {
                        (minimums.track_width_mm, minimums.track_width_source.clone())
                    } else {
                        (minimums.clearance_mm, minimums.clearance_source.clone())
                    }
                });
        let close = |a: f64, b: f64| (a - b).abs() < 1.0e-9;
        let below = stated
            .as_ref()
            .and_then(|(_, value)| (*value < resolved - 1.0e-9).then_some(*value));
        let text = match &stated {
            Some((class, value)) if close(*value, resolved) => {
                format!("class {class} (net_settings.classes in the .kicad_pro)")
            }
            _ if close(minimum, resolved) => format!("the board's minimum: {minimum_source}"),
            Some(_) if report.custom_rules_file.is_some() => format!(
                "a custom rule ({})",
                report.custom_rules_file.as_deref().unwrap_or_default()
            ),
            None => "no class of the project states it: the board's most common width (vias: size and drill), else KiCad's defaults".into(),
            Some((class, value)) => format!(
                "class {class} states {value} mm; raised by the board's floors (min_connection or a custom rule)"
            ),
        };
        (below, text)
    };
    let (below, text) = source(0, rules.track_width_mm);
    rules.class_track_width_mm = below;
    rules.track_width_source = text;
    let (below, text) = source(1, rules.clearance_mm);
    rules.class_clearance_mm = below;
    rules.clearance_source = text;
    rules
}

/// How near a hot spot is to a pad of a net to be listed with it (at
/// most five, the most contested first).
const HOT_SPOT_REACH_MM: f64 = 3.0;

/// KiCad's DRC reports at most this many items of a kind (KiCad 10).
const KICAD_REPORT_CAP: usize = 499;

fn explanation(counts: &KiCadNetCounts, pours: bool, capped: bool) -> String {
    let mut parts = vec![format!(
        "the result board's copper (pours not counted) leaves {} group{} of pads, {} link{} missing",
        counts.components,
        if counts.components == 1 { "" } else { "s" },
        counts.missing_links,
        if counts.missing_links == 1 { "" } else { "s" }
    )];
    if let (Some(terminals), Some(unconnected)) =
        (counts.router_terminals, counts.router_unconnected_terminals)
    {
        parts.push(format!(
            "the router counts {unconnected} of its {terminals} terminals unconnected (its search's state at the end: terminals not joined to the net's routed tree, pads it merged before routing as one; not a count of the copper written)"
        ));
    }
    match counts.native_pairs {
        Some(pairs) => {
            parts.push(format!(
                "KiCad reports {pairs} unconnected pair{} (one per missing link after its zone fill)",
                if pairs == 1 { "" } else { "s" }
            ));
            if capped && pairs < counts.missing_links {
                parts.push(format!(
                    "KiCad's report stops at {KICAD_REPORT_CAP} unconnected items, so its count for this net may be short"
                ));
            } else if pairs < counts.missing_links && pours {
                parts.push(format!(
                    "the net's zone fill joins {} of the groups",
                    counts.missing_links - pairs
                ));
            } else if pairs != counts.missing_links {
                parts.push(
                    "KiCad's pairs and the missing links differ: KiCad's connectivity decides"
                        .into(),
                );
            }
        }
        None => parts.push("no KiCad DRC report was read (--native-check runs one)".into()),
    }
    parts.join("; ")
}

/// A pad named by `--terminal`: its reference, number and net.
type NamedTerminal = (String, String, String);

/// What `inspect_routing_run` reads from a run before it looks at a net.
struct RunInputs {
    files: KiCadRunFiles,
    /// board-router.json.
    report: serde_json::Value,
    verdict: Option<serde_json::Value>,
    /// KiCad's DRC report: where it was read from and what it holds.
    drc_path: Option<PathBuf>,
    drc: Option<serde_json::Value>,
    config: Option<KiCadBoardRouterConfig>,
    reference: Option<LoweredBoard>,
    /// Whose placement the reference's routes are on (with `--reference`).
    reference_placement: Option<KiCadReferencePlacement>,
}

/// Reads the run's files, the result board (returned apart: probing its
/// groups borrows it mutably) and the reference board, in the order their
/// errors are reported.
fn read_run_inputs(inspection: &KiCadRunInspection) -> Result<(RunInputs, LoweredBoard), String> {
    let files = run_files(&inspection.run)?;
    let directory = &files.directory;
    let board_id = &files.board_id;
    let report = read_value(&directory.join("board-router.json"))?;
    let verdict = [
        directory.join("verdict.json"),
        inspection.run.join("verdict.json"),
    ]
    .iter()
    .find_map(|path| read_value(path).ok());
    let drc_path = inspection
        .drc
        .clone()
        .or_else(|| Some(directory.join("drc.json")).filter(|path| path.is_file()));
    let drc = drc_path.as_ref().map(|path| read_value(path)).transpose()?;
    let result = LoweredBoard::read(directory, board_id)?;
    let config = resolve_project_rules(
        &directory.join(format!("{board_id}.kicad_pro")),
        &directory.join(format!("{board_id}.kicad_pcb")),
    )
    .ok();
    let reference = match &inspection.reference {
        Some(reference) => {
            if !reference.join(format!("{board_id}.kicad_pcb")).is_file() {
                return Err(crate::verdict::input_error(format!(
                    "{} is missing: the reference project holds no board {board_id}",
                    reference.join(format!("{board_id}.kicad_pcb")).display()
                )));
            }
            Some(LoweredBoard::read(reference, board_id)?)
        }
        None => None,
    };
    // A layout's report: the run directory's (its board is in result/).
    let layout_path = [
        inspection.run.join("board-layout.json"),
        directory.join("board-layout.json"),
    ]
    .into_iter()
    .find(|path| path.is_file());
    let layout = layout_path
        .as_ref()
        .and_then(|path| read_value(path).ok().map(|value| (value, path.clone())));
    let reference_placement = match (&inspection.reference, &reference) {
        (Some(project), Some(board)) => Some(reference_placement(
            &std::path::absolute(project)
                .unwrap_or_else(|_| project.clone())
                .display()
                .to_string(),
            &board.pcb,
            &result.pcb,
            layout.as_ref().map(|(value, path)| (value, path.as_path())),
            verdict.as_ref(),
        )),
        _ => None,
    };
    let inputs = RunInputs {
        files,
        report,
        verdict,
        drc_path,
        drc,
        config,
        reference,
        reference_placement,
    };
    Ok((inputs, result))
}

/// The router's nets of board-router.json by connection name.
fn router_nets_of(report: &serde_json::Value) -> BTreeMap<String, serde_json::Value> {
    report["nets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|net| Some((net["connection"].as_str()?.to_string(), net.clone())))
        .collect()
}

/// The pad `--terminal` named, with its net.
fn named_terminal(
    inspection: &KiCadRunInspection,
    result: &LoweredBoard,
) -> Result<Option<NamedTerminal>, String> {
    let Some(text) = &inspection.terminal else {
        return Ok(None);
    };
    let (reference, number) = text.split_once(':').ok_or_else(|| {
        crate::verdict::input_error(format!("--terminal {text}: give REF:PAD (U3:52)"))
    })?;
    let pad = result
        .pads
        .iter()
        .find(|pad| pad.reference == reference && pad.number == number)
        .ok_or_else(|| {
            crate::verdict::input_error(format!("--terminal {text}: no such pad on the board"))
        })?;
    let net = pad.net.clone().ok_or_else(|| {
        crate::verdict::input_error(format!("--terminal {text}: the pad has no net"))
    })?;
    Ok(Some((reference.to_string(), number.to_string(), net)))
}

/// The nets to inspect: the terminal's, the named one, or every net the
/// router left open.
fn selected_nets(
    inspection: &KiCadRunInspection,
    result: &LoweredBoard,
    router_nets: &BTreeMap<String, serde_json::Value>,
    terminal: &Option<NamedTerminal>,
) -> Result<Vec<String>, String> {
    Ok(if let Some((_, _, net)) = terminal {
        vec![net.clone()]
    } else if let Some(net) = &inspection.net {
        let net = result.names.normalize(net).to_string();
        if result.net_id(&net).is_none() && !router_nets.contains_key(&net) {
            return Err(crate::verdict::input_error(format!(
                "--net {net}: the board has no routable net of that name"
            )));
        }
        vec![net]
    } else {
        router_nets
            .iter()
            .filter(|(_, net)| is_open_net(net))
            .map(|(name, _)| name.clone())
            .collect()
    })
}

/// Whether the router left the net open: not routed and two terminals or
/// more.
fn is_open_net(net: &serde_json::Value) -> bool {
    net["status"].as_str() != Some("routed") && net["terminals"].as_u64().unwrap_or(0) >= 2
}

/// KiCad's unconnected pairs by net, and how many name no net.
fn native_pairs_by_net(
    drc: Option<&serde_json::Value>,
    names: &NetNames,
) -> (BTreeMap<String, Vec<KiCadNativePair>>, usize) {
    let mut native_by_net: BTreeMap<String, Vec<KiCadNativePair>> = BTreeMap::new();
    let mut native_unattributed = 0;
    if let Some(drc) = drc {
        for pair in drc["unconnected_items"].as_array().into_iter().flatten() {
            let items: Vec<KiCadNativeItem> = pair["items"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|item| KiCadNativeItem {
                    description: item["description"].as_str().unwrap_or("").to_string(),
                    at: [
                        item["pos"]["x"].as_f64().unwrap_or(0.0),
                        item["pos"]["y"].as_f64().unwrap_or(0.0),
                    ],
                    uuid: item["uuid"].as_str().map(str::to_string),
                })
                .collect();
            match items
                .iter()
                .find_map(|item| described_net(&item.description))
            {
                Some(net) => native_by_net
                    .entry(names.normalize(&net).to_string())
                    .or_default()
                    .push(KiCadNativePair {
                        items,
                        components: Vec::new(),
                    }),
                None => native_unattributed += 1,
            }
        }
    }
    (native_by_net, native_unattributed)
}

/// What every net's diagnosis reads: the run, both boards, the router's
/// report and the arguments.
struct NetInputs<'a> {
    run: &'a RunInputs,
    router_nets: &'a BTreeMap<String, serde_json::Value>,
    terminal: &'a Option<NamedTerminal>,
    /// board-router.json's rule overrides.
    overrides: &'a [serde_json::Value],
    hot_spots: &'a [serde_json::Value],
    /// KiCad's report holds as many unconnected items as it keeps.
    native_capped: bool,
}

/// KiCad's pairs of the net, each item placed in a group of pads.
/// `None` without a DRC report; takes the net's pairs out of
/// `native_by_net`.
fn net_native_pairs(
    name: &str,
    inputs: &NetInputs,
    result: &mut LoweredBoard,
    components: &[KiCadInspectedComponent],
    native_by_net: &mut BTreeMap<String, Vec<KiCadNativePair>>,
) -> Option<Vec<KiCadNativePair>> {
    let native_pairs = inputs.run.drc.as_ref().map(|_| {
        let mut pairs = native_by_net.remove(name).unwrap_or_default();
        for pair in &mut pairs {
            pair.components = pair
                .items
                .iter()
                .map(|item| {
                    components.iter().position(|component| {
                        component.pads.iter().any(|pad| {
                            item.uuid.is_some() && pad.uuid == item.uuid
                                || distance_squared(pad.at, item.at) < 1.0e-6
                                    && item.description.starts_with("Pad ")
                        })
                    })
                })
                .collect();
        }
        pairs
    });
    // KiCad's items that are a track, an arc or a via: the group of
    // the copper they lie on.
    native_pairs.map(|mut pairs| {
        let Some(id) = result.net_id(name) else {
            return pairs;
        };
        let mut wanted = Vec::new();
        for (pair_index, pair) in pairs.iter().enumerate() {
            for (item_index, item) in pair.items.iter().enumerate() {
                if pair.components[item_index].is_some() {
                    continue;
                }
                if let Some(layer) = copper_item_layer(&item.description, &result.layers) {
                    wanted.push(((pair_index, item_index), (item.at, layer)));
                }
            }
        }
        let probes: Vec<([f64; 2], Option<usize>)> =
            wanted.iter().map(|(_, probe)| *probe).collect();
        let found = result.item_components(id, &probes);
        for (((pair_index, item_index), _), group) in wanted.into_iter().zip(found) {
            pairs[pair_index].components[item_index] = group;
        }
        pairs
    })
}

/// The reference board's route of the net: its groups and whether they
/// join.
fn reference_route(
    name: &str,
    reference: &LoweredBoard,
    reference_placement: Option<&KiCadReferencePlacement>,
) -> KiCadReferenceRoute {
    let copper = net_copper(&reference.pcb, &reference.names, name);
    let (groups, pour_layers) = reference.net_id(name).map_or((0, Vec::new()), |id| {
        (reference.components(id).len(), reference.pour_layers(id))
    });
    let (connected, connected_reason) = if groups <= 1 {
        (Some(true), None)
    } else if pour_layers.is_empty() {
        (Some(false), None)
    } else {
        (
            None,
            Some(format!(
                "its tracks, arcs and vias leave {groups} groups; its zone on {} may join them (zone fill is not modelled here: KiCad's DRC of the reference decides)",
                pour_layers.join(", ")
            )),
        )
    };
    KiCadReferenceRoute {
        placement: reference_placement
            .map_or_else(String::new, |placement| placement.placement.clone()),
        connected,
        connected_reason,
        components: groups,
        pour_layers,
        copper,
    }
}

/// The router's hot spots within reach of a pad of the net, at most five.
fn hot_spots_near(
    hot_spots: &[serde_json::Value],
    components: &[KiCadInspectedComponent],
) -> Vec<serde_json::Value> {
    let pads: Vec<&KiCadInspectedPad> = components
        .iter()
        .flat_map(|component| &component.pads)
        .collect();
    hot_spots
        .iter()
        .enumerate()
        .filter_map(|(index, spot)| {
            let at = [spot["at"][0].as_f64()?, spot["at"][1].as_f64()?];
            let nearest = pads
                .iter()
                .map(|pad| distance_squared(pad.at, at).sqrt())
                .fold(f64::INFINITY, f64::min);
            (nearest <= HOT_SPOT_REACH_MM).then(|| {
                serde_json::json!({
                    "hot_spot": index,
                    "at": spot["at"],
                    "layer": spot["layer"],
                    "history": spot["history"],
                    "nearest_pad_mm": round(nearest),
                    "conflicted_nets": spot["conflicted_nets"],
                })
            })
        })
        .take(5)
        .collect()
}

/// A net with a zone: keeps only the groups KiCad's pairs name and the
/// terminal's (its fill joins the others or not, unmodelled here), and
/// renumbers the pairs' groups. Returns how many groups it dropped.
fn omit_unnamed_groups(
    components: &mut Vec<KiCadInspectedComponent>,
    native_pairs: &mut Option<Vec<KiCadNativePair>>,
    terminal: &Option<NamedTerminal>,
) -> usize {
    let keep: Vec<bool> = (0..components.len())
        .map(|index| {
            native_pairs
                .iter()
                .flatten()
                .any(|pair| pair.components.contains(&Some(index)))
                || terminal.as_ref().is_some_and(|(reference, number, _)| {
                    components[index]
                        .pads
                        .iter()
                        .any(|pad| &pad.reference == reference && &pad.number == number)
                })
        })
        .collect();
    let mut renumbered = vec![None; components.len()];
    let mut next = 0;
    for (index, kept) in keep.iter().enumerate() {
        if *kept {
            renumbered[index] = Some(next);
            next += 1;
        }
    }
    for pair in native_pairs.iter_mut().flatten() {
        for component in &mut pair.components {
            *component = component.and_then(|index| renumbered[index]);
        }
    }
    let omitted = components.len() - next;
    let mut index = 0;
    components.retain(|_| {
        index += 1;
        keep[index - 1]
    });
    omitted
}

/// The group holding the `--terminal` pad.
fn terminal_entry(
    terminal: &Option<NamedTerminal>,
    components: &[KiCadInspectedComponent],
) -> Option<serde_json::Value> {
    terminal.as_ref().map(|(reference, number, _)| {
        let component = components.iter().position(|component| {
            component
                .pads
                .iter()
                .any(|pad| &pad.reference == reference && &pad.number == number)
        });
        serde_json::json!({
            "ref": reference,
            "number": number,
            "component": component,
            "in_largest": component.map(|index| components[index].largest),
        })
    })
}

/// The diagnosis of one net; takes the net's KiCad pairs out of
/// `native_by_net`.
fn diagnose_net(
    name: &str,
    inputs: &NetInputs,
    result: &mut LoweredBoard,
    native_by_net: &mut BTreeMap<String, Vec<KiCadNativePair>>,
) -> KiCadNetDiagnosis {
    let run = inputs.run;
    let router = inputs.router_nets.get(name);
    let status = router
        .and_then(|net| net["status"].as_str())
        .unwrap_or("not in board-router.json")
        .to_string();
    let (components, pour_layers) = match result.net_id(name) {
        Some(id) => (result.components(id), result.pour_layers(id)),
        None => (Vec::new(), Vec::new()),
    };
    let native_pairs = net_native_pairs(name, inputs, result, &components, native_by_net);
    let counts = KiCadNetCounts {
        router_terminals: router
            .and_then(|net| net["terminals"].as_u64())
            .map(|n| n as usize),
        router_unconnected_terminals: router
            .and_then(|net| net["unconnected_terminals"].as_u64())
            .map(|n| n as usize),
        components: components.len(),
        missing_links: components.len().saturating_sub(1),
        native_pairs: native_pairs.as_ref().map(Vec::len),
        explanation: String::new(),
    };
    let counts = KiCadNetCounts {
        explanation: explanation(&counts, !pour_layers.is_empty(), inputs.native_capped),
        ..counts
    };
    let copper = net_copper(&result.pcb, &result.names, name);
    let rules = inspected_rules(run.config.as_ref(), name, inputs.overrides, &copper);
    let reference_route = run
        .reference
        .as_ref()
        .map(|reference| reference_route(name, reference, run.reference_placement.as_ref()));
    let hot_spots_near = hot_spots_near(inputs.hot_spots, &components);
    let dead_pads: Vec<serde_json::Value> = run.report["diagnostics"]["dead_pads"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|pad| pad["net"].as_str() == Some(name))
        .cloned()
        .collect();
    let mut components = components;
    let mut native_pairs = native_pairs;
    let components_omitted = if pour_layers.is_empty() {
        0
    } else {
        omit_unnamed_groups(&mut components, &mut native_pairs, inputs.terminal)
    };
    let terminal_entry = terminal_entry(inputs.terminal, &components);
    KiCadNetDiagnosis {
        net: name.to_string(),
        status,
        counts,
        components,
        components_basis: "the result board's pads, tracks, arcs and vias joined where their copper overlaps on a shared layer (KiCad's connectivity without zone fill)".into(),
        pour_layers,
        components_omitted,
        native_pairs,
        copper,
        rules,
        reference: reference_route,
        dead_pads,
        hot_spots_near,
        terminal: terminal_entry,
        access: serde_json::json!({"status": "unknown", "reason": "not measured: this command runs no access probe"}),
        cause: serde_json::json!({"status": "unknown", "reason": "not measured: this command reads the run's outputs; a cause needs a discriminating experiment"}),
    }
}

/// The checks' `native`: KiCad's own check of the result board.
fn native_check_block(
    drc: Option<&serde_json::Value>,
    drc_path: Option<&Path>,
    router_nets: &BTreeMap<String, serde_json::Value>,
    native_by_net: &BTreeMap<String, Vec<KiCadNativePair>>,
    native_unattributed: usize,
    native_capped: bool,
) -> serde_json::Value {
    let tally = drc.map(crate::verdict::tally_drc);
    match (drc, &tally) {
        (Some(_), Some(tally)) => serde_json::json!({
            "status": if tally.errors.is_empty() && tally.unconnected == 0 { "passed" } else { "failed" },
            "report": drc_path.map(|path| path.display().to_string()),
            "unconnected_pairs": tally.unconnected,
            "unconnected_pairs_without_net": native_unattributed,
            "capped": native_capped,
            "pairs_on_nets_not_inspected": native_by_net
                .iter()
                .map(|(net, pairs)| (net.clone(), serde_json::json!({
                    "pairs": pairs.len(),
                    "router_status": router_nets.get(net).and_then(|net| net["status"].as_str()),
                })))
                .collect::<serde_json::Map<String, serde_json::Value>>(),
            "errors": tally.errors,
        }),
        _ => serde_json::json!({
            "status": "not_run",
            "reason": "the run has no drc.json (KiCad's check was skipped); --native-check runs one",
        }),
    }
}

/// The report's `checks`: the router's counts, KiCad's (`native`), and
/// this command's groups.
fn run_checks(
    report: &serde_json::Value,
    router_nets: &BTreeMap<String, serde_json::Value>,
    native: serde_json::Value,
    nets: &[KiCadNetDiagnosis],
) -> serde_json::Value {
    let open_nets = router_nets.values().filter(|net| is_open_net(net)).count();
    serde_json::json!({
        "router": {
            "routable_connections": report["routable_connections"],
            "routed_connections": report["routed_connections"],
            "open_nets": open_nets,
            "unconnected_terminals": report["unconnected_terminals"],
            "counted": "board-router.json: per net the router's terminals its search left unjoined (its own state, pads merged before routing as one terminal)",
        },
        "native": native,
        "components": {
            "nets": nets.len(),
            "groups": nets.iter().map(|net| net.counts.components).sum::<usize>(),
            "missing_links": nets.iter().map(|net| net.counts.missing_links).sum::<usize>(),
            "counted": "this command, on the result board's copper without pours: groups of pads per net inspected and the links missing between them",
        },
    })
}

/// The report's `attempts`: the ladder's ledger, or why there is none.
fn run_attempts(report: &serde_json::Value) -> serde_json::Value {
    if report["attempts"].is_array() {
        serde_json::json!({
            "ledger": report["attempts"],
            "selected": report["attempts"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|attempt| attempt["selected"] == true),
            "work_seconds": report["work_seconds"],
            "work_bound_seconds": report["work_bound_seconds"],
            "negotiation_stop": report["negotiation_stop"],
            "skipped": report["skipped"],
            "cut": report["cut"],
            "attempts_failed": report["attempts_failed"],
        })
    } else {
        serde_json::json!({
            "ledger": null,
            "reason": "not recorded: board-router.json predates the attempt ledger (2026-10-23) or the run was no route-mode ladder",
            "work_seconds": report["work_seconds"],
            "skipped": report["skipped"],
            "cut": report["cut"],
        })
    }
}

/// The report's `run`: which files it read.
fn run_block(inspection: &KiCadRunInspection, inputs: &RunInputs) -> serde_json::Value {
    let directory = &inputs.files.directory;
    let board_id = &inputs.files.board_id;
    let path_text = |path: &Path| {
        std::path::absolute(path)
            .unwrap_or_else(|_| path.to_path_buf())
            .display()
            .to_string()
    };
    serde_json::json!({
        "directory": path_text(&inspection.run),
        "board_id": board_id,
        "board": path_text(&directory.join(format!("{board_id}.kicad_pcb"))),
        "board_sha256": file_sha256(&directory.join(format!("{board_id}.kicad_pcb"))).ok(),
        "report": path_text(&directory.join("board-router.json")),
        "drc": inputs.drc_path.as_deref().map(path_text),
        "reference": inspection.reference.as_deref().map(path_text),
        "verdict": inputs.verdict.as_ref().map(|verdict| serde_json::json!({
            "status": verdict["status"],
            "reason": verdict["reason"],
            "kicad_checked": verdict["kicad_checked"],
        })),
    })
}

/// The diagnosis of a run (`inspect-routing-run`).
pub fn inspect_routing_run(inspection: &KiCadRunInspection) -> Result<KiCadRunDiagnosis, String> {
    let (mut run, mut result) = read_run_inputs(inspection)?;
    let router_nets = router_nets_of(&run.report);
    let terminal = named_terminal(inspection, &result)?;
    let selected = selected_nets(inspection, &result, &router_nets, &terminal)?;
    let (mut native_by_net, native_unattributed) =
        native_pairs_by_net(run.drc.as_ref(), &result.names);
    let native_capped = run.drc.as_ref().is_some_and(|drc| {
        drc["unconnected_items"].as_array().map_or(0, Vec::len) >= KICAD_REPORT_CAP
    });
    let overrides: Vec<serde_json::Value> = run.report["rules"]["overrides"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let hot_spots = linked_hot_spots(&run.report, &result);
    let nets: Vec<KiCadNetDiagnosis> = {
        let inputs = NetInputs {
            run: &run,
            router_nets: &router_nets,
            terminal: &terminal,
            overrides: &overrides,
            hot_spots: &hot_spots,
            native_capped,
        };
        selected
            .iter()
            .map(|name| diagnose_net(name, &inputs, &mut result, &mut native_by_net))
            .collect()
    };
    let native = native_check_block(
        run.drc.as_ref(),
        run.drc_path.as_deref(),
        &router_nets,
        &native_by_net,
        native_unattributed,
        native_capped,
    );
    let checks = run_checks(&run.report, &router_nets, native, &nets);
    let reference_placement = run.reference_placement.take();
    Ok(KiCadRunDiagnosis {
        schema_version: 1,
        units: "mm".into(),
        run: run_block(inspection, &run),
        checks,
        attempts: run_attempts(&run.report),
        hot_spots,
        reference: reference_placement,
        nets,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOARD: &str = r#"(kicad_pcb (version 20241229) (generator "pcbnew")
  (general (thickness 1.6)) (paper "A4")
  (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
  (gr_rect (start 0 0) (end 20 20) (stroke (width 0.05) (type default)) (layer "Edge.Cuts"))
  (footprint "test" (layer "F.Cu") (at 10 10)
    (property "Reference" "U1" (at 0 -2) (layer "F.SilkS")
      (effects (font (size 1 1) (thickness 0.15))))
    (pad "1" smd rect (at -0.5 0) (size 0.3 1) (layers "F.Cu") (net "A"))
    (pad "2" smd rect (at 0.5 0) (size 0.3 1) (layers "F.Cu") (net "B")))
  (footprint "test" (layer "F.Cu") (at 16 10)
    (property "Reference" "J1" (at 0 -2) (layer "F.SilkS")
      (effects (font (size 1 1) (thickness 0.15))))
    (pad "1" smd circle (at 0 0) (size 1 1) (layers "F.Cu") (net "A"))
    (pad "2" smd circle (at 0 2) (size 1 1) (layers "F.Cu") (net "B")))
  (segment (start 10.5 10) (end 16 12) (width 0.2) (layer "F.Cu") (net "B"))
  (segment (start 16 10) (end 17 10) (width 0.15) (layer "F.Cu") (net "A")))
"#;

    /// The reference's routes are said to be on the designer's own
    /// placement when a part of the run's board lies elsewhere on it (a
    /// layout's), with the layout's own routes as the evidence for this
    /// placement; on this placement when every part is where the reference
    /// has it (route mode's).
    #[test]
    fn the_reference_block_says_whose_placement_it_routes() {
        let reference = parse(BOARD).unwrap();
        let same = reference_placement("/designer", &reference, &reference, None, None);
        assert_eq!(same.placement, "this placement");
        assert_eq!((same.parts_in_place, same.parts), (2, 2));
        assert!(
            same.connected_means
                .starts_with("the designer's route on this placement: every part"),
            "{}",
            same.connected_means
        );
        assert!(same.evidence_for_this_placement.is_none());
        assert_eq!(same.reference_tracks, 2);
        // An unrouted reference (a stripped task input) shows nothing.
        let bare = parse(&BOARD.replace("(segment ", "(gr_line ")).unwrap();
        let unrouted = reference_placement("/designer", &bare, &reference, None, None);
        assert_eq!(unrouted.reference_tracks, 0);
        assert!(
            unrouted
                .connected_means
                .starts_with("the reference board has no tracks or arcs"),
            "{}",
            unrouted.connected_means
        );
        let moved = parse(&BOARD.replace("(at 16 10)", "(at 15 11)")).unwrap();
        let layout = serde_json::json!({
            "first_unconnected_terminals": 8, "first_vias": 314, "first_length_mm": 10480.8,
            "moves": [{"kept": false}, {"kept": true}],
            "move_descent": {"confirmed": 6, "kept": 2, "open_before": 8, "open_after": 4, "parts_moved": ["C16"]},
            "routed": {"unconnected_terminals": 4},
        });
        let verdict = serde_json::json!({"kicad_unconnected_items": 4});
        let other = reference_placement(
            "/designer",
            &reference,
            &moved,
            Some((&layout, Path::new("/run/board-layout.json"))),
            Some(&verdict),
        );
        assert_eq!(other.placement, "the designer's own placement");
        assert_eq!((other.parts_in_place, other.parts), (1, 2));
        assert_eq!(
            other.connected_means,
            "the designer's route on the designer's own placement: 1 of the 2 parts of this run's board lie elsewhere on the reference, so reference.connected shows that a net routes under these rules there, not on this placement, and no route of the designer's on this placement exists; the evidence for this placement is the layout's own routes (evidence_for_this_placement: its first route and its move descent)"
        );
        let evidence = other.evidence_for_this_placement.unwrap();
        assert_eq!(evidence["first_route_unconnected_terminals"], 8);
        assert_eq!(evidence["moves"]["routed"], 6);
        assert_eq!(evidence["moves"]["open_after"], 4);
        assert_eq!(evidence["kicad_unconnected_items"], 4);
        // A report before the move descent's record: its moves counted.
        let mut old = layout.clone();
        old.as_object_mut().unwrap().remove("move_descent");
        let other = reference_placement(
            "/designer",
            &reference,
            &moved,
            Some((&old, Path::new("/run/board-layout.json"))),
            None,
        );
        let evidence = other.evidence_for_this_placement.unwrap();
        assert_eq!(
            evidence["moves"],
            serde_json::json!({"routed": 2, "kept": 1})
        );
    }

    /// A hot spot's tile names the pads in it, the nets whose copper
    /// crosses it on its layer (the board's own and the router's) and
    /// those of them still in conflict; on the other layer only the
    /// router's track of A.
    #[test]
    fn a_hot_spot_names_its_pads_and_nets() {
        let directory =
            std::env::temp_dir().join(format!("pcb-maker-hot-spot-links-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("t.kicad_pcb"), BOARD).unwrap();
        let lowered = LoweredBoard::read(&directory, "t").unwrap();
        fs::remove_dir_all(&directory).unwrap();
        let (board, table) = (&lowered.board, &lowered.layers);
        let layer = |name: &str| table.names.iter().position(|known| known == name).unwrap();
        let a = board.nets.iter().position(|net| net.name == "A").unwrap();
        let mut routes = vec![core::NetRoute::default(); board.nets.len()];
        routes[a].segments.push(core::Segment {
            layer: layer("B.Cu"),
            start: [9.0, 12.0],
            end: [10.0, 8.0],
            width: 0.2,
        });
        let conflicted = BTreeSet::from(["A".to_string()]);
        let front = crate::board_router::hot_spot_links(
            board,
            &routes,
            layer("F.Cu"),
            [9.0, 9.0, 11.0, 11.0],
            &conflicted,
        );
        let pads: Vec<(String, String, Option<String>)> = front
            .pads
            .iter()
            .map(|pad| (pad.reference.clone(), pad.number.clone(), pad.net.clone()))
            .collect();
        assert_eq!(
            pads,
            [
                ("U1".into(), "1".into(), Some("A".into())),
                ("U1".into(), "2".into(), Some("B".into()))
            ]
        );
        assert_eq!(front.pads[0].at, [9.5, 10.0]);
        assert_eq!(front.pads_in_tile, 2);
        assert_eq!(front.nets, ["B"]);
        assert_eq!(front.conflicted_nets, ["A"]);
        let back = crate::board_router::hot_spot_links(
            board,
            &routes,
            layer("B.Cu"),
            [9.0, 9.0, 11.0, 11.0],
            &conflicted,
        );
        assert!(back.pads.is_empty());
        assert_eq!(back.nets, ["A"]);
        assert_eq!(back.conflicted_nets, ["A"]);
        // A tile with nothing in it.
        let empty = crate::board_router::hot_spot_links(
            board,
            &routes,
            layer("F.Cu"),
            [1.0, 1.0, 2.0, 2.0],
            &conflicted,
        );
        assert_eq!(empty, crate::board_router::KiCadHotSpotLinks::default());
    }

    /// An override the run allowed: the widths written on its nets, how
    /// many are below the minimum, and KiCad's findings of its kind on its
    /// nets; a clearance has no widths.
    #[test]
    fn an_allowed_override_reports_what_was_written_and_found() {
        let directory = std::env::temp_dir().join(format!(
            "pcb-maker-override-outcomes-{}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).unwrap();
        let board = directory.join("t.kicad_pcb");
        fs::write(
            &board,
            BOARD.replace(
                "(net \"A\")))\n",
                "(net \"A\"))\n  (segment (start 17 10) (end 17 11) (width 0.2) (layer \"F.Cu\") (net \"A\")))\n",
            ),
        )
        .unwrap();
        let finding = |kind: &str, net: &str| {
            serde_json::json!({"type": kind, "severity": "error",
                "items": [{"description": format!("Track [{net}] on F.Cu, length 1.0000 mm"), "pos": {"x": 16.0, "y": 10.0}}]})
        };
        let drc = serde_json::json!({"violations": [
            finding("track_width", "A"), finding("track_width", "B"), finding("clearance", "A")]});
        let width = crate::project_rules::KiCadRuleOverride {
            source: "net_classes[Narrow]".into(),
            quantity: "track_width".into(),
            requested_mm: 0.15,
            project_minimum_mm: 0.2,
            project_source: "board.design_settings.rules.min_track_width in the .kicad_pro".into(),
            nets: vec!["A".into()],
        };
        let clearance = crate::project_rules::KiCadRuleOverride {
            source: "net_classes[Narrow]".into(),
            quantity: "clearance".into(),
            requested_mm: 0.1,
            project_minimum_mm: 0.15,
            ..width.clone()
        };
        let outcomes = override_outcomes(&[width, clearance], &board, Some(&drc));
        assert_eq!(outcomes[0].routed_with_mm, 0.15);
        assert_eq!(outcomes[0].native_required_mm, 0.2);
        assert_eq!(outcomes[0].emitted_track_widths_mm, Some(vec![0.15, 0.2]));
        assert_eq!(outcomes[0].emitted_below_minimum, Some(1));
        assert_eq!(outcomes[0].native_findings, Some(1));
        assert_eq!(outcomes[1].emitted_track_widths_mm, None);
        assert_eq!(outcomes[1].native_findings, Some(1));
        // KiCad did not check: no findings, not zero.
        let unchecked = override_outcomes(&outcomes_input(), &board, None);
        assert_eq!(unchecked[0].native_findings, None);
        fs::remove_dir_all(directory).unwrap();
    }

    fn outcomes_input() -> Vec<crate::project_rules::KiCadRuleOverride> {
        vec![crate::project_rules::KiCadRuleOverride {
            source: "default_rules".into(),
            quantity: "track_width".into(),
            requested_mm: 0.15,
            project_minimum_mm: 0.2,
            project_source: "x".into(),
            nets: Vec::new(),
        }]
    }

    #[test]
    fn kicad_items_name_their_copper_layer() {
        let pcb = parse(BOARD).unwrap();
        let table = LayerTable::from_pcb(&pcb).unwrap();
        let back = table.names.iter().position(|name| name == "B.Cu");
        assert_eq!(
            copper_item_layer("Track [SDA] on B.Cu, length 0.2983 mm", &table),
            Some(back)
        );
        assert_eq!(
            copper_item_layer("Track (arc) [SDA] on B.Cu, length 1 mm", &table),
            Some(back)
        );
        assert_eq!(
            copper_item_layer("Via [SDA] on F.Cu - B.Cu", &table),
            Some(None)
        );
        assert_eq!(copper_item_layer("Pad 1 [SDA] of U1 on F.Cu", &table), None);
    }

    fn group(reference: &str, number: &str) -> KiCadInspectedComponent {
        KiCadInspectedComponent {
            pads: vec![KiCadInspectedPad {
                reference: reference.into(),
                number: number.into(),
                uuid: None,
                at: [0.0, 0.0],
                layers: Vec::new(),
            }],
            other: Vec::new(),
            largest: false,
        }
    }

    /// A net with a zone keeps the groups KiCad's pairs name and the
    /// terminal's, and the pairs' indices follow the groups left.
    #[test]
    fn a_zone_net_keeps_the_named_groups_and_renumbers_the_pairs() {
        let mut components = vec![group("U1", "1"), group("U2", "1"), group("U3", "1")];
        let pair = |groups: Vec<Option<usize>>| KiCadNativePair {
            items: Vec::new(),
            components: groups,
        };
        let mut pairs = Some(vec![pair(vec![Some(2), None])]);
        let terminal = Some(("U1".to_string(), "1".to_string(), "A".to_string()));
        let omitted = omit_unnamed_groups(&mut components, &mut pairs, &terminal);
        assert_eq!(omitted, 1);
        let refs: Vec<&str> = components
            .iter()
            .map(|component| component.pads[0].reference.as_str())
            .collect();
        assert_eq!(refs, ["U1", "U3"]);
        assert_eq!(pairs.unwrap()[0].components, [Some(1), None]);
        let entry = terminal_entry(&terminal, &components).unwrap();
        assert_eq!(entry["component"], 0);
    }

    #[test]
    fn an_open_net_is_unrouted_with_two_terminals_or_more() {
        let net = |status: &str, terminals: u64| serde_json::json!({"status": status, "terminals": terminals});
        assert!(is_open_net(&net("partial", 2)));
        assert!(!is_open_net(&net("routed", 5)));
        assert!(!is_open_net(&net("partial", 1)));
    }
}
