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
    /// Factor on the routing halos the pin counts ask for (not on
    /// `halo_overrides_mm`): less room for escapes, shorter wires.
    pub halo_scale: f64,
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
    /// The board's copper-to-hole clearance: a non-plated hole keeps it
    /// from other parts' copper. Read from the project when not given.
    pub hole_clearance_mm: Option<f64>,
    /// The board's copper-to-edge clearance: pads of parts held at an edge
    /// keep it. Read from the project when not given.
    #[serde(default)]
    pub copper_edge_clearance_mm: Option<f64>,
    /// When nothing else fits, bodies may shrink to their fabrication
    /// outline and pads (courtyards overlap). Read from the project when
    /// not given: allowed unless KiCad's DRC treats a courtyard overlap as
    /// an error.
    pub tight_bodies: Option<bool>,
    /// Whether the project makes a hole inside another part's courtyard an
    /// error (`pth_inside_courtyard` or `npth_inside_courtyard`); `None`:
    /// read it from the project. Where it does not, a part's holes and pads
    /// on the other side keep only the copper clearance from that side's
    /// pads (leads through the board still keep off bodies).
    pub holes_in_courtyards_error: Option<bool>,
    /// Decoupling capacitors (small capacitors between a supply rail and
    /// ground) are pulled to the supply pins of the ICs on their rail,
    /// unless a constraint already names them.
    pub auto_decoupling: bool,
    /// How close (body to pin) a decoupling capacitor up to 4.7 uF is
    /// pulled to its IC's supply pin (2.5 mm). At 2.5 mm a third of the
    /// capacitors ended within 3 mm of their pin where designers put 60 %
    /// there (quick tier); 1 mm gave 25-78 % on five boards with the same
    /// routes (not measured on the tier).
    pub decoupling_mm: f64,
    /// On a board of several pieces (two boards side by side), the parts
    /// a net joins to fixed parts on one piece stay on that piece. On:
    /// zpn_devboard passes (its parts had spread over an empty breakaway,
    /// 129 open after the first route); nothing changes on the quick tier.
    pub keep_on_pieces: bool,
    /// On a panel of copies joined by tabs (one outline piece), each
    /// copy's movable parts stay in its copy's region (see
    /// `keep_copies_apart`): the copies are told apart by their nets. On:
    /// boards whose nets form one group do not change.
    pub keep_copies: bool,
    /// A footprint without a courtyard whose pads stand in rows of pins
    /// (three or more at most 2.6 mm apart, holding three quarters of its
    /// pads: a QFP, an SOIC, a header, a card slot) blocks with the whole
    /// box of its pads, as a body there would; other courtyard-less
    /// footprints (a key switch, a hot-swap socket) block with their pads
    /// alone. On: hackrf's header and SOIC sat inside a turned LQFP-144's
    /// pad ring, homebrew_486's second ISA slot between the first's rows.
    pub solid_pin_rows: bool,
    /// A footprint's solder-mask openings on its own side (graphics on
    /// F.Mask or B.Mask) keep other footprints' pads and bodies off also
    /// where the footprint blocks with its courtyard's shape rather than
    /// its whole body (the body's box holds them; a courtyard-less part
    /// blocks with them already): SNSP-CPU-01's J2 (a 12 x 5 courtyard
    /// around four pins, a 22 x 32 mm F.Mask polygon beside it) held T1
    /// and C61, and KiCad reported solder_mask_bridge for each of their
    /// pads. They keep the bodies' spacing (at least the copper clearance)
    /// or KiCad's mask margin where that is larger (`mask_opening_growth`).
    /// On.
    pub mask_openings: bool,
    /// The project's `solder_mask_to_copper_clearance`: covered copper
    /// keeps it from a mask opening. Read from the project when not given.
    pub solder_mask_to_copper_mm: Option<f64>,
    /// Neighbours keep the larger of their routing halos between them,
    /// not the sum (`core::Problem::shared_halos`).
    pub shared_halos: bool,
    /// The annealer keeps the legality test's gaps across the board
    /// (`core::anneal::AnnealConfig::legal_far_side`). Off by default.
    pub anneal_far_side_gaps: bool,
    /// Routing room from congestion (`core::room`): after a placement with
    /// the pin-count halos, each part's halo is scaled by the routing
    /// demand (RUDY, nets a pour carries left out) around it relative to
    /// the board's median, within the same halo area, and the parts anneal
    /// again from there. Off by default.
    #[serde(default)]
    pub routing_room: bool,
    /// Share of the pin-count halos' area the routing-room halos take.
    #[serde(default = "default_routing_room_share")]
    pub routing_room_share: f64,
    /// Parts with less demand than this share of the median get no halo.
    #[serde(default = "default_routing_room_floor")]
    pub routing_room_floor: f64,
    /// The largest factor on a part's pin-count halo.
    #[serde(default = "default_routing_room_most")]
    pub routing_room_most: f64,
    /// Every part's pin-count halo is scaled by its demand, also where the
    /// level left it none (`core::room::RoutingRoom::all_parts`); false:
    /// only the halos the level kept are redistributed.
    #[serde(default = "default_routing_room_all_parts")]
    pub routing_room_all_parts: bool,
    /// Identical parts that share a bus pin for pin (SIMM sockets, SRAM
    /// banks) are placed as a row (see `bus_rows`). Off by default.
    pub bus_rows: bool,
    /// The gap between a bus row's parts: their halo times the share of
    /// pads off the bus, times this.
    pub bus_row_gap_scale: f64,
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
    /// The placement's orientation search starts from a canonical pose
    /// (`core::orient`): every movable part that may turn at the first of
    /// its angle options (in a fixed order), so that the placement is the
    /// same whether the board holds 0 degrees (an import from a schematic)
    /// or the designer's angles (a stack). On; off: the parts start at
    /// the angles the board holds.
    pub orientation_search: bool,
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
            halo_scale: 1.0,
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
            hole_clearance_mm: None,
            copper_edge_clearance_mm: None,
            tight_bodies: None,
            holes_in_courtyards_error: None,
            auto_decoupling: true,
            decoupling_mm: 2.5,
            keep_on_pieces: true,
            keep_copies: true,
            solid_pin_rows: true,
            mask_openings: true,
            solder_mask_to_copper_mm: None,
            shared_halos: false,
            anneal_far_side_gaps: false,
            routing_room: false,
            routing_room_share: default_routing_room_share(),
            routing_room_floor: default_routing_room_floor(),
            routing_room_most: default_routing_room_most(),
            routing_room_all_parts: default_routing_room_all_parts(),
            bus_rows: false,
            bus_row_gap_scale: 1.0,
            placement_seeds: 3,
            supply_via_room: false,
            supply_via_room_mm: None,
            routing_demand: 0.0,
            replicate_channels: false,
            orientation_search: true,
        }
    }
}

fn default_routing_room_share() -> f64 {
    1.0
}

fn default_routing_room_floor() -> f64 {
    0.5
}

fn default_routing_room_most() -> f64 {
    2.0
}

fn default_routing_room_all_parts() -> bool {
    true
}

