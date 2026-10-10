// Copyright (C) 2026 Toit contributors.

//! Whole-board component placement through the `pcb-placer` core.

use super::*;
use crate::placement_constraints::{
    KEEPOUT_NAME, KiCadConstraintStatus, KiCadConstraintsSource, apply_constraints, apply_keepouts, apply_outline,
    apply_sides, constraint_report, natural, resolve_constraints,
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
    /// tightly as halos allow, 0 spreads them over the whole board. 0 by
    /// default: the spare room then lies between the parts, where routing
    /// needs it (routed in route mode, Sisu's placement left 11 open in
    /// KiCad at 0 and 20 at 0.6; OpenESC mini 62-70 against 80-86 over two
    /// seeds; wirelength within 3 %).
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
    /// Keep room for a via beside every surface pad of a pour net (a
    /// supply pad reaches its plane through a via next to it): the body
    /// grows outward past such pads by `supply_via_room_mm` (the via plus
    /// its clearance, from the rules when not given). Off by default.
    #[serde(default)]
    pub supply_via_room: bool,
    #[serde(default)]
    pub supply_via_room_mm: Option<f64>,
    /// Routing demand as charge in the global placement: every net's
    /// estimated wire area (half perimeter times the track pitch, over its
    /// signal layers) spreads over its bounding box and pushes parts apart
    /// where the wires will run, scaled by this (0: off).
    #[serde(default)]
    pub routing_demand: f64,
    /// Repeated channels (sheet instances of one schematic file with the
    /// same parts: an ESC's four phases, a keyboard's columns) are placed
    /// alike: a first placement finds the best-packed instance, whose
    /// arrangement every instance then takes as one rigid part. Off by
    /// default.
    #[serde(default)]
    pub replicate_channels: bool,
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
            whitespace_fill: 0.0,
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
            supply_via_room: false,
            supply_via_room_mm: None,
            routing_demand: 0.0,
            replicate_channels: false,
        }
    }
}

