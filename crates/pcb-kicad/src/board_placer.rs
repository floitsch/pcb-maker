// Copyright (C) 2026 Toit contributors.

//! Whole-board component placement through the `pcb-placer` core.

use super::*;
use crate::placement_constraints::{
    KEEPOUT_NAME, KiCadConstraintStatus, KiCadConstraintsSource, apply_constraints, apply_keepouts, apply_outline,
    apply_sides, constraint_report, resolve_constraints,
};
use pcb_placer as core;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct KiCadBoardPlacerConfig {
    /// Minimum gap between two courtyards on colliding sides.
    pub spacing_mm: f64,
    /// Footprint origins are snapped to multiples of this.
    pub grid_mm: f64,
    pub keep_rotation: bool,
    /// References that must keep their pose, in addition to locked
    /// footprints, footprints without nets, and footprints on the board edge.
    pub fixed: Vec<String>,
    /// Glob patterns (`*` and `?`) of references that stay fixed.
    pub fixed_patterns: Vec<String>,
    /// Through-hole parts with at least four pins (connectors, headers)
    /// whose body comes within this distance of the outline stay where the
    /// designer put them: at the edge, where they are reachable.
    #[serde(default = "default_edge_keep_mm")]
    pub edge_keep_mm: f64,
    /// References that may move even though a default rule would fix them.
    pub free: Vec<String>,
    /// Share of the whitespace taken by filler charge: 1 packs the parts as
    /// tightly as halos allow, 0 spreads them over the whole board.
    pub whitespace_fill: f64,
    /// Every footprint keeps a halo of `pins / pins_per_halo_track` tracks
    /// (at `track_pitch_mm`, capped at `maximum_halo_mm`) free around its
    /// courtyard so its pins can escape.
    pub track_pitch_mm: f64,
    pub pins_per_halo_track: f64,
    pub maximum_halo_mm: f64,
    /// Per-reference halos replacing the pin-count rule (routing feedback).
    pub halo_overrides_mm: BTreeMap<String, f64>,
    /// Share of the free board that bodies, halos and spacing may claim.
    pub maximum_utilization: f64,
    /// Distance movable courtyards keep from the board edge (at least the
    /// copper-to-edge clearance, since pads may reach the courtyard).
    pub edge_margin_mm: f64,
    pub seed: u64,
    /// Wall-clock time by which placement returns its best result (set by
    /// the layout from its budget; placement of a 404-part panel ran past
    /// 25 minutes).
    #[serde(skip)]
    pub deadline: Option<std::time::Instant>,
    /// Placement work each seed may do, in seconds of an idle machine (see
    /// `core::WORK_PER_SECOND`): set by the layout from its budget, so that
    /// a busy or a slower machine places the same way.
    #[serde(skip)]
    pub work_seconds: Option<f64>,
    /// Where the project lets courtyards overlap, a part may reach over
    /// other parts' pads on its side at the last placement levels, keeping
    /// only copper apart (as katia's designer put connectors partly over
    /// hot-swap socket pads). Off by default: such pads may carry a body.
    pub overlap_far_side_pads: bool,
    /// Placement constraints: a `constraints.json` path (relative to the
    /// source directory) or the constraints inline.
    pub constraints: Option<KiCadConstraintsSource>,
    /// Millimetres of wirelength one millimetre of a missed near or
    /// relative constraint costs.
    pub constraint_weight: f64,
    /// The largest copper clearance of the board's rules: bodies are never
    /// placed closer than this (pads may sit on a body's edge). Read from
    /// the project when not given.
    pub copper_clearance_mm: Option<f64>,
    /// The board's copper-to-edge clearance: pads of parts held at an edge
    /// keep it. Read from the project when not given.
    #[serde(default)]
    pub copper_edge_clearance_mm: Option<f64>,
    /// When nothing else fits, bodies may shrink to their fabrication
    /// outline and pads (courtyards overlap). Read from the project when
    /// not given: allowed unless KiCad's DRC treats a courtyard overlap as
    /// an error.
    pub tight_bodies: Option<bool>,
    /// Decoupling capacitors (small capacitors between a supply rail and
    /// ground) are pulled to the supply pins of the ICs on their rail,
    /// unless a constraint already names them.
    pub auto_decoupling: bool,
    /// Placements tried with different seeds (in parallel); the best is
    /// kept: fewest illegal parts, least missed constraints, least wire.
    pub placement_seeds: usize,
}