/// The placer's routing-room settings (`KiCadBoardPlacerConfig::routing_room`):
/// every net is demand but those a pour carries.
fn routing_room(config: &KiCadBoardPlacerConfig, lowered: &LoweredPlacement) -> Option<core::room::RoutingRoom> {
    config.routing_room.then(|| core::room::RoutingRoom {
        net_demand: lowered.poured.iter().map(|poured| if *poured { 0.0 } else { 1.0 }).collect(),
        share: config.routing_room_share,
        floor: config.routing_room_floor,
        most: config.routing_room_most,
        all_parts: config.routing_room_all_parts,
        ..Default::default()
    })
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
    /// Per footprint, the halo the placement gave it where routing room
    /// set the halos (`core::Relaxation::halos`).
    #[serde(skip)]
    pub halos: Option<std::sync::Arc<Vec<f64>>>,
    /// The other seeds' legal placements (poses, relaxation and how far
    /// they miss the constraints in all), next best first.
    #[serde(skip)]
    pub alternatives: Vec<(Vec<core::Pose>, core::Relaxation, f64)>,
    /// What to change when parts found no legal place.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hints: Vec<String>,
    /// Room as numbers: utilization per side against the usable area, the
    /// area missing, the smallest outline that holds the parts and, when
    /// parts found no place, an outline to try (`placement_room`).
    pub room: crate::placement_room::KiCadRoom,
    /// Even the tightest bodies need more area than a side has: the
    /// placer said no at once.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub overfull: bool,
    /// How many seeds were kept (the work budget may stop them before
    /// `placement_seeds`).
    #[serde(default)]
    pub seeds: usize,
    /// The edges parts on `"edge": "any"` were given, by part.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub edges_chosen: BTreeMap<String, String>,
    pub seconds: f64,
    /// The most placement work a seed did, in seconds of an idle machine.
    #[serde(default)]
    pub work_seconds: f64,
    /// The kept seeds' work as the clock sees it: as the reference machine
    /// runs them, `REFERENCE_THREADS` at once in a pool (`race_seeds`), a
    /// seed starting as a thread frees. It depends on the seeds' work
    /// alone, not on the threads or the load.
    #[serde(default)]
    pub pool_work_seconds: f64,
    /// The kept seeds' work in all (as one after the other).
    #[serde(default)]
    pub seeds_work_seconds: f64,
    /// How many seeds ran: the kept ones and those a batch ran past the
    /// budget (they count for nothing).
    #[serde(default)]
    pub seeds_run: usize,
    /// The seed of the placement kept (`seed` plus its offset).
    #[serde(default)]
    pub seed_kept: u64,
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
/// Whether the project's DRC makes a hole inside a courtyard an error
/// (KiCad's default ignores it).
pub(super) fn holes_in_courtyards_error(project: &Path) -> bool {
    fs::read_to_string(project)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .is_some_and(|project| {
            let severities = &project["board"]["design_settings"]["rule_severities"];
            ["pth_inside_courtyard", "npth_inside_courtyard"].iter().any(|rule| severities[rule].as_str() == Some("error"))
        })
}

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

/// How far a footprint's mask opening keeps other parts beyond the bodies'
/// spacing (at least the copper clearance): KiCad's solder_mask_bridge
/// flags another footprint's pad on the mask layer within the minimum web
/// plus the pad's expansion (the board's `pad_to_mask_clearance`; a
/// graphic opening has none), and covered copper within the project's
/// `solder_mask_to_copper_clearance`. Zero where the copper clearance is
/// the larger (most boards).
fn mask_opening_growth(pcb: &Expr, config: &KiCadBoardPlacerConfig) -> f64 {
    let setup = |key: &str| {
        pcb.child("setup")
            .and_then(|setup| form_f64(setup, key, 1).ok())
            .unwrap_or(0.0)
            .max(0.0)
    };
    let margin = (setup("solder_mask_min_width") + setup("pad_to_mask_clearance"))
        .max(config.solder_mask_to_copper_mm.unwrap_or(0.0).max(0.0));
    (margin - config.copper_clearance_mm.unwrap_or(0.2)).max(0.0)
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
    // Zones inside a footprint (a U.FL's keepout) are in board
    // coordinates: they move and turn with it. MokyaLora's moved U.FL left
    // its keepout behind, and KiCad found another part's pad in it.
    if (at[0] != current[0] || at[1] != current[1] || delta != 0.0)
        && let Expr::List(children) = footprint
    {
        for child in children.iter_mut().filter(|child| child.head() == Some("zone")) {
            move_points(child, current, at);
        }
    }
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

/// Moves every `xy` point below `node` (board coordinates) from a
/// footprint at `from` to the same place relative to it at `to`.
fn move_points(node: &mut Expr, from: [f64; 3], to: [f64; 3]) {
    let Expr::List(items) = node else {
        return;
    };
    if items.first().and_then(Expr::atom) == Some("xy") {
        let value = |items: &[Expr], index: usize| items.get(index).and_then(Expr::atom).and_then(|atom| atom.parse::<f64>().ok());
        if let (Some(x), Some(y)) = (value(items, 1), value(items, 2)) {
            let local = core::problem::rotate([x - from[0], y - from[1]], from[2]);
            let board = core::problem::rotate(local, -to[2]);
            let round = |value: f64| Expr::Atom((((value * 1.0e6).round() / 1.0e6) + 0.0).to_string());
            items[1] = round(to[0] + board[0]);
            items[2] = round(to[1] + board[1]);
        }
        return;
    }
    for item in items.iter_mut().skip(1) {
        move_points(item, from, to);
    }
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
/// suits; up to 4.7 µF within `decoupling_mm` (2.5 mm) of the pin, bulk capacitors within
/// 5 mm. Crystals go within 3 mm of the IC pins they drive. Capacitors that a user
/// constraint names, fixed ones and ones a row carries keep to those.
fn decoupling_relations(pcb: &Expr, problem: &core::Problem, near_mm: f64) -> Result<Vec<core::constraints::Relation>, String> {
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
            Relation::Near { anchor: Anchor::Area(_, _, _, half), .. } if *half != [0.0, 0.0] => None,
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
        relations.push(Relation::NearAny { part: index, anchors, max: if bulk { 5.0 } else { near_mm } });
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

/// Whether a footprint's pads (own-frame centres) stand in rows of pins:
/// at least four pads, three quarters of them in a row or column of three
/// or more whose neighbours are at most 2.6 mm apart (a QFP's sides, a
/// header's rows, a card slot). Pads at one position count once. A key
/// switch's or a hot-swap socket's few pads, spread over its box, do not.
fn pads_in_pin_rows(centers: &[[f64; 2]]) -> bool {
    const PITCH: f64 = 2.6;
    const TOLERANCE: f64 = 0.05;
    let mut pads: Vec<[f64; 2]> = Vec::new();
    for center in centers {
        if !pads.iter().any(|pad| (pad[0] - center[0]).abs() < 1.0e-3 && (pad[1] - center[1]).abs() < 1.0e-3) {
            pads.push(*center);
        }
    }
    if pads.len() < 4 {
        return false;
    }
    let mut member = vec![false; pads.len()];
    for axis in 0..2 {
        // Lines of pads sharing the other coordinate (within the tolerance
        // of the line's first pad).
        let mut lines: Vec<(f64, Vec<usize>)> = Vec::new();
        for (index, pad) in pads.iter().enumerate() {
            let across = pad[1 - axis];
            match lines.iter_mut().find(|(at, _)| (at - across).abs() <= TOLERANCE) {
                Some((_, line)) => line.push(index),
                None => lines.push((across, vec![index])),
            }
        }
        for (_, mut line) in lines {
            line.sort_by(|a, b| pads[*a][axis].total_cmp(&pads[*b][axis]));
            let mut start = 0;
            for end in 1..=line.len() {
                let broken = end == line.len() || {
                    let gap = pads[line[end]][axis] - pads[line[end - 1]][axis];
                    gap <= 1.0e-6 || gap > PITCH
                };
                if broken {
                    if end - start >= 3 {
                        for index in &line[start..end] {
                            member[*index] = true;
                        }
                    }
                    start = end;
                }
            }
        }
    }
    member.iter().filter(|member| **member).count() as f64 >= 0.75 * pads.len() as f64
}

/// The courtyard (own frame) as horizontal strips, one per span between
/// the corners' heights, each as wide as the courtyard there (slanted
/// sides counted at their wider end); strips alike stacked into one.
/// Empty when the courtyard is not closed or has too many corners (arcs).
fn courtyard_slabs(footprint: &Expr, skip: Option<&str>) -> Result<Vec<[f64; 4]>, String> {
    let mut edges: Vec<([f64; 2], [f64; 2])> = Vec::new();
    let mut chain = |points: &[[f64; 2]], closed: bool, edges: &mut Vec<([f64; 2], [f64; 2])>| {
        for pair in points.windows(2) {
            edges.push((pair[0], pair[1]));
        }
        if closed && points.len() > 2 {
            edges.push((points[points.len() - 1], points[0]));
        }
    };
    for child in footprint.children() {
        if !form_atom(child, "layer", 1)
            .is_some_and(|layer| layer.ends_with(".CrtYd") && !skip.is_some_and(|side| layer.starts_with(side)))
        {
            continue;
        }
        match child.head() {
            Some("fp_line" | "fp_arc") => chain(&outline::outline_points(child)?, false, &mut edges),
            Some("fp_rect") => {
                let (a, b) = (form_xy(child, "start")?, form_xy(child, "end")?);
                chain(&[a, [b[0], a[1]], b, [a[0], b[1]]], true, &mut edges);
            }
            Some("fp_poly") => chain(&outline::pts_points(child)?, true, &mut edges),
            Some("fp_circle") => {
                let center = form_xy(child, "center")?;
                let radius = distance_squared(center, form_xy(child, "end")?).sqrt();
                chain(&outline::circle_points(center, radius), false, &mut edges);
            }
            _ => {}
        }
    }
    let mut heights: Vec<f64> = edges.iter().flat_map(|(a, b)| [a[1], b[1]]).collect();
    heights.sort_by(f64::total_cmp);
    heights.dedup_by(|a, b| (*a - *b).abs() < 1.0e-6);
    if edges.is_empty() || heights.len() > 64 {
        return Ok(Vec::new());
    }
    let x_at = |(a, b): ([f64; 2], [f64; 2]), y: f64| {
        let t = ((y - a[1]) / (b[1] - a[1])).clamp(0.0, 1.0);
        a[0] + t * (b[0] - a[0])
    };
    let mut slabs: Vec<[f64; 4]> = Vec::new();
    for span in heights.windows(2) {
        let (low, high) = (span[0], span[1]);
        let middle = (low + high) / 2.0;
        let mut crossing: Vec<([f64; 2], [f64; 2])> =
            edges.iter().copied().filter(|(a, b)| (a[1] > middle) != (b[1] > middle)).collect();
        if crossing.len() % 2 != 0 {
            return Ok(Vec::new());
        }
        crossing.sort_by(|p, q| x_at(*p, middle).total_cmp(&x_at(*q, middle)));
        for pair in crossing.chunks(2) {
            let left = x_at(pair[0], low).min(x_at(pair[0], high));
            let right = x_at(pair[1], low).max(x_at(pair[1], high));
            match slabs.iter_mut().find(|slab| {
                (slab[3] - low).abs() < 1.0e-6 && (slab[0] - left).abs() < 1.0e-6 && (slab[2] - right).abs() < 1.0e-6
            }) {
                Some(slab) => slab[3] = high,
                None => slabs.push([left, low, right, high]),
            }
        }
    }
    Ok(slabs)
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
    /// Per net of the problem: whether a pour carries it.
    pub poured: Vec<bool>,
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
        // Inner layers only: no part sits there, but a through-hole pad
        // crosses them (hackrf's keepouts under its SMA pins on In1/In2:
        // KiCad found the moved connectors' ground pins in them). Kept out
        // as copper on both sides, an SMD pad too (it would not need to be).
        (false, false) if !layers.is_empty() && (bodies || !pads) => return Ok(None),
        (false, false) if !layers.is_empty() => core::Side::Both,
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
        far_copper: Vec::new(),
        far_courtyard: Vec::new(),
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

/// On a board of several pieces (two boards side by side), no copper
/// joins one piece to another: a net lies on one piece. The parts a net
/// joins (directly or through other parts) stay on the piece of the fixed
/// parts among them, as a region constraint: zpn_devboard's movable parts
/// spread over its second piece, which holds one connector, and left 129
/// connections open after the first route (the designer's placement: 1).
fn keep_on_pieces(problem: &mut core::Problem, footprints: usize) {
    if problem.pieces.len() < 2 {
        return;
    }
    // Parts joined by nets, by union-find.
    let mut parent: Vec<usize> = (0..footprints).collect();
    fn root(parent: &mut [usize], mut part: usize) -> usize {
        while parent[part] != part {
            parent[part] = parent[parent[part]];
            part = parent[part];
        }
        part
    }
    let mut first_on_net: BTreeMap<usize, usize> = BTreeMap::new();
    for part in 0..footprints {
        for pin in &problem.components[part].pins {
            match first_on_net.get(&pin.net) {
                Some(&other) => {
                    let (a, b) = (root(&mut parent, part), root(&mut parent, other));
                    parent[a] = b;
                }
                None => {
                    first_on_net.insert(pin.net, part);
                }
            }
        }
    }
    // The pieces the fixed parts of each group sit on.
    let piece_of = |problem: &core::Problem, part: usize| {
        let center = problem.components[part].center(problem.poses[part]);
        problem.pieces.iter().position(|b| center[0] >= b[0] && center[0] <= b[2] && center[1] >= b[1] && center[1] <= b[3])
    };
    let mut anchors: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for part in 0..footprints {
        if problem.components[part].fixed && !problem.components[part].pins.is_empty() {
            let group = root(&mut parent, part);
            if let Some(piece) = piece_of(problem, part) {
                anchors.entry(group).or_default().insert(piece);
            }
        }
    }
    if std::env::var_os("PCB_PLACER_DEBUG").is_some() {
        eprintln!("pieces: {} pieces, net groups anchored on them {:?}", problem.pieces.len(), anchors.values().map(|pieces| pieces.len()).collect::<Vec<_>>());
    }
    let constrained: BTreeSet<usize> = problem.constraints.regions.iter().map(|(part, _)| *part).collect();
    for part in 0..footprints {
        if problem.components[part].fixed || problem.components[part].pins.is_empty() || constrained.contains(&part) {
            continue;
        }
        let group = root(&mut parent, part);
        if let Some(pieces) = anchors.get(&group)
            && pieces.len() == 1
        {
            let piece = problem.pieces[*pieces.iter().next().expect("one piece")];
            problem.constraints.regions.push((part, piece));
        }
    }
}

/// A panel of copies joined by tabs is one outline piece, so
/// `keep_on_pieces` cannot tell the copies apart; their nets can: each
/// copy's parts form a group no net leaves (KiKit names a copy's nets
/// `Board_N-...`), and the copies' groups hold the same references. Where
/// two or more such groups each have fixed parts and the boxes of those
/// do not overlap, every movable part of a copy stays in its copy's
/// region: the board's box, cut between the copy's fixed parts and every
/// other copy's, on the axis they are farther apart (halfway, moved by
/// half the difference of the board's margins beyond all copies' fixed
/// parts), and starts there (global placement begins at its centre). On the
/// 0xCB pcb-panel (four copies, one piece) 15-22 parts of one copy landed
/// on another's area. Boards whose nets form one group (most boards, and
/// panels whose copies share net names) are left alone. Returns the
/// number of copies confined.
fn keep_copies_apart(problem: &mut core::Problem, footprints: usize) -> usize {
    if problem.pieces.len() >= 2 || footprints < 4 {
        return 0;
    }
    let mut parent: Vec<usize> = (0..footprints).collect();
    fn root(parent: &mut [usize], mut part: usize) -> usize {
        while parent[part] != part {
            parent[part] = parent[parent[part]];
            part = parent[part];
        }
        part
    }
    let mut first_on_net: BTreeMap<usize, usize> = BTreeMap::new();
    for part in 0..footprints {
        for pin in &problem.components[part].pins {
            match first_on_net.get(&pin.net) {
                Some(&other) => {
                    let (a, b) = (root(&mut parent, part), root(&mut parent, other));
                    parent[a] = b;
                }
                None => {
                    first_on_net.insert(pin.net, part);
                }
            }
        }
    }
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for part in 0..footprints {
        if !problem.components[part].pins.is_empty() {
            groups.entry(root(&mut parent, part)).or_default().push(part);
        }
    }
    if groups.len() < 2 {
        return 0;
    }
    // A copy: a group whose references (with pin counts) another group
    // repeats.
    let signature = |members: &[usize]| -> Vec<(String, usize)> {
        let mut parts: Vec<(String, usize)> =
            members.iter().map(|part| (problem.components[*part].name.clone(), problem.components[*part].pins.len())).collect();
        parts.sort();
        parts
    };
    let mut kinds: BTreeMap<Vec<(String, usize)>, usize> = BTreeMap::new();
    for members in groups.values() {
        *kinds.entry(signature(members)).or_default() += 1;
    }
    let body_box = |problem: &core::Problem, part: usize| {
        let component = &problem.components[part];
        let (center, half) = (component.center(problem.poses[part]), component.half_extent(problem.poses[part].angle));
        [center[0] - half[0], center[1] - half[1], center[0] + half[0], center[1] + half[1]]
    };
    // Each copy's fixed parts' box, and its movable parts.
    let mut copies: Vec<([f64; 4], Vec<usize>)> = Vec::new();
    for members in groups.values() {
        if kinds[&signature(members)] < 2 {
            continue;
        }
        let mut anchor = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        let mut movable = Vec::new();
        for &part in members {
            if problem.components[part].fixed {
                let b = body_box(problem, part);
                anchor = [anchor[0].min(b[0]), anchor[1].min(b[1]), anchor[2].max(b[2]), anchor[3].max(b[3])];
            } else {
                movable.push(part);
            }
        }
        if anchor[0].is_finite() && !movable.is_empty() {
            copies.push((anchor, movable));
        }
    }
    if copies.len() < 2 {
        return 0;
    }
    // The copies' fixed parts must keep apart: interleaved copies have no
    // region of their own.
    let gap = |a: &[f64; 4], b: &[f64; 4]| -> [f64; 2] { [(b[0] - a[2]).max(a[0] - b[2]), (b[1] - a[3]).max(a[1] - b[3])] };
    for (index, (a, _)) in copies.iter().enumerate() {
        for (b, _) in &copies[index + 1..] {
            let [x, y] = gap(a, b);
            if x <= 0.0 && y <= 0.0 {
                return 0;
            }
        }
    }
    let bounds = problem.bounds();
    let constrained: BTreeSet<usize> = problem.constraints.regions.iter().map(|(part, _)| *part).collect();
    // The copies repeat one layout, so each copy's region lies around its
    // fixed parts as the board lies around all copies' fixed parts: a cut
    // halfway between two copies' fixed parts moves by half the difference
    // of the board's margins beyond them (kept between the two). Halfway
    // alone cut SmartSpin2k's copies (fixed parts at the top of each copy,
    // 2.3 mm from the board's top edge, 40.5 mm from its bottom) through
    // their own parts: 162 of the designer's 630 movable parts lay outside
    // their copy's region, none with the shift.
    let all = copies.iter().fold([f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY], |b, (a, _)| {
        [b[0].min(a[0]), b[1].min(a[1]), b[2].max(a[2]), b[3].max(a[3])]
    });
    let shift = [
        ((bounds[2] - all[2]) - (all[0] - bounds[0])) / 2.0,
        ((bounds[3] - all[3]) - (all[1] - bounds[1])) / 2.0,
    ];
    let cut = |before: f64, after: f64, axis: usize| ((before + after) / 2.0 + shift[axis]).clamp(before, after);
    let mut regions = Vec::new();
    for (index, (anchor, _)) in copies.iter().enumerate() {
        let mut region = bounds;
        for (other, (theirs, _)) in copies.iter().enumerate() {
            if other == index {
                continue;
            }
            let [x, y] = gap(anchor, theirs);
            if x >= y {
                if theirs[0] >= anchor[2] {
                    region[2] = region[2].min(cut(anchor[2], theirs[0], 0));
                } else {
                    region[0] = region[0].max(cut(theirs[2], anchor[0], 0));
                }
            } else if theirs[1] >= anchor[3] {
                region[3] = region[3].min(cut(anchor[3], theirs[1], 1));
            } else {
                region[1] = region[1].max(cut(theirs[3], anchor[1], 1));
            }
        }
        regions.push(region);
    }
    // Each copy's parts start in their own region (its centre): from the
    // common start (a netlist's stack, the fixed pins' centroid) global
    // placement clamped the other copies' parts against their regions'
    // near borders, where they piled up (SmartSpin2k: 57 parts without a
    // legal place).
    for ((_, movable), region) in copies.iter().zip(&regions) {
        let centre = [(region[0] + region[2]) / 2.0, (region[1] + region[3]) / 2.0];
        for &part in movable {
            if !constrained.contains(&part) {
                problem.constraints.regions.push((part, *region));
                problem.constraints.starts.push((part, centre));
            }
        }
    }
    if std::env::var_os("PCB_PLACER_DEBUG").is_some() {
        eprintln!(
            "copies: {} copies held apart ({} movable parts), regions {:?}",
            copies.len(),
            copies.iter().map(|(_, movable)| movable.len()).sum::<usize>(),
            regions.iter().map(|r| r.map(|v| (v * 10.0).round() / 10.0)).collect::<Vec<_>>()
        );
    }
    copies.len()
}

/// Bus rows: identical footprints (the same library footprint, on one
/// side) whose pads of one number carry one signal net, pin for pin (the
/// SIMM sockets of a video card, a bank of SRAMs), placed as a row so
/// that every bus net runs straight from part to part. The designers line
/// them up (video's eight SIMMs at a 8.89 mm pitch, mackerel-08's seven
/// DIP-32s at 20 mm); scattered, video's bus left 384 connections open
/// after the first route against none on the designer's placement.
/// Parts a constraint names, fixed ones, repeated references and boards
/// of several pieces (panels) are left alone. The parts
/// stack along their bodies' shorter side, `gap` apart (per part: its
/// halo times the share of its pads not on the bus), in as many lines as
/// make the block's shape closest to the board's. Only through-hole
/// parts (SIMM sockets, DIPs, slots): surface parts need room between
/// their facing pads that a rigid block does not give.
fn bus_rows(
    pcb: &Expr,
    problem: &core::Problem,
    references: &[String],
    footprints: usize,
    constraints: Option<&crate::placement_constraints::KiCadPlacementConstraints>,
    gap_scale: f64,
) -> Vec<crate::placement_constraints::KiCadRowConstraint> {
    use crate::placement_constraints::KiCadRowConstraint;
    // Every string a constraint holds, as a pattern: a part it matches is
    // named (pin anchors name their part before the colon).
    let mut named: Vec<String> = Vec::new();
    fn strings(value: &serde_json::Value, into: &mut Vec<String>) {
        match value {
            serde_json::Value::String(text) => into.push(text.split(':').next().unwrap_or("").to_string()),
            serde_json::Value::Array(items) => items.iter().for_each(|item| strings(item, into)),
            serde_json::Value::Object(map) => map.values().for_each(|item| strings(item, into)),
            _ => {}
        }
    }
    if let Some(constraints) = constraints
        && let Ok(value) = serde_json::to_value(constraints)
    {
        strings(&value, &mut named);
    }
    // A panel (several pieces, references repeated per copy) keeps its
    // copies apart: a row would pull them together.
    if problem.pieces.len() > 1 {
        return Vec::new();
    }
    let mut copies: BTreeMap<&str, usize> = BTreeMap::new();
    for reference in &references[..footprints] {
        *copies.entry(reference.as_str()).or_default() += 1;
    }
    let footprint_items: Vec<&Expr> = pcb.children().iter().filter(|item| item.head() == Some("footprint")).collect();
    let mut pin_count: BTreeMap<&str, usize> = BTreeMap::new();
    let mut pads: Vec<BTreeMap<String, &str>> = Vec::new();
    for footprint in &footprint_items {
        let mut own = BTreeMap::new();
        for pad in footprint.children().iter().filter(|child| child.head() == Some("pad")) {
            let (Some(number), Some(net)) = (
                pad.children().get(1).and_then(Expr::atom),
                node_net(pad).filter(|raw| placer_net(raw)).map(normalize_net),
            ) else {
                continue;
            };
            if own.insert(number.to_string(), net).is_none() {
                *pin_count.entry(net).or_default() += 1;
            }
        }
        pads.push(own);
    }
    // Candidates by library footprint and side.
    let mut kinds: BTreeMap<(String, bool), Vec<usize>> = BTreeMap::new();
    for part in 0..footprints.min(footprint_items.len()) {
        let component = &problem.components[part];
        if component.fixed
            || pads[part].len() < 8
            || copies[references[part].as_str()] > 1
            || named.iter().any(|pattern| glob_matches(pattern, &references[part]))
        {
            continue;
        }
        let Some(library) = footprint_items[part].children().get(1).and_then(Expr::atom) else {
            continue;
        };
        kinds.entry((library.to_string(), component.side == core::Side::Back)).or_default().push(part);
    }
    let bounds = problem.bounds();
    let board = [bounds[2] - bounds[0], bounds[3] - bounds[1]];
    let mut rows = Vec::new();
    for parts in kinds.values().filter(|parts| parts.len() >= 2) {
        // Pairs sharing a bus: a quarter of the pads on common signal nets
        // (power and big nets do not count), at least four.
        let shared = |a: usize, b: usize| {
            pads[a]
                .iter()
                .filter(|(number, net)| pin_count[**net] <= 40 && pads[b].get(*number) == Some(*net))
                .count()
        };
        let mut parent: Vec<usize> = (0..parts.len()).collect();
        fn root(parent: &mut [usize], mut item: usize) -> usize {
            while parent[item] != item {
                parent[item] = parent[parent[item]];
                item = parent[item];
            }
            item
        }
        let mut share = vec![0.0f64; parts.len()];
        for i in 0..parts.len() {
            for j in i + 1..parts.len() {
                let common = shared(parts[i], parts[j]);
                if common >= 4 && common as f64 >= 0.25 * pads[parts[i]].len() as f64 {
                    let (a, b) = (root(&mut parent, i), root(&mut parent, j));
                    parent[a] = b;
                    let fraction = common as f64 / pads[parts[i]].len() as f64;
                    share[i] = share[i].max(fraction);
                    share[j] = share[j].max(fraction);
                }
            }
        }
        let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for i in 0..parts.len() {
            let group = root(&mut parent, i);
            groups.entry(group).or_default().push(i);
        }
        for members in groups.values().filter(|members| members.len() >= 2) {
            let mut members: Vec<usize> = members.iter().map(|i| parts[*i]).collect();
            members.sort_by_key(|part| natural(&references[*part]));
            let leader = &problem.components[members[0]];
            let fraction = members
                .iter()
                .map(|part| share[parts.iter().position(|p| p == part).expect("member")])
                .fold(1.0f64, f64::min);
            // Through-hole parts only: they take a bus straight from pin to
            // pin through the board. Surface parts' pads face each other
            // across the gap and need their whole halo to escape (the
            // designers' SOP and TSOP banks: 4.4-5.9 mm apart); so spaced,
            // homebrew_486's eight SRAMs found no legal spot as a block,
            // and closer they cost two connections and 360 vias.
            if !members.iter().all(|part| !problem.components[*part].far_side.is_empty()) {
                continue;
            }
            let gap = (leader.halo * (1.0 - fraction) * gap_scale).max(problem.spacing);
            // Along the body's shorter side, in the leader's frame; the
            // row's axis is in the board's frame at the leader's angle.
            let size = leader.body_size;
            let short_x = size[0] <= size[1];
            let (along, across) = if short_x { (size[0], size[1]) } else { (size[1], size[0]) };
            let (pitch, line_pitch) = (along + gap, across + gap);
            let quarter = ((problem.poses[members[0]].angle / 90.0).round() as i64).rem_euclid(2) == 1;
            let axis = if short_x != quarter { "x" } else { "y" };
            // Lines: the block's shape closest to the board's (either way
            // round, as the row turns).
            let count = members.len();
            let fit = |lines: usize| {
                let columns = count.div_ceil(lines);
                let (a, b) = (columns as f64 * pitch, lines as f64 * line_pitch);
                (a / board[0]).max(b / board[1]).min((b / board[0]).max(a / board[1]))
            };
            let lines = (1..=count).min_by(|a, b| fit(*a).total_cmp(&fit(*b)).then(a.cmp(b))).unwrap_or(1);
            if std::env::var_os("PCB_PLACER_DEBUG").is_some() {
                eprintln!(
                    "bus row: {:?} pitch {pitch:.2} mm, lines {lines} ({line_pitch:.2} mm), shared {fraction:.2}, axis {axis}",
                    members.iter().map(|part| references[*part].as_str()).collect::<Vec<_>>()
                );
            }
            rows.push(KiCadRowConstraint {
                parts: members.iter().map(|part| references[*part].clone()).collect(),
                pitch_mm: pitch,
                axis: Some(axis.to_string()),
                columns: (lines > 1).then(|| count.div_ceil(lines)),
                row_pitch_mm: (lines > 1).then_some(line_pitch),
                turns: true,
            });
        }
    }
    rows
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
    // Each footprint's uuid: entries for a repeated reference name their
    // footprint by it.
    let mut uuids = Vec::new();
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
    let mask_growth = mask_opening_growth(pcb, config);
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
        let mut far_copper = Vec::new();
        // A part whose plated holes hold leads standing out of the board:
        // a through-hole part (KiCad 6 on says so; older boards say nothing
        // for one, and then a hole of a via's size holds no lead). A
        // mounting hole's or an SMD part's plated holes hold none.
        let attributes: Vec<&str> = footprint
            .child("attr")
            .map(|attr| attr.children().iter().skip(1).filter_map(Expr::atom).collect())
            .unwrap_or_default();
        let leaded_part = attributes.contains(&"through_hole");
        let unattributed = footprint.child("attr").is_none();
        let holes_error = config.holes_in_courtyards_error == Some(true);
        let mut own_pads = Vec::new();
        let mut pin_centers: Vec<[f64; 2]> = Vec::new();
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
            // A non-plated hole keeps its own clearance, and the board's
            // hole clearance, from other copper: the clearance tests add
            // the copper clearance to the box, so the box takes the rest.
            let keep_away = if pad_type == "np_thru_hole" {
                local_clearance::pad_clearance(pad, footprint)?
                    .max(config.hole_clearance_mm.unwrap_or(0.0) - config.copper_clearance_mm.unwrap_or(0.2))
                    .max(0.0)
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
            let mut pad_box = [
                (copper[0] - half[0]).min(pad_at[0] - half[0].min(half[1])),
                (copper[1] - half[1]).min(pad_at[1] - half[0].min(half[1])),
                (copper[0] + half[0]).max(pad_at[0] + half[0].min(half[1])),
                (copper[1] + half[1]).max(pad_at[1] + half[0].min(half[1])),
            ];
            // A custom pad's copper is its primitives too (OpenFC's test
            // pads: a 1 mm anchor and a 2.2 x 1.6 polygon).
            for (point, radius) in custom_pad_points(pad) {
                let at = [pad_at[0] + cos * point[0] - sin * point[1], pad_at[1] + sin * point[0] + cos * point[1]];
                pad_box = [
                    pad_box[0].min(at[0] - radius),
                    pad_box[1].min(at[1] - radius),
                    pad_box[2].max(at[0] + radius),
                    pad_box[3].max(at[1] + radius),
                ];
            }
            // The hole alone (no clearance ring), at the pad's anchor.
            let hole_half = (half[0] - keep_away).min(half[1] - keep_away).max(0.0);
            let hole_box = [pad_at[0] - hole_half, pad_at[1] - hole_half, pad_at[0] + hole_half, pad_at[1] + hole_half];
            own_pads.push(pad_box);
            pin_centers.push([pad_at[0], pad_at[1]]);
            let on_other_side = pad.child("layers").is_some_and(|layers| {
                layers.children().iter().skip(1).filter_map(Expr::atom).any(|layer| layer == other_copper || layer == "F&B.Cu")
            });
            if matches!(pad_type, "thru_hole" | "np_thru_hole") || on_other_side {
                through = true;
                // What the part occupies on the other side. A lead through
                // the board (a plated hole of a part that is not SMD) keeps
                // off bodies there; a non-plated hole, an SMD part's vias
                // and its pads on that side only keep the copper clearance
                // from copper, unless the project forbids holes inside
                // courtyards (then the hole itself keeps off them too).
                let drill = pad
                    .child("drill")
                    .and_then(|drill| drill.children().iter().skip(1).find_map(|item| item.atom()?.parse::<f64>().ok()))
                    .unwrap_or(0.0);
                let lead = pad_type == "thru_hole" && (leaded_part || (unattributed && drill >= 0.6));
                if lead {
                    far_side.push(pad_box);
                } else {
                    far_copper.push(pad_box);
                    if pad_type != "smd" && holes_error {
                        far_side.push(hole_box);
                    }
                }
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
        let mut own_graphic_boxes = Vec::new();
        // The mask openings on the part's side, grown to KiCad's mask
        // margin beyond the bodies' spacing.
        let mut own_mask_boxes: Vec<[f64; 4]> = Vec::new();
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
                own_graphic_boxes.push(graphic);
                if layer.ends_with(".Mask") {
                    own_mask_boxes.push([
                        graphic[0] - mask_growth,
                        graphic[1] - mask_growth,
                        graphic[2] + mask_growth,
                        graphic[3] + mask_growth,
                    ]);
                }
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
            far_copper.extend(far_graphics.iter().copied());
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
            // In a fixed order, not from the board's angle: the searches
            // walk them in this order (`core::orient`).
            if config.orientation_search {
                core::orient::sort_angles(&mut angles);
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
        let both_courtyards = courtyard_on(own_prefix) && courtyard_on(other_prefix);
        let mut courtyard = courtyard_shapes_on_side(footprint, both_courtyards.then_some(other_prefix))?;
        // A courtyard far from rectangular (a hot-swap socket's outline)
        // blocks with its own shape, as strips, and the pads: ghoul's
        // designer put resistor networks, regulators and mounting holes in
        // the corners of the switches' boxes, outside their courtyards.
        let mut shaped = Vec::new();
        // A footprint without any courtyard blocks with its copper (pads,
        // copper and mask graphics) alone, and holes may lie inside it:
        // KiCad checks its copper, not a box around it (hackclub
        // keyboar_'s Kailh sockets have no courtyard; the designer put
        // every switch's diode between its pads).
        let courtless = !courtyard_on("F.") && !courtyard_on("B.") && !own_pads.is_empty() && !artwork;
        // Unless its pads stand in rows of pins: then a body fills their
        // box (`solid_pin_rows`), and the part blocks with that box.
        let solid = courtless && config.solid_pin_rows && pads_in_pin_rows(&pin_centers);
        if solid {
            // The body is that box already, unless copper or mask graphics
            // reach past it.
            if !own_graphic_boxes.is_empty() {
                let pads = own_pads.iter().fold(
                    [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY],
                    |bounds, pad| [bounds[0].min(pad[0]), bounds[1].min(pad[1]), bounds[2].max(pad[2]), bounds[3].max(pad[3])],
                );
                shaped = vec![pads];
                shaped.extend(own_graphic_boxes.iter().copied());
            }
        } else if courtless {
            shaped = own_pads.clone();
            shaped.extend(own_graphic_boxes.iter().copied());
        } else if !round && !artwork {
            let slabs = courtyard_slabs(footprint, both_courtyards.then_some(other_prefix))?;
            let area: f64 = slabs.iter().map(|b| (b[2] - b[0]) * (b[3] - b[1])).sum();
            if !slabs.is_empty() && area < 0.75 * body_size[0] * body_size[1] {
                if slabs.len() > 1 && courtyard.0.is_empty() {
                    courtyard.0 = slabs.clone();
                }
                shaped = slabs;
                shaped.extend(own_pads.iter().copied());
                // Its mask openings beside the courtyard block too: they
                // are in the body's box, which this shape stands in for,
                // and another part's pads there bridge the mask
                // (`mask_openings`). Held to the body's box (grown as they
                // are): a stroke's half width past it would widen every
                // neighbour search (`far_reach`).
                if config.mask_openings {
                    let body = [
                        body_center[0] - body_size[0] / 2.0 - mask_growth,
                        body_center[1] - body_size[1] / 2.0 - mask_growth,
                        body_center[0] + body_size[0] / 2.0 + mask_growth,
                        body_center[1] + body_size[1] / 2.0 + mask_growth,
                    ];
                    shaped.extend(own_mask_boxes.iter().map(|opening| {
                        [opening[0].max(body[0]), opening[1].max(body[1]), opening[2].min(body[2]), opening[3].min(body[3])]
                    }));
                }
            }
        }
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
                    * config.halo_scale
            } else {
                0.0
            },
            pins,
            side,
            fixed: false,
            angle_options,
            far_side: if through || far_graphics { far_side } else { Vec::new() },
            far_copper,
            far_courtyard: if artwork { Vec::new() } else { far_courtyard.into_iter().collect() },
            hollow: shaped,
            tight: if config.tight_bodies == Some(true) {
                tight_body(footprint, (courtyard_on(own_prefix) && courtyard_on(other_prefix)).then_some(other_prefix))?
            } else {
                None
            },
            edge_inset,
            courtyards: courtyard.0,
            holes_inside: courtyard.1 || (courtless && !solid),
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
        uuids.push(footprint_uuid(footprint));
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
            far_copper: Vec::new(),
            far_courtyard: Vec::new(),
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
    // Those an `outline` constraint cut (the board's own graphics, which
    // it replaced) keep bodies off as well.
    let outline_given =
        matches!(&config.constraints, Some(KiCadConstraintsSource::Inline(constraints)) if constraints.outline.is_some());
    let mut solid_cutouts = Vec::new();
    for (number, cutout) in loops.cutouts.iter().enumerate() {
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
        if outline_given && loops.board_cutouts.get(number).copied().unwrap_or(false) {
            solid_cutouts.push(components.len());
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
            far_copper: Vec::new(),
            far_courtyard: Vec::new(),
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
        shared_halos: config.shared_halos,
        pieces: loops.pieces.clone(),
        constraints: Default::default(),
    };
    // KiCad reports copper touching the edge even where the clearance is
    // zero (hackrf's SMA connector P4, its pads flush with the outline: 5
    // copper_edge_clearance findings; 0.01 mm off it, none).
    problem.constraints.copper_edge = config.copper_edge_clearance_mm.unwrap_or(0.0).max(0.02);
    problem.constraints.solid_cutouts = solid_cutouts;
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
    let user_constraints = match &config.constraints {
        Some(KiCadConstraintsSource::Inline(constraints)) => Some(constraints),
        _ => None,
    };
    let automatic_rows = if config.bus_rows {
        bus_rows(pcb, &problem, &references, footprint_count, user_constraints, config.bus_row_gap_scale)
    } else {
        Vec::new()
    };
    let with_rows = (!automatic_rows.is_empty()).then(|| {
        let mut constraints = user_constraints.cloned().unwrap_or_default();
        constraints.row.extend(automatic_rows);
        constraints
    });
    let constraint_warnings = match (&config.constraints, &with_rows) {
        (_, Some(constraints)) | (Some(KiCadConstraintsSource::Inline(constraints)), None) => apply_constraints(
            &mut problem,
            &references,
            &pad_offsets,
            &pad_boxes,
            &keepout_boxes,
            &mouths,
            &locked,
            &uuids,
            constraints,
            config.constraint_weight,
        )?,
        (None, None) => Vec::new(),
        (Some(KiCadConstraintsSource::File(path)), None) => {
            return Err(format!("constraints file {} was not resolved", path.display()));
        }
    };
    // A polygon outline: parts held at an edge keep to the nearest side of
    // the polygon that faces it.
    if user_constraints.and_then(|constraints| constraints.outline.as_ref()).is_some_and(|outline| outline.polygonal()) {
        problem.constraints.sides = core::constraints::outline_sides(&loops.piece_loops);
    }
    // Fixed parts on top of each other place and route without a word,
    // and the board comes back with opens: say so (fence issue E). Checked
    // after the constraints, which fix and free parts (move_all frees a
    // stack fresh from a netlist that the default rules had fixed).
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
    // A fixed footprint's own rule areas (an antenna module's keepout, in
    // board coordinates) are obstacles like the board's: PixelWave's
    // ESP32 on the back keeps parts out of its antenna area on both sides,
    // and KiCad flagged 50 pads and parts we put there. A movable part's
    // keepout moves with it: it blocks the other side as far-side box, or,
    // where it forbids pads but not footprints, as far-side copper
    // (MokyaLora's U.FL on the back keeps pads out on the front).
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
            } else if component.side.collides(match problem.components[index].side {
                core::Side::Front => core::Side::Back,
                core::Side::Back => core::Side::Front,
                _ => core::Side::Neither,
            }) {
                let at = source_at[index];
                let half = [component.body_size[0] / 2.0, component.body_size[1] / 2.0];
                let corners = [[-half[0], -half[1]], [half[0], -half[1]], [half[0], half[1]], [-half[0], half[1]]];
                let local = corners
                    .map(|corner| core::problem::rotate([pose.position[0] + corner[0] - at[0], pose.position[1] + corner[1] - at[1]], at[2]));
                let local = local.iter().fold(
                    [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY],
                    |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])],
                );
                if bodies {
                    problem.components[index].far_side.push(local);
                } else {
                    problem.components[index].far_copper.push(local);
                }
            }
        }
    }
    if config.auto_decoupling {
        let automatic = decoupling_relations(pcb, &problem, config.decoupling_mm)?;
        if !automatic.is_empty() {
            if problem.constraints.relation_weight == 0.0 {
                problem.constraints.relation_weight = config.constraint_weight;
            }
            problem.constraints.automatic = automatic.len();
            problem.constraints.relations.extend(automatic);
        }
    }
    if config.keep_on_pieces {
        keep_on_pieces(&mut problem, footprint_count);
    }
    if config.keep_copies {
        keep_copies_apart(&mut problem, footprint_count);
    }
    apply_channels(&mut problem, &references, &pad_boxes, channels)?;
    problem.constraints.link_pairs();
    let mut constraint_warnings = constraint_warnings;
    constraint_warnings.extend(stacked_warnings);
    let mut poured = vec![false; problem.net_weights.len()];
    for (name, id) in &net_ids {
        if *id < poured.len() && pour_nets.contains(name.as_str()) {
            poured[*id] = true;
        }
    }
    Ok(LoweredPlacement {
        problem,
        references,
        source_at,
        constraint_warnings,
        routing_layers,
        identities,
        poured,
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
        carried.far_copper.clear();
        carried.far_courtyard.clear();
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
                carried.far_copper.extend(component.far_copper.iter().map(place));
                carried.far_courtyard.extend(component.far_courtyard.iter().map(place));
            } else {
                carried.far_side.push(own_body);
                carried.hollow.extend(component.far_side.iter().chain(&component.far_copper).map(place));
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
            component.far_copper.clear();
            component.far_courtyard.clear();
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
    let polygonal = constraints.outline.as_ref().is_some_and(|outline| outline.polygonal());
    let trial = place_kicad_board(source_directory, board_id, &directory, &free);
    let loops = fs::read_to_string(directory.join(format!("{board_id}.kicad_pcb")))
        .ok()
        .and_then(|text| crate::canonical_order::parse_board(&text).ok())
        .and_then(|pcb| outline::board_loops(&pcb).ok());
    // On a polygon, the distance to the nearest side facing each way.
    let sides = match (&loops, polygonal) {
        (Some(loops), true) => core::constraints::outline_sides(&loops.piece_loops),
        _ => Vec::new(),
    };
    let bounds = loops.map(|loops| {
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
            let box_distances = [x - half_x - bounds[0], y - half_y - bounds[1], bounds[2] - x - half_x, bounds[3] - y - half_y];
            let distance = |slot: usize, edge: core::constraints::Edge| {
                if sides.is_empty() {
                    box_distances[slot]
                } else {
                    core::constraints::side_distance(&sides, edge, [x, y], [half_x, half_y]).unwrap_or(f64::INFINITY)
                }
            };
            use core::constraints::Edge;
            let edges = [
                ("left", distance(0, Edge::Left), !along_x),
                ("top", distance(1, Edge::Top), along_x),
                ("right", distance(2, Edge::Right), !along_x),
                ("bottom", distance(3, Edge::Bottom), along_x),
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
/// The movable parts' poses in the designer's board (by reference, in
/// order for repeated references); fixed parts keep their own.
fn designer_poses(lowered: &LoweredPlacement, designer: &Path) -> Result<Vec<core::Pose>, String> {
    let text = fs::read_to_string(designer).map_err(|error| format!("{}: {error}", designer.display()))?;
    let pcb = crate::canonical_order::parse_board(&text)?;
    let mut at_by_reference: BTreeMap<String, Vec<[f64; 3]>> = BTreeMap::new();
    for footprint in pcb.children().iter().filter(|item| item.head() == Some("footprint")) {
        at_by_reference.entry(footprint_reference(footprint).unwrap_or_default()).or_default().push(form_at(footprint)?);
    }
    let mut poses = lowered.problem.poses.clone();
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for (index, reference) in lowered.references.iter().enumerate() {
        if lowered.problem.components[index].fixed {
            continue;
        }
        let nth = seen.entry(reference.clone()).or_default();
        if let Some(at) = at_by_reference.get(reference).and_then(|ats| ats.get(*nth)) {
            poses[index] = core::Pose { position: [at[0], at[1]], angle: at[2] };
        }
        *nth += 1;
    }
    Ok(poses)
}

/// Measurement aid (`PCB_PLACER_TRACE=<file>`): where the wirelength goes
/// between the placer's stages, for every seed's placement, as JSON: the
/// weighted wirelength after global placement, annealing, legalization
/// and refinement (and at the designer's poses with
/// `PCB_PLACER_DESIGNER`), the relaxation level, how crowded the global
/// placement is at that level, and per movable part how far each stage
/// moved it and what it met where the annealer left it.
fn trace_placements(lowered: &LoweredPlacement, placements: &[core::Placement], anneal: &core::anneal::AnnealConfig, path: &Path) -> Result<(), String> {
    let designer = match std::env::var_os("PCB_PLACER_DESIGNER") {
        Some(designer) => Some(designer_poses(lowered, Path::new(&designer))?),
        None => None,
    };
    let base = &lowered.problem;
    let count = base.components.len();
    let nets = base.net_weights.len();
    let mut members: Vec<Vec<(usize, usize)>> = vec![Vec::new(); nets];
    for (index, component) in base.components.iter().enumerate() {
        for (pin, description) in component.pins.iter().enumerate() {
            members[description.net].push((index, pin));
        }
    }
    let net_lengths = |poses: &[core::Pose]| -> Vec<f64> {
        members
            .iter()
            .enumerate()
            .map(|(net, pins)| {
                if pins.len() < 2 {
                    return 0.0;
                }
                let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
                for (component, pin) in pins {
                    let c = &base.components[*component];
                    let at = c.pin_position(&c.pins[*pin], poses[*component]);
                    b = [b[0].min(at[0]), b[1].min(at[1]), b[2].max(at[0]), b[3].max(at[1])];
                }
                base.net_weights[net] * ((b[2] - b[0]) + (b[3] - b[1]))
            })
            .collect()
    };
    let incident: Vec<Vec<usize>> = base
        .components
        .iter()
        .map(|component| {
            let mut nets: Vec<usize> = component.pins.iter().map(|pin| pin.net).collect();
            nets.sort_unstable();
            nets.dedup();
            nets
        })
        .collect();
    let mut seeds = Vec::new();
    for placement in placements {
        let mut relaxed = base.clone();
        placement.relaxation.apply(&mut relaxed);
        let stages: Vec<(&str, &Vec<core::Pose>)> = placement.stage_poses.iter().map(|(name, poses)| (*name, poses)).collect();
        let lengths: Vec<(&str, Vec<f64>)> = stages.iter().map(|(name, poses)| (*name, net_lengths(poses))).collect();
        let designer_lengths = designer.as_ref().map(|poses| net_lengths(poses));
        // Crowding: the share of movable parts that meet another part or
        // leave the board at each stage, at the level the placement ended
        // with; and the pairwise overlap of the inflated boxes.
        let mut crowding = serde_json::Map::new();
        for (name, poses) in &stages {
            let mut conflicted = 0;
            let mut overlap = 0.0;
            let mut area = 0.0;
            for index in 0..count {
                let component = &relaxed.components[index];
                if component.fixed {
                    continue;
                }
                let half = component.half_extent(poses[index].angle);
                let margin = component.halo + relaxed.spacing / 2.0;
                let grown = [half[0] + margin, half[1] + margin];
                area += 4.0 * grown[0] * grown[1];
                let center = component.center(poses[index]);
                for other in 0..count {
                    if other == index || relaxed.components[other].copper_only || component.copper_only {
                        continue;
                    }
                    let theirs = &relaxed.components[other];
                    if !component.side.collides(theirs.side) {
                        continue;
                    }
                    let other_half = theirs.half_extent(poses[other].angle);
                    let other_margin = if theirs.fixed { relaxed.spacing / 2.0 } else { theirs.halo + relaxed.spacing / 2.0 };
                    let other_center = theirs.center(poses[other]);
                    let w = grown[0] + other_half[0] + other_margin - (center[0] - other_center[0]).abs();
                    let h = grown[1] + other_half[1] + other_margin - (center[1] - other_center[1]).abs();
                    if w > 0.0 && h > 0.0 {
                        // Movable pairs count once.
                        overlap += if theirs.fixed { w * h } else { w * h / 2.0 };
                    }
                }
                if !core::legal::is_legal(&relaxed, poses, index, poses[index], 0..count) {
                    conflicted += 1;
                }
            }
            crowding.insert(
                name.to_string(),
                serde_json::json!({"conflicted": conflicted, "overlap_share": overlap / area.max(1.0e-9)}),
            );
        }
        let stage_of = |name: &str| stages.iter().position(|(stage, _)| *stage == name);
        // What the annealer itself sees there (a conflict it does not see
        // is a mismatch of the two models).
        let anneal_overlaps = stage_of("annealed").map(|stage| core::anneal::overlaps(&relaxed, stages[stage].1, anneal));
        let mut parts = Vec::new();
        for index in 0..count {
            let component = &relaxed.components[index];
            if component.fixed {
                continue;
            }
            let centers: Vec<[f64; 2]> = stages.iter().map(|(_, poses)| component.center(poses[index])).collect();
            let cost = |lengths: &[f64]| -> f64 { incident[index].iter().map(|net| lengths[*net]).sum() };
            let mut entry = serde_json::json!({
                "reference": lowered.references[index],
                "pins": component.pins.len(),
                "body": [component.body_size[0], component.body_size[1]],
                "halo": component.halo,
                "side": format!("{:?}", component.side),
                "edge": relaxed.constraints.edges.iter().any(|(part, _, _)| *part == index),
                "centers": centers,
                "angles": stages.iter().map(|(_, poses)| poses[index].angle).collect::<Vec<_>>(),
                "net_cost": lengths.iter().map(|(_, lengths)| cost(lengths)).collect::<Vec<_>>(),
            });
            if let (Some(poses), Some(lengths)) = (&designer, &designer_lengths) {
                entry["designer_center"] = serde_json::json!(component.center(poses[index]));
                entry["designer_net_cost"] = serde_json::json!(cost(lengths));
            }
            // What the part met where the annealer left it.
            if let Some(annealed) = stage_of("annealed").map(|stage| stages[stage].1) {
                let mut met = Vec::new();
                if !core::legal::is_legal(&relaxed, annealed, index, annealed[index], std::iter::empty()) {
                    met.push("outline or hard constraint".to_string());
                }
                for other in 0..count {
                    if other == index {
                        continue;
                    }
                    if let Some(kind) = core::legal::conflict_kind(&relaxed, annealed, index, annealed[index], other) {
                        met.push(format!(
                            "{}{}: {kind}",
                            lowered.references[other],
                            if relaxed.components[other].fixed { " (fixed)" } else { "" }
                        ));
                    }
                }
                entry["met_annealed"] = serde_json::json!(met);
                entry["anneal_overlap"] = serde_json::json!(anneal_overlaps.as_ref().map(|overlaps| overlaps[index]));
            }
            parts.push(entry);
        }
        let relaxation = &placement.relaxation;
        seeds.push(serde_json::json!({
            "stages": stages.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
            "wirelength": lengths.iter().map(|(_, lengths)| lengths.iter().sum::<f64>()).collect::<Vec<_>>(),
            "designer_wirelength": designer_lengths.as_ref().map(|lengths| lengths.iter().sum::<f64>()),
            "unplaced": placement.unplaced.len(),
            "illegal": placement.illegal.len(),
            "work": placement.work,
            "global_iterations": placement.global_iterations,
            "global_overflow": placement.global_overflow,
            "relaxation": {
                "spacing": relaxation.spacing, "grid": relaxation.grid, "halo_scale": relaxation.halo_scale,
                "small_halo_scale": relaxation.small_halo_scale, "edge_inset": relaxation.edge_inset,
                "tight": relaxation.tight, "courtyard_spacing": relaxation.courtyard_spacing,
            },
            "crowding": crowding,
            "parts": parts,
        }));
    }
    let text = serde_json::to_string(&serde_json::json!({"seeds": seeds})).map_err(|error| error.to_string())?;
    fs::write(path, text).map_err(|error| format!("{}: {error}", path.display()))
}

fn diagnose_designer(lowered: &LoweredPlacement, designer: &Path) -> Result<(), String> {
    let mut problem = lowered.problem.clone();
    let poses = designer_poses(lowered, designer)?;
    eprintln!("designer: wirelength {:.1} mm at these poses", problem.wirelength(&poses));
    let level = std::env::var("PCB_PLACER_DESIGNER_LEVEL").unwrap_or_default();
    if level != "full" {
        problem.spacing = problem.min_spacing;
        problem.grid = problem.grid.min(0.1);
        problem.edge_margin = problem.edge_margin.min((problem.constraints.copper_edge + 0.05).min(problem.edge_margin));
        problem.constraints.courtyard_spacing = level == "touch";
        problem.constraints.courtyard_spacing = level == "touch";
        for component in &mut problem.components {
            component.halo = 0.0;
            if level == "tight" || level == "touch" {
                component.use_tight_body();
            }
        }
    }
    let count = problem.components.len();
    let bounds = problem.bounds();
    eprintln!("designer: outline bounds {bounds:?}, {} pieces, edge margin {}, {} points: {:?}", problem.pieces.len(), problem.edge_margin, problem.outline.len(), problem.outline.iter().step_by((problem.outline.len() / 40).max(1)).collect::<Vec<_>>());
    let mut illegal = 0;
    let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
    for index in 0..count {
        if problem.components[index].fixed {
            continue;
        }
        let mut reasons = Vec::new();
        let outside = !core::legal::is_legal(&problem, &poses, index, poses[index], std::iter::empty());
        if outside {
            reasons.push("outline or hard constraint".to_string());
            *kinds.entry("outline or hard constraint").or_insert(0usize) += 1;
        }
        for other in (0..count).filter(|other| !outside && *other != index) {
            if let Some(kind) = core::legal::conflict_kind(&problem, &poses, index, poses[index], other) {
                reasons.push(format!("{} ({:?}): {kind}", lowered.references[other], problem.components[other].side));
                *kinds.entry(kind).or_insert(0usize) += 1;
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
    eprintln!("designer: {illegal} movable parts illegal at the designer's poses (level {level:?}); conflicts by kind {kinds:?}");
    Ok(())
}

/// The threads of the reference machine the placement race's seeds count
/// as running on (a pool this wide, whatever the process's threads). The
/// router's `EXPANSIONS_PER_SECOND` is calibrated on one search thread;
/// this pool width is the placement's own convention.
pub(crate) const REFERENCE_THREADS: usize = 8;

/// A placement race's outcome (`race_seeds`).
pub(crate) struct SeedRace<T> {
    /// The kept seeds' offsets and results, in seed order.
    pub kept: Vec<(u64, T)>,
    /// How many seeds ran (the kept ones and those past the budget).
    pub run: u64,
    /// The kept seeds' work as `REFERENCE_THREADS` threads would run each
    /// batch (`pool_makespan`), summed over the batches.
    pub pool_work: u64,
}

/// How long `threads` threads take for `works`, each starting in turn on
/// the thread that frees first.
pub(crate) fn pool_makespan(works: &[u64], threads: usize) -> u64 {
    let mut free = vec![0u64; threads.max(1)];
    for work in works {
        let earliest = (0..free.len()).min_by_key(|thread| free[*thread]).unwrap_or(0);
        free[earliest] += work;
    }
    free.into_iter().max().unwrap_or(0)
}

/// Runs seeds `0..seeds` of a race in a pool of `threads` and keeps them as
/// running one after the other would: seed k is kept while the work of the
/// seeds before it is under `limit` (the first `minimum` always). The
/// seeds run in batches, each in a pool where a seed starts as a thread
/// frees: the first holds the `minimum`; each later one as many seeds as
/// the budget left holds at the kept seeds' mean work (all that are left
/// without a limit). The batches, so the seeds run and the ones kept,
/// follow from the seeds' work alone, whatever `threads` and whichever
/// seed finishes first; seeds a batch ran past the budget count for
/// nothing.
pub(crate) fn race_seeds<T: Send>(
    seeds: u64,
    minimum: u64,
    limit: Option<u64>,
    threads: usize,
    work: impl Fn(&T) -> u64,
    run: impl Fn(u64) -> T + Sync,
) -> SeedRace<T> {
    let mut race = SeedRace { kept: Vec::new(), run: 0, pool_work: 0 };
    let mut used = 0u64;
    let mut next = 0u64;
    while next < seeds {
        let batch = match limit {
            _ if race.kept.is_empty() => minimum.max(1),
            None => seeds - next,
            Some(limit) => {
                let mean = (used / race.kept.len() as u64).max(1);
                limit.saturating_sub(used).div_ceil(mean).max(1)
            }
        }
        .min(seeds - next);
        let results = run_pool(next..next + batch, threads, &run);
        race.run += batch;
        let mut batch_works = Vec::new();
        let mut spent = false;
        for (offset, result) in (next..).zip(results) {
            if race.kept.len() as u64 >= minimum && limit.is_some_and(|limit| used >= limit) {
                spent = true;
                break;
            }
            let own = work(&result);
            used += own;
            batch_works.push(own);
            race.kept.push((offset, result));
        }
        race.pool_work += pool_makespan(&batch_works, REFERENCE_THREADS);
        next += batch;
        // The next seed would not be kept either: it does not run.
        if spent || (race.kept.len() as u64 >= minimum && limit.is_some_and(|limit| used >= limit)) {
            break;
        }
    }
    race
}

/// Runs `run` on every index of `range` with `threads` threads, each taking
/// the next index as it frees; the results in index order.
fn run_pool<T: Send>(range: std::ops::Range<u64>, threads: usize, run: &(impl Fn(u64) -> T + Sync)) -> Vec<T> {
    let count = (range.end - range.start) as usize;
    let next = std::sync::atomic::AtomicU64::new(range.start);
    let slots: Vec<std::sync::Mutex<Option<T>>> = (0..count).map(|_| std::sync::Mutex::new(None)).collect();
    std::thread::scope(|scope| {
        for _ in 0..threads.clamp(1, count.max(1)) {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if index >= range.end {
                        break;
                    }
                    let result = run(index);
                    *slots[(index - range.start) as usize].lock().expect("placement slot") = Some(result);
                }
            });
        }
    });
    slots
        .into_iter()
        .map(|slot| slot.into_inner().expect("placement slot").expect("placement thread"))
        .collect()
}

pub fn place_kicad_board(
    source_directory: &Path,
    board_id: &str,
    output_directory: &Path,
    config: &KiCadBoardPlacerConfig,
) -> Result<KiCadBoardPlacerResult, String> {
    let started = std::time::Instant::now();
    let source_board = source_directory.join(format!("{board_id}.kicad_pcb"));
    // Errors up to the lowering are the input's (`input_error`).
    let source = fs::read_to_string(&source_board)
        .map_err(|error| crate::verdict::input_error(format!("failed to read {}: {error}", source_board.display())))?;
    let mut pcb = crate::canonical_order::parse_board(&source).map_err(crate::verdict::input_error)?;
    let mut config = resolve_constraints(config, source_directory).map_err(crate::verdict::input_error)?;
    // Parts added by the import (`add`) stay where the entry puts them.
    if let Some(KiCadConstraintsSource::Inline(constraints)) = &mut config.constraints {
        crate::placement_constraints::expand_additions(constraints, &pcb).map_err(crate::verdict::input_error)?;
    }
    if config.copper_clearance_mm.is_none()
        || config.copper_edge_clearance_mm.is_none()
        || config.hole_clearance_mm.is_none()
        || (config.mask_openings && config.solder_mask_to_copper_mm.is_none())
        || (config.supply_via_room && config.supply_via_room_mm.is_none())
    {
        let rules = project_rules::resolve_project_rules(&source_directory.join(format!("{board_id}.kicad_pro")), &source_board).ok();
        if config.copper_clearance_mm.is_none() {
            config.copper_clearance_mm = rules.as_ref().map(largest_clearance);
        }
        if config.copper_edge_clearance_mm.is_none() {
            config.copper_edge_clearance_mm = rules.as_ref().map(|rules| rules.edge_clearance_mm);
        }
        if config.hole_clearance_mm.is_none() {
            config.hole_clearance_mm = rules.as_ref().map(|rules| rules.hole_clearance_mm);
        }
        if config.supply_via_room && config.supply_via_room_mm.is_none() {
            config.supply_via_room_mm = rules.as_ref().map(via_room).filter(|room| *room > 0.0);
        }
        if config.solder_mask_to_copper_mm.is_none() {
            config.solder_mask_to_copper_mm = rules.as_ref().and_then(|rules| rules.solder_mask_to_copper_mm);
        }
    }
    if config.tight_bodies.is_none() {
        config.tight_bodies = Some(courtyards_may_overlap(&source_directory.join(format!("{board_id}.kicad_pro"))));
    }
    if config.holes_in_courtyards_error.is_none() {
        config.holes_in_courtyards_error =
            Some(holes_in_courtyards_error(&source_directory.join(format!("{board_id}.kicad_pro"))));
    }
    let edges_chosen = choose_any_edges(source_directory, board_id, &mut config).map_err(crate::verdict::input_error)?;
    let config = &config;
    let mut outline_size = None;
    if let Some(KiCadConstraintsSource::Inline(constraints)) = &config.constraints
        && let Some(outline) = &constraints.outline
    {
        let mut part_area = 0.0;
        for footprint in pcb.children().iter().filter(|item| item.head() == Some("footprint")) {
            let (_, size, _) = local_body(footprint).map_err(crate::verdict::input_error)?;
            part_area += (size[0] + config.spacing_mm) * (size[1] + config.spacing_mm);
        }
        outline_size = Some(apply_outline(&mut pcb, outline, part_area).map_err(crate::verdict::input_error)?);
    }
    if let Some(KiCadConstraintsSource::Inline(constraints)) = &config.constraints {
        apply_keepouts(&mut pcb, &constraints.keepout).map_err(crate::verdict::input_error)?;
        apply_sides(&mut pcb, constraints).map_err(crate::verdict::input_error)?;
    }
    let mut lowered = lower_placement(&pcb, config, &[]).map_err(crate::verdict::input_error)?;
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
    placer_config.anneal.legal_far_side = config.anneal_far_side_gaps;
    placer_config.routing_room = routing_room(config, &lowered);
    placer_config.orientation_search = config.orientation_search;
    // Several seeds, in parallel; the placement with the fewest illegal
    // parts, then the least missed constraints, then the least wirelength
    // wins. Placement takes seconds; the constraints are what the user asked
    // for.
    let seeds = config.placement_seeds.max(1) as u64;
    // The seeds share the placement's work budget: seed k is kept only
    // while the seeds before it, in seed order, used less than the budget
    // (three seeds at least). They run in a pool (`race_seeds`), as many
    // at once as the process has search threads; which seeds run and
    // which are kept depends on their work alone, not on the threads or
    // the wall clock (SmartSpin2k: sixteen seeds took 1218 s of placement
    // on four cores and left the route no time).
    let minimum_seeds = seeds.min(3);
    let threads = rayon::current_num_threads().max(1);
    // The kept seeds' work as the clock of the reference machine sees it
    // (`REFERENCE_THREADS` at once), and in all; the seeds run, the kept
    // seed. Summed over the races (the repeated channels race twice).
    let pool_work = std::cell::Cell::new(0u64);
    let seeds_work = std::cell::Cell::new(0u64);
    let seeds_run = std::cell::Cell::new(0u64);
    let place_seeds = |problem: &core::Problem| -> Vec<(u64, core::Placement)> {
        let race = race_seeds(seeds, minimum_seeds, placer_config.work_limit, threads, |placement: &core::Placement| placement.work, |offset| {
            let mut seeded = placer_config.clone();
            seeded.global.seed = config.seed + offset;
            seeded.anneal.seed = seeded.anneal.seed.wrapping_add(offset);
            core::place(problem, &seeded)
        });
        pool_work.set(pool_work.get() + race.pool_work);
        seeds_work.set(seeds_work.get() + race.kept.iter().map(|(_, placement)| placement.work).sum::<u64>());
        seeds_run.set(seeds_run.get() + race.run);
        let mut placements: Vec<(u64, core::Placement)> =
            race.kept.into_iter().map(|(offset, placement)| (config.seed + offset, placement)).collect();
        // A placement that needed courtyards touching (the last relaxation
        // level) is a last resort: it ranks below every legal one that did
        // not, however short its wires.
        let key = |placement: &core::Placement| {
            let missed: f64 = placement.constraints.iter().map(|status| status.violation).sum();
            (
                placement.unplaced.len() + placement.illegal.len(),
                placement.relaxation.courtyard_spacing as usize,
                placement.routing_room_failed as usize,
                missed,
                placement.wirelength_final,
            )
        };
        // Stable: a tie goes to the earlier seed.
        placements.sort_by(|(_, a), (_, b)| key(a).partial_cmp(&key(b)).unwrap_or(std::cmp::Ordering::Equal));
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
            let templates = channel_templates(&lowered, &groups, &placements[0].1.poses);
            eprintln!(
                "channels: {} ({} instances of {} parts): placed alike",
                groups.len(),
                templates.len(),
                groups.iter().map(|instances| instances[0].len()).max().unwrap_or(0)
            );
            let replicated = lower_placement(&pcb, config, &templates)?;
            let again = place_seeds(&replicated.problem);
            if again[0].1.unplaced.is_empty() && again[0].1.illegal.is_empty() {
                lowered = replicated;
                placements = again;
            } else {
                let stuck: Vec<&str> = again[0]
                    .1
                    .unplaced
                    .iter()
                    .chain(&again[0].1.illegal)
                    .map(|index| replicated.references[*index].as_str())
                    .collect();
                eprintln!("channels: the replicated placement left {stuck:?} without room; the free one stands");
            }
        }
    }
    let (kept_seeds, mut placements): (Vec<u64>, Vec<core::Placement>) = placements.into_iter().unzip();
    if let Some(trace) = std::env::var_os("PCB_PLACER_TRACE") {
        trace_placements(&lowered, &placements, &placer_config.anneal, Path::new(&trace))?;
    }
    let work_seconds =
        placements.iter().map(|placement| placement.work).max().unwrap_or(0) as f64 / core::WORK_PER_SECOND;
    let seeds_placed = placements.len();
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
        if placement.overfull {
            hints.push(
                "even the tightest bodies need more area than a side has: no placement fits every part; room.suggested_outline_mm is an outline that holds them"
                    .into(),
            );
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
    let room = crate::placement_room::room(
        problem,
        &lowered.references,
        !placement.unplaced.is_empty() || !placement.illegal.is_empty(),
    );
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
        halos: placement.relaxation.halos.clone(),
        utilization,
        hints,
        room,
        overfull: placement.overfull,
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
        pool_work_seconds: pool_work.get() as f64 / core::WORK_PER_SECOND,
        seeds_work_seconds: seeds_work.get() as f64 / core::WORK_PER_SECOND,
        seeds: seeds_placed,
        seeds_run: seeds_run.get() as usize,
        seed_kept: kept_seeds[0],
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
    fn placement_seeds_are_judged_in_seed_order_whatever_the_threads() {
        let works = [50u64, 10, 70, 20, 30, 5, 40, 60, 25, 15, 35, 45];
        // One seed after the other: kept while the seeds before used less
        // than the limit, three at least.
        let sequential = |limit: Option<u64>| -> Vec<u64> {
            let mut kept = Vec::new();
            let mut used = 0;
            for (seed, work) in works.iter().enumerate() {
                if kept.len() >= 3 && limit.is_some_and(|limit| used >= limit) {
                    break;
                }
                used += work;
                kept.push(seed as u64);
            }
            kept
        };
        for limit in [None, Some(1), Some(100), Some(150), Some(200), Some(10_000)] {
            let mut outcomes = Vec::new();
            for threads in [1, 2, 3, 4, 8, 16] {
                let ran = std::sync::Mutex::new(Vec::new());
                let race = race_seeds(works.len() as u64, 3, limit, threads, |result: &(u64, u64)| result.1, |seed| {
                    // Later seeds finish first.
                    std::thread::sleep(std::time::Duration::from_micros(300 * (works.len() as u64 - seed)));
                    ran.lock().unwrap().push(seed);
                    (seed, works[seed as usize])
                });
                let kept: Vec<u64> = race.kept.iter().map(|(offset, _)| *offset).collect();
                assert_eq!(kept, sequential(limit), "limit {limit:?}, {threads} threads");
                assert!(race.kept.iter().all(|(offset, (seed, _))| offset == seed));
                let mut ran = ran.into_inner().unwrap();
                ran.sort();
                assert_eq!(ran.len() as u64, race.run);
                outcomes.push((ran, race.pool_work));
            }
            assert!(outcomes.windows(2).all(|pair| pair[0] == pair[1]), "limit {limit:?}: {outcomes:?}");
        }
        // Up to the limit: the kept seeds and no more ran.
        let race = race_seeds(works.len() as u64, 3, Some(150), 4, |result: &u64| *result, |seed| works[seed as usize]);
        assert_eq!(race.kept.len(), 4);
        assert_eq!(race.pool_work, 70 + 20);
        assert_eq!(pool_makespan(&[4, 4, 4], REFERENCE_THREADS), 4);
        assert_eq!(pool_makespan(&[3; 9], REFERENCE_THREADS), 6);
        assert_eq!(pool_makespan(&[10, 1, 1, 1, 1, 1, 1, 1, 1, 1], 8), 10);
    }

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
        assert_eq!(problem.components[1].far_copper, vec![[-3.0, -3.0, 3.0, 3.0]]);
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
    fn an_inner_layer_pad_keepout_keeps_pads_out() {
        let pcb = parse(
            r#"(kicad_pcb
            (layers (0 "F.Cu" signal) (1 "In1.Cu" signal) (31 "B.Cu" signal))
            (gr_rect (start 0 0) (end 60 30) (layer "Edge.Cuts"))
            (zone (layers "In1.Cu") (keepout (tracks not_allowed) (vias not_allowed) (pads not_allowed) (copperpour not_allowed) (footprints allowed))
                (polygon (pts (xy 2 2) (xy 10 2) (xy 10 10) (xy 2 10))))
            (footprint "J" (layer "F.Cu") (at 40 20) (attr through_hole)
                (fp_rect (start -2 -1) (end 2 1) (layer "F.CrtYd"))
                (pad "1" thru_hole circle (at -1 0) (size 1 1) (drill 0.6) (layers "*.Cu") (net 1 "A"))
                (pad "2" thru_hole circle (at 1 0) (size 1 1) (drill 0.6) (layers "*.Cu") (net 2 "B"))))"#,
        )
        .unwrap();
        let lowered = lower_placement(&pcb, &KiCadBoardPlacerConfig::default(), &[]).unwrap();
        let problem = &lowered.problem;
        let mut poses = problem.poses.clone();
        assert!(core::legal::illegal_components(problem, &poses).is_empty());
        poses[0].position = [6.0, 6.0];
        assert!(core::legal::illegal_components(problem, &poses).contains(&0));
    }

    #[test]
    fn a_movable_parts_pad_keepout_moves_with_it() {
        // MokyaLora's U.FL on the back: its own keepout forbids pads on
        // every layer (footprints allowed). Placed elsewhere, the keepout
        // goes with it, in the model and in the written board.
        let pcb = parse(
            r#"(kicad_pcb
            (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
            (gr_rect (start 0 0) (end 60 30) (layer "Edge.Cuts"))
            (footprint "UFL" (layer "B.Cu") (at 10 10)
                (fp_rect (start -1.5 -1.5) (end 1.5 1.5) (layer "B.CrtYd"))
                (pad "1" smd rect (at 0 0) (size 1 1) (layers "B.Cu") (net 1 "A"))
                (zone (layers "F.Cu" "B.Cu") (keepout (tracks not_allowed) (vias not_allowed) (pads not_allowed) (copperpour not_allowed) (footprints allowed))
                    (polygon (pts (xy 11 9) (xy 13 9) (xy 13 11) (xy 11 11)))))
            (footprint "R" (layer "F.Cu") (at 45 15)
                (fp_rect (start -1 -0.5) (end 1 0.5) (layer "F.CrtYd"))
                (pad "1" smd rect (at -0.5 0) (size 0.6 0.8) (layers "F.Cu") (net 1 "A"))
                (pad "2" smd rect (at 0.5 0) (size 0.6 0.8) (layers "F.Cu") (net 2 "B"))))"#,
        )
        .unwrap();
        let lowered = lower_placement(&pcb, &KiCadBoardPlacerConfig::default(), &[]).unwrap();
        let problem = &lowered.problem;
        let (ufl, front) = (0, 1);
        assert!(!problem.components[ufl].fixed);
        let mut poses = problem.poses.clone();
        poses[ufl] = core::Pose { position: [30.0, 15.0], angle: 90.0 };
        let legal = |poses: &[core::Pose]| core::legal::illegal_components(problem, poses).is_empty();
        // The keepout's first pad sits 2 mm along the U.FL's x axis: turned
        // by 90 degrees, 2 mm above it (y grows downwards).
        poses[front].position = [30.5, 13.0];
        assert!(!legal(&poses), "a front pad in the moved keepout");
        poses[front].position = [30.5, 18.0];
        assert!(legal(&poses), "the keepout is off the front part's pads");
        // The written board carries the keepout along.
        let mut footprint = pcb.children().iter().find(|item| item.head() == Some("footprint")).unwrap().clone();
        write_footprint_pose(&mut footprint, poses[ufl]).unwrap();
        let zone = footprint.child("zone").unwrap();
        let points: Vec<[f64; 2]> = zone
            .child("polygon")
            .and_then(|polygon| polygon.child("pts"))
            .unwrap()
            .children()
            .iter()
            .filter(|child| child.head() == Some("xy"))
            .map(|point| [point.children()[1].atom().unwrap().parse().unwrap(), point.children()[2].atom().unwrap().parse().unwrap()])
            .collect();
        assert_eq!(points, vec![[29.0, 14.0], [29.0, 12.0], [31.0, 12.0], [31.0, 14.0]]);
    }

    #[test]
    fn parts_stay_on_the_piece_of_the_fixed_parts_they_join() {
        // Two pieces side by side (zpn_devboard: a board and an empty
        // breakaway); J1 is locked on the right one. R1 joins J1 through
        // its net, R2 through R1: both stay on the right piece. R3 joins
        // nothing fixed and may go anywhere.
        let pcb = parse(
            r#"(kicad_pcb
            (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
            (gr_rect (start 0 0) (end 30 30) (layer "Edge.Cuts"))
            (gr_rect (start 40 0) (end 90 30) (layer "Edge.Cuts"))
            (footprint "J" (layer "F.Cu") (at 60 15) (locked yes)
                (fp_rect (start -1 -1) (end 1 1) (layer "F.CrtYd"))
                (property "Reference" "J1")
                (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "A")))
            (footprint "R" (layer "F.Cu") (at 10 10)
                (fp_rect (start -1 -0.5) (end 1 0.5) (layer "F.CrtYd"))
                (property "Reference" "R1")
                (pad "1" smd rect (at -0.5 0) (size 0.6 0.8) (layers "F.Cu") (net 1 "A"))
                (pad "2" smd rect (at 0.5 0) (size 0.6 0.8) (layers "F.Cu") (net 2 "B")))
            (footprint "R" (layer "F.Cu") (at 10 20)
                (fp_rect (start -1 -0.5) (end 1 0.5) (layer "F.CrtYd"))
                (property "Reference" "R2")
                (pad "1" smd rect (at -0.5 0) (size 0.6 0.8) (layers "F.Cu") (net 2 "B"))
                (pad "2" smd rect (at 0.5 0) (size 0.6 0.8) (layers "F.Cu") (net 3 "C")))
            (footprint "R" (layer "F.Cu") (at 20 20)
                (fp_rect (start -1 -0.5) (end 1 0.5) (layer "F.CrtYd"))
                (property "Reference" "R3")
                (pad "1" smd rect (at -0.5 0) (size 0.6 0.8) (layers "F.Cu") (net 4 "D"))
                (pad "2" smd rect (at 0.5 0) (size 0.6 0.8) (layers "F.Cu") (net 5 "E"))))"#,
        )
        .unwrap();
        let config = KiCadBoardPlacerConfig { keep_on_pieces: true, ..KiCadBoardPlacerConfig::default() };
        let lowered = lower_placement(&pcb, &config, &[]).unwrap();
        let problem = &lowered.problem;
        assert_eq!(problem.pieces.len(), 2);
        let right = *problem.pieces.iter().find(|piece| piece[0] > 35.0).unwrap();
        let mut regions = problem.constraints.regions.clone();
        regions.sort_by_key(|(part, _)| *part);
        assert_eq!(regions, vec![(1, right), (2, right)]);
        // On by default; `keep_on_pieces: false` switches it off.
        let lowered = lower_placement(&pcb, &KiCadBoardPlacerConfig { keep_on_pieces: false, ..KiCadBoardPlacerConfig::default() }, &[]).unwrap();
        assert!(lowered.problem.constraints.regions.is_empty());
    }

    #[test]
    fn a_shaped_outline_gives_the_placer_its_sides_pieces_and_solid_cutouts() {
        // Two pieces from the constraints: an L (its step at y = 10) and a
        // 20 mm square with a window. J1 is locked on the square; R1 joins
        // it by its net and stays on that piece (keep_on_pieces). J2 brings
        // a slot of its own (Edge.Cuts in the footprint): copper keeps off
        // it, bodies may reach over it, unlike the window.
        let mut pcb = parse(
            r#"(kicad_pcb
            (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
            (footprint "J" (layer "F.Cu") (at 80 15) (locked yes)
                (fp_rect (start -1 -1) (end 1 1) (layer "F.CrtYd"))
                (property "Reference" "J1")
                (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "A")))
            (footprint "J" (layer "F.Cu") (at 30 20) (locked yes)
                (fp_rect (start -2 -2) (end 2 2) (layer "F.CrtYd"))
                (fp_rect (start -0.5 -1) (end 0.5 1) (layer "Edge.Cuts"))
                (property "Reference" "J2")
                (pad "1" smd rect (at 1.5 0) (size 0.5 0.5) (layers "F.Cu") (net 3 "C")))
            (footprint "R" (layer "F.Cu") (at 10 10)
                (fp_rect (start -1 -0.5) (end 1 0.5) (layer "F.CrtYd"))
                (property "Reference" "R1")
                (pad "1" smd rect (at -0.5 0) (size 0.6 0.8) (layers "F.Cu") (net 1 "A"))
                (pad "2" smd rect (at 0.5 0) (size 0.6 0.8) (layers "F.Cu") (net 2 "B"))))"#,
        )
        .unwrap();
        let constraints: KiCadPlacementConstraints = serde_json::from_str(
            r#"{"version": 1, "edge": [{"part": "R1", "edge": "top"}],
                "outline": {"x": 0, "y": 0, "pieces": [
                    {"polygon": [[0, 0], [20, 0], [20, 10], [60, 10], [60, 30], [0, 30]]},
                    {"width": 20, "height": 20, "x": 70, "y": 0, "cutouts": [{"x": [2, 6], "y": [2, 6]}]}]}}"#,
        )
        .unwrap();
        apply_outline(&mut pcb, constraints.outline.as_ref().unwrap(), 0.0).unwrap();
        let config = KiCadBoardPlacerConfig {
            constraints: Some(KiCadConstraintsSource::Inline(constraints.clone())),
            ..KiCadBoardPlacerConfig::default()
        };
        let lowered = lower_placement(&pcb, &config, &[]).unwrap();
        let problem = &lowered.problem;
        assert_eq!(problem.pieces, vec![[0.0, 0.0, 60.0, 30.0], [70.0, 0.0, 90.0, 20.0]]);
        // Two pieces' sides: six and four.
        assert_eq!(problem.constraints.sides.len(), 10);
        // The window keeps bodies off, J2's slot copper only.
        let holes: Vec<usize> = (0..problem.components.len()).filter(|index| problem.components[*index].copper_only).collect();
        assert_eq!(holes.len(), 2);
        assert_eq!(problem.constraints.solid_cutouts.len(), 1);
        let window = &problem.components[problem.constraints.solid_cutouts[0]];
        assert_eq!(window.body_size, [4.0, 4.0]);
        assert_eq!(problem.poses[problem.constraints.solid_cutouts[0]].position, [74.0, 4.0]);
        // R1 is on J1's piece.
        let r1 = lowered.references.iter().position(|reference| reference == "R1").unwrap();
        assert!(problem.constraints.regions.contains(&(r1, [70.0, 0.0, 90.0, 20.0])), "{:?}", problem.constraints.regions);
        // A rectangle keeps the box's sides and has no solid cutouts.
        let rectangle: KiCadPlacementConstraints =
            serde_json::from_str(r#"{"version": 1, "outline": {"width": 60, "height": 30, "x": 0, "y": 0, "corner_radius": 2}}"#).unwrap();
        apply_outline(&mut pcb, rectangle.outline.as_ref().unwrap(), 0.0).unwrap();
        let config = KiCadBoardPlacerConfig { constraints: Some(KiCadConstraintsSource::Inline(rectangle)), ..KiCadBoardPlacerConfig::default() };
        let lowered = lower_placement(&pcb, &config, &[]).unwrap();
        assert!(lowered.problem.constraints.sides.is_empty());
        assert!(lowered.problem.constraints.solid_cutouts.is_empty());
    }

    #[test]
    fn copies_of_a_tabbed_panel_keep_their_parts() {
        // One outline piece holding two copies (KiKit names their nets
        // Board_0-... and Board_1-...): each copy's J1 is locked, its R1
        // joins it through a net. Each R1 stays on its copy's side of the
        // line halfway between the copies' fixed parts.
        let copy = |index: usize, x: f64| {
            format!(
                r#"(footprint "J" (layer "F.Cu") (at {x} 15) (locked yes)
                (fp_rect (start -2 -2) (end 2 2) (layer "F.CrtYd"))
                (property "Reference" "J1")
                (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "Board_{index}-A")))
            (footprint "R" (layer "F.Cu") (at 50 15)
                (fp_rect (start -1 -0.5) (end 1 0.5) (layer "F.CrtYd"))
                (property "Reference" "R1")
                (pad "1" smd rect (at -0.5 0) (size 0.6 0.8) (layers "F.Cu") (net "Board_{index}-A"))
                (pad "2" smd rect (at 0.5 0) (size 0.6 0.8) (layers "F.Cu") (net "Board_{index}-B")))"#
            )
        };
        let board = |copies: &str| {
            parse(&format!(
                r#"(kicad_pcb
            (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
            (gr_rect (start 0 0) (end 100 30) (layer "Edge.Cuts"))
            {copies})"#
            ))
            .unwrap()
        };
        let pcb = board(&format!("{}\n{}", copy(0, 10.0), copy(1, 90.0)));
        let lowered = lower_placement(&pcb, &KiCadBoardPlacerConfig::default(), &[]).unwrap();
        let problem = &lowered.problem;
        assert!(problem.pieces.len() < 2);
        let mut regions = problem.constraints.regions.clone();
        regions.sort_by_key(|(part, _)| *part);
        // J1 bodies end at 12 and start at 88: the cut is at 50.
        assert_eq!(regions, vec![(1, [0.0, 0.0, 50.0, 30.0]), (3, [50.0, 0.0, 100.0, 30.0])]);
        // Each copy's parts start global placement at their region's centre.
        let mut starts = problem.constraints.starts.clone();
        starts.sort_by_key(|(part, _)| *part);
        assert_eq!(starts, vec![(1, [25.0, 15.0]), (3, [75.0, 15.0])]);
        // Copies whose fixed parts sit at their left end (SmartSpin2k's at
        // the top): the cut moves by half the difference of the board's
        // margins beyond them (3 mm left, 43 mm right: 20 mm) from halfway
        // (30) to 50, where the copies meet; it stays between the copies'
        // fixed parts (55 mm apart: 23 mm right of the first copy's).
        let regions_of = |first: f64, second: f64| {
            let pcb = board(&format!("{}\n{}", copy(0, first), copy(1, second)));
            let mut regions = lower_placement(&pcb, &KiCadBoardPlacerConfig::default(), &[]).unwrap().problem.constraints.regions;
            regions.sort_by_key(|(part, _)| *part);
            regions
        };
        assert_eq!(regions_of(5.0, 55.0), vec![(1, [0.0, 0.0, 50.0, 30.0]), (3, [50.0, 0.0, 100.0, 30.0])]);
        assert_eq!(regions_of(5.0, 25.0), vec![(1, [0.0, 0.0, 23.0, 30.0]), (3, [23.0, 0.0, 100.0, 30.0])]);
        // Switched off, or a single copy, or copies whose nets join: none.
        let off = KiCadBoardPlacerConfig { keep_copies: false, ..KiCadBoardPlacerConfig::default() };
        assert!(lower_placement(&pcb, &off, &[]).unwrap().problem.constraints.regions.is_empty());
        let single = board(&copy(0, 10.0));
        assert!(lower_placement(&single, &KiCadBoardPlacerConfig::default(), &[]).unwrap().problem.constraints.regions.is_empty());
        let shared = board(&format!("{}\n{}", copy(0, 10.0), copy(0, 90.0)));
        assert!(lower_placement(&shared, &KiCadBoardPlacerConfig::default(), &[]).unwrap().problem.constraints.regions.is_empty());
        // Parts that are not copies of each other (another reference) stay
        // free too: a board of two circuits is not a panel.
        let other = board(&format!("{}\n{}", copy(0, 10.0), copy(1, 90.0).replace("\"R1\"", "\"R2\"")));
        assert!(lower_placement(&other, &KiCadBoardPlacerConfig::default(), &[]).unwrap().problem.constraints.regions.is_empty());
    }

    #[test]
    fn routing_room_leaves_the_poured_nets_out_of_the_demand() {
        // GND has a zone; A and B do not.
        let pcb = parse(
            r#"(kicad_pcb
            (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
            (gr_rect (start 0 0) (end 30 30) (layer "Edge.Cuts"))
            (zone (net 3) (net_name "GND") (layer "B.Cu")
                (polygon (pts (xy 0 0) (xy 30 0) (xy 30 30) (xy 0 30))))
            (footprint "R" (layer "F.Cu") (at 10 10)
                (fp_rect (start -1 -0.5) (end 1 0.5) (layer "F.CrtYd"))
                (property "Reference" "R1")
                (pad "1" smd rect (at -0.5 0) (size 0.6 0.8) (layers "F.Cu") (net 1 "A"))
                (pad "2" smd rect (at 0.5 0) (size 0.6 0.8) (layers "F.Cu") (net 3 "GND")))
            (footprint "R" (layer "F.Cu") (at 20 20)
                (fp_rect (start -1 -0.5) (end 1 0.5) (layer "F.CrtYd"))
                (property "Reference" "R2")
                (pad "1" smd rect (at -0.5 0) (size 0.6 0.8) (layers "F.Cu") (net 1 "A"))
                (pad "2" smd rect (at 0.5 0) (size 0.6 0.8) (layers "F.Cu") (net 2 "B"))))"#,
        )
        .unwrap();
        let config = KiCadBoardPlacerConfig::default();
        let lowered = lower_placement(&pcb, &config, &[]).unwrap();
        let net_of = |reference: &str, pad: usize| {
            let index = lowered.references.iter().position(|name| name == reference).unwrap();
            lowered.problem.components[index].pins[pad].net
        };
        assert!(lowered.poured[net_of("R1", 1)], "{:?}", lowered.poured);
        assert!(!lowered.poured[net_of("R1", 0)] && !lowered.poured[net_of("R2", 1)]);
        // Off by default; on, the demand leaves GND out.
        assert!(routing_room(&config, &lowered).is_none());
        let on = KiCadBoardPlacerConfig { routing_room: true, routing_room_share: 0.75, ..config };
        let room = routing_room(&on, &lowered).unwrap();
        assert_eq!(room.net_demand[net_of("R1", 1)], 0.0);
        assert_eq!(room.net_demand[net_of("R1", 0)], 1.0);
        assert_eq!(room.share, 0.75);
    }

    #[test]
    fn identical_parts_on_one_bus_become_a_row() {
        // Three SIMM-like sockets (10 x 2 mm, eight pads) carry the same
        // eight bus nets pin for pin; a fourth part of another footprint
        // and a fifth socket a constraint names stay free.
        let socket = |reference: &str, x: f64| {
            let pads: String = (1..=8)
                .map(|pin| format!(r#"(pad "{pin}" thru_hole circle (at {} 0) (size 0.8 0.8) (drill 0.4) (layers "*.Cu") (net {pin} "B{pin}"))"#, pin as f64 - 4.5))
                .collect();
            format!(
                r#"(footprint "Lib:SIMM" (layer "F.Cu") (at {x} 15) (attr through_hole)
                    (fp_rect (start -5 -1) (end 5 1) (layer "F.CrtYd"))
                    (property "Reference" "{reference}") {pads})"#
            )
        };
        let pcb = parse(&format!(
            r#"(kicad_pcb
            (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
            (gr_rect (start 0 0) (end 80 60) (layer "Edge.Cuts"))
            {} {} {} {}
            (footprint "Lib:R" (layer "F.Cu") (at 40 40)
                (fp_rect (start -1 -0.5) (end 1 0.5) (layer "F.CrtYd"))
                (property "Reference" "R1")
                (pad "1" smd rect (at -0.5 0) (size 0.6 0.8) (layers "F.Cu") (net 1 "B1"))
                (pad "2" smd rect (at 0.5 0) (size 0.6 0.8) (layers "F.Cu") (net 9 "X"))))"#,
            socket("U1", 20.0),
            socket("U2", 30.0),
            socket("U3", 40.0),
            socket("U4", 50.0),
        ))
        .unwrap();
        let constraints: crate::placement_constraints::KiCadPlacementConstraints =
            serde_json::from_str(r#"{"version": 1, "near": [{"part": "U4", "part_of": "R1", "max_mm": 5}]}"#).unwrap();
        let config = KiCadBoardPlacerConfig {
            bus_rows: true,
            constraints: Some(KiCadConstraintsSource::Inline(constraints)),
            ..KiCadBoardPlacerConfig::default()
        };
        let lowered = lower_placement(&pcb, &config, &[]).unwrap();
        let problem = &lowered.problem;
        let followers: Vec<(usize, usize, [f64; 2])> =
            problem.constraints.followers.iter().map(|follower| (follower.part, follower.leader, follower.offset)).collect();
        // U2 and U3 follow U1, stacked along the sockets' short side (y)
        // with the spacing between them: every bus pad lines up.
        assert_eq!(followers.len(), 2, "{followers:?}");
        let pitch = 2.0 + problem.spacing;
        for (index, (part, leader, offset)) in followers.iter().enumerate() {
            assert_eq!((*part, *leader), (index + 1, 0));
            assert!(offset[0].abs() < 1.0e-9 && (offset[1].abs() - (index + 1) as f64 * pitch).abs() < 1.0e-9, "{offset:?}");
        }
        // The block may take quarter turns.
        assert_eq!(problem.components[0].angle_options.len(), 4);
        // Off by default.
        let lowered = lower_placement(&pcb, &KiCadBoardPlacerConfig::default(), &[]).unwrap();
        assert!(lowered.problem.constraints.followers.is_empty());
    }

    #[test]
    fn a_row_of_shaped_parts_blocks_with_every_members_shape() {
        // Two L-shaped sockets as a row, 12 mm apart along x: the second
        // one's L blocks as the first one's does, its notch stays free.
        let socket = |reference: &str, x: f64| {
            format!(
                r#"(footprint "Socket" (layer "F.Cu") (at {x} 15)
                (property "Reference" "{reference}")
                (fp_poly (pts (xy -5 -5) (xy -1 -5) (xy -1 1) (xy 5 1) (xy 5 5) (xy -5 5)) (layer "F.CrtYd") (width 0.05))
                (pad "1" smd rect (at -3 3) (size 1 1) (layers "F.Cu") (net 1 "A")))"#
            )
        };
        let pcb = parse(&format!(
            r#"(kicad_pcb
            (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
            (gr_rect (start 0 0) (end 60 30) (layer "Edge.Cuts"))
            {} {}
            (footprint "R" (layer "F.Cu") (at 50 25)
                (property "Reference" "R1")
                (fp_rect (start -1 -0.5) (end 1 0.5) (layer "F.CrtYd"))
                (pad "1" smd rect (at -0.5 0) (size 0.6 0.8) (layers "F.Cu") (net 1 "A"))
                (pad "2" smd rect (at 0.5 0) (size 0.6 0.8) (layers "F.Cu") (net 2 "B"))))"#,
            socket("J1", 10.0),
            socket("J2", 40.0),
        ))
        .unwrap();
        let constraints: crate::placement_constraints::KiCadPlacementConstraints =
            serde_json::from_str(r#"{"version": 1, "row": [{"parts": ["J1", "J2"], "pitch_mm": 12}]}"#).unwrap();
        let config = KiCadBoardPlacerConfig {
            constraints: Some(KiCadConstraintsSource::Inline(constraints)),
            ..KiCadBoardPlacerConfig::default()
        };
        let lowered = lower_placement(&pcb, &config, &[]).unwrap();
        let problem = &lowered.problem;
        let mut poses = problem.poses.clone();
        let legal = |poses: &[core::Pose]| !core::legal::illegal_components(problem, poses).contains(&2);
        // J2 sits at x 22: in its notch legal, in its L not.
        poses[2].position = [22.0 + 2.5, 15.0 - 2.5];
        assert!(legal(&poses));
        poses[2].position = [22.0 - 3.0, 15.0 - 2.5];
        assert!(!legal(&poses));
    }

    #[test]
    fn a_part_fits_in_the_notch_of_an_l_shaped_courtyard() {
        // A socket whose courtyard is an L (10 x 10 with its top-right
        // 6 x 6 quarter cut away), and a small part.
        let pcb = parse(
            r#"(kicad_pcb
            (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
            (gr_rect (start 0 0) (end 60 30) (layer "Edge.Cuts"))
            (footprint "Socket" (layer "F.Cu") (at 20 15)
                (fp_poly (pts (xy -5 -5) (xy -1 -5) (xy -1 1) (xy 5 1) (xy 5 5) (xy -5 5)) (layer "F.CrtYd") (width 0.05))
                (pad "1" smd rect (at -3 3) (size 1 1) (layers "F.Cu") (net 1 "A")))
            (footprint "R" (layer "F.Cu") (at 45 15)
                (fp_rect (start -1 -0.5) (end 1 0.5) (layer "F.CrtYd"))
                (pad "1" smd rect (at -0.5 0) (size 0.6 0.8) (layers "F.Cu") (net 1 "A"))
                (pad "2" smd rect (at 0.5 0) (size 0.6 0.8) (layers "F.Cu") (net 2 "B"))))"#,
        )
        .unwrap();
        let lowered = lower_placement(&pcb, &KiCadBoardPlacerConfig::default(), &[]).unwrap();
        let problem = &lowered.problem;
        assert!(!problem.components[0].hollow.is_empty(), "the L blocks with its own shape");
        let mut poses = problem.poses.clone();
        let legal = |poses: &[core::Pose]| !core::legal::illegal_components(problem, poses).contains(&1);
        // In the notch (local x 1..5, y -5..1): legal; in the L: not.
        poses[1].position = [20.0 + 2.5, 15.0 - 2.5];
        assert!(legal(&poses));
        poses[1].position = [20.0 - 3.0, 15.0 - 2.5];
        assert!(!legal(&poses));
    }

    #[test]
    fn a_fixed_parts_mask_opening_keeps_other_parts_pads_out() {
        // SNSP-CPU-01's J2 (SNES_Library:RFModulator, at its angle): a
        // 12 x 5 courtyard around four pins, a filled F.Mask polygon of
        // 22 x 32 mm beside it and a smaller F.Mask rectangle. The courtyard
        // is far smaller than the body's box, so the part blocks with the
        // courtyard's shape and its pads, and the layout placed T1, C61,
        // C63, C64 and C66 under the polygon (KiCad: solder_mask_bridge for
        // every pad there). And an 0805 capacitor.
        let pcb = parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
            (gr_rect (start 0 0) (end 120 80) (layer "Edge.Cuts"))
            (footprint "SNES_Library:RFModulator" (layer "F.Cu") (at 60 40 -90) (property "Reference" "J2")
                (fp_rect (start -3.429 -1.27) (end 3.81 6.9596) (stroke (width 0.1) (type solid)) (fill yes) (layer "F.Mask"))
                (fp_poly (pts (xy -10.922 38.862) (xy -10.922 6.604) (xy -3.429 6.6) (xy -3.429 6.9596) (xy -6.096 6.9596)
                    (xy -6.096 10.668) (xy 6.096 10.668) (xy 6.096 6.9596) (xy 3.81 6.9596) (xy 3.81 6.604) (xy 11.43 6.604) (xy 11.43 38.862))
                    (stroke (width 0.1) (type solid)) (fill yes) (layer "F.Mask"))
                (fp_rect (start -6 6.5) (end 6 11.5) (stroke (width 0.05) (type default)) (fill no) (layer "F.CrtYd"))
                (pad "1" thru_hole rect (at -3.81 8.89 270) (size 2 2) (drill 1) (layers "*.Cu" "*.Mask") (net 1 "VCC"))
                (pad "2" thru_hole rect (at -1.27 8.89 270) (size 2 2) (drill 1) (layers "*.Cu" "*.Mask") (net 2 "VIDEO"))
                (pad "3" thru_hole rect (at 1.27 8.89 270) (size 2 2) (drill 1) (layers "*.Cu" "*.Mask") (net 3 "AUDIO"))
                (pad "4" thru_hole rect (at 3.81 8.89 270) (size 2 2) (drill 1) (layers "*.Cu" "*.Mask") (net 4 "GND")))
            (footprint "C_0805" (layer "F.Cu") (at 100 60) (property "Reference" "C61")
                (fp_rect (start -1.7 -0.95) (end 1.7 0.95) (layer "F.CrtYd"))
                (pad "1" smd rect (at -0.95 0) (size 1 1.45) (layers "F.Cu" "F.Paste" "F.Mask") (net 2 "VIDEO"))
                (pad "2" smd rect (at 0.95 0) (size 1 1.45) (layers "F.Cu" "F.Paste" "F.Mask") (net 4 "GND"))))"#,
        )
        .unwrap();
        let config = KiCadBoardPlacerConfig { fixed: vec!["J2".into()], ..KiCadBoardPlacerConfig::default() };
        let mut placement = lower_placement(&pcb, &config, &[]).unwrap();
        let (j2, c) = (0, 1);
        assert_eq!(placement.references[..2], ["J2".to_string(), "C61".to_string()]);
        assert!(placement.problem.components[j2].fixed);
        assert!(!placement.problem.components[j2].hollow.is_empty(), "J2 blocks with its courtyard's shape");
        // A point of J2's own frame on the board.
        let at = |problem: &core::Problem, local: [f64; 2]| {
            let pose = problem.poses[j2];
            let offset = problem.components[j2].offset(local, pose.angle);
            [pose.position[0] + offset[0], pose.position[1] + offset[1]]
        };
        for fixed in [true, false] {
            placement.problem.components[j2].fixed = fixed;
            let problem = &placement.problem;
            let mut poses = problem.poses.clone();
            let legal = |poses: &[core::Pose]| !core::legal::illegal_components(problem, poses).contains(&c);
            let seen = |poses: &[core::Pose]| core::anneal::overlaps(problem, poses, &core::anneal::AnnealConfig::default())[c];
            // Where C61 was: deep inside the polygon, 13 mm from the
            // courtyard; and under the small rectangle beside the pins.
            for local in [[0.0, 25.0], [8.0, 15.0], [0.0, 2.0]] {
                poses[c].position = at(problem, local);
                assert!(!legal(&poses), "fixed {fixed}: under the opening at {local:?}");
                assert!(seen(&poses) > 0.0, "fixed {fixed}: the annealer sees the opening at {local:?}");
            }
            // Beside the polygon (its edge at local x 11.43): legal.
            poses[c].position = at(problem, [16.0, 25.0]);
            assert!(legal(&poses), "fixed {fixed}: beside the opening");
            assert_eq!(seen(&poses), 0.0);
        }
        // Off, the shape is the courtyard and the pads alone, as before.
        let off = lower_placement(&pcb, &KiCadBoardPlacerConfig { mask_openings: false, ..config.clone() }, &[]).unwrap();
        let mut poses = off.problem.poses.clone();
        poses[c].position = at(&off.problem, [0.0, 25.0]);
        assert!(!core::legal::illegal_components(&off.problem, &poses).contains(&c));
    }

    #[test]
    fn a_mask_opening_keeps_kicads_mask_margin_beyond_the_copper_clearance() {
        let board = |setup: &str| parse(&format!(r#"(kicad_pcb (setup {setup}))"#)).unwrap();
        let config = |clearance: f64, to_copper: Option<f64>| KiCadBoardPlacerConfig {
            copper_clearance_mm: Some(clearance),
            solder_mask_to_copper_mm: to_copper,
            ..KiCadBoardPlacerConfig::default()
        };
        // SNSP: everything 0, the clearance decides.
        assert_eq!(mask_opening_growth(&board("(pad_to_mask_clearance 0)"), &config(0.2, Some(0.0))), 0.0);
        // eternal-keypad: a web of 0.25 and an expansion of 0.051 against
        // a clearance of 0.1995.
        let keypad = mask_opening_growth(&board("(pad_to_mask_clearance 0.051) (solder_mask_min_width 0.25)"), &config(0.1995, None));
        assert!((keypad - 0.1015).abs() < 1.0e-9, "{keypad}");
        // A mask-to-copper clearance larger than the web and expansion.
        let covered = mask_opening_growth(&board("(pad_to_mask_clearance 0.05) (solder_mask_min_width 0.1)"), &config(0.2, Some(0.3)));
        assert!((covered - 0.1).abs() < 1.0e-9, "{covered}");
    }

    #[test]
    fn courtyard_less_parts_at_45_degrees_meet_by_their_turned_pads() {
        // Two 0402s without courtyards, turned by 45 degrees, side by side
        // across their long axes: the squares around their pads overlap,
        // the pads do not.
        let part = |x: f64, y: f64| {
            format!(
                r#"(footprint "R" (layer "F.Cu") (at {x} {y} 45)
                (pad "1" smd rect (at -0.5 0 45) (size 0.6 0.5) (layers "F.Cu") (net 1 "A"))
                (pad "2" smd rect (at 0.5 0 45) (size 0.6 0.5) (layers "F.Cu") (net 2 "B")))"#
            )
        };
        // KiCad turns by -45: the part's own y axis points to (sin 45, cos 45)
        // on the board, 1.2 mm along it.
        let step = 1.2 / 2f64.sqrt();
        let pcb = parse(&format!(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
            (gr_rect (start 0 0) (end 30 30) (layer "Edge.Cuts")) {} {})"#,
            part(15.0, 15.0),
            part(15.0 + step, 15.0 + step)
        ))
        .unwrap();
        let lowered = lower_placement(&pcb, &KiCadBoardPlacerConfig::default(), &[]).unwrap();
        let problem = &lowered.problem;
        assert!(!problem.components[0].hollow.is_empty(), "no courtyard: the pads block");
        assert!(core::legal::illegal_components(problem, &problem.poses).is_empty());
        // Half as far apart, the pads meet.
        let mut poses = problem.poses.clone();
        poses[1].position = [15.0 + step / 2.0, 15.0 + step / 2.0];
        assert!(core::legal::illegal_components(problem, &poses).contains(&1));
    }

    #[test]
    fn a_diode_sits_between_the_pads_of_a_socket_without_a_courtyard() {
        // A hot-swap socket without a courtyard (two pads 14 mm apart and
        // a centre hole), and a diode with one.
        let pcb = parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
            (gr_rect (start 0 0) (end 60 30) (layer "Edge.Cuts"))
            (footprint "Socket" (layer "F.Cu") (at 20 15)
                (pad "" np_thru_hole circle (at 0 0) (size 4 4) (drill 4) (layers "*.Cu" "*.Mask"))
                (pad "1" smd rect (at -7 -3) (size 2.5 2.5) (layers "F.Cu") (net 1 "A"))
                (pad "2" smd rect (at 7 -3) (size 2.5 2.5) (layers "F.Cu") (net 2 "B")))
            (footprint "D" (layer "F.Cu") (at 45 15)
                (fp_rect (start -1.5 -0.8) (end 1.5 0.8) (layer "F.CrtYd"))
                (pad "1" smd rect (at -0.9 0) (size 0.8 1) (layers "F.Cu") (net 1 "A"))
                (pad "2" smd rect (at 0.9 0) (size 0.8 1) (layers "F.Cu") (net 3 "C"))))"#,
        )
        .unwrap();
        let lowered = lower_placement(&pcb, &KiCadBoardPlacerConfig::default(), &[]).unwrap();
        let problem = &lowered.problem;
        let mut poses = problem.poses.clone();
        let legal = |poses: &[core::Pose]| !core::legal::illegal_components(problem, poses).contains(&1);
        // Between the pads, clear of the hole: legal (KiCad checks copper).
        poses[1].position = [20.0, 15.0 - 4.0];
        assert!(legal(&poses));
        // On a pad or on the hole: not.
        poses[1].position = [20.0 - 7.0, 15.0 - 3.0];
        assert!(!legal(&poses));
        poses[1].position = [20.0, 15.0];
        assert!(!legal(&poses));
    }

    #[test]
    fn pads_in_rows_of_pins_mean_a_body_fills_their_box() {
        let grid = |columns: &[f64], rows: &[f64]| -> Vec<[f64; 2]> {
            columns.iter().flat_map(|x| rows.iter().map(move |y| [*x, *y])).collect()
        };
        // A 2 x 3 header, an SOIC-8, a card slot's two rows.
        assert!(pads_in_pin_rows(&grid(&[-2.54, 0.0, 2.54], &[-1.27, 1.27])));
        assert!(pads_in_pin_rows(&grid(&[-2.7, 2.7], &[-1.905, -0.635, 0.635, 1.905])));
        assert!(pads_in_pin_rows(&grid(&(0..31).map(|i| i as f64 * 2.54).collect::<Vec<_>>(), &[-5.08, 0.0])));
        // A QFP's four sides of 0.5 mm pitch with an exposed pad.
        let mut quad: Vec<[f64; 2]> = (0..8).map(|i| -1.75 + 0.5 * i as f64).flat_map(|t| [[-4.0, t], [4.0, t], [t, -4.0], [t, 4.0]]).collect();
        quad.push([0.0, 0.0]);
        assert!(pads_in_pin_rows(&quad));
        // A key switch (ULK's PG1316: six pads over 14 mm), a hot-swap
        // socket, a two-pad part, a pad stacked on itself.
        assert!(!pads_in_pin_rows(&[[-6.35, -6.0], [-6.35, 6.0], [-2.5, 2.65], [2.5, 2.65], [6.35, -6.0], [6.35, 6.0]]));
        assert!(!pads_in_pin_rows(&[[-7.56, -2.54], [-5.08, 0.0], [-3.81, -2.54], [0.0, 0.0], [2.54, -5.08], [5.08, 0.0], [6.29, -5.08]]));
        assert!(!pads_in_pin_rows(&[[-0.5, 0.0], [0.5, 0.0]]));
        assert!(!pads_in_pin_rows(&[[0.0, 0.0], [0.0, 0.0], [1.0, 0.0], [1.0, 0.0]]));
    }

    #[test]
    fn a_courtyard_less_quad_blocks_inside_its_pad_ring() {
        // hackrf's LQFP without a courtyard: 4 x 6 pads of 0.5 mm pitch on
        // a 10 mm ring, and an SOT-23-sized part with a courtyard.
        let mut pads = String::new();
        for i in 0..6 {
            let t = -1.25 + 0.5 * i as f64;
            for (x, y, w, h) in [(-5.0, t, 1.2, 0.3), (5.0, t, 1.2, 0.3), (t, -5.0, 0.3, 1.2), (t, 5.0, 0.3, 1.2)] {
                pads += &format!(r#"(pad "{i}" smd rect (at {x} {y}) (size {w} {h}) (layers "F.Cu") (net 1 "A"))"#);
            }
        }
        let pcb = parse(&format!(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
            (gr_rect (start 0 0) (end 60 30) (layer "Edge.Cuts"))
            (footprint "QFP" (layer "F.Cu") (at 20 15) {pads})
            (footprint "Q" (layer "F.Cu") (at 45 15)
                (fp_rect (start -1.5 -1.5) (end 1.5 1.5) (layer "F.CrtYd"))
                (pad "1" smd rect (at -0.95 1) (size 0.6 0.8) (layers "F.Cu") (net 1 "A"))
                (pad "2" smd rect (at 0.95 1) (size 0.6 0.8) (layers "F.Cu") (net 2 "B"))
                (pad "3" smd rect (at 0 -1) (size 0.6 0.8) (layers "F.Cu") (net 3 "C"))))"#
        ))
        .unwrap();
        let inside = |config: &KiCadBoardPlacerConfig| {
            let lowered = lower_placement(&pcb, config, &[]).unwrap();
            let problem = &lowered.problem;
            let mut poses = problem.poses.clone();
            poses[1].position = [20.0, 15.0];
            !core::legal::illegal_components(problem, &poses).contains(&1)
        };
        assert!(!inside(&KiCadBoardPlacerConfig::default()), "the body fills the pad ring");
        let off = KiCadBoardPlacerConfig { solid_pin_rows: false, ..KiCadBoardPlacerConfig::default() };
        assert!(inside(&off), "off: only the pads block");
    }

    #[test]
    fn a_header_without_a_courtyard_keeps_parts_from_between_its_rows() {
        // Two 2 x 10 headers of 2.54 mm pitch whose rows lie 5.08 mm apart
        // (a card slot's): the second may not interleave with the first.
        let header = |reference: &str, y: f64| {
            let mut pads = String::new();
            for i in 0..10 {
                for row in [-2.54, 2.54] {
                    pads += &format!(
                        r#"(pad "{i}" thru_hole circle (at {} {row}) (size 1.6 1.6) (drill 1) (layers "*.Cu" "*.Mask") (net 1 "A"))"#,
                        i as f64 * 2.54
                    );
                }
            }
            format!(r#"(footprint "Slot" (layer "F.Cu") (at 10 {y}) (property "Reference" "{reference}") (attr through_hole) {pads})"#)
        };
        let pcb = parse(&format!(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
            (gr_rect (start 0 0) (end 60 40) (layer "Edge.Cuts")) {} {})"#,
            header("J1", 10.0),
            header("J2", 30.0)
        ))
        .unwrap();
        let interleaved = |config: &KiCadBoardPlacerConfig| {
            let lowered = lower_placement(&pcb, config, &[]).unwrap();
            let problem = &lowered.problem;
            let mut poses = problem.poses.clone();
            // J2's top row between J1's rows (at y 10), its bottom row
            // below them.
            poses[1].position = [10.0, 10.0 + 2.54];
            !core::legal::illegal_components(problem, &poses).contains(&1)
        };
        assert!(!interleaved(&KiCadBoardPlacerConfig::default()));
        assert!(interleaved(&KiCadBoardPlacerConfig { solid_pin_rows: false, ..KiCadBoardPlacerConfig::default() }));
    }

    #[test]
    fn copper_keeps_off_the_edge_where_the_rule_is_zero() {
        // An edge connector held on the left edge of a board whose copper
        // edge clearance is zero: its pads may not touch the outline.
        let pcb = parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
            (gr_rect (start 0 0) (end 40 30) (layer "Edge.Cuts"))
            (footprint "SMA" (layer "F.Cu") (at 20 15) (property "Reference" "P4")
                (pad "1" smd rect (at 2 0) (size 4 1.8) (layers "F.Cu") (net 1 "A"))
                (pad "2" smd rect (at 2 -3.5) (size 4 2.6) (layers "F.Cu") (net 2 "B"))
                (pad "2" smd rect (at 2 3.5) (size 4 2.6) (layers "F.Cu") (net 2 "B"))))"#,
        )
        .unwrap();
        let constraints: crate::placement_constraints::KiCadPlacementConstraints =
            serde_json::from_str(r#"{"version": 1, "edge": [{"part": "P4", "edge": "left", "max_mm": 1.5}]}"#).unwrap();
        let config = KiCadBoardPlacerConfig {
            constraints: Some(KiCadConstraintsSource::Inline(constraints)),
            copper_edge_clearance_mm: Some(0.0),
            ..KiCadBoardPlacerConfig::default()
        };
        let lowered = lower_placement(&pcb, &config, &[]).unwrap();
        let problem = &lowered.problem;
        assert!(problem.constraints.copper_edge >= 0.01);
        let offsets = core::constraints::copper_edge_offsets(problem, 0, 0.0);
        assert!(offsets[0] >= 0.01, "{offsets:?}");
    }

    #[test]
    fn far_side_pads_keep_off_copper_and_leads_keep_off_bodies() {
        // An SMD part with pads on both sides (a reversible footprint) and
        // a through-hole part, both on the front; a part on the back.
        let board = |holes_error: bool| {
            let pcb = parse(
                r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
                (gr_rect (start 0 0) (end 60 30) (layer "Edge.Cuts"))
                (footprint "Reversible" (layer "F.Cu") (at 15 15) (attr smd)
                    (fp_rect (start -2 -1) (end 2 1) (layer "F.CrtYd"))
                    (pad "1" smd rect (at -1.5 0) (size 0.8 1) (layers "F.Cu") (net 1 "A"))
                    (pad "1" smd rect (at -1.5 0) (size 0.8 1) (layers "B.Cu") (net 1 "A"))
                    (pad "2" smd rect (at 1.5 0) (size 0.8 1) (layers "F.Cu") (net 2 "B"))
                    (pad "2" smd rect (at 1.5 0) (size 0.8 1) (layers "B.Cu") (net 2 "B")))
                (footprint "Header" (layer "F.Cu") (at 40 15) (attr through_hole)
                    (fp_rect (start -2 -1.5) (end 2 1.5) (layer "F.CrtYd"))
                    (pad "1" thru_hole circle (at -1.27 0) (size 1.7 1.7) (drill 1) (layers "*.Cu" "*.Mask") (net 1 "A"))
                    (pad "2" thru_hole circle (at 1.27 0) (size 1.7 1.7) (drill 1) (layers "*.Cu" "*.Mask") (net 2 "B")))
                (footprint "Back" (layer "B.Cu") (at 30 3)
                    (fp_rect (start -3 -0.6) (end 3 0.6) (layer "B.CrtYd"))
                    (pad "1" smd rect (at -2.5 0) (size 0.8 0.9) (layers "B.Cu") (net 1 "A"))
                    (pad "2" smd rect (at 2.5 0) (size 0.8 0.9) (layers "B.Cu") (net 3 "C"))))"#,
            )
            .unwrap();
            let config = KiCadBoardPlacerConfig { holes_in_courtyards_error: Some(holes_error), ..Default::default() };
            lower_placement(&pcb, &config, &[]).unwrap()
        };
        let lowered = board(false);
        let problem = &lowered.problem;
        assert!(problem.components[0].far_side.is_empty() && problem.components[0].far_copper.len() == 2);
        assert_eq!(problem.components[1].far_side.len(), 2, "the header's leads");
        let mut poses = problem.poses.clone();
        let legal = |problem: &core::Problem, poses: &[core::Pose]| !core::legal::illegal_components(problem, poses).contains(&2);
        // The back part's body over the reversible part's back pads, its
        // own pads clear of them: not an error.
        poses[2].position = [15.0, 15.0];
        assert!(legal(problem, &poses));
        // Its pad on theirs: an error.
        poses[2].position = [15.0 + 1.0, 15.0];
        assert!(!legal(problem, &poses));
        // Over the header's leads: the leads stand out of the board.
        poses[2].position = [40.0, 15.0];
        assert!(!legal(problem, &poses));
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
            far_copper: Vec::new(),
            far_courtyard: Vec::new(),
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
                shared_halos: false,
                pieces: Vec::new(),
            },
            references: ["A1", "B1", "C1", "A2", "B2", "C2"].iter().map(|s| s.to_string()).collect(),
            source_at: vec![[0.0; 3]; 6],
            constraint_warnings: Vec::new(),
            routing_layers: 2.0,
            identities,
            poured: vec![false; 5],
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