/// Room a via with its clearance takes, by the board's largest rules.
pub(super) fn via_room(config: &KiCadBoardRouterConfig) -> f64 {
    config
        .connection_rules
        .values()
        .chain(config.default_rules.iter())
        .map(|rules| rules.via_size_mm + rules.clearance_mm)
        .fold(0.0, f64::max)
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
    /// Courtyards came as close as their copper allows (touching).
    #[serde(default)]
    pub courtyard_spacing: bool,
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

/// The box ([min x, min y, max x, max y], own frame) of a footprint
/// graphic (line, rectangle, arc, circle or polygon) with its stroke;
/// `None` for anything else.
fn graphic_box(child: &Expr) -> Result<Option<[f64; 4]>, String> {
    if !matches!(child.head(), Some("fp_line" | "fp_rect" | "fp_arc" | "fp_circle" | "fp_poly")) {
        return Ok(None);
    }
    let width = child
        .child("stroke")
        .and_then(|stroke| form_atom(stroke, "width", 1))
        .or_else(|| form_atom(child, "width", 1))
        .and_then(|width| width.parse::<f64>().ok())
        .unwrap_or(0.0);
    let mut points = outline::outline_points(child)?;
    if child.head() == Some("fp_circle") {
        let center = form_xy(child, "center")?;
        let radius = distance_squared(center, form_xy(child, "end")?).sqrt();
        points.extend([[center[0] - radius, center[1] - radius], [center[0] + radius, center[1] + radius]]);
    }
    points.extend(outline::pts_points(child)?);
    let bounds = points.into_iter().fold(
        [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY],
        |bounds, point| {
            [
                bounds[0].min(point[0] - width / 2.0),
                bounds[1].min(point[1] - width / 2.0),
                bounds[2].max(point[0] + width / 2.0),
                bounds[3].max(point[1] + width / 2.0),
            ]
        },
    );
    Ok(bounds[0].is_finite().then_some(bounds))
}

/// The body without the courtyard's margin: fabrication outline and pads
/// ([min x, min y, max x, max y], own frame). `None` without a fabrication
/// outline, or where it is no smaller than the courtyard.
fn tight_body(footprint: &Expr, skip: Option<&str>) -> Result<Option<[f64; 4]>, String> {
    let (center, size, _, outlined) = body_on_side(footprint, ".Fab", skip)?;
    let (_, courtyard, _, _) = body_on_side(footprint, ".CrtYd", skip)?;
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
    body_on_side(footprint, outline, None)
}

/// `body_with_outline` leaving out the outline and mask graphics on the
/// layers of one side (`"F."` or `"B."`).
fn body_on_side(footprint: &Expr, outline: &str, skip: Option<&str>) -> Result<([f64; 2], [f64; 2], bool, usize), String> {
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
    // A footprint's own mask graphics open the mask over its area: another
    // part's pads placed there bridge (SNSP-CPU-01's J2 polygon: 63 pads
    // of other parts under it). They are part of the body, tight or not.
    let counts = |child: &Expr| {
        form_atom(child, "layer", 1).is_some_and(|layer| {
            (layer.ends_with(outline) || layer.ends_with(".Mask")) && !skip.is_some_and(|side| layer.starts_with(side))
        })
    };
    for child in footprint.children() {
        match child.head() {
            Some("fp_line" | "fp_rect" | "fp_arc") if counts(child) => {
                straight += 1;
                for point in outline::outline_points(child)? {
                    include(point, 0.0);
                }
            }
            Some("fp_circle") if counts(child) => {
                circles += 1;
                let center = form_xy(child, "center")?;
                let end = form_xy(child, "end")?;
                include(center, distance_squared(center, end).sqrt());
            }
            Some("fp_poly") if counts(child) => {
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
    courtyard_shapes_on_side(footprint, None)
}

/// `courtyard_shapes` leaving out the courtyard on one side's layers.
fn courtyard_shapes_on_side(footprint: &Expr, skip: Option<&str>) -> Result<(Vec<[f64; 4]>, bool), String> {
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
        if !form_atom(child, "layer", 1)
            .is_some_and(|layer| layer.ends_with(".CrtYd") && !skip.is_some_and(|side| layer.starts_with(side)))
        {
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
    /// Copper layers signals can use (the board's layers less the planes
    /// a multilayer stack keeps): what the routing demand spreads over.
    pub routing_layers: f64,
    /// Per footprint: which sheet instance it comes from, and what it is.
    pub identities: Vec<Option<ChannelIdentity>>,
}

/// What tells a footprint's channel: its sheet instance (`/ESC1/`), the
/// sheet's file, and the footprint with its value.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ChannelIdentity {
    pub sheet: String,
    pub file: String,
    pub footprint: String,
    pub value: String,
}

/// One instance of a channel: its parts with their offsets (in the first
/// part's frame) and angles relative to the first part, which carries them.
pub(super) type ChannelInstance = Vec<(usize, [f64; 2], f64)>;

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

/// A rule area as a fixed obstacle for the placer, with its bounds and
/// whether it keeps bodies out: one that forbids footprints blocks bodies
/// on the sides of its layers; one that allows footprints but forbids pads
/// keeps copper out only, as a cutout does (bodies may reach over it).
/// `None` for other zones.
fn rule_area_obstacle(
    item: &Expr,
) -> Result<Option<(core::Component, core::Pose, pcb_router::geometry::Aabb, bool)>, String> {
    let Some(keepout) = item.child("keepout") else {
        return Ok(None);
    };
    let bodies = form_atom(keepout, "footprints", 1) == Some("not_allowed");
    let pads = form_atom(keepout, "pads", 1) == Some("not_allowed");
    if !bodies && !pads {
        return Ok(None);
    }
    let Some(polygon) = item.child("polygon").and_then(|polygon| polygon.child("pts")) else {
        return Ok(None);
    };
    let mut points = Vec::new();
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
        points.push([coordinate(1)?, coordinate(2)?]);
    }
    let Some(bounds) = points
        .iter()
        .map(|xy| pcb_router::geometry::Aabb { minimum: *xy, maximum: *xy })
        .reduce(pcb_router::geometry::Aabb::union)
    else {
        return Ok(None);
    };
    let layers: Vec<&str> = item
        .child("layers")
        .map(|layers| layers.children().iter().skip(1).filter_map(Expr::atom).collect())
        .or_else(|| form_atom(item, "layer", 1).map(|layer| vec![layer]))
        .unwrap_or_default();
    let all = layers.iter().any(|layer| layer.starts_with('*') || *layer == "F&B.Cu");
    let side = match (all || layers.contains(&"F.Cu"), all || layers.contains(&"B.Cu")) {
        (true, false) => core::Side::Front,
        (false, true) => core::Side::Back,
        // Inner layers only: no part sits there.
        (false, false) if !layers.is_empty() => return Ok(None),
        _ => core::Side::Both,
    };
    let component = core::Component {
        name: "keepout".into(),
        body_center: [0.0, 0.0],
        body_size: [bounds.maximum[0] - bounds.minimum[0], bounds.maximum[1] - bounds.minimum[1]],
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
        copper_only: !bodies,
        cutout_outline: if bodies { Vec::new() } else { points },
        tight_hollow: Vec::new(),
    };
    let pose = core::Pose {
        position: [(bounds.minimum[0] + bounds.maximum[0]) / 2.0, (bounds.minimum[1] + bounds.maximum[1]) / 2.0],
        angle: 0.0,
    };
    Ok(Some((component, pose, bounds, bodies)))
}

pub(super) fn lower_placement(
    pcb: &Expr,
    config: &KiCadBoardPlacerConfig,
    channels: &[ChannelInstance],
) -> Result<LoweredPlacement, String> {
    crate::note_net_names(pcb);
    let mut identities = Vec::new();
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
    // The nets with copper pours, whose surface pads want a via beside
    // them, and the layers signals can route on: a four-layer stack keeps
    // one plane, six or more two (what the harvested designers do).
    let layer_table = crate::board_router::LayerTable::from_pcb(pcb)?;
    let pour_nets: BTreeSet<String> = crate::board_router::pours(pcb, &layer_table)?
        .into_iter()
        .map(|pour| normalize_net(&pour.net).to_string())
        .collect();
    let copper_layers = layer_table.len();
    let routing_layers = match copper_layers {
        0..=2 => 2.0,
        3..=5 => (copper_layers - 1) as f64,
        _ => (copper_layers - 2) as f64,
    };
    let via_room = if config.supply_via_room { config.supply_via_room_mm.unwrap_or(0.8) } else { 0.0 };
    for footprint in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
    {
        let at = form_at(footprint)?;
        let reference = footprint_reference(footprint).unwrap_or_default();
        let (mut body_center, mut body_size, round) = local_body(footprint)?;
        // A footprint with a courtyard on each side (a hot-swap switch: the
        // socket's on the back, the switch's on the front; a reversible
        // part) takes its own side's courtyard as its body and the other's
        // as what it occupies there: KiCad checks courtyards side by side
        // (urchin's back switches claimed the front switch's square on the
        // back too, and found no room among the mounting holes and logos
        // the designer put between them).
        let courtyard_on = |prefix: &str| {
            footprint.children().iter().any(|child| {
                matches!(child.head(), Some("fp_line" | "fp_rect" | "fp_arc" | "fp_circle" | "fp_poly"))
                    && form_atom(child, "layer", 1).is_some_and(|layer| layer.starts_with(prefix) && layer.ends_with(".CrtYd"))
            })
        };
        // A part without SMD pads whose only courtyard is on the other side
        // sits on that side (urchin's tenting puck: a front footprint with
        // its courtyard on B.CrtYd).
        let back = {
            let back = on_back(footprint);
            let (own, other) = if back { ("B.", "F.") } else { ("F.", "B.") };
            let smd = footprint.children().iter().any(|child| {
                child.head() == Some("pad") && child.children().get(2).and_then(Expr::atom) == Some("smd")
            });
            back != (!smd && !courtyard_on(own) && courtyard_on(other))
        };
        let (own_prefix, other_prefix) = if back { ("B.", "F.") } else { ("F.", "B.") };
        let mut far_courtyard = None;
        if courtyard_on(own_prefix) && courtyard_on(other_prefix) {
            let (center, size, own_round, _) = body_on_side(footprint, ".CrtYd", Some(other_prefix))?;
            if !own_round {
                body_center = center;
                body_size = size;
            }
            far_courtyard = footprint
                .children()
                .iter()
                .filter(|child| {
                    form_atom(child, "layer", 1).is_some_and(|layer| layer.starts_with(other_prefix) && layer.ends_with(".CrtYd"))
                })
                .map(graphic_box)
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .flatten()
                .reduce(|a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])])
                // Where the project lets courtyards overlap, KiCad does not
                // check the other side's either (ErgoSNM's diodes sit in
                // the keys' socket outlines on the back).
                .filter(|_| config.tight_bodies != Some(true));
        }
        let mut pins = Vec::new();
        let mut through = false;
        let mut far_side = Vec::new();
        let mut own_pads = Vec::new();
        // Surface pads of pour nets: the body grows past them by the via
        // room, on the side they lie nearest to.
        let mut supply_pads: Vec<[f64; 4]> = Vec::new();
        let mut named = BTreeMap::new();
        // The copper layer of the side the part is not on: an edge-mount
        // connector's SMD pads there occupy that side too.
        let other_copper = if back { "F.Cu" } else { "B.Cu" };
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
            if via_room > 0.0 && pad_type == "smd" && pour_nets.contains(net) {
                supply_pads.push(pad_box);
            }
            let next = net_ids.len();
            let net = *net_ids.entry(net.to_string()).or_insert(next);
            let pad_at = form_at(pad)?;
            pins.push(core::Pin {
                offset: [pad_at[0], pad_at[1]],
                net,
            });
        }
        if !supply_pads.is_empty() && !round {
            let mut body = [
                body_center[0] - body_size[0] / 2.0,
                body_center[1] - body_size[1] / 2.0,
                body_center[0] + body_size[0] / 2.0,
                body_center[1] + body_size[1] / 2.0,
            ];
            for pad in &supply_pads {
                // The side the pad lies nearest to; a pad deep inside the
                // body (an exposed pad) takes its via inside itself.
                let gaps = [pad[0] - body[0], pad[1] - body[1], body[2] - pad[2], body[3] - pad[3]];
                let (side, gap) = gaps
                    .iter()
                    .copied()
                    .enumerate()
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .unwrap();
                if gap >= via_room {
                    continue;
                }
                match side {
                    0 => body[0] = body[0].min(pad[0] - via_room),
                    1 => body[1] = body[1].min(pad[1] - via_room),
                    2 => body[2] = body[2].max(pad[2] + via_room),
                    _ => body[3] = body[3].max(pad[3] + via_room),
                }
            }
            body_center = [(body[0] + body[2]) / 2.0, (body[1] + body[3]) / 2.0];
            body_size = [body[2] - body[0], body[3] - body[1]];
        }
        // Copper and mask graphics on the outer layers of the side the part
        // is not on occupy that side as well: Castor's LOGO footprint sits
        // on the front with its copper polygon on B.Cu, and a part placed
        // on the back over it was shorted and bridged.
        let (own_side, other_side) = if back {
            (core::Side::Back, core::Side::Front)
        } else {
            (core::Side::Front, core::Side::Back)
        };
        let other_mask = if other_copper == "F.Cu" { "F.Mask" } else { "B.Mask" };
        let mut own_graphics = false;
        let mut far_graphics = Vec::new();
        for child in footprint.children() {
            let Some(layer) = form_atom(child, "layer", 1) else {
                continue;
            };
            if !layer.ends_with(".Cu") && !layer.ends_with(".Mask") {
                continue;
            }
            let Some(graphic) = graphic_box(child)? else {
                continue;
            };
            if layer == other_copper || layer == other_mask {
                far_graphics.push(graphic);
            } else if matches!(layer, "F.Cu" | "B.Cu" | "F.Mask" | "B.Mask") {
                own_graphics = true;
            }
        }
        // Artwork (a logo) has neither pads nor a courtyard: it occupies the
        // sides its copper and mask graphics are on (its body is their box),
        // and nothing without any, rather than being a 1 mm wall at its
        // origin.
        let artwork = !footprint.children().iter().any(|child| {
            child.head() == Some("pad")
                || (matches!(child.head(), Some("fp_line" | "fp_rect" | "fp_arc" | "fp_circle" | "fp_poly"))
                    && form_atom(child, "layer", 1).is_some_and(|layer| layer.ends_with(".CrtYd")))
        });
        // A through-hole part sits on its footprint's side; on the other
        // side only its holes and pads (`far_side`) are in the way, and its
        // copper and mask graphics there.
        let side = if artwork {
            match (own_graphics, !far_graphics.is_empty()) {
                (false, false) => core::Side::Neither,
                (true, false) => own_side,
                (false, true) => other_side,
                (true, true) => core::Side::Both,
            }
        } else {
            far_side.extend(far_graphics.iter().copied());
            far_side.extend(far_courtyard);
            own_side
        };
        let far_graphics = !artwork && (!far_graphics.is_empty() || far_courtyard.is_some());
        let angle_options = if config.keep_rotation {
            vec![at[2]]
        } else {
            let mut angles: Vec<f64> = (0..4).map(|quarter| normalize_angle(at[2] + 90.0 * quarter as f64)).collect();
            // A part at an odd angle (a thumb cluster's keys at -30) may
            // also turn square to the board: the placer's bodies are boxes
            // around the turned part, half as large again at 30 degrees
            // (ErgoSNM: two keys found no room).
            if normalize_angle(at[2]) % 90.0 > 1.0e-6 {
                angles.extend([0.0, 90.0, 180.0, 270.0]);
            }
            angles
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
                if !form_atom(child, "layer", 1).is_some_and(|layer| layer.ends_with(".Cu")) {
                    continue;
                }
                if let Some(graphic) = graphic_box(child)? {
                    pads = [
                        pads[0].min(graphic[0]),
                        pads[1].min(graphic[1]),
                        pads[2].max(graphic[2]),
                        pads[3].max(graphic[3]),
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
        let courtyard =
            courtyard_shapes_on_side(footprint, (courtyard_on(own_prefix) && courtyard_on(other_prefix)).then_some(other_prefix))?;
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
            far_side: if through || far_graphics { far_side } else { Vec::new() },
            hollow: Vec::new(),
            tight: if config.tight_bodies == Some(true) {
                tight_body(footprint, (courtyard_on(own_prefix) && courtyard_on(other_prefix)).then_some(other_prefix))?
            } else {
                None
            },
            edge_inset,
            courtyards: courtyard.0,
            holes_inside: courtyard.1,
            pads: own_pads.clone(),
            copper_only: false,
            cutout_outline: Vec::new(),
            tight_hollow: Vec::new(),
        });
        poses.push(core::Pose {
            position: [at[0], at[1]],
            angle: at[2],
        });
        identities.push(footprint.children().get(1).and_then(Expr::atom).map(|id| ChannelIdentity {
            sheet: form_atom(footprint, "sheetname", 1).unwrap_or("").to_string(),
            file: form_atom(footprint, "sheetfile", 1).unwrap_or("").to_string(),
            footprint: id.to_string(),
            value: footprint
                .children()
                .iter()
                .find(|item| item.head() == Some("property") && item.children().get(1).and_then(Expr::atom) == Some("Value"))
                .and_then(|item| item.children().get(2).and_then(Expr::atom))
                .unwrap_or("")
                .to_string(),
        }));
        references.push(reference);
        source_at.push(at);
        pad_offsets.push(named);
        pad_boxes.push(own_pads);
        keepout_boxes.push(footprint_keepout_box(footprint, at)?);
        mouths.push(connector_mouth(footprint)?);
    }

    // Rule areas that forbid footprints are obstacles too, and a part the
    // designer placed reaching into one (an antenna module at its keepout)
    // is there on purpose and stays. One that allows footprints but forbids
    // pads keeps copper out only (d20's back: KiCad flagged 27 of our pads
    // in it).
    let footprint_count = components.len();
    let mut keepouts: Vec<pcb_router::geometry::Aabb> = Vec::new();
    for item in pcb.children().iter().filter(|item| item.head() == Some("zone")) {
        let Some((component, pose, bounds, bodies)) = rule_area_obstacle(item)? else {
            continue;
        };
        // A part inside a constraint keepout is not there on purpose.
        if bodies && !form_atom(item, "name", 1).is_some_and(|name| name.starts_with(KEEPOUT_NAME)) {
            keepouts.push(bounds);
        }
        components.push(component);
        poses.push(pose);
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
            tight_hollow: Vec::new(),
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
            tight_hollow: Vec::new(),
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
    // Fixed parts on top of each other place and route without a word,
    // and the board comes back with opens: say so (fence issue E).
    let mut stacked_warnings = Vec::new();
    {
        let fixed: Vec<usize> = (0..footprint_count).filter(|index| problem.components[*index].fixed && !problem.components[*index].pins.is_empty()).collect();
        for (a, first) in fixed.iter().enumerate() {
            for second in &fixed[a + 1..] {
                let (ca, cb) = (&problem.components[*first], &problem.components[*second]);
                if !ca.side.collides(cb.side) {
                    continue;
                }
                let (pa, pb) = (problem.poses[*first], problem.poses[*second]);
                let (center_a, half_a) = (ca.center(pa), ca.half_extent(pa.angle));
                let (center_b, half_b) = (cb.center(pb), cb.half_extent(pb.angle));
                let overlap = (0..2).all(|axis| (center_a[axis] - center_b[axis]).abs() < half_a[axis] + half_b[axis] - 0.05);
                if overlap {
                    stacked_warnings.push(format!("fixed parts {} and {} overlap", references[*first], references[*second]));
                }
            }
        }
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
    // A fixed footprint's own rule areas (an antenna module's keepout, in
    // board coordinates) are obstacles like the board's: PixelWave's
    // ESP32 on the back keeps parts out of its antenna area on both sides,
    // and KiCad flagged 50 pads and parts we put there. A movable part's
    // keepout moves with it: it blocks the other side as far-side box.
    for (index, footprint) in pcb.children().iter().filter(|item| item.head() == Some("footprint")).enumerate() {
        for zone in footprint.children().iter().filter(|child| child.head() == Some("zone")) {
            let Some((component, pose, _, bodies)) = rule_area_obstacle(zone)? else {
                continue;
            };
            if problem.components[index].fixed {
                problem.components.push(component);
                problem.poses.push(pose);
                references.push("keepout".into());
                source_at.push([0.0; 3]);
            } else if bodies && component.side.collides(match problem.components[index].side {
                core::Side::Front => core::Side::Back,
                core::Side::Back => core::Side::Front,
                _ => core::Side::Neither,
            }) {
                let at = source_at[index];
                let half = [component.body_size[0] / 2.0, component.body_size[1] / 2.0];
                let corners = [[-half[0], -half[1]], [half[0], -half[1]], [half[0], half[1]], [-half[0], half[1]]];
                let local = corners
                    .map(|corner| core::problem::rotate([pose.position[0] + corner[0] - at[0], pose.position[1] + corner[1] - at[1]], at[2]));
                problem.components[index].far_side.push(local.iter().fold(
                    [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY],
                    |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])],
                ));
            }
        }
    }
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
    apply_channels(&mut problem, &references, &pad_boxes, channels)?;
    problem.constraints.link_pairs();
    let mut constraint_warnings = constraint_warnings;
    constraint_warnings.extend(stacked_warnings);
    Ok(LoweredPlacement {
        problem,
        references,
        source_at,
        constraint_warnings,
        routing_layers,
        identities,
    })
}