impl Default for KiCadBoardPlacerConfig {
    fn default() -> Self {
        Self {
            spacing_mm: 0.5,
            grid_mm: 0.635,
            keep_rotation: false,
            fixed: Vec::new(),
            fixed_patterns: Vec::new(),
            edge_keep_mm: default_edge_keep_mm(),
            free: Vec::new(),
            whitespace_fill: 0.6,
            track_pitch_mm: 0.65,
            pins_per_halo_track: 8.0,
            maximum_halo_mm: 4.0,
            halo_overrides_mm: BTreeMap::new(),
            maximum_utilization: 0.6,
            edge_margin_mm: 0.5,
            seed: 1,
            deadline: None,
            work_seconds: None,
            overlap_far_side_pads: false,
            constraints: None,
            constraint_weight: 50.0,
            copper_clearance_mm: None,
            copper_edge_clearance_mm: None,
            tight_bodies: None,
            auto_decoupling: true,
            placement_seeds: 3,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadPlacedFootprint {
    pub reference: String,
    pub fixed: bool,
    /// Halo requested for this footprint, before fitting to the board.
    pub halo_mm: f64,
    /// Final body centre and half extents on the board.
    pub body_center: [f64; 2],
    pub body_half: [f64; 2],
    pub source_at: [f64; 3],
    pub at: [f64; 3],
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadBoardPlacerResult {
    pub board_id: String,
    pub components: usize,
    pub movable: usize,
    pub nets: usize,
    pub global_iterations: usize,
    pub global_overflow: f64,
    pub wirelength_source_mm: f64,
    pub wirelength_global_mm: f64,
    pub wirelength_legal_mm: f64,
    pub wirelength_final_mm: f64,
    pub unplaced: Vec<String>,
    pub illegal: Vec<String>,
    /// Every placement constraint with whether the placement keeps it.
    pub constraints: Vec<KiCadConstraintStatus>,
    pub constraint_warnings: Vec<String>,
    /// The board size a constraints outline set (automatic or given).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outline_mm: Option<[f64; 2]>,
    /// Share of the board the parts' bodies (with spacing) take, front and
    /// back.
    pub utilization: [f64; 2],
    /// Whether the placement needed bodies without their courtyard margin.
    pub tight_bodies: bool,
    /// The spacing, grid and halo factor the placement ended with, and
    /// whether bodies came closer to the edge than the full margin.
    pub spacing_mm: f64,
    pub grid_mm: f64,
    pub halo_scale: f64,
    /// The factor on the halos of parts with few pins (`halo_scale` is the
    /// many-pin parts').
    #[serde(default)]
    pub small_halo_scale: f64,
    pub edge_inset: bool,
    /// Parts held at an edge were placed by their copper alone (their
    /// courtyard may overhang the outline, as a card edge's does its tab).
    pub edge_copper: bool,
    /// Distance movable bodies kept from the edge (the placement's margin,
    /// or the rules' copper-to-edge clearance when nothing else fit).
    #[serde(default = "default_edge_rule")]
    pub edge_rule_mm: f64,
    /// The other seeds' legal placements (poses, relaxation and how far
    /// they miss the constraints in all), next best first.
    #[serde(skip)]
    pub alternatives: Vec<(Vec<core::Pose>, core::Relaxation, f64)>,
    /// What to change when parts found no legal place.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hints: Vec<String>,
    /// The edges parts on `"edge": "any"` were given, by part.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub edges_chosen: BTreeMap<String, String>,
    pub seconds: f64,
    /// The most placement work a seed did, in seconds of an idle machine.
    #[serde(default)]
    pub work_seconds: f64,
    pub footprints: Vec<KiCadPlacedFootprint>,
}

/// Whether a net's name marks it as one designers keep short: a token
/// (between non-alphanumerics) naming a switcher's switch node, feedback or
/// bootstrap, an RF or antenna feed, or a crystal pin.
fn keep_short(name: &str) -> bool {
    const TOKENS: [&str; 14] =
        ["SW", "LX", "FB", "VFB", "BST", "BOOT", "RF", "ANT", "XTAL", "XIN", "XOUT", "XI", "XO", "OSC"];
    name.split(|character: char| !character.is_ascii_alphanumeric())
        .any(|token| TOKENS.iter().any(|wanted| token.eq_ignore_ascii_case(wanted)))
}

/// Whether a net (by its raw name) connects parts; see `routable_net`.
fn placer_net(raw: &str) -> bool {
    board_router::routable_net(raw)
}

fn normalize_angle(angle: f64) -> f64 {
    let wrapped = angle.rem_euclid(360.0);
    if wrapped.abs() < 1.0e-9 || (wrapped - 360.0).abs() < 1.0e-9 {
        0.0
    } else {
        wrapped
    }
}

/// Whether the project's DRC lets courtyards overlap (its severity is not
/// `error`; KiCad's default is).
pub(super) fn courtyards_may_overlap(project: &Path) -> bool {
    fs::read_to_string(project)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|project| {
            project["board"]["design_settings"]["rule_severities"]["courtyards_overlap"]
                .as_str()
                .map(|severity| severity != "error")
        })
        .unwrap_or(false)
}

/// The largest copper clearance of a router configuration's rules.
pub(super) fn largest_clearance(config: &KiCadBoardRouterConfig) -> f64 {
    config
        .connection_rules
        .values()
        .chain(config.default_rules.iter())
        .map(|rules| rules.clearance_mm)
        .fold(0.0, f64::max)
}

/// Bounding box, in the footprint's own frame, of its courtyard and pads.
pub(crate) fn local_body(footprint: &Expr) -> Result<([f64; 2], [f64; 2], bool), String> {
    body_with_outline(footprint, ".CrtYd").map(|(center, size, round, _)| (center, size, round))
}

/// The body without the courtyard's margin: fabrication outline and pads
/// ([min x, min y, max x, max y], own frame). `None` without a fabrication
/// outline, or where it is no smaller than the courtyard.
fn tight_body(footprint: &Expr) -> Result<Option<[f64; 4]>, String> {
    let (center, size, _, outlined) = body_with_outline(footprint, ".Fab")?;
    let (_, courtyard, _) = local_body(footprint)?;
    if outlined == 0 || size[0] * size[1] >= courtyard[0] * courtyard[1] - 1.0e-9 {
        return Ok(None);
    }
    Ok(Some([
        center[0] - size[0] / 2.0,
        center[1] - size[1] / 2.0,
        center[0] + size[0] / 2.0,
        center[1] + size[1] / 2.0,
    ]))
}

/// The box around the graphics on layers ending in `outline` and the pads,
/// whether it is a disc, and how many outline graphics there are.
fn body_with_outline(footprint: &Expr, outline: &str) -> Result<([f64; 2], [f64; 2], bool, usize), String> {
    let mut minimum = [f64::INFINITY; 2];
    let mut maximum = [f64::NEG_INFINITY; 2];
    let mut include = |point: [f64; 2], radius: f64| {
        for axis in 0..2 {
            minimum[axis] = minimum[axis].min(point[axis] - radius);
            maximum[axis] = maximum[axis].max(point[axis] + radius);
        }
    };
    let footprint_angle = form_at(footprint)?[2];
    let mut circles = 0;
    let mut straight = 0;
    for child in footprint.children() {
        match child.head() {
            Some("fp_line" | "fp_rect" | "fp_arc")
                if form_atom(child, "layer", 1).is_some_and(|layer| layer.ends_with(outline)) =>
            {
                straight += 1;
                for point in outline::outline_points(child)? {
                    include(point, 0.0);
                }
            }
            Some("fp_circle")
                if form_atom(child, "layer", 1).is_some_and(|layer| layer.ends_with(outline)) =>
            {
                circles += 1;
                let center = form_xy(child, "center")?;
                let end = form_xy(child, "end")?;
                include(center, distance_squared(center, end).sqrt());
            }
            Some("fp_poly")
                if form_atom(child, "layer", 1).is_some_and(|layer| layer.ends_with(outline)) =>
            {
                straight += 1;
                for point in outline::pts_points(child)? {
                    include(point, 0.0);
                }
            }
            Some("pad") => {
                let at = form_at(child)?;
                let size = pad_extent(child);
                // Pad angles are absolute; bring the pad box into the local frame.
                let (sin, cos) = (-(at[2] - footprint_angle)).to_radians().sin_cos();
                // Copper must keep a hole's local clearance, so no other
                // part's pads may come that close either.
                let keep_away = if child.children().get(2).and_then(Expr::atom)
                    == Some("np_thru_hole")
                {
                    local_clearance::pad_clearance(child, footprint)?
                } else {
                    0.0
                };
                let half = [
                    (cos.abs() * size[0] + sin.abs() * size[1]) / 2.0 + keep_away,
                    (sin.abs() * size[0] + cos.abs() * size[1]) / 2.0 + keep_away,
                ];
                // A drill offset moves the copper away from the anchor, also
                // on SMD pads.
                let offset = child
                    .child("drill")
                    .and_then(|drill| drill.child("offset"))
                    .and_then(|offset| {
                        Some([
                            expression_coordinate(offset, 1, "pad offset x").ok()?,
                            expression_coordinate(offset, 2, "pad offset y").ok()?,
                        ])
                    })
                    .unwrap_or([0.0, 0.0]);
                let center = [
                    at[0] + cos * offset[0] - sin * offset[1],
                    at[1] + sin * offset[0] + cos * offset[1],
                ];
                for corner in [[-1.0, -1.0], [1.0, 1.0]] {
                    include(
                        [center[0] + corner[0] * half[0], center[1] + corner[1] * half[1]],
                        0.0,
                    );
                }
                // A custom pad's copper is its primitives too (Sisu's dome
                // switch: a 0.7 mm anchor and a ring of 3.1 mm radius around
                // the switch centre), in the pad's own frame.
                for (point, radius) in custom_pad_points(child) {
                    include([at[0] + cos * point[0] - sin * point[1], at[1] + sin * point[0] + cos * point[1]], radius);
                }
            }
            _ => {}
        }
    }
    if !minimum[0].is_finite() {
        // Neither courtyard nor pads: copper artwork (a logo on copper)
        // still takes its room.
        for child in footprint.children() {
            if !(matches!(child.head(), Some("fp_line" | "fp_rect" | "fp_arc" | "fp_circle" | "fp_poly"))
                && form_atom(child, "layer", 1).is_some_and(|layer| layer.ends_with(".Cu")))
            {
                continue;
            }
            let mut points = outline::outline_points(child)?;
            if child.child("center").is_some() {
                points.push(form_xy(child, "center")?);
            }
            for point in outline::pts_points(child)? {
                points.push(point);
            }
            for point in points {
                for axis in 0..2 {
                    minimum[axis] = minimum[axis].min(point[axis]);
                    maximum[axis] = maximum[axis].max(point[axis]);
                }
            }
        }
    }
    if !minimum[0].is_finite() {
        return Ok(([0.0, 0.0], [1.0, 1.0], false, 0));
    }
    let size = [
        (maximum[0] - minimum[0]).max(0.2),
        (maximum[1] - minimum[1]).max(0.2),
    ];
    // A single circular courtyard containing all pads is a disc.
    let round = circles == 1 && straight == 0 && (size[0] - size[1]).abs() < 1.0e-6;
    Ok((
        [(minimum[0] + maximum[0]) / 2.0, (minimum[1] + maximum[1]) / 2.0],
        size,
        round,
        circles + straight,
    ))
}

/// Moves a footprint to `pose`, keeping pad and text angles (which KiCad
/// stores as absolute values) consistent. Returns the written pose.
pub(super) fn write_footprint_pose(footprint: &mut Expr, pose: core::Pose) -> Result<[f64; 3], String> {
    let current = form_at(footprint)?;
    let at = [
        (pose.position[0] * 1.0e6).round() / 1.0e6,
        (pose.position[1] * 1.0e6).round() / 1.0e6,
        normalize_angle(pose.angle),
    ];
    let delta = normalize_angle(at[2] - current[2]);
    set_form_at(footprint, at)?;
    if delta != 0.0
        && let Expr::List(children) = footprint
    {
        for child in children.iter_mut() {
            if matches!(child.head(), Some("pad" | "property" | "fp_text"))
                && child.child("at").is_some()
            {
                let child_at = form_at(child)?;
                set_form_at(
                    child,
                    [child_at[0], child_at[1], normalize_angle(child_at[2] + delta)],
                )?;
            }
        }
    }
    Ok(at)
}

/// A pad's size in its own frame, at least its drill (a hole may be
/// drawn with a token pad).
/// The points (with a radius around them) that bound a custom pad's
/// primitives, in the pad's frame; empty for other pads.
fn custom_pad_points(pad: &Expr) -> Vec<([f64; 2], f64)> {
    let Some(primitives) = pad.child("primitives") else {
        return Vec::new();
    };
    let mut points = Vec::new();
    for primitive in primitives.children().iter().skip(1) {
        let width = primitive
            .child("stroke")
            .and_then(|stroke| form_f64(stroke, "width", 1).ok())
            .or_else(|| form_f64(primitive, "width", 1).ok())
            .unwrap_or(0.0)
            .max(0.0);
        match primitive.head() {
            Some("gr_circle") => {
                if let (Ok(center), Ok(end)) = (form_xy(primitive, "center"), form_xy(primitive, "end")) {
                    points.push((center, distance_squared(center, end).sqrt() + width / 2.0));
                }
            }
            _ => {
                for head in ["start", "mid", "end"] {
                    if let Ok(point) = form_xy(primitive, head) {
                        points.push((point, width / 2.0));
                    }
                }
                for point in outline::pts_points(primitive).unwrap_or_default() {
                    points.push((point, width / 2.0));
                }
            }
        }
    }
    points
}

fn pad_extent(pad: &Expr) -> [f64; 2] {
    let size = form_xy(pad, "size").unwrap_or([0.0, 0.0]);
    let drill: Vec<f64> = pad
        .child("drill")
        .map(|drill| drill.children().iter().skip(1).filter_map(Expr::atom).filter_map(|atom| atom.parse().ok()).collect())
        .unwrap_or_default();
    let drill = match drill.as_slice() {
        [diameter] => [*diameter, *diameter],
        [width, height, ..] => [*width, *height],
        [] => [0.0, 0.0],
    };
    [size[0].max(drill[0]), size[1].max(drill[1])]
}

/// Decoupling capacitors pulled to IC supply pins: every capacitor between
/// a rail and ground goes near a supply pin of an IC on that rail, whichever
/// suits; up to 4.7 µF within 2.5 mm of the pin, bulk capacitors within
/// 5 mm. Crystals go within 3 mm of the IC pins they drive. Capacitors that a user
/// constraint names, fixed ones and ones a row carries keep to those.
fn decoupling_relations(pcb: &Expr, problem: &core::Problem) -> Result<Vec<core::constraints::Relation>, String> {
    use crate::quality::{capacitance, ground, rail_name};
    use core::constraints::{Anchor, Relation};
    struct Supply {
        part: usize,
        offset: [f64; 2],
        net: String,
    }
    let footprints: Vec<&Expr> = pcb.children().iter().filter(|item| item.head() == Some("footprint")).collect();
    // A user's "near that part" leaves room for "at its supply pin";
    // anything else a user said about a capacitor stands.
    let named: BTreeSet<usize> = problem
        .constraints
        .relations
        .iter()
        .filter_map(|relation| match relation {
            Relation::Near { anchor: Anchor::Body(_), .. } => None,
            other => other.parts()[0],
        })
        .chain(problem.constraints.edges.iter().map(|(part, _, _)| *part))
        .chain(problem.constraints.regions.iter().map(|(part, _)| *part))
        .chain(problem.constraints.followers.iter().map(|follower| follower.part))
        .collect();
    let prefix = |reference: &str| reference.trim_end_matches(|c: char| c.is_ascii_digit() || c == '_').to_string();
    let mut supplies: Vec<Supply> = Vec::new();
    // Capacitors between ground and another net: decoupling when that net
    // is a rail (by name, or an IC's supply pin is on it).
    let mut capacitors: Vec<(usize, String, bool)> = Vec::new();
    // Crystals and the nets on their signal pins; IC pads by net.
    let mut crystals: Vec<(usize, Vec<String>)> = Vec::new();
    let mut ic_pads: BTreeMap<String, Vec<(usize, [f64; 2])>> = BTreeMap::new();
    // IC pads whose pin function names a switch node (`SW`, `LX`), by net.
    let mut switch_pads: BTreeMap<String, Vec<(usize, [f64; 2])>> = BTreeMap::new();
    // Inductors and their switch-node nets.
    let mut inductors: Vec<(usize, Vec<String>)> = Vec::new();
    // ESD protection and the nets it guards; connector pads by net.
    let mut protectors: Vec<(usize, Vec<String>)> = Vec::new();
    let mut connector_pads: BTreeMap<String, Vec<(usize, [f64; 2])>> = BTreeMap::new();
    for (index, footprint) in footprints.iter().enumerate() {
        let reference = footprint_reference(footprint).unwrap_or_default();
        let pads: Vec<(Option<String>, [f64; 2], bool, bool)> = footprint
            .children()
            .iter()
            .filter(|child| child.head() == Some("pad"))
            .map(|pad| {
                let at = form_at(pad).unwrap_or([0.0; 3]);
                // The pin function without KiCad's appended pin number.
                let function = form_atom(pad, "pinfunction", 1).unwrap_or("");
                let function = function.rsplit_once('_').filter(|(_, number)| number.chars().all(|c| c.is_ascii_digit())).map_or(function, |(name, _)| name);
                let function = function.to_ascii_uppercase();
                let switch = matches!(function.as_str(), "SW" | "LX" | "SWITCH" | "PH" | "SWN" | "SWP" | "BST" | "VSW")
                    || function.starts_with("SW") && function[2..].chars().all(|c| c.is_ascii_digit())
                    || function.starts_with("LX") && function[2..].chars().all(|c| c.is_ascii_digit());
                (
                    node_net(pad).filter(|raw| placer_net(raw)).map(|raw| normalize_net(raw).to_string()),
                    [at[0], at[1]],
                    form_atom(pad, "pintype", 1) == Some("power_in"),
                    switch,
                )
            })
            .collect();
        let value = footprint
            .children()
            .iter()
            .find(|item| item.head() == Some("property") && item.children().get(1).and_then(Expr::atom) == Some("Value"))
            .and_then(|item| item.children().get(2).and_then(Expr::atom))
            .unwrap_or("")
            .to_ascii_uppercase();
        let protector = crate::quality::esd_protector(&value)
            && !problem.components[index].fixed
            && !named.contains(&index);
        if matches!(prefix(&reference).as_str(), "J" | "P" | "CN" | "CON" | "USB") {
            for (net, offset, _, _) in &pads {
                if let Some(net) = net.as_ref().filter(|net| !ground(net)) {
                    connector_pads.entry(net.clone()).or_default().push((index, *offset));
                }
            }
        }
        if protector {
            let nets: Vec<String> = pads.iter().filter_map(|(net, _, _, _)| net.clone()).filter(|net| !ground(net)).collect();
            protectors.push((index, nets));
            continue;
        }
        match prefix(&reference).as_str() {
            "U" | "IC" if pads.len() >= 3 => {
                let mut seen = BTreeSet::new();
                for (net, offset, power, switch) in &pads {
                    let Some(net) = net else {
                        continue;
                    };
                    ic_pads.entry(net.clone()).or_default().push((index, *offset));
                    if *switch {
                        switch_pads.entry(net.clone()).or_default().push((index, *offset));
                    }
                    // One anchor per rail and IC: its first supply pin.
                    if !ground(net) && (*power || rail_name(net)) && seen.insert(net.clone()) {
                        supplies.push(Supply { part: index, offset: *offset, net: net.clone() });
                    }
                }
            }
            _ if {
                let name = footprint.children().get(1).and_then(Expr::atom).unwrap_or("").to_ascii_lowercase();
                prefix(&reference) == "Y" || name.contains("crystal") || name.contains("resonator")
            } =>
            {
                if named.contains(&index) || problem.components[index].fixed {
                    continue;
                }
                let nets: Vec<String> = pads
                    .iter()
                    .filter_map(|(net, _, _, _)| net.clone())
                    .filter(|net| !ground(net) && !rail_name(net))
                    .collect();
                crystals.push((index, nets));
            }
            "L" if pads.len() == 2 => {
                if named.contains(&index) || problem.components[index].fixed {
                    continue;
                }
                // The switch node: the net that is neither ground nor a rail.
                let nets: Vec<String> = pads
                    .iter()
                    .filter_map(|(net, _, _, _)| net.clone())
                    .filter(|net| !ground(net) && !rail_name(net))
                    .collect();
                if nets.len() == 1 {
                    inductors.push((index, nets));
                }
            }
            "C" if pads.len() == 2 => {
                if named.contains(&index) || problem.components[index].fixed {
                    continue;
                }
                let (Some(a), Some(b)) = (&pads[0].0, &pads[1].0) else {
                    continue;
                };
                let rail = match (ground(a), ground(b)) {
                    (false, true) => a.clone(),
                    (true, false) => b.clone(),
                    _ => continue,
                };
                let value = footprint
                    .children()
                    .iter()
                    .find(|item| item.head() == Some("property") && item.children().get(1).and_then(Expr::atom) == Some("Value"))
                    .and_then(|item| item.children().get(2).and_then(Expr::atom))
                    .unwrap_or("");
                let bulk = capacitance(value).is_some_and(|farads| farads >= 4.7e-6);
                capacitors.push((index, rail, bulk));
            }
            _ => {}
        }
    }
    // Each capacitor near whichever supply pin of its rail the placer finds
    // best: the netlist does not say which IC a capacitor serves.
    let mut relations = Vec::new();
    for (index, rail, bulk) in capacitors {
        let anchors: Vec<Anchor> = supplies
            .iter()
            .filter(|supply| supply.net == rail)
            .map(|supply| Anchor::Point(supply.part, supply.offset))
            .collect();
        if anchors.is_empty() {
            continue;
        }
        relations.push(Relation::NearAny { part: index, anchors, max: if bulk { 5.0 } else { 2.5 } });
    }
    // A switcher's inductor at the IC's switch pin: a small hot loop. Only
    // where a pin function says the pin switches (a supply filter's
    // inductor also sits between an IC pin and a rail, and pulling it in
    // costs routability: Brushless_ESC).
    for (index, nets) in inductors {
        let anchors: Vec<Anchor> = nets
            .iter()
            .flat_map(|net| switch_pads.get(net).into_iter().flatten())
            .map(|(ic, offset)| Anchor::Point(*ic, *offset))
            .collect();
        if !anchors.is_empty() {
            relations.push(Relation::NearAny { part: index, anchors, max: 3.0 });
        }
    }
    // ESD protection at the connector pins it guards: a surge should meet
    // it before anything else.
    for (index, nets) in protectors {
        let anchors: Vec<Anchor> = nets
            .iter()
            .flat_map(|net| connector_pads.get(net).into_iter().flatten())
            .map(|(connector, offset)| Anchor::Point(*connector, *offset))
            .collect();
        if !anchors.is_empty() {
            relations.push(Relation::NearAny { part: index, anchors, max: 3.0 });
        }
    }
    // A crystal close to the IC pins it drives (short, quiet oscillator
    // traces).
    for (index, nets) in crystals {
        for net in nets {
            for (ic, offset) in ic_pads.get(&net).into_iter().flatten() {
                relations.push(Relation::Near { part: index, anchor: Anchor::Point(*ic, *offset), max: 3.0 });
            }
        }
    }
    Ok(relations)
}

/// Boxes ([min x, min y, max x, max y], own frame) of the separate shapes
/// a footprint's courtyard is drawn as (empty when it is one shape, or
/// none), and whether the courtyard is malformed: lines and arcs whose end
/// points do not each join exactly two of them (a zero-length line does
/// not), which KiCad cannot build and does not check. Lines and arcs that
/// share end points form one shape.
fn courtyard_shapes(footprint: &Expr) -> Result<(Vec<[f64; 4]>, bool), String> {
    let mut degrees: Vec<([f64; 2], usize)> = Vec::new();
    let mut shapes: Vec<(Vec<[f64; 2]>, [f64; 4])> = Vec::new();
    let grow = |bounds: &mut [f64; 4], point: [f64; 2], radius: f64| {
        bounds[0] = bounds[0].min(point[0] - radius);
        bounds[1] = bounds[1].min(point[1] - radius);
        bounds[2] = bounds[2].max(point[0] + radius);
        bounds[3] = bounds[3].max(point[1] + radius);
    };
    let empty = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for child in footprint.children() {
        if !form_atom(child, "layer", 1).is_some_and(|layer| layer.ends_with(".CrtYd")) {
            continue;
        }
        let mut bounds = empty;
        let mut ends = Vec::new();
        match child.head() {
            Some("fp_line" | "fp_arc") => {
                for point in outline::outline_points(child)? {
                    grow(&mut bounds, point, 0.0);
                }
                ends = vec![form_xy(child, "start")?, form_xy(child, "end")?];
                for end in &ends {
                    match degrees.iter_mut().find(|(point, _)| distance_squared(*point, *end) < 1.0e-6) {
                        Some((_, degree)) => *degree += 1,
                        None => degrees.push((*end, 1)),
                    }
                }
            }
            Some("fp_rect") => {
                grow(&mut bounds, form_xy(child, "start")?, 0.0);
                grow(&mut bounds, form_xy(child, "end")?, 0.0);
            }
            Some("fp_circle") => {
                let center = form_xy(child, "center")?;
                grow(&mut bounds, center, distance_squared(center, form_xy(child, "end")?).sqrt());
            }
            Some("fp_poly") => {
                for point in outline::pts_points(child)? {
                    grow(&mut bounds, point, 0.0);
                }
            }
            _ => continue,
        }
        // Merge with every shape sharing an end point.
        let touches = |points: &[[f64; 2]]| {
            ends.iter().any(|end| points.iter().any(|point| distance_squared(*end, *point) < 1.0e-6))
        };
        let (mut merged, rest): (Vec<_>, Vec<_>) = shapes.drain(..).partition(|(points, _)| touches(points));
        shapes = rest;
        let mut points = ends.clone();
        for (other_points, other_bounds) in merged.drain(..) {
            points.extend(other_points);
            grow(&mut bounds, [other_bounds[0], other_bounds[1]], 0.0);
            grow(&mut bounds, [other_bounds[2], other_bounds[3]], 0.0);
        }
        shapes.push((points, bounds));
    }
    let malformed = degrees.iter().any(|(_, degree)| *degree != 2);
    if shapes.len() < 2 {
        return Ok((Vec::new(), malformed));
    }
    Ok((shapes.into_iter().map(|(_, bounds)| bounds).collect(), malformed))
}

/// Whether a footprint sits on the bottom side. Its SMD pads decide when
/// they all sit on one outer layer: converted footprints (EasyEDA) may claim
/// the front while their copper is on the back.
pub(crate) fn on_back(footprint: &Expr) -> bool {
    let (mut front, mut back) = (0, 0);
    for pad in footprint.children().iter().filter(|child| child.head() == Some("pad")) {
        if pad.children().get(2).and_then(Expr::atom) != Some("smd") {
            continue;
        }
        let layers: Vec<&str> = pad
            .child("layers")
            .map(|layers| layers.children().iter().skip(1).filter_map(Expr::atom).collect())
            .unwrap_or_default();
        front += layers.contains(&"F.Cu") as usize;
        back += layers.contains(&"B.Cu") as usize;
    }
    match (front, back) {
        (0, 1..) => true,
        (1.., 0) => false,
        _ => form_atom(footprint, "layer", 1) == Some("B.Cu"),
    }
}

/// Which way a connector opens, in its own frame: the side where the
/// courtyard reaches farthest beyond the pads (a USB receptacle's shell, a
/// barrel jack's body), if one side clearly does.
pub(crate) fn connector_mouth(footprint: &Expr) -> Result<Option<[f64; 2]>, String> {
    let footprint_angle = form_at(footprint)?[2];
    let mut courtyard = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    let mut pads = courtyard;
    let grow = |bounds: &mut [f64; 4], point: [f64; 2], half: [f64; 2]| {
        bounds[0] = bounds[0].min(point[0] - half[0]);
        bounds[1] = bounds[1].min(point[1] - half[1]);
        bounds[2] = bounds[2].max(point[0] + half[0]);
        bounds[3] = bounds[3].max(point[1] + half[1]);
    };
    for child in footprint.children() {
        let on_courtyard = form_atom(child, "layer", 1).is_some_and(|layer| layer.ends_with(".CrtYd"));
        match child.head() {
            Some("fp_line" | "fp_rect" | "fp_arc") if on_courtyard => {
                for point in outline::outline_points(child)? {
                    grow(&mut courtyard, point, [0.0, 0.0]);
                }
            }
            Some("fp_poly") if on_courtyard => {
                for point in outline::pts_points(child)? {
                    grow(&mut courtyard, point, [0.0, 0.0]);
                }
            }
            Some("pad") => {
                let at = form_at(child)?;
                let size = form_xy(child, "size").unwrap_or([0.0, 0.0]);
                let (sin, cos) = (-(at[2] - footprint_angle)).to_radians().sin_cos();
                grow(
                    &mut pads,
                    [at[0], at[1]],
                    [
                        (cos.abs() * size[0] + sin.abs() * size[1]) / 2.0,
                        (sin.abs() * size[0] + cos.abs() * size[1]) / 2.0,
                    ],
                );
            }
            _ => {}
        }
    }
    if !courtyard[0].is_finite() || !pads[0].is_finite() {
        return Ok(None);
    }
    let mut sides = [
        (pads[0] - courtyard[0], [-1.0, 0.0]),
        (pads[1] - courtyard[1], [0.0, -1.0]),
        (courtyard[2] - pads[2], [1.0, 0.0]),
        (courtyard[3] - pads[3], [0.0, 1.0]),
    ];
    sides.sort_by(|a, b| b.0.total_cmp(&a.0));
    Ok((sides[0].0 >= 1.0 && sides[0].0 >= 1.5 * sides[1].0.max(0.0)).then_some(sides[0].1))
}

/// Bounding box, in the footprint's own frame, of its keepout zones.
fn footprint_keepout_box(footprint: &Expr, at: [f64; 3]) -> Result<Option<[f64; 4]>, String> {
    let mut bounds: Option<[f64; 4]> = None;
    for zone in footprint
        .children()
        .iter()
        .filter(|child| child.head() == Some("zone") && child.child("keepout").is_some())
    {
        let Some(points) = zone.child("polygon").and_then(|polygon| polygon.child("pts")) else {
            continue;
        };
        for point in points.children().iter().filter(|child| child.head() == Some("xy")) {
            let board = [
                expression_coordinate(point, 1, "keepout x")?,
                expression_coordinate(point, 2, "keepout y")?,
            ];
            // Zones inside footprints are stored in board coordinates.
            let local = core::problem::rotate([board[0] - at[0], board[1] - at[1]], at[2]);
            let entry = bounds.get_or_insert([local[0], local[1], local[0], local[1]]);
            entry[0] = entry[0].min(local[0]);
            entry[1] = entry[1].min(local[1]);
            entry[2] = entry[2].max(local[0]);
            entry[3] = entry[3].max(local[1]);
        }
    }
    Ok(bounds)
}

pub(super) struct LoweredPlacement {
    pub problem: core::Problem,
    pub references: Vec<String>,
    pub source_at: Vec<[f64; 3]>,
    pub constraint_warnings: Vec<String>,
}

fn default_edge_rule() -> f64 {
    f64::INFINITY
}

fn default_edge_keep_mm() -> f64 {
    3.0
}

/// `*` matches any run of characters, `?` one character, and a backslash
/// makes the next character literal (`REF\\*\\*` is KiCad's unannotated
/// `REF**` itself).
pub(crate) fn glob_matches(pattern: &str, text: &str) -> bool {
    #[derive(Clone, Copy, PartialEq)]
    enum Token {
        Any,
        One,
        Literal(char),
    }
    let mut tokens = Vec::new();
    let mut characters = pattern.chars();
    while let Some(character) = characters.next() {
        tokens.push(match character {
            '*' => Token::Any,
            '?' => Token::One,
            '\\' => Token::Literal(characters.next().unwrap_or('\\')),
            other => Token::Literal(other),
        });
    }
    let text: Vec<char> = text.chars().collect();
    let mut table = vec![vec![false; text.len() + 1]; tokens.len() + 1];
    table[0][0] = true;
    for p in 1..=tokens.len() {
        if tokens[p - 1] == Token::Any {
            table[p][0] = table[p - 1][0];
        }
        for t in 1..=text.len() {
            table[p][t] = match tokens[p - 1] {
                Token::Any => table[p - 1][t] || table[p][t - 1],
                Token::One => table[p - 1][t - 1],
                Token::Literal(c) => table[p - 1][t - 1] && c == text[t - 1],
            };
        }
    }
    table[tokens.len()][text.len()]
}

pub(super) fn lower_placement(
    pcb: &Expr,
    config: &KiCadBoardPlacerConfig,
) -> Result<LoweredPlacement, String> {
    let loops = outline::board_loops(pcb)?;
    let mut net_ids = BTreeMap::<String, usize>::new();
    let mut components = Vec::new();
    let mut poses = Vec::new();
    let mut references = Vec::new();
    let mut source_at = Vec::new();
    let mut locked = Vec::new();
    let mut connector = Vec::new();
    // Every pad by name, for constraints that point at a pin, and the
    // footprint's own keepout zones (an antenna), for overhang constraints.
    let mut pad_offsets: Vec<BTreeMap<String, [f64; 2]>> = Vec::new();
    let mut pad_boxes: Vec<Vec<[f64; 4]>> = Vec::new();
    let mut keepout_boxes: Vec<Option<[f64; 4]>> = Vec::new();
    let mut mouths: Vec<Option<[f64; 2]>> = Vec::new();
    for footprint in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
    {
        let at = form_at(footprint)?;
        let reference = footprint_reference(footprint).unwrap_or_default();
        let (body_center, body_size, round) = local_body(footprint)?;
        let mut pins = Vec::new();
        let mut through = false;
        let mut far_side = Vec::new();
        let mut own_pads = Vec::new();
        let mut named = BTreeMap::new();
        // The copper layer of the side the part is not on: an edge-mount
        // connector's SMD pads there occupy that side too.
        let other_copper = if on_back(footprint) { "F.Cu" } else { "B.Cu" };
        for pad in footprint
            .children()
            .iter()
            .filter(|child| child.head() == Some("pad"))
        {
            if let (Some(name), Ok(at)) = (pad.children().get(1).and_then(Expr::atom), form_at(pad)) {
                named.entry(name.to_string()).or_insert([at[0], at[1]]);
            }
            let pad_type = pad.children().get(2).and_then(Expr::atom).unwrap_or("");
            // The pad (or hole) with the clearance a hole keeps, in the
            // footprint's frame.
            let pad_at = form_at(pad)?;
            let size = pad_extent(pad);
            let (sin, cos) = (-(pad_at[2] - at[2])).to_radians().sin_cos();
            let keep_away = if pad_type == "np_thru_hole" {
                local_clearance::pad_clearance(pad, footprint)?
            } else {
                0.0
            };
            let half = [
                (cos.abs() * size[0] + sin.abs() * size[1]) / 2.0 + keep_away,
                (sin.abs() * size[0] + cos.abs() * size[1]) / 2.0 + keep_away,
            ];
            // A drill offset moves the copper away from the hole (castellated
            // module pads): the box covers both.
            let offset = pad
                .child("drill")
                .and_then(|drill| drill.child("offset"))
                .and_then(|offset| {
                    Some([
                        expression_coordinate(offset, 1, "pad offset x").ok()?,
                        expression_coordinate(offset, 2, "pad offset y").ok()?,
                    ])
                })
                .unwrap_or([0.0, 0.0]);
            let copper = [
                pad_at[0] + cos * offset[0] - sin * offset[1],
                pad_at[1] + sin * offset[0] + cos * offset[1],
            ];
            let pad_box = [
                (copper[0] - half[0]).min(pad_at[0] - half[0].min(half[1])),
                (copper[1] - half[1]).min(pad_at[1] - half[0].min(half[1])),
                (copper[0] + half[0]).max(pad_at[0] + half[0].min(half[1])),
                (copper[1] + half[1]).max(pad_at[1] + half[0].min(half[1])),
            ];
            own_pads.push(pad_box);
            let on_other_side = pad.child("layers").is_some_and(|layers| {
                layers.children().iter().skip(1).filter_map(Expr::atom).any(|layer| layer == other_copper || layer == "F&B.Cu")
            });
            if matches!(pad_type, "thru_hole" | "np_thru_hole") || on_other_side {
                through = true;
                // What the part occupies on the other side.
                far_side.push(pad_box);
            }
            let Some(net) = node_net(pad).filter(|raw| placer_net(raw)).map(normalize_net) else {
                continue;
            };
            let next = net_ids.len();
            let net = *net_ids.entry(net.to_string()).or_insert(next);
            let pad_at = form_at(pad)?;
            pins.push(core::Pin {
                offset: [pad_at[0], pad_at[1]],
                net,
            });
        }
        // Artwork (a logo) has neither pads nor a courtyard: it occupies
        // nothing, rather than being a 1 mm wall at its origin.
        let artwork = !footprint.children().iter().any(|child| {
            child.head() == Some("pad")
                || (matches!(child.head(), Some("fp_line" | "fp_rect" | "fp_arc" | "fp_circle" | "fp_poly"))
                    && form_atom(child, "layer", 1)
                        .is_some_and(|layer| layer.ends_with(".CrtYd") || layer.ends_with(".Cu")))
        });
        // A through-hole part sits on its footprint's side; on the other
        // side only its holes and pads (`far_side`) are in the way.
        let side = if artwork {
            core::Side::Neither
        } else if on_back(footprint) {
            core::Side::Back
        } else {
            core::Side::Front
        };
        let angle_options = if config.keep_rotation {
            vec![at[2]]
        } else {
            (0..4)
                .map(|quarter| normalize_angle(at[2] + 90.0 * quarter as f64))
                .collect()
        };
        // How far the copper stays inside the body: the body may come that
        // much closer to the board edge.
        let edge_inset = if own_pads.is_empty() {
            0.0
        } else {
            let mut pads = own_pads.iter().fold(
                [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY],
                |bounds, pad| [bounds[0].min(pad[0]), bounds[1].min(pad[1]), bounds[2].max(pad[2]), bounds[3].max(pad[3])],
            );
            // Copper graphics of the footprint count as copper too.
            for child in footprint.children() {
                if !(matches!(child.head(), Some("fp_line" | "fp_rect" | "fp_arc" | "fp_circle" | "fp_poly"))
                    && form_atom(child, "layer", 1).is_some_and(|layer| layer.ends_with(".Cu")))
                {
                    continue;
                }
                let width = child
                    .child("stroke")
                    .and_then(|stroke| form_atom(stroke, "width", 1))
                    .and_then(|width| width.parse::<f64>().ok())
                    .unwrap_or(0.0);
                let mut points = outline::outline_points(child)?;
                if child.head() == Some("fp_circle") {
                    let center = form_xy(child, "center")?;
                    let radius = distance_squared(center, form_xy(child, "end")?).sqrt();
                    points.extend([[center[0] - radius, center[1] - radius], [center[0] + radius, center[1] + radius]]);
                }
                for point in outline::pts_points(child)? {
                    points.push(point);
                }
                for point in points {
                    pads = [
                        pads[0].min(point[0] - width / 2.0),
                        pads[1].min(point[1] - width / 2.0),
                        pads[2].max(point[0] + width / 2.0),
                        pads[3].max(point[1] + width / 2.0),
                    ];
                }
            }
            let body = [
                body_center[0] - body_size[0] / 2.0,
                body_center[1] - body_size[1] / 2.0,
                body_center[0] + body_size[0] / 2.0,
                body_center[1] + body_size[1] / 2.0,
            ];
            [pads[0] - body[0], pads[1] - body[1], body[2] - pads[2], body[3] - pads[3]]
                .into_iter()
                .fold(f64::INFINITY, f64::min)
                .max(0.0)
        };
        let courtyard = courtyard_shapes(footprint)?;
        connector.push(through && pins.len() >= 4);
        locked.push(
            footprint.child("locked").is_some()
                || footprint
                    .children()
                    .iter()
                    .any(|child| child.atom() == Some("locked")),
        );
        components.push(core::Component {
            name: reference.clone(),
            body_center,
            body_size,
            round,
            halo: if let Some(halo) = config.halo_overrides_mm.get(&reference) {
                *halo
            } else if config.pins_per_halo_track > 0.0 {
                ((pins.len() as f64 / config.pins_per_halo_track).ceil() * config.track_pitch_mm)
                    .min(config.maximum_halo_mm)
            } else {
                0.0
            },
            pins,
            side,
            fixed: false,
            angle_options,
            far_side: if through { far_side } else { Vec::new() },
            hollow: Vec::new(),
            tight: if config.tight_bodies == Some(true) { tight_body(footprint)? } else { None },
            edge_inset,
            courtyards: courtyard.0,
            holes_inside: courtyard.1,
            pads: own_pads.clone(),
            copper_only: false,
            cutout_outline: Vec::new(),
        });
        poses.push(core::Pose {
            position: [at[0], at[1]],
            angle: at[2],
        });
        references.push(reference);
        source_at.push(at);
        pad_offsets.push(named);
        pad_boxes.push(own_pads);
        keepout_boxes.push(footprint_keepout_box(footprint, at)?);
        mouths.push(connector_mouth(footprint)?);
    }

    // Rule areas that forbid footprints are obstacles too, and a part the
    // designer placed reaching into one (an antenna module at its keepout)
    // is there on purpose and stays.
    let footprint_count = components.len();
    let mut keepouts: Vec<pcb_router::geometry::Aabb> = Vec::new();
    for item in pcb.children() {
        if item.head() != Some("zone")
            || !item.child("keepout").is_some_and(|keepout| {
                form_atom(keepout, "footprints", 1) == Some("not_allowed")
            })
        {
            continue;
        }
        let Some(polygon) = item.child("polygon").and_then(|polygon| polygon.child("pts")) else {
            continue;
        };
        let mut bounds: Option<pcb_router::geometry::Aabb> = None;
        for point in polygon.children().iter().filter(|child| child.head() == Some("xy")) {
            let coordinate = |index: usize| -> Result<f64, String> {
                point
                    .children()
                    .get(index)
                    .and_then(Expr::atom)
                    .ok_or_else(|| "keepout point without coordinates".to_string())?
                    .parse::<f64>()
                    .map_err(|error| format!("invalid keepout coordinate: {error}"))
            };
            let xy = [coordinate(1)?, coordinate(2)?];
            let corner = pcb_router::geometry::Aabb {
                minimum: xy,
                maximum: xy,
            };
            bounds = Some(bounds.map_or(corner, |bounds| bounds.union(corner)));
        }
        let Some(bounds) = bounds else {
            continue;
        };
        let layers: Vec<&str> = item
            .child("layers")
            .map(|layers| layers.children().iter().skip(1).filter_map(Expr::atom).collect())
            .or_else(|| form_atom(item, "layer", 1).map(|layer| vec![layer]))
            .unwrap_or_default();
        let side = match (layers.iter().any(|l| *l == "F.Cu"), layers.iter().any(|l| *l == "B.Cu")) {
            (true, false) => core::Side::Front,
            (false, true) => core::Side::Back,
            _ => core::Side::Both,
        };
        // A part inside a constraint keepout is not there on purpose.
        if !form_atom(item, "name", 1).is_some_and(|name| name.starts_with(KEEPOUT_NAME)) {
            keepouts.push(bounds);
        }
        components.push(core::Component {
            name: "keepout".into(),
            body_center: [0.0, 0.0],
            body_size: [
                bounds.maximum[0] - bounds.minimum[0],
                bounds.maximum[1] - bounds.minimum[1],
            ],
            round: false,
            halo: 0.0,
            pins: Vec::new(),
            side,
            fixed: true,
            angle_options: vec![0.0],
            far_side: Vec::new(),
            hollow: Vec::new(),
            tight: None,
            edge_inset: 0.0,
            courtyards: Vec::new(),
            holes_inside: false,
            pads: Vec::new(),
            copper_only: false,
            cutout_outline: Vec::new(),
        });
        poses.push(core::Pose {
            position: [
                (bounds.minimum[0] + bounds.maximum[0]) / 2.0,
                (bounds.minimum[1] + bounds.maximum[1]) / 2.0,
            ],
            angle: 0.0,
        });
        references.push("keepout".into());
        source_at.push([0.0; 3]);
        locked.push(true);
    }

    // Copper-layer graphics are part of the board: parts must not be
    // placed on top of them. They follow the footprints, so footprint
    // indices stay aligned with the board file. (Copper and silkscreen
    // texts move off the parts and the copper after placement instead:
    // see `labels`.)
    // KiCad 9 draws a graphic on several layers at once (`layers`):
    // Sisu's GND rectangles on F.Cu and F.Mask.
    let outer_copper = |item: &Expr| -> Vec<usize> {
        let names: Vec<&str> = match form_atom(item, "layer", 1) {
            Some(name) => vec![name],
            None => item.child("layers").map(Expr::children).unwrap_or_default().iter().skip(1).filter_map(Expr::atom).collect(),
        };
        // A mask opening too: a part's pads under it would bridge (Sisu's
        // "REV B2" on B.Mask got two parts' pads in layout).
        let mut sides: Vec<usize> = names
            .into_iter()
            .filter_map(|name| match name {
                "F.Mask" => Some(0),
                "B.Mask" => Some(1),
                name => copper_layer_index(name),
            })
            .collect();
        sides.sort_unstable();
        sides.dedup();
        sides
    };
    for (item, layer) in pcb
        .children()
        .iter()
        .flat_map(|item| outer_copper(item).into_iter().map(move |layer| (item, layer)))
    {
        let on_mask = matches!(form_atom(item, "layer", 1), Some("F.Mask" | "B.Mask"));
        if !matches!(item.head(), Some("gr_line" | "gr_rect" | "gr_arc" | "gr_circle" | "gr_poly"))
            && !(on_mask && item.head() == Some("gr_text"))
        {
            continue;
        }
        let Some(bounds) = board_router::copper_graphic_shapes(item)?
            .iter()
            .map(pcb_router::Shape::aabb)
            .reduce(pcb_router::geometry::Aabb::union)
        else {
            continue;
        };
        components.push(core::Component {
            name: "copper graphic".into(),
            body_center: [0.0, 0.0],
            body_size: [
                bounds.maximum[0] - bounds.minimum[0],
                bounds.maximum[1] - bounds.minimum[1],
            ],
            round: false,
            halo: 0.0,
            pins: Vec::new(),
            side: if layer == 0 {
                core::Side::Front
            } else {
                core::Side::Back
            },
            fixed: true,
            angle_options: vec![0.0],
            far_side: Vec::new(),
            hollow: Vec::new(),
            tight: None,
            edge_inset: 0.0,
            courtyards: Vec::new(),
            holes_inside: false,
            pads: Vec::new(),
            copper_only: false,
            cutout_outline: Vec::new(),
        });
        poses.push(core::Pose {
            position: [
                (bounds.minimum[0] + bounds.maximum[0]) / 2.0,
                (bounds.minimum[1] + bounds.maximum[1]) / 2.0,
            ],
            angle: 0.0,
        });
        references.push("copper graphic".into());
        source_at.push([0.0; 3]);
        locked.push(true);
    }

    // Cutouts are board edge too: parts keep the edge margin from them.
    for cutout in &loops.cutouts {
        let (mut minimum, mut maximum) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        for point in cutout {
            for axis in 0..2 {
                minimum[axis] = minimum[axis].min(point[axis]);
                maximum[axis] = maximum[axis].max(point[axis]);
            }
        }
        if minimum[0] > maximum[0] {
            continue;
        }
        // The hole itself: the copper keeps the edge margin from it in the
        // legality test, which looser levels bring down to the rules'
        // clearance. Grown here too, the margin counted twice and never
        // relaxed (katia: parts found no room beside its cutouts).
        components.push(core::Component {
            name: "cutout".into(),
            body_center: [0.0, 0.0],
            body_size: [maximum[0] - minimum[0], maximum[1] - minimum[1]],
            round: false,
            halo: 0.0,
            pins: Vec::new(),
            side: core::Side::Both,
            fixed: true,
            angle_options: vec![0.0],
            far_side: Vec::new(),
            hollow: Vec::new(),
            tight: None,
            edge_inset: 0.0,
            courtyards: Vec::new(),
            holes_inside: false,
            pads: Vec::new(),
            copper_only: true,
            cutout_outline: cutout.clone(),
        });
        poses.push(core::Pose {
            position: [
                (minimum[0] + maximum[0]) / 2.0,
                (minimum[1] + maximum[1]) / 2.0,
            ],
            angle: 0.0,
        });
        references.push("cutout".into());
        source_at.push([0.0; 3]);
        locked.push(true);
    }

    let mut pin_counts = vec![0usize; net_ids.len()];
    for component in &components {
        for pin in &component.pins {
            pin_counts[pin.net] += 1;
        }
    }
    // Large nets (power) would otherwise dominate and collapse the layout.
    // Nets designers keep short count three times: a switcher's switch
    // node, feedback and bootstrap, RF and antenna feeds, crystal pins (by
    // the pin function KiCad puts in the net's name, "Net-(U9-FB)"; Sisu's
    // feedback divider ended 46 mm from its regulator, its RF feed 52 mm
    // long).
    let mut names = vec![""; net_ids.len()];
    for (name, id) in &net_ids {
        names[*id] = name.as_str();
    }
    let net_weights = pin_counts
        .iter()
        .zip(&names)
        .map(|(pins, name)| {
            let weight = (3.0 / (*pins as f64 - 1.0).max(1.0)).min(1.0);
            if keep_short(name) { 3.0 * weight } else { weight }
        })
        .collect();
    let mut problem = core::Problem {
        outline: loops.outline.clone(),
        components,
        net_weights,
        poses,
        spacing: config.spacing_mm,
        grid: config.grid_mm,
        edge_margin: config.edge_margin_mm,
        min_spacing: config.copper_clearance_mm.unwrap_or(0.2),
        far_side_pads_only: false,
        pieces: loops.pieces.clone(),
        constraints: Default::default(),
    };
    problem.constraints.copper_edge = config.copper_edge_clearance_mm.unwrap_or(0.0).max(0.0);
    for index in 0..footprint_count {
        let reference = &references[index];
        let component = &problem.components[index];
        let pose = problem.poses[index];
        let (center, half) = (component.center(pose), component.half_extent(pose.angle));
        let reach = config.edge_keep_mm;
        let in_keepout = keepouts.iter().any(|keepout| {
            center[0] + half[0] + reach > keepout.minimum[0]
                && center[0] - half[0] - reach < keepout.maximum[0]
                && center[1] + half[1] + reach > keepout.minimum[1]
                && center[1] - half[1] - reach < keepout.maximum[1]
        });
        let on_edge = !core::legal::body_inside_outline(&problem, index, problem.poses[index])
            || in_keepout
            || (connector[index]
                && core::legal::body_near_outline(&problem, index, problem.poses[index], config.edge_keep_mm));
        let default_fixed = locked[index] || problem.components[index].pins.is_empty() || on_edge;
        problem.components[index].fixed = config.fixed.contains(reference)
            || config.fixed_patterns.iter().any(|pattern| glob_matches(pattern, reference))
            || (default_fixed && !config.free.contains(reference));
    }
    let constraint_warnings = match &config.constraints {
        None => Vec::new(),
        Some(KiCadConstraintsSource::Inline(constraints)) => apply_constraints(
            &mut problem,
            &references,
            &pad_offsets,
            &pad_boxes,
            &keepout_boxes,
            &mouths,
            &locked,
            constraints,
            config.constraint_weight,
        )?,
        Some(KiCadConstraintsSource::File(path)) => {
            return Err(format!("constraints file {} was not resolved", path.display()));
        }
    };
    if config.auto_decoupling {
        let automatic = decoupling_relations(pcb, &problem)?;
        if !automatic.is_empty() {
            if problem.constraints.relation_weight == 0.0 {
                problem.constraints.relation_weight = config.constraint_weight;
            }
            problem.constraints.automatic = automatic.len();
            problem.constraints.relations.extend(automatic);
        }
    }
    problem.constraints.link_pairs();
    Ok(LoweredPlacement {
        problem,
        references,
        source_at,
        constraint_warnings,
    })
}

/// Places the movable footprints of `<source>/<board_id>.kicad_pcb`, removes
/// all routed copper, and writes the project plus an HTML playback to
/// `output`.
/// Parts on `"edge": "any"` settle first without that constraint (in a
/// placement thrown away); each then keeps to the board edge nearest to
/// where it settled, the one along its long side on a near tie. Rewrites
/// the constraints and returns the choices.
fn choose_any_edges(
    source_directory: &Path,
    board_id: &str,
    config: &mut KiCadBoardPlacerConfig,
) -> Result<BTreeMap<String, String>, String> {
    let Some(KiCadConstraintsSource::Inline(constraints)) = &config.constraints else {
        return Ok(BTreeMap::new());
    };
    if !constraints.edge.iter().any(|entry| entry.edge == "any") {
        return Ok(BTreeMap::new());
    }
    let mut free = config.clone();
    if let Some(KiCadConstraintsSource::Inline(constraints)) = &mut free.constraints {
        constraints.edge.retain(|entry| entry.edge != "any");
    }
    let directory = std::env::temp_dir().join(format!(
        "pcb-maker-any-edge-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).map(|elapsed| elapsed.as_nanos()).unwrap_or(0)
    ));
    let trial = place_kicad_board(source_directory, board_id, &directory, &free);
    let bounds = fs::read_to_string(directory.join(format!("{board_id}.kicad_pcb")))
        .ok()
        .and_then(|text| parse(&text).ok())
        .and_then(|pcb| outline::board_loops(&pcb).ok())
        .map(|loops| {
            loops.outline.iter().fold([f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY], |b, p| {
                [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])]
            })
        });
    let _ = fs::remove_dir_all(&directory);
    let trial = trial?;
    let bounds = bounds.ok_or("\"edge\": \"any\" needs a board outline (or an `outline` constraint)")?;
    let mut chosen = BTreeMap::new();
    if let Some(KiCadConstraintsSource::Inline(constraints)) = &mut config.constraints {
        for entry in constraints.edge.iter_mut().filter(|entry| entry.edge == "any") {
            let placed = trial
                .footprints
                .iter()
                .find(|footprint| footprint.reference == entry.part)
                .ok_or_else(|| format!("constraint names unknown part {:?}", entry.part))?;
            let ([x, y], [half_x, half_y]) = (placed.body_center, placed.body_half);
            let along_x = half_x >= half_y;
            let edges = [
                ("left", x - half_x - bounds[0], !along_x),
                ("top", y - half_y - bounds[1], along_x),
                ("right", bounds[2] - x - half_x, !along_x),
                ("bottom", bounds[3] - y - half_y, along_x),
            ];
            let cost = |(_, distance, along): &(&str, f64, bool)| distance - if *along { 1.0 } else { 0.0 };
            let (name, _, _) = edges.iter().min_by(|a, b| cost(a).total_cmp(&cost(b))).expect("four edges");
            entry.edge = name.to_string();
            chosen.insert(entry.part.clone(), name.to_string());
        }
    }
    Ok(chosen)
}

pub fn place_kicad_board(
    source_directory: &Path,
    board_id: &str,
    output_directory: &Path,
    config: &KiCadBoardPlacerConfig,
) -> Result<KiCadBoardPlacerResult, String> {
    let started = std::time::Instant::now();
    let source_board = source_directory.join(format!("{board_id}.kicad_pcb"));
    let source = fs::read_to_string(&source_board)
        .map_err(|error| format!("failed to read {}: {error}", source_board.display()))?;
    let mut pcb = parse(&source)?;
    let mut config = resolve_constraints(config, source_directory)?;
    if config.copper_clearance_mm.is_none() || config.copper_edge_clearance_mm.is_none() {
        let rules = project_rules::resolve_project_rules(&source_directory.join(format!("{board_id}.kicad_pro")), &source_board).ok();
        if config.copper_clearance_mm.is_none() {
            config.copper_clearance_mm = rules.as_ref().map(largest_clearance);
        }
        if config.copper_edge_clearance_mm.is_none() {
            config.copper_edge_clearance_mm = rules.as_ref().map(|rules| rules.edge_clearance_mm);
        }
    }
    if config.tight_bodies.is_none() {
        config.tight_bodies = Some(courtyards_may_overlap(&source_directory.join(format!("{board_id}.kicad_pro"))));
    }
    let edges_chosen = choose_any_edges(source_directory, board_id, &mut config)?;
    let config = &config;
    let mut outline_size = None;
    if let Some(KiCadConstraintsSource::Inline(constraints)) = &config.constraints
        && let Some(outline) = &constraints.outline
    {
        let mut part_area = 0.0;
        for footprint in pcb.children().iter().filter(|item| item.head() == Some("footprint")) {
            let (_, size, _) = local_body(footprint)?;
            part_area += (size[0] + config.spacing_mm) * (size[1] + config.spacing_mm);
        }
        outline_size = Some(apply_outline(&mut pcb, outline, part_area)?);
    }
    if let Some(KiCadConstraintsSource::Inline(constraints)) = &config.constraints {
        apply_keepouts(&mut pcb, &constraints.keepout)?;
        apply_sides(&mut pcb, constraints)?;
    }
    let lowered = lower_placement(&pcb, config)?;
    for warning in &lowered.constraint_warnings {
        eprintln!("warning: {warning}");
    }
    let mut placer_config = core::Config::new();
    placer_config.global.whitespace_fill = config.whitespace_fill;
    placer_config.maximum_utilization = config.maximum_utilization;
    placer_config.global.seed = config.seed;
    placer_config.anneal.deadline = config.deadline;
    placer_config.work_limit = config.work_seconds.map(|seconds| (seconds.max(0.0) * core::WORK_PER_SECOND) as u64);
    placer_config.overlap_far_side_pads = config.overlap_far_side_pads;
    // Several seeds, in parallel; the placement with the fewest illegal
    // parts, then the least missed constraints, then the least wirelength
    // wins. Placement takes seconds; the constraints are what the user asked
    // for.
    let seeds = config.placement_seeds.max(1) as u64;
    let mut placements: Vec<core::Placement> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..seeds)
            .map(|offset| {
                let mut seeded = placer_config.clone();
                seeded.global.seed = config.seed + offset;
                seeded.anneal.seed = seeded.anneal.seed.wrapping_add(offset);
                let problem = &lowered.problem;
                scope.spawn(move || core::place(problem, &seeded))
            })
            .collect();
        handles.into_iter().map(|handle| handle.join().expect("placement thread")).collect()
    });
    let key = |placement: &core::Placement| {
        let missed: f64 = placement.constraints.iter().map(|status| status.violation).sum();
        (placement.unplaced.len() + placement.illegal.len(), missed, placement.wirelength_final)
    };
    placements.sort_by(|a, b| key(a).partial_cmp(&key(b)).unwrap_or(std::cmp::Ordering::Equal));
    let work_seconds =
        placements.iter().map(|placement| placement.work).max().unwrap_or(0) as f64 / core::WORK_PER_SECOND;
    let placement = placements.remove(0);
    // The other legal placements, for layout to race when the best one by
    // wirelength does not route: wirelength is not routability.
    let alternatives = placements
        .into_iter()
        .filter(|other| other.unplaced.is_empty() && other.illegal.is_empty())
        .map(|other| {
            let missed = other.constraints.iter().map(|status| status.violation).sum();
            (other.poses, other.relaxation, missed)
        })
        .collect();

    let Expr::List(items) = &mut pcb else {
        return Err("PCB root is not a list".into());
    };
    items.retain(|item| !matches!(item.head(), Some("segment" | "arc" | "via")));
    let mut index = 0;
    let mut footprints = Vec::new();
    for footprint in items
        .iter_mut()
        .filter(|item| item.head() == Some("footprint"))
    {
        let pose = placement.poses[index];
        let source_at = lowered.source_at[index];
        let at = write_footprint_pose(footprint, pose)?;
        footprints.push(KiCadPlacedFootprint {
            reference: lowered.references[index].clone(),
            fixed: lowered.problem.components[index].fixed,
            halo_mm: lowered.problem.components[index].halo,
            body_center: lowered.problem.components[index].center(pose),
            body_half: lowered.problem.components[index].half_extent(pose.angle),
            source_at,
            at,
        });
        index += 1;
    }

    // The written board must put every pad where the placer believed it is.
    index = 0;
    for footprint in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
    {
        let footprint_at = form_at(footprint)?;
        let component = &lowered.problem.components[index];
        let mut pin = 0;
        for pad in footprint
            .children()
            .iter()
            .filter(|child| child.head() == Some("pad"))
        {
            if !node_net(pad).is_some_and(placer_net) {
                continue;
            }
            let written = lower_pad(pad, footprint_at)?.center;
            // A part a row carries has no pins of its own: its pads follow
            // its own pose.
            let expected = match component.pins.get(pin) {
                Some(own) => component.pin_position(own, placement.poses[index]),
                None => {
                    let local = form_at(pad)?;
                    component.pin_position(&core::Pin { offset: [local[0], local[1]], net: 0 }, placement.poses[index])
                }
            };
            if distance_squared(written, expected).sqrt() > 1.0e-5 {
                return Err(format!(
                    "internal error: pad of {} written at {written:?}, expected {expected:?}",
                    lowered.references[index]
                ));
            }
            pin += 1;
        }
        index += 1;
    }

    if output_directory.exists() {
        return Err(format!(
            "output directory {} already exists",
            output_directory.display()
        ));
    }
    copy_directory_tree(source_directory, output_directory)?;
    let output_board = output_directory.join(format!("{board_id}.kicad_pcb"));
    fs::write(&output_board, format!("{}\n", encode(&pcb)))
        .map_err(|error| format!("failed to write {}: {error}", output_board.display()))?;
    let html = core::view::playback_html(
        &lowered.problem,
        &placement.frames,
        &format!("{board_id} placement"),
    );
    let html_path = output_directory.join("placement.html");
    fs::write(&html_path, html)
        .map_err(|error| format!("failed to write {}: {error}", html_path.display()))?;

    let names = |indices: &[usize]| -> Vec<String> {
        indices
            .iter()
            .map(|index| lowered.references[*index].clone())
            .collect()
    };
    // How full each side is, and what to change when parts did not fit.
    let problem = &lowered.problem;
    let outline_area = {
        let outline = &problem.outline;
        (0..outline.len())
            .map(|index| {
                let (a, b) = (outline[index], outline[(index + 1) % outline.len()]);
                a[0] * b[1] - b[0] * a[1]
            })
            .sum::<f64>()
            .abs()
            / 2.0
    };
    let mut used = [0.0f64; 2];
    for (index, component) in problem.components.iter().enumerate() {
        let half = component.half_extent(placement.poses[index].angle);
        let area = if component.hollow.is_empty() {
            (2.0 * half[0] + problem.spacing) * (2.0 * half[1] + problem.spacing)
        } else {
            component.blocking_area()
        };
        match component.side {
            core::Side::Front => used[0] += area,
            core::Side::Back => used[1] += area,
            core::Side::Both => {
                used[0] += area;
                used[1] += area;
            }
            core::Side::Neither => {}
        }
    }
    let utilization = used.map(|used| (used / outline_area.max(1.0e-9) * 1000.0).round() / 1000.0);
    let mut hints = Vec::new();
    if !placement.unplaced.is_empty() || !placement.illegal.is_empty() {
        for (side, share) in ["front", "back"].iter().zip(utilization) {
            if share > 0.7 {
                hints.push(format!(
                    "the {side} side is {:.0} % full: enlarge the outline, move parts to the other side, or fix fewer parts",
                    share * 100.0
                ));
            }
        }
        let constraints = &problem.constraints;
        // Fixed parts are only illegal where a part that found no place
        // lies on them; the moving parts are the ones to talk about.
        for index in placement
            .unplaced
            .iter()
            .chain(&placement.illegal)
            .filter(|index| !problem.components[**index].fixed)
        {
            let name = &lowered.references[*index];
            if constraints.edges.iter().any(|(part, _, _)| part == index)
                || constraints.regions.iter().any(|(part, _)| part == index)
                || constraints.overhangs.iter().any(|(part, _, _)| part == index)
            {
                hints.push(format!(
                    "{name} is held by an edge, region or overhang constraint: loosen it (a larger max_mm or region)"
                ));
            }
        }
        if hints.is_empty() {
            hints.push(
                "no free spot fits the part's courtyard with the copper clearance around it: give the board more room"
                    .into(),
            );
        }
        hints.sort();
        hints.dedup();
    }
    let result = KiCadBoardPlacerResult {
        alternatives,
        tight_bodies: placement.relaxation.tight,
        spacing_mm: placement.relaxation.spacing,
        grid_mm: placement.relaxation.grid,
        halo_scale: placement.relaxation.halo_scale,
        small_halo_scale: placement.relaxation.small_halo_scale,
        edge_inset: placement.relaxation.edge_inset,
        edge_copper: placement.relaxation.edge_copper,
        edge_rule_mm: placement.relaxation.edge_rule,
        utilization,
        hints,
        board_id: board_id.into(),
        components: lowered.problem.components.len(),
        movable: lowered
            .problem
            .components
            .iter()
            .filter(|component| !component.fixed)
            .count(),
        nets: lowered.problem.net_weights.len(),
        global_iterations: placement.global_iterations,
        global_overflow: placement.global_overflow,
        wirelength_source_mm: placement.wirelength_initial,
        wirelength_global_mm: placement.wirelength_global,
        wirelength_legal_mm: placement.wirelength_legal,
        wirelength_final_mm: placement.wirelength_final,
        unplaced: names(&placement.unplaced),
        illegal: names(&placement.illegal),
        constraints: constraint_report(&lowered.problem, &placement.poses, &lowered.references),
        constraint_warnings: lowered.constraint_warnings.clone(),
        outline_mm: outline_size,
        edges_chosen,
        seconds: started.elapsed().as_secs_f64(),
        work_seconds,
        footprints,
    };
    let report_path = output_directory.join("board-placer.json");
    fs::write(
        &report_path,
        serde_json::to_string_pretty(&result).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("failed to write {}: {error}", report_path.display()))?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nets_named_for_a_switch_node_feedback_rf_or_crystal_are_kept_short() {
        for name in ["Net-(U9-FB)", "Net-(U1-SW)", "/RF/ANT", "Net-(J2-RF)", "Net-(U3-XOUT)", "/Supply/5V_BOOT"] {
            assert!(keep_short(name), "{name}");
        }
        // A key switch, a supply, a signal that merely contains the letters.
        for name in ["Net-(SW1-Pad2)", "+3V3", "/SWDIO", "Net-(U4-COL0)", "/FBUS_TX"] {
            assert!(!keep_short(name), "{name}");
        }
    }

    #[test]
    fn a_courtyard_drawn_as_several_shapes_keeps_them_apart() {
        let rectangle = |x0: f64, y0: f64, x1: f64, y1: f64| {
            [[x0, y0, x1, y0], [x1, y0, x1, y1], [x1, y1, x0, y1], [x0, y1, x0, y0]]
                .iter()
                .map(|[a, b, c, d]| format!("(fp_line (start {a} {b}) (end {c} {d}) (layer \"F.CrtYd\"))"))
                .collect::<String>()
        };
        // An outline marking two connectors: two rectangles, lines in any order.
        let mut lines = rectangle(0.0, 0.0, 10.0, 5.0);
        lines.insert_str(0, &rectangle(40.0, 30.0, 50.0, 40.0));
        let footprint = parse(&format!("(footprint \"Board\" (layer \"F.Cu\") (at 0 0) {lines})")).unwrap();
        let (mut shapes, malformed) = courtyard_shapes(&footprint).unwrap();
        assert!(!malformed);
        shapes.sort_by(|a, b| a[0].total_cmp(&b[0]));
        assert_eq!(shapes, vec![[0.0, 0.0, 10.0, 5.0], [40.0, 30.0, 50.0, 40.0]]);
        // One rectangle is one shape: the body covers it.
        let footprint =
            parse(&format!("(footprint \"Part\" (layer \"F.Cu\") (at 0 0) {})", rectangle(0.0, 0.0, 2.0, 1.0))).unwrap();
        assert_eq!(courtyard_shapes(&footprint).unwrap(), (Vec::new(), false));
        // A zero-length line at a corner: KiCad cannot build it.
        let footprint = parse(&format!(
            "(footprint \"Part\" (layer \"F.Cu\") (at 0 0) {} (fp_line (start 2 1) (end 2 1) (layer \"F.CrtYd\")))",
            rectangle(0.0, 0.0, 2.0, 1.0)
        ))
        .unwrap();
        assert!(courtyard_shapes(&footprint).unwrap().1);
    }
}

#[cfg(test)]
mod glob_tests {
    use super::glob_matches;

    #[test]
    fn globs_match_runs_single_characters_and_escaped_literals() {
        assert!(glob_matches("U*", "U12"));
        assert!(glob_matches("R?", "R1") && !glob_matches("R?", "R12"));
        assert!(glob_matches("REF**", "REF**_2"));
        assert!(glob_matches("REF\\*\\*", "REF**") && !glob_matches("REF\\*\\*", "REF**_2") && !glob_matches("REF\\*\\*", "REF12"));
    }
}