#[cfg(test)]
mod order_tests {
    use super::*;

    /// A small board in a given order: twelve parts on an 8-pin chip's
    /// nets, each with pads and graphics; `reversed` writes the footprints,
    /// and each footprint's pads and graphics, the other way round.
    fn board(reversed: bool) -> String {
        let order = |mut items: Vec<String>| {
            if reversed {
                items.reverse();
            }
            items.concat()
        };
        let part = |reference: &str, uuid: &str, at: [f64; 2], pads: Vec<String>, half: [f64; 2]| {
            let graphics = vec![
                format!(r#"(fp_rect (start {} {}) (end {} {}) (layer "F.CrtYd") (uuid "{uuid}-c"))"#, -half[0], -half[1], half[0], half[1]),
                format!(r#"(fp_line (start {} {}) (end {} {}) (layer "F.SilkS") (uuid "{uuid}-s1"))"#, -half[0], -half[1], half[0], -half[1]),
                format!(r#"(fp_line (start {} {}) (end {} {}) (layer "F.SilkS") (uuid "{uuid}-s2"))"#, -half[0], half[1], half[0], half[1]),
                format!(r#"(fp_line (start {} {}) (end {} {}) (layer "F.Fab") (uuid "{uuid}-f"))"#, -half[0], 0.0, half[0], 0.0),
            ];
            format!(
                r#"(footprint "Lib:{reference}" (layer "F.Cu") (uuid "{uuid}") (at {} {})
                    (property "Reference" "{reference}") (attr smd) {} {})"#,
                at[0],
                at[1],
                order(graphics),
                order(pads)
            )
        };
        let pad = |number: &str, x: f64, y: f64, net: usize| {
            format!(r#"(pad "{number}" smd rect (at {x} {y}) (size 0.6 0.9) (layers "F.Cu") (net {net} "N{net}"))"#)
        };
        let mut footprints = Vec::new();
        let chip: Vec<String> = (1..=8)
            .map(|pin| {
                let (x, y) = if pin <= 4 { (-2.5, pin as f64 - 2.5) } else { (2.5, 6.5 - pin as f64) };
                pad(&pin.to_string(), x, y, pin)
            })
            .collect();
        footprints.push(part("U1", "a1", [25.0, 20.0], chip, [3.5, 2.5]));
        for index in 1..=8 {
            let pads = vec![pad("1", -0.8, 0.0, index), pad("2", 0.8, 0.0, index % 8 + 1)];
            footprints.push(part(&format!("R{index}"), &format!("b{index}"), [5.0 + 4.0 * index as f64, 35.0], pads, [1.5, 0.8]));
        }
        for index in 1..=3 {
            let pads = vec![pad("1", -0.8, 0.0, index * 2), pad("2", 0.8, 0.0, 9)];
            footprints.push(part(&format!("C{index}"), &format!("c{index}"), [45.0, 5.0 + 4.0 * index as f64], pads, [1.5, 0.8]));
        }
        format!(
            r#"(kicad_pcb (version 20240108)
            (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
            (net 0 "") {}
            (gr_rect (start 0 0) (end 50 40) (layer "Edge.Cuts"))
            {})"#,
            (1..=9).map(|net| format!(r#"(net {net} "N{net}")"#)).collect::<String>(),
            order(footprints)
        )
    }

    #[test]
    fn the_lowering_does_not_depend_on_the_order_of_the_file() {
        let (forward, backward) = (board(false), board(true));
        assert_ne!(forward, backward);
        let config = KiCadBoardPlacerConfig::default();
        // As read, the parts come in the file's order.
        let raw = |text: &str| lower_placement(&parse(text).unwrap(), &config, &[]).unwrap().references;
        assert_ne!(raw(&forward), raw(&backward));
        // In the canonical order the problems are the same.
        let lowered = |text: &str| {
            let lowered = lower_placement(&crate::canonical_order::parse_board(text).unwrap(), &config, &[]).unwrap();
            (lowered.references.clone(), format!("{:?}", lowered.problem))
        };
        let (references, problem) = lowered(&forward);
        assert_eq!(references, ["C1", "C2", "C3", "R1", "R2", "R3", "R4", "R5", "R6", "R7", "R8", "U1"]);
        assert_eq!((references, problem), lowered(&backward));
    }

    #[test]
    fn a_placement_does_not_depend_on_the_order_of_the_file() {
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("pcb-maker-order-{}-{nonce}", std::process::id()));
        let config = KiCadBoardPlacerConfig { placement_seeds: 2, ..KiCadBoardPlacerConfig::default() };
        let mut placed = Vec::new();
        for (name, reversed) in [("forward", false), ("backward", true)] {
            let source = root.join(format!("{name}-source"));
            fs::create_dir_all(&source).unwrap();
            fs::write(source.join("board.kicad_pcb"), board(reversed)).unwrap();
            let output = root.join(format!("{name}-placed"));
            let result = place_kicad_board(&source, "board", &output, &config).unwrap();
            assert!(result.unplaced.is_empty() && result.illegal.is_empty(), "{:?} {:?}", result.unplaced, result.illegal);
            placed.push((result.wirelength_final_mm, fs::read_to_string(output.join("board.kicad_pcb")).unwrap()));
        }
        let _ = fs::remove_dir_all(&root);
        // The same poses, and the same board written: the output is in the
        // canonical order whatever order the input was in.
        assert_eq!(placed[0].0, placed[1].0);
        assert!(placed[0].1 == placed[1].1, "the placed boards differ");
    }
}