/// A box turned by `angle` (a quarter turn or any other) about the origin.
fn turned_box(b: [f64; 4], angle: f64) -> [f64; 4] {
    let corners = [[b[0], b[1]], [b[2], b[1]], [b[2], b[3]], [b[0], b[3]]].map(|corner| core::problem::rotate(corner, angle));
    corners.iter().fold([f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY], |acc, c| {
        [acc[0].min(c[0]), acc[1].min(c[1]), acc[2].max(c[0]), acc[3].max(c[1])]
    })
}

/// The repeated channels of a board: groups of sheet instances of one
/// file whose movable parts match one to one (same footprints and values,
/// paired in reference order, with the nets wired alike). Each group lists
/// its instances, each instance its parts in the paired order.
pub(super) fn channel_groups(lowered: &LoweredPlacement) -> Vec<Vec<Vec<usize>>> {
    let problem = &lowered.problem;
    // Instance (sheet, file) -> its movable parts.
    let mut instances: BTreeMap<(String, String), Vec<usize>> = BTreeMap::new();
    for (index, identity) in lowered.identities.iter().enumerate() {
        let Some(identity) = identity else { continue };
        if identity.sheet.is_empty() || identity.sheet == "/" || identity.file.is_empty() {
            continue;
        }
        instances.entry((identity.sheet.clone(), identity.file.clone())).or_default().push(index);
    }
    // Instances by file and signature (what parts they hold).
    let mut by_signature: BTreeMap<(String, Vec<(String, String, bool)>), Vec<Vec<usize>>> = BTreeMap::new();
    for ((_, file), mut members) in instances {
        if members.len() < 3 {
            continue;
        }
        let identity = |index: usize| lowered.identities[index].as_ref().unwrap();
        members.sort_by(|a, b| {
            (&identity(*a).footprint, &identity(*a).value, natural(&lowered.references[*a]))
                .cmp(&(&identity(*b).footprint, &identity(*b).value, natural(&lowered.references[*b])))
        });
        let signature: Vec<(String, String, bool)> = members
            .iter()
            .map(|index| {
                let component = &problem.components[*index];
                (identity(*index).footprint.clone(), identity(*index).value.clone(), component.side == core::Side::Back)
            })
            .collect();
        by_signature.entry((file, signature)).or_default().push(members);
    }
    let mut groups = Vec::new();
    for (_, instances) in by_signature {
        if instances.len() < 2 {
            continue;
        }
        // Every part movable, and the nets wired alike: the first
        // instance's nets map one to one onto each other's.
        let movable = instances.iter().flatten().all(|index| !problem.components[*index].fixed);
        let alike = instances[1..].iter().all(|other| {
            let mut map: BTreeMap<usize, usize> = BTreeMap::new();
            instances[0].iter().zip(other).all(|(a, b)| {
                let (pins_a, pins_b) = (&problem.components[*a].pins, &problem.components[*b].pins);
                pins_a.len() == pins_b.len()
                    && pins_a.iter().zip(pins_b).all(|(pin_a, pin_b)| *map.entry(pin_a.net).or_insert(pin_b.net) == pin_b.net)
            })
        });
        if movable && alike {
            groups.push(instances);
        }
    }
    groups
}

