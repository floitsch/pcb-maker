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
    /// Whether its copper joins every pad of the net (pours not counted;
    /// `components` says how many groups).
    pub connected: bool,
    pub components: usize,
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
    /// (an observation, not a cause).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hot_spots_near: Vec<serde_json::Value>,
    /// The pad `--terminal` named: its group.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal: Option<serde_json::Value>,
    /// Not measured by this command: no access probe or cut ran.
    pub access: serde_json::Value,
    pub cause: serde_json::Value,
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

/// The net named in a DRC item's description (`Pad 52 [QSPI_SCLK] of
/// U3 on F.Cu`).
fn described_net(description: &str) -> Option<&str> {
    let start = description.find('[')?;
    let end = description[start..].find(']')? + start;
    Some(&description[start + 1..end])
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

/// The diagnosis of a run (`inspect-routing-run`).
pub fn inspect_routing_run(inspection: &KiCadRunInspection) -> Result<KiCadRunDiagnosis, String> {
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
    let router_nets: BTreeMap<String, serde_json::Value> = report["nets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|net| Some((net["connection"].as_str()?.to_string(), net.clone())))
        .collect();

    // The terminal named: its net.
    let terminal = match &inspection.terminal {
        Some(text) => {
            let (reference, number) = text.split_once(':').ok_or_else(|| {
                crate::verdict::input_error(format!("--terminal {text}: give REF:PAD (U3:52)"))
            })?;
            let pad = result
                .pads
                .iter()
                .find(|pad| pad.reference == reference && pad.number == number)
                .ok_or_else(|| {
                    crate::verdict::input_error(format!(
                        "--terminal {text}: no such pad on the board"
                    ))
                })?;
            let net = pad.net.clone().ok_or_else(|| {
                crate::verdict::input_error(format!("--terminal {text}: the pad has no net"))
            })?;
            Some((reference.to_string(), number.to_string(), net))
        }
        None => None,
    };
    let selected: Vec<String> = if let Some((_, _, net)) = &terminal {
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
            .filter(|(_, net)| {
                net["status"].as_str() != Some("routed")
                    && net["terminals"].as_u64().unwrap_or(0) >= 2
            })
            .map(|(name, _)| name.clone())
            .collect()
    };

    // KiCad's unconnected pairs by net.
    let mut native_by_net: BTreeMap<String, Vec<KiCadNativePair>> = BTreeMap::new();
    let mut native_unattributed = 0;
    if let Some(drc) = &drc {
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
                    .entry(result.names.normalize(net).to_string())
                    .or_default()
                    .push(KiCadNativePair {
                        items,
                        components: Vec::new(),
                    }),
                None => native_unattributed += 1,
            }
        }
    }

    let native_capped = drc.as_ref().is_some_and(|drc| {
        drc["unconnected_items"].as_array().map_or(0, Vec::len) >= KICAD_REPORT_CAP
    });
    let overrides: Vec<serde_json::Value> = report["rules"]["overrides"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let hot_spots: Vec<serde_json::Value> = report["diagnostics"]["hot_spots"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut nets = Vec::new();
    for name in &selected {
        let router = router_nets.get(name);
        let status = router
            .and_then(|net| net["status"].as_str())
            .unwrap_or("not in board-router.json")
            .to_string();
        let (components, pour_layers) = match result.net_id(name) {
            Some(id) => (result.components(id), result.pour_layers(id)),
            None => (Vec::new(), Vec::new()),
        };
        let native_pairs = drc.as_ref().map(|_| {
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
            explanation: explanation(&counts, !pour_layers.is_empty(), native_capped),
            ..counts
        };
        let copper = net_copper(&result.pcb, &result.names, name);
        let rules = inspected_rules(config.as_ref(), name, &overrides, &copper);
        let reference_route = reference.as_ref().map(|reference| {
            let copper = net_copper(&reference.pcb, &reference.names, name);
            let groups = reference
                .net_id(name)
                .map_or(0, |id| reference.components(id).len());
            KiCadReferenceRoute {
                connected: groups <= 1,
                components: groups,
                copper,
            }
        });
        let pads: Vec<&KiCadInspectedPad> = components
            .iter()
            .flat_map(|component| &component.pads)
            .collect();
        let hot_spots_near: Vec<serde_json::Value> = hot_spots
            .iter()
            .filter_map(|spot| {
                let at = [spot["at"][0].as_f64()?, spot["at"][1].as_f64()?];
                let nearest = pads
                    .iter()
                    .map(|pad| distance_squared(pad.at, at).sqrt())
                    .fold(f64::INFINITY, f64::min);
                (nearest <= HOT_SPOT_REACH_MM).then(|| {
                    let mut spot = spot.clone();
                    spot["nearest_pad_mm"] = round(nearest).into();
                    spot
                })
            })
            .take(5)
            .collect();
        let dead_pads: Vec<serde_json::Value> = report["diagnostics"]["dead_pads"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|pad| pad["net"].as_str() == Some(name))
            .cloned()
            .collect();
        // A net with a zone: only the groups KiCad's pairs name and the
        // terminal's (its fill joins the others or not, unmodelled here).
        let mut components = components;
        let mut native_pairs = native_pairs;
        let mut components_omitted = 0;
        if !pour_layers.is_empty() {
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
            components_omitted = components.len() - next;
            let mut index = 0;
            components.retain(|_| {
                index += 1;
                keep[index - 1]
            });
        }
        let terminal_entry = terminal.as_ref().map(|(reference, number, _)| {
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
        });
        nets.push(KiCadNetDiagnosis {
            net: name.clone(),
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
        });
    }

    let tally = drc.as_ref().map(crate::verdict::tally_drc);
    let native = match (&drc, &tally) {
        (Some(_), Some(tally)) => serde_json::json!({
            "status": if tally.errors.is_empty() && tally.unconnected == 0 { "passed" } else { "failed" },
            "report": drc_path.as_ref().map(|path| path.display().to_string()),
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
    };
    let open_nets = router_nets
        .values()
        .filter(|net| {
            net["status"].as_str() != Some("routed") && net["terminals"].as_u64().unwrap_or(0) >= 2
        })
        .count();
    let checks = serde_json::json!({
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
    });
    let attempts = if report["attempts"].is_array() {
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
    };
    let path_text = |path: &Path| {
        std::path::absolute(path)
            .unwrap_or_else(|_| path.to_path_buf())
            .display()
            .to_string()
    };
    let run = serde_json::json!({
        "directory": path_text(&inspection.run),
        "board_id": board_id,
        "board": path_text(&directory.join(format!("{board_id}.kicad_pcb"))),
        "board_sha256": file_sha256(&directory.join(format!("{board_id}.kicad_pcb"))).ok(),
        "report": path_text(&directory.join("board-router.json")),
        "drc": drc_path.as_deref().map(path_text),
        "reference": inspection.reference.as_deref().map(path_text),
        "verdict": verdict.as_ref().map(|verdict| serde_json::json!({
            "status": verdict["status"],
            "reason": verdict["reason"],
            "kicad_checked": verdict["kicad_checked"],
        })),
    });
    Ok(KiCadRunDiagnosis {
        schema_version: 1,
        units: "mm".into(),
        run,
        checks,
        attempts,
        nets,
    })
}
