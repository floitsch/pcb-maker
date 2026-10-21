// Copyright (C) 2026 Toit contributors.

//! The placer's configuration.

use super::*;
use pcb_placer as core;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct KiCadBoardPlacerConfig {
    /// Minimum gap between two courtyards on colliding sides.
    pub spacing_mm: f64,
    /// Footprint origins are snapped to multiples of this.
    pub grid_mm: f64,
    /// Movable parts keep the angle the board gives them; otherwise each
    /// may take any quarter turn of it (and square angles too when it
    /// stands at an odd one).
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
    /// Pins per track of halo (see `track_pitch_mm`); 0 gives no part a
    /// pin-count halo.
    pub pins_per_halo_track: f64,
    /// Factor on the routing halos the pin counts ask for (not on
    /// `halo_overrides_mm`): less room for escapes, shorter wires.
    pub halo_scale: f64,
    /// The largest pin-count halo, in mm, before `halo_scale`.
    pub maximum_halo_mm: f64,
    /// Per-reference halos replacing the pin-count rule (routing feedback).
    pub halo_overrides_mm: BTreeMap<String, f64>,
    /// Share of the free board that bodies, halos and spacing may claim.
    pub maximum_utilization: f64,
    /// Distance movable courtyards keep from the board edge (at least the
    /// copper-to-edge clearance, since pads may reach the courtyard).
    pub edge_margin_mm: f64,
    /// The global placement's seed for the first of the
    /// `placement_seeds`; seed k of the race uses this plus k.
    pub seed: u64,
    /// Wall-clock time by which placement returns its best result (set by
    /// the layout from its budget; placement of a 404-part panel ran past
    /// 25 minutes).
    #[serde(skip)]
    pub deadline: Option<std::time::Instant>,
    /// Placement work each seed may do, in seconds of an idle machine (see
    /// `core::work_per_second`): set by the layout from its budget, so that
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
    /// Placements tried with different seeds (in parallel); the best is
    /// kept: fewest illegal parts, least missed constraints, least wire.
    pub placement_seeds: usize,
    /// Routing demand as charge in the global placement: every net's
    /// estimated wire area (half perimeter times the track pitch, over its
    /// signal layers) spreads over its bounding box and pushes parts apart
    /// where the wires will run, scaled by this (0: off).
    #[serde(default)]
    pub routing_demand: f64,
    /// The placement's orientation search starts from a canonical pose
    /// (`core::orient`): every movable part that may turn at the first of
    /// its angle options (in a fixed order), so that the placement is the
    /// same whether the board holds 0 degrees (an import from a schematic)
    /// or the designer's angles (a stack). On; off: the parts start at
    /// the angles the board holds.
    pub orientation_search: bool,
    /// The board's own texts (header pin names, a title) and silkscreen
    /// graphics (a logo, a frame around a group) stay where the input put
    /// them: each is a placement obstacle on its side with a keepout's
    /// semantics (texts on F.SilkS, B.SilkS, F.Cu and B.Cu by their box
    /// from font size, thickness and justification; graphics on the
    /// silkscreen by their strokes, in clusters, leaving out a cluster that
    /// spans most of the board; a part meets a silkscreen mark without its
    /// routing halo; a mark on a movable part at its input position
    /// belongs to that part and is none: `attached_marks` in
    /// `board-placer.json`), and a part that finds no room off them at a level
    /// lies on them (`parts_on_marks` in `board-placer.json`); the layout
    /// routes around copper texts instead of moving them afterwards, and
    /// the polish moves only the parts' reference and value labels.
    /// Movable parts whose
    /// silkscreen or pads lie on a mark are reported (`labels.overlaps` in
    /// `board-layout.json`, the verdict's next_actions); fixed parts on a
    /// mark get a warning. On; off: the behaviour before 2026-10-21 (the
    /// marks are no obstacles, the polish moves them off the parts).
    pub keep_board_marks: bool,
    /// The board's marks that belong to a movable part, taken from the
    /// input (`attach_marks`: the mark's place among the board's marks,
    /// the part); set by the layout for the lowerings of the placed board,
    /// where the parts no longer lie where the input had them.
    #[serde(skip)]
    pub attached_marks: Option<Vec<(usize, usize)>>,
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
            placement_seeds: 3,
            routing_demand: 0.0,
            orientation_search: true,
            keep_board_marks: true,
            attached_marks: None,
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
pub(super) fn routing_room(
    config: &KiCadBoardPlacerConfig,
    lowered: &LoweredPlacement,
) -> Option<core::room::RoutingRoom> {
    config.routing_room.then(|| core::room::RoutingRoom {
        net_demand: lowered
            .poured
            .iter()
            .map(|poured| if *poured { 0.0 } else { 1.0 })
            .collect(),
        share: config.routing_room_share,
        floor: config.routing_room_floor,
        most: config.routing_room_most,
        all_parts: config.routing_room_all_parts,
        ..Default::default()
    })
}

fn default_edge_keep_mm() -> f64 {
    3.0
}