/// From a placement of the whole board: every instance of every group
/// arranged like the group's best-packed instance (the one whose own nets
/// are shortest), as offsets and angles relative to its first part.
pub(super) fn channel_templates(lowered: &LoweredPlacement, groups: &[Vec<Vec<usize>>], poses: &[core::Pose]) -> Vec<ChannelInstance> {
    let problem = &lowered.problem;
    let mut templates = Vec::new();
    for instances in groups {
        let best = instances
            .iter()
            .min_by(|a, b| {
                let local = |members: &Vec<usize>| -> f64 {
                    let inside: BTreeSet<usize> = members.iter().copied().collect();
                    let mut boxes: BTreeMap<usize, [f64; 4]> = BTreeMap::new();
                    for (index, component) in problem.components.iter().enumerate() {
                        for pin in &component.pins {
                            let at = component.pin_position(pin, poses[index]);
                            let entry = boxes.entry(pin.net).or_insert([f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY]);
                            if inside.contains(&index) {
                                *entry = [entry[0].min(at[0]), entry[1].min(at[1]), entry[2].max(at[0]), entry[3].max(at[1])];
                            } else {
                                // A net leaving the instance is not its own.
                                *entry = [f64::NAN; 4];
                            }
                        }
                    }
                    boxes.values().filter(|b| b[0].is_finite()).map(|b| (b[2] - b[0]) + (b[3] - b[1])).sum()
                };
                local(a).total_cmp(&local(b))
            })
            .unwrap();
        let leader = poses[best[0]];
        let arrangement: Vec<([f64; 2], f64)> = best
            .iter()
            .map(|member| {
                let pose = poses[*member];
                let delta = [pose.position[0] - leader.position[0], pose.position[1] - leader.position[1]];
                (core::problem::rotate(delta, leader.angle), (pose.angle - leader.angle).rem_euclid(360.0))
            })
            .collect();
        for members in instances {
            // PCB_PLACER_CHANNEL_SELF: every instance keeps its own
            // arrangement (a check of the rigid parts: the free placement
            // seats them, so the rigid one must).
            if std::env::var_os("PCB_PLACER_CHANNEL_SELF").is_some() {
                let leader = poses[members[0]];
                templates.push(
                    members
                        .iter()
                        .map(|member| {
                            let pose = poses[*member];
                            let delta = [pose.position[0] - leader.position[0], pose.position[1] - leader.position[1]];
                            (*member, core::problem::rotate(delta, leader.angle), (pose.angle - leader.angle).rem_euclid(360.0))
                        })
                        .collect(),
                );
                continue;
            }
            templates.push(members.iter().zip(&arrangement).map(|(member, (offset, angle))| (*member, *offset, *angle)).collect());
        }
    }
    templates
}

/// Turns every channel instance into one part, as `apply_rows` does for
/// a row: the first part carries the others' bodies (as separate blocking
/// boxes, so parts may sit in the channel's gaps), pins and pads at their
/// places; the others follow it.
fn apply_channels(
    problem: &mut core::Problem,
    references: &[String],
    pad_boxes: &[Vec<[f64; 4]>],
    channels: &[ChannelInstance],
) -> Result<(), String> {
    for instance in channels {
        let Some(&(leader, _, _)) = instance.first() else { continue };
        if instance.iter().any(|(member, _, _)| {
            problem.constraints.followers.iter().any(|follower| follower.part == *member || follower.leader == *member)
                || problem.components[*member].fixed
        }) {
            return Err(format!("channel of {} overlaps a row or a fixed part", references[leader]));
        }
        let mut body = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        let mut tight = body;
        let mut carried = problem.components[leader].clone();
        carried.pins.clear();
        carried.far_side.clear();
        carried.pads.clear();
        carried.hollow.clear();
        carried.tight_hollow.clear();
        let leader_side = carried.side;
        for (member, offset, angle) in instance {
            let component = &problem.components[*member];
            // A member on the other side blocks there, as a through-hole
            // part's pins do; what it has on the leader's side joins the
            // leader's boxes.
            let same_side = component.side == leader_side || component.side == core::Side::Both;
            // A member-local point lies at offset + rotate(point, -angle)
            // in the leader's frame (see `Problem::sync_followers`).
            let place = |b: &[f64; 4]| -> [f64; 4] {
                let turned = turned_box(*b, -angle);
                [turned[0] + offset[0], turned[1] + offset[1], turned[2] + offset[0], turned[3] + offset[1]]
            };
            let own_body = place(&[
                component.body_center[0] - component.body_size[0] / 2.0,
                component.body_center[1] - component.body_size[1] / 2.0,
                component.body_center[0] + component.body_size[0] / 2.0,
                component.body_center[1] + component.body_size[1] / 2.0,
            ]);
            body = [body[0].min(own_body[0]), body[1].min(own_body[1]), body[2].max(own_body[2]), body[3].max(own_body[3])];
            // At the tight level the members are tight too (ESC mini's
            // boards fit only so).
            let own_tight = component.tight.map_or(own_body, |t| place(&t));
            tight = [tight[0].min(own_tight[0]), tight[1].min(own_tight[1]), tight[2].max(own_tight[2]), tight[3].max(own_tight[3])];
            if same_side {
                carried.hollow.push(own_body);
                carried.tight_hollow.push(own_tight);
                carried.far_side.extend(component.far_side.iter().map(place));
            } else {
                carried.far_side.push(own_body);
                carried.hollow.extend(component.far_side.iter().map(place));
            }
            carried.pins.extend(component.pins.iter().map(|pin| {
                let at = core::problem::rotate(pin.offset, -angle);
                core::Pin { offset: [at[0] + offset[0], at[1] + offset[1]], net: pin.net }
            }));
            carried.pads.extend(pad_boxes[*member].iter().map(place));
            carried.halo = carried.halo.max(component.halo);
            carried.edge_inset = carried.edge_inset.min(component.edge_inset);
            if *member != leader {
                problem.constraints.followers.push(core::constraints::Follower {
                    part: *member,
                    leader,
                    offset: *offset,
                    angle: *angle,
                });
            }
        }
        carried.body_center = [(body[0] + body[2]) / 2.0, (body[1] + body[3]) / 2.0];
        carried.body_size = [body[2] - body[0], body[3] - body[1]];
        carried.round = false;
        carried.tight = (carried.tight_hollow.len() == carried.hollow.len() && !carried.tight_hollow.is_empty()).then_some(tight);
        carried.courtyards.clear();
        carried.holes_inside = true;
        if carried.hollow.is_empty() {
            // Everything on the other side: the leader's own side is free.
            carried.hollow.push([carried.body_center[0], carried.body_center[1], carried.body_center[0], carried.body_center[1]]);
        }
        if std::env::var_os("PCB_PLACER_DEBUG").is_some() {
            eprintln!(
                "channel of {}: {} parts, {:.1} x {:.1} mm, {} boxes on its side, {} on the other",
                references[leader],
                instance.len(),
                carried.body_size[0],
                carried.body_size[1],
                carried.hollow.len(),
                carried.far_side.len()
            );
        }
        problem.components[leader] = carried;
        for (member, _, _) in &instance[1..] {
            let component = &mut problem.components[*member];
            component.side = core::Side::Neither;
            component.fixed = true;
            component.pins.clear();
            component.far_side.clear();
            component.pads.clear();
            component.hollow.clear();
            component.halo = 0.0;
            component.body_size = [0.01, 0.01];
        }
    }
    if !channels.is_empty() {
        let mut poses = problem.poses.clone();
        problem.sync_followers(&mut poses);
        problem.poses = poses;
    }
    Ok(())
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

/// Debugging aid (`PCB_PLACER_DESIGNER=<designer's .kicad_pcb>`): the
/// designer's poses judged by the placer's model at its loosest level, with
/// what each illegal movable part meets.
fn diagnose_designer(lowered: &LoweredPlacement, designer: &Path) -> Result<(), String> {
    let text = fs::read_to_string(designer).map_err(|error| format!("{}: {error}", designer.display()))?;
    let pcb = parse(&text)?;
    let mut at_by_reference: BTreeMap<String, Vec<[f64; 3]>> = BTreeMap::new();
    for footprint in pcb.children().iter().filter(|item| item.head() == Some("footprint")) {
        at_by_reference.entry(footprint_reference(footprint).unwrap_or_default()).or_default().push(form_at(footprint)?);
    }
    let mut problem = lowered.problem.clone();
    let mut poses = problem.poses.clone();
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for (index, reference) in lowered.references.iter().enumerate() {
        if problem.components[index].fixed {
            continue;
        }
        let nth = seen.entry(reference.clone()).or_default();
        if let Some(at) = at_by_reference.get(reference).and_then(|ats| ats.get(*nth)) {
            poses[index] = core::Pose { position: [at[0], at[1]], angle: at[2] };
        }
        *nth += 1;
    }
    let level = std::env::var("PCB_PLACER_DESIGNER_LEVEL").unwrap_or_default();
    if level != "full" {
        problem.spacing = problem.min_spacing;
        problem.grid = problem.grid.min(0.1);
        problem.edge_margin = problem.edge_margin.min((problem.constraints.copper_edge + 0.05).min(problem.edge_margin));
        for component in &mut problem.components {
            component.halo = 0.0;
            if level == "tight" {
                component.use_tight_body();
            }
        }
    }
    let count = problem.components.len();
    let bounds = problem.bounds();
    eprintln!("designer: outline bounds {bounds:?}, {} pieces, edge margin {}, {} points: {:?}", problem.pieces.len(), problem.edge_margin, problem.outline.len(), problem.outline.iter().step_by((problem.outline.len() / 40).max(1)).collect::<Vec<_>>());
    let mut illegal = 0;
    for index in 0..count {
        if problem.components[index].fixed {
            continue;
        }
        let mut reasons = Vec::new();
        let outside = !core::legal::is_legal(&problem, &poses, index, poses[index], std::iter::empty());
        if outside {
            reasons.push("outline or hard constraint".to_string());
        }
        for other in (0..count).filter(|_| !outside) {
            if other != index && !core::legal::is_legal(&problem, &poses, index, poses[index], std::iter::once(other)) {
                reasons.push(format!("{} ({:?})", lowered.references[other], problem.components[other].side));
            }
        }
        if !reasons.is_empty() {
            illegal += 1;
            let component = &problem.components[index];
            eprintln!(
                "designer: {} ({:?}, body {:.2} x {:.2}, at {:?}) meets {}",
                lowered.references[index],
                component.side,
                component.body_size[0],
                component.body_size[1],
                poses[index],
                reasons.join(", ")
            );
        }
    }
    eprintln!("designer: {illegal} movable parts illegal at the designer's poses (level {level:?})");
    Ok(())
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
    if config.copper_clearance_mm.is_none()
        || config.copper_edge_clearance_mm.is_none()
        || (config.supply_via_room && config.supply_via_room_mm.is_none())
    {
        let rules = project_rules::resolve_project_rules(&source_directory.join(format!("{board_id}.kicad_pro")), &source_board).ok();
        if config.copper_clearance_mm.is_none() {
            config.copper_clearance_mm = rules.as_ref().map(largest_clearance);
        }
        if config.copper_edge_clearance_mm.is_none() {
            config.copper_edge_clearance_mm = rules.as_ref().map(|rules| rules.edge_clearance_mm);
        }
        if config.supply_via_room && config.supply_via_room_mm.is_none() {
            config.supply_via_room_mm = rules.as_ref().map(via_room).filter(|room| *room > 0.0);
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
    let mut lowered = lower_placement(&pcb, config, &[])?;
    if let Some(designer) = std::env::var_os("PCB_PLACER_DESIGNER") {
        diagnose_designer(&lowered, Path::new(&designer))?;
    }
    for warning in &lowered.constraint_warnings {
        eprintln!("warning: {warning}");
    }
    let mut placer_config = core::Config::new();
    placer_config.global.whitespace_fill = config.whitespace_fill;
    placer_config.global.routing_demand = config.routing_demand;
    placer_config.global.routing_layers = lowered.routing_layers;
    placer_config.global.track_pitch = config.track_pitch_mm;
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
    let place_seeds = |problem: &core::Problem| -> Vec<core::Placement> {
        let mut placements: Vec<core::Placement> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..seeds)
                .map(|offset| {
                    let mut seeded = placer_config.clone();
                    seeded.global.seed = config.seed + offset;
                    seeded.anneal.seed = seeded.anneal.seed.wrapping_add(offset);
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
        placements
    };
    // Repeated channels: a first placement, with every instance's parts
    // pulled together by a net of their own, shows which instance packs
    // best; every instance then takes that arrangement as one rigid part
    // and the board is placed again. Kept when every part finds room.
    let groups = if config.replicate_channels { channel_groups(&lowered) } else { Vec::new() };
    let mut placements = if groups.is_empty() {
        place_seeds(&lowered.problem)
    } else {
        let mut cohesive = lowered.problem.clone();
        for members in groups.iter().flatten() {
            let net = cohesive.net_weights.len();
            cohesive.net_weights.push(0.5);
            for member in members {
                let component = &mut cohesive.components[*member];
                component.pins.push(core::Pin { offset: component.body_center, net });
            }
        }
        place_seeds(&cohesive)
    };
    if config.replicate_channels {
        if !groups.is_empty() {
            let templates = channel_templates(&lowered, &groups, &placements[0].poses);
            eprintln!(
                "channels: {} ({} instances of {} parts): placed alike",
                groups.len(),
                templates.len(),
                groups.iter().map(|instances| instances[0].len()).max().unwrap_or(0)
            );
            let replicated = lower_placement(&pcb, config, &templates)?;
            let again = place_seeds(&replicated.problem);
            if again[0].unplaced.is_empty() && again[0].illegal.is_empty() {
                lowered = replicated;
                placements = again;
            } else {
                let stuck: Vec<&str> = again[0]
                    .unplaced
                    .iter()
                    .chain(&again[0].illegal)
                    .map(|index| replicated.references[*index].as_str())
                    .collect();
                eprintln!("channels: the replicated placement left {stuck:?} without room; the free one stands");
            }
        }
    }
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
        courtyard_spacing: placement.relaxation.courtyard_spacing,
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
    fn a_front_footprints_copper_on_the_back_blocks_back_parts_there() {
        // A netless logo on the front whose copper polygon lies on B.Cu
        // (Castor's LOGO), and a part with a courtyard whose B.Mask graphic
        // opens the back's mask; a back 0603 between two nets.
        let pcb = parse(
            r#"(kicad_pcb
            (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
            (gr_rect (start 0 0) (end 40 20) (layer "Edge.Cuts"))
            (footprint "LOGO" (layer "F.Cu") (at 10 10)
                (fp_poly (pts (xy -2 -2) (xy 2 -2) (xy 2 2) (xy -2 2)) (layer "B.Cu") (width 0)))
            (footprint "Marked" (layer "F.Cu") (at 30 10)
                (fp_rect (start -1 -1) (end 1 1) (layer "F.CrtYd"))
                (fp_poly (pts (xy -3 -3) (xy 3 -3) (xy 3 3) (xy -3 3)) (layer "B.Mask") (width 0))
                (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "A")))
            (footprint "L_0603" (layer "B.Cu") (at 20 3)
                (fp_rect (start -1.5 -0.7) (end 1.5 0.7) (layer "B.CrtYd"))
                (pad "1" smd rect (at -0.8 0) (size 0.8 0.9) (layers "B.Cu") (net 1 "A"))
                (pad "2" smd rect (at 0.8 0) (size 0.8 0.9) (layers "B.Cu") (net 2 "B"))))"#,
        )
        .unwrap();
        let lowered = lower_placement(&pcb, &KiCadBoardPlacerConfig::default(), &[]).unwrap();
        let problem = &lowered.problem;
        let logo = &problem.components[0];
        assert_eq!(logo.side, core::Side::Back, "artwork occupies the side of its copper");
        assert_eq!(problem.components[1].side, core::Side::Front);
        assert_eq!(problem.components[1].far_side, vec![[-3.0, -3.0, 3.0, 3.0]]);
        let part = 2;
        assert_eq!(problem.components[part].side, core::Side::Back);
        let mut poses = problem.poses.clone();
        assert!(!core::legal::illegal_components(problem, &poses).contains(&part));
        // Over the logo's copper, and under the other part's mask opening.
        for position in [[10.0, 10.5], [30.0, 12.0]] {
            poses[part].position = position;
            assert!(core::legal::illegal_components(problem, &poses).contains(&part), "{position:?}");
        }
        // A front part may sit over the logo: its copper is on the back.
        let mut front = problem.components[part].clone();
        front.side = core::Side::Front;
        let mut flipped = problem.clone();
        flipped.components[part] = front;
        poses[part].position = [10.0, 10.5];
        assert!(!core::legal::illegal_components(&flipped, &poses).contains(&part));
    }

    #[test]
    fn rule_areas_keep_out_what_they_forbid() {
        // A back rule area that allows footprints but forbids pads (d20),
        // and a fixed back module whose own keepout forbids footprints on
        // both sides (PixelWave's ESP32 antenna area).
        let pcb = parse(
            r#"(kicad_pcb
            (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
            (gr_rect (start 0 0) (end 60 30) (layer "Edge.Cuts"))
            (zone (layers "B.Cu") (keepout (tracks not_allowed) (vias not_allowed) (pads not_allowed) (copperpour not_allowed) (footprints allowed))
                (polygon (pts (xy 2 2) (xy 20 2) (xy 20 28) (xy 2 28))))
            (footprint "Module" (layer "B.Cu") (at 40 10) (locked yes)
                (fp_rect (start -3 -3) (end 3 3) (layer "B.CrtYd"))
                (pad "1" smd rect (at 0 0) (size 1 1) (layers "B.Cu") (net 1 "A"))
                (zone (layers "F.Cu" "B.Cu") (keepout (tracks not_allowed) (vias not_allowed) (pads not_allowed) (copperpour not_allowed) (footprints not_allowed))
                    (polygon (pts (xy 35 14) (xy 45 14) (xy 45 24) (xy 35 24)))))
            (footprint "Back" (layer "B.Cu") (at 30 3)
                (fp_rect (start -5 -0.7) (end 5 0.7) (layer "B.CrtYd"))
                (pad "1" smd rect (at -2.5 0) (size 0.8 0.9) (layers "B.Cu") (net 1 "A"))
                (pad "2" smd rect (at 2.5 0) (size 0.8 0.9) (layers "B.Cu") (net 2 "B")))
            (footprint "Front" (layer "F.Cu") (at 30 27)
                (fp_rect (start -1 -0.7) (end 1 0.7) (layer "F.CrtYd"))
                (pad "1" smd rect (at -0.5 0) (size 0.6 0.9) (layers "F.Cu") (net 1 "A"))
                (pad "2" smd rect (at 0.5 0) (size 0.6 0.9) (layers "F.Cu") (net 2 "B"))))"#,
        )
        .unwrap();
        let lowered = lower_placement(&pcb, &KiCadBoardPlacerConfig::default(), &[]).unwrap();
        let problem = &lowered.problem;
        let (back, front) = (1, 2);
        let mut poses = problem.poses.clone();
        assert!(core::legal::illegal_components(problem, &poses).is_empty());
        let legal = |poses: &[core::Pose], part: usize| !core::legal::illegal_components(problem, poses).contains(&part);
        // Pads in the pad keepout: not legal; the body reaching over its
        // edge with the pads outside: legal.
        poses[back].position = [10.0, 15.0];
        assert!(!legal(&poses, back));
        poses[back].position = [23.5, 15.0];
        assert!(legal(&poses, back));
        // A front part is not on the pad keepout's layer.
        poses[front].position = [10.0, 15.0];
        assert!(legal(&poses, front));
        // The fixed module's keepout blocks both sides.
        poses[front].position = [40.0, 20.0];
        assert!(!legal(&poses, front));
        poses[back].position = [40.0, 20.0];
        assert!(!legal(&poses, back));
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
mod channel_tests {
    use super::*;

    /// Two instances of a three-part channel (A-B on one net, B-C on
    /// another, A on the shared ground), instance 2 placed far apart.
    fn lowered() -> LoweredPlacement {
        let part = |name: &str, pins: Vec<(f64, usize)>| core::Component {
            name: name.into(),
            body_center: [0.0, 0.0],
            body_size: [2.0, 1.0],
            round: false,
            halo: 0.0,
            pins: pins.into_iter().map(|(x, net)| core::Pin { offset: [x, 0.0], net }).collect(),
            side: core::Side::Front,
            fixed: false,
            angle_options: vec![0.0, 90.0, 180.0, 270.0],
            far_side: Vec::new(),
            hollow: Vec::new(),
            tight: None,
            edge_inset: 0.0,
            courtyards: Vec::new(),
            holes_inside: false,
            pads: vec![[-1.0, -0.5, 1.0, 0.5]],
            copper_only: false,
            cutout_outline: Vec::new(),
            tight_hollow: Vec::new(),
        };
        // Nets: 0 GND, 1-2 instance 1's, 3-4 instance 2's.
        let components = vec![
            part("A1", vec![(-0.5, 0), (0.5, 1)]),
            part("B1", vec![(-0.5, 1), (0.5, 2)]),
            part("C1", vec![(-0.5, 2)]),
            part("A2", vec![(-0.5, 0), (0.5, 3)]),
            part("B2", vec![(-0.5, 3), (0.5, 4)]),
            part("C2", vec![(-0.5, 4)]),
        ];
        let poses = [[0.0, 0.0], [3.0, 0.0], [6.0, 0.0], [20.0, 20.0], [20.0, 30.0], [30.0, 30.0]]
            .map(|position| core::Pose { position, angle: 0.0 })
            .to_vec();
        let identities = ["A1", "B1", "C1", "A2", "B2", "C2"]
            .iter()
            .map(|name| {
                Some(ChannelIdentity {
                    sheet: format!("/CH{}/", &name[1..]),
                    file: "channel.kicad_sch".into(),
                    footprint: format!("Lib:{}", &name[..1]),
                    value: "x".into(),
                })
            })
            .collect();
        LoweredPlacement {
            problem: core::Problem {
                outline: vec![[0.0, 0.0], [50.0, 0.0], [50.0, 50.0], [0.0, 50.0]],
                components,
                net_weights: vec![1.0; 5],
                poses,
                spacing: 0.5,
                grid: 0.1,
                edge_margin: 0.5,
                min_spacing: 0.2,
                constraints: Default::default(),
                far_side_pads_only: false,
                pieces: Vec::new(),
            },
            references: ["A1", "B1", "C1", "A2", "B2", "C2"].iter().map(|s| s.to_string()).collect(),
            source_at: vec![[0.0; 3]; 6],
            constraint_warnings: Vec::new(),
            routing_layers: 2.0,
            identities,
        }
    }

    #[test]
    fn channels_are_found_paired_and_placed_as_the_compact_instance() {
        let lowered = lowered();
        let groups = channel_groups(&lowered);
        assert_eq!(groups, vec![vec![vec![0, 1, 2], vec![3, 4, 5]]]);
        let templates = channel_templates(&lowered, &groups, &lowered.problem.poses);
        // Instance 1 is the compact one: both take its arrangement.
        assert_eq!(templates.len(), 2);
        assert_eq!(templates[0], vec![(0, [0.0, 0.0], 0.0), (1, [3.0, 0.0], 0.0), (2, [6.0, 0.0], 0.0)]);
        assert_eq!(templates[1], vec![(3, [0.0, 0.0], 0.0), (4, [3.0, 0.0], 0.0), (5, [6.0, 0.0], 0.0)]);
        let mut problem = lowered.problem.clone();
        let pad_boxes: Vec<Vec<[f64; 4]>> = vec![vec![[-1.0, -0.5, 1.0, 0.5]]; 6];
        apply_channels(&mut problem, &lowered.references, &pad_boxes, &templates).unwrap();
        assert_eq!(problem.constraints.followers.len(), 4);
        let leader = &problem.components[3];
        assert_eq!(leader.pins.len(), 5);
        assert_eq!(leader.hollow.len(), 3);
        assert_eq!(leader.body_size, [8.0, 1.0]);
        assert_eq!(problem.components[4].side, core::Side::Neither);
        // Followers moved to the template's places.
        assert_eq!(problem.poses[5].position, [26.0, 20.0]);
    }

    #[test]
    fn channels_wired_differently_are_not_replicated() {
        let mut lowered = lowered();
        // C2 hangs on the ground instead of its own net.
        lowered.problem.components[5].pins[0].net = 0;
        assert!(channel_groups(&lowered).is_empty());
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
