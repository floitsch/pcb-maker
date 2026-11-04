// Copyright (C) 2026 Toit contributors.

//! The placer's configuration.

use super::*;
use pcb_placer as core;

/// What the placer's tightest levels call legal (`legality`).
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum KiCadPlacerLegality {
    /// The placer's own model: bodies (courtyard and pads) keep the copper
    /// clearance apart; at the last level KiCad's legality (courtyards
    /// touch where the pads keep the copper clearance pad to pad, the
    /// board's silkscreen marks the project does not rate an error no
    /// obstacle, leads keep the copper clearance from the other side's
    /// pads).
    #[default]
    Model,
    /// The model before 2026-10-27: at the last level, too, bodies keep
    /// the clearance their copper's least inset needs, the marks are
    /// obstacles and leads keep off the other side's bodies.
    Strict,
    /// What KiCad's DRC accepts, at the levels whose spacing is the rules'
    /// least: courtyards may touch where the pads keep the copper
    /// clearance pad to pad, and a body may come to the edge where its
    /// pads keep the edge margin (docs/placer.md, "The legality model
    /// against KiCad").
    Kicad,
}

/// How a net's weight in the placer's wirelength falls with its pin count
/// (`net_weight`; docs/placer.md, "The designer as teacher (2026-11-04)",
/// "Net weights"). The nets designers keep short (`keep_short`) count three
/// times on top, whichever weighting.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum KiCadNetWeight {
    /// `min(1, 3 / (pins - 1))`: a net of up to four pins counts 1, a
    /// 20-pin rail 0.16 (the placer since its start).
    #[default]
    Steep,
    /// `min(1, 3 / sqrt(pins - 1))`: up to ten pins count 1, a 20-pin rail
    /// 0.69.
    Sqrt,
    /// Every net counts 1.
    Flat,
    /// Steep for signals, 1 for the rails and buses: nets of
    /// `RAIL_PINS` pins or more and those whose name is a supply's or
    /// ground's (`quality::rail_name`, `quality::ground`).
    Rail,
}

/// The pin count from which the `rail` weighting counts a net as a rail
/// or a bus whatever its name (the nets of nine or more pins the
/// diagnosis found 1.22 times the designer's length).
pub const RAIL_PINS: usize = 9;

impl KiCadNetWeight {
    /// The weight of a net of `pins` pins (before `keep_short`); `named_rail`:
    /// its name is a supply's or ground's.
    pub fn weight(self, pins: usize, named_rail: bool) -> f64 {
        let steep = (3.0 / (pins as f64 - 1.0).max(1.0)).min(1.0);
        match self {
            KiCadNetWeight::Steep => steep,
            KiCadNetWeight::Sqrt => (3.0 / (pins as f64 - 1.0).max(1.0).sqrt()).min(1.0),
            KiCadNetWeight::Flat => 1.0,
            KiCadNetWeight::Rail => {
                if named_rail || pins >= RAIL_PINS {
                    1.0
                } else {
                    steep
                }
            }
        }
    }

    /// Whether a net's name is a supply's or ground's (what `rail` counts
    /// as a rail whatever its pin count).
    pub fn named_rail(name: &str) -> bool {
        crate::quality::ground(name) || crate::quality::rail_name(name)
    }
}

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
    /// the project when not given: its classes' largest, and a custom
    /// rule's between the pads of every two nets on some layer (the rules
    /// report's `pad_clearance_mm`: OpenESC's 0.16 mm on the outer layers).
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
    /// Whether the project makes the board's silkscreen over a part an
    /// error (`silk_overlap` or `silk_over_copper`); `None`: read it from
    /// the project. Where it does not, the board's silkscreen marks are no
    /// obstacle at the placer's last level (KiCad's legality), only a
    /// preference at the others.
    pub silk_over_parts_error: Option<bool>,
    /// Decoupling capacitors (small capacitors between a supply rail and
    /// ground) are pulled to the supply pins of the ICs on their rail,
    /// unless a constraint already names them.
    pub auto_decoupling: bool,
    /// How close (body to pin) a decoupling capacitor below 4.7 uF is
    /// pulled to its IC's supply pin; `None` (the default): 1 mm with
    /// `decoupling_near`, else 2.5 mm. At 2.5 mm (first pins only) a third
    /// of the capacitors ended within 3 mm of their pin where designers put
    /// 60 % there (quick tier). Bulk capacitors (4.7 uF and more) 5 mm.
    pub decoupling_mm: Option<f64>,
    /// Decoupling at the pins (default off; docs/placer.md, "Decoupling at
    /// the pins (2026-10-26)"): each decoupling capacitor is pulled to the
    /// nearest of every supply pin of its rail on the ICs (an RP2350's ten
    /// +3V3 pins), not only to each IC's first pin on the rail, at 1 mm
    /// (`decoupling_mm`); and near relations follow their anchors in the
    /// legaliser (`follow_anchors`: a part near another is legalised right
    /// after it, from its annealed place moved as far as its anchor moved,
    /// and pays for the relation at each spot). The trial's rp2350-dual-usb
    /// went from 14 % of its capacitors within 3 mm of a pin to 50 %; on
    /// the eleven dense boards the capacitors' median fell on nine and rose
    /// on the two OpenESC boards, KiCad's unconnected within the noise
    /// (summed medians 974 to 987), so by the agreed rule it is a switch.
    pub decoupling_near: bool,
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
    /// Panel-aware placement (on since 2026-10-26; false turns it off):
    /// where the board holds
    /// copies of one circuit (the same footprints, nets that map one to
    /// one; told apart by net islands, numbered or repeated references),
    /// the first copy is placed with the ordinary placer inside its region
    /// (from the copies' fixed parts, the outline's pieces, or slices of
    /// the board), the placement is replicated to every copy by the
    /// transform its fixed parts give (a translation and a quarter turn),
    /// and the replicated parts that are not legal where they land and
    /// the parts in no copy are placed after it. `board-placer.json` and
    /// the verdict report the copies (`panel_copies`; docs/placer.md,
    /// "Panel copies (2026-10-26)"). Boards without copies place as with
    /// it off.
    pub panel_copies: bool,
    /// Sub-boards of a panel of different boards (on by default; false
    /// turns it off): where the outline falls into pieces once its tabs
    /// are taken out and nets of one name lie on several of them (the
    /// OpenESC 4in1 panel), each part whose sub-board is known (a fixed
    /// part's place, the reference board's, nets no evidence splits)
    /// stays in its sub-board's region, the same-named nets pull each
    /// sub-board's pins only, and no part sits on a tab. Panels of copies
    /// (`panel_copies`) are placed as such. `board-placer.json` and the
    /// verdict report the sub-boards (`sub_boards`; docs/placer.md,
    /// "Sub-boards in a panel (2026-10-27)").
    pub sub_boards: bool,
    /// The reference board (the layout's `--reference`): where its
    /// footprints lie says which sub-board each part belongs to. Set by
    /// the layout, not read from the configuration.
    #[serde(skip)]
    pub sub_board_reference: Option<PathBuf>,
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
    /// What the levels whose spacing is the rules' least call legal:
    /// `"model"` (the default) or `"kicad"` (`KiCadPlacerLegality`).
    pub legality: KiCadPlacerLegality,
    /// How a net's weight in the wirelength falls with its pin count, the
    /// same weight in the global placement, the anneal, the legaliser, the
    /// refinement and the relation repair: `"steep"` (the default,
    /// `min(1, 3 / (pins - 1))`), `"sqrt"` (`min(1, 3 / sqrt(pins - 1))`),
    /// `"flat"` (every net 1) or `"rail"` (steep for signals, 1 for nets of
    /// nine pins or more and those named as a supply or ground); the nets
    /// designers keep short count three times on top (docs/placer.md, "The
    /// designer as teacher (2026-11-04)", "Net weights").
    pub net_weight: KiCadNetWeight,
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
    /// The global route inside the placer's search (an experiment, off):
    /// the anneal re-evaluates the global route of its placement and
    /// prefers moves away from overflowing boundaries, proposing the parts
    /// whose nets cross them more often; the coupled loop moves those
    /// parts first (docs/global-route.md, "Phase 2").
    #[serde(default)]
    pub global_guidance: Option<KiCadGlobalGuidance>,
    /// Escape demand in placement (on; `null` turns it off): where each
    /// part's nets leave its body, and the channels between parts that
    /// must carry them, shape the global placement's density, the rotation
    /// choice, the legaliser and the refinement; in a layout the
    /// refinement corrects the estimate by the global route's overflow
    /// attribution (docs/placer.md, "Escape demand (2026-10-23)").
    #[serde(default = "default_escape_demand")]
    pub escape_demand: Option<KiCadEscapeDemand>,
    /// Ratsnest crossings as a placement cost (on since 2026-10-24; `null`
    /// turns it off): per net the minimum spanning tree
    /// of its pins, and the pairs of tree lines of different nets that
    /// cross. The legaliser and the refinement pay `weight` mm of
    /// wirelength per crossing the moved part's nets make, and the
    /// layout's race ranks the seeds' placements by their crossings beside
    /// the congestion score. Keys `weight`, `legalise`, `refine`,
    /// `rank_seeds`, `anneal`, `global` (defaults 3 mm, on, on, on, on
    /// since 2026-10-28, off; docs/placer.md, "Ratsnest crossings
    /// (2026-10-24)").
    #[serde(default = "default_ratsnest_crossings")]
    pub ratsnest_crossings: Option<KiCadRatsnestCrossings>,
    /// Bus-group moves (off by default; `{}` for the defaults): banks of
    /// parts of one footprint that share a bus (`min_shared` nets of up
    /// to `bus_net_pins` pins between two, at most `max_bank` parts), and
    /// each other part with its passives (up to `passive_pins` pins and
    /// `passive_area_mm2`: those a relation binds to it, a decoupling
    /// capacitor, and those sharing a net within `reach_mm`), move as
    /// units. After the global placement each bank takes the order and
    /// arrangement (in place, side by side along x or y) that lowers its
    /// nets' wirelength plus crossings (`arrange`); the anneal shifts,
    /// turns, swaps and reorders groups each stage (`anneal`,
    /// `anneal_moves` tries per group, a bank's members moving only with
    /// it: `rigid_banks`), and the refinement does so legally, each unit
    /// placed whole (`refine`), all judged by wirelength plus their nets'
    /// crossings. Defaults: `min_shared` 4, `bus_net_pins` 24,
    /// `passive_pins` 4, `passive_area_mm2` 16, `reach_mm` 3, `arrange`,
    /// `anneal`, `rigid_banks` and `refine` true, `anneal_moves` 2,
    /// `max_bank` 16 (docs/placer.md, "Bus-group moves (2026-10-26)").
    #[serde(default)]
    pub bus_groups: Option<KiCadBusGroups>,
    /// Rows of repeated parts (off by default; `{}` for the defaults;
    /// docs/placer.md, "The designer as teacher (2026-11-04)"): parts of
    /// one kind (body size, pin count, side) pay `weight` mm of wire per
    /// millimetre their body centre stands out of line (the smaller of its
    /// x and y offsets) with the nearest of their kind within `reach_mm`,
    /// at most `cap_mm` (and `cap_mm` more when turned otherwise) in the
    /// anneal (the reach at least one and a half body sides); after the refinement each moves up to `cap_mm` to line up
    /// where that stays legal and costs at most 0.5 mm of wire (`finish`).
    /// Defaults: `weight` 1, `cap_mm` 1, `reach_mm` 5, `finish` true.
    #[serde(default)]
    pub align_rows: Option<KiCadAlignRows>,
    /// Differential pairs' series parts as rows of two (off by default;
    /// docs/placer.md, "Pair rows (2026-11-04)"): two free parts of one
    /// footprint with two pads each, whose pads of one number lie on the
    /// two nets of a pair KiCad pairs by name (`TX_P` and `TX_N`: a pair's
    /// AC-coupling capacitors or series resistors) and that no constraint
    /// names, are placed as a `row` of two, side by side across their pads,
    /// so that the pair's pads at them lie side by side and the router can
    /// couple the pair through them (router.md, "Pair points and links").
    /// The pitch is the pair's track width plus gap (its rules' or net
    /// class's), at least the courtyards' width and the pads' clearance;
    /// the row turns so that each side faces its own nets' far pins (the
    /// pair's source and destination), any quarter turn where they tell no
    /// direction. `constraints[]` reports each as `pair_row` with its pitch
    /// and how it faces (`detail`). Boards without such parts place as with
    /// it off.
    pub pair_rows: bool,
    /// The router's configuration the guidance lowers with (set by the
    /// layout; the default otherwise).
    #[serde(skip)]
    pub guidance_router: Option<KiCadBoardRouterConfig>,
    /// The far side under a via in a BGA's pad stays free (on): where the
    /// router's via-in-pad rule admits a footprint (the designer's vias in
    /// its pads, or the router's `via_in_pad`), each enclosed pad's via
    /// goes through the board, and other parts on the other side keep off
    /// it as off a through-hole pin, with the clearances a via needs
    /// (docs/placer.md, "Length maxima and the far side of vias in pads
    /// (2026-10-28)"). Nothing changes on a board without such a footprint.
    pub via_in_pad_far_side: bool,
    /// Room around grid arrays (on): around each grid array whose inner
    /// balls the router's fan-out serves with dog-bones in the diagonal
    /// gaps (`bga_fanout`; a BGA at 0.8 mm or coarser whose class via is
    /// too large for a gap: Glasgow's U30; none where the inner balls leave
    /// by vias in their pads or every ball on the surface), other
    /// parts keep off a band on the array's side for the outer rings'
    /// surface escapes (the rings that leave on the surface, a track and
    /// a clearance each, then the class via and its clearances), and off
    /// the far side under the array and a band beyond it for the via
    /// ring (a track and a clearance per ring deeper than the surface
    /// rings), where only the array's own decoupling capacitors may sit,
    /// outside the fan-out's via sites. Fixed parts stay where they are
    /// (reported). Nothing changes on a board without a grid array
    /// (docs/placer.md, "Room around grid arrays (2026-11-04)").
    pub bga_room: bool,
    /// Length maxima (on): for each net with a `length` rule and a
    /// maximum (the project's `.kicad_dru`, as the router reads it), every
    /// two of its pins on different parts are kept within 0.8 times the
    /// maximum, octilinearly, by a penalty in the global placement, the
    /// anneal, the legaliser and the refinement, and beyond the maximum
    /// by a penalty steep enough that no wirelength pays for it (the
    /// router cannot make a net shorter than its pads lie apart). Nothing
    /// changes on a board without such a rule.
    pub length_maxima: bool,
    /// The router's rules the two above read (via-in-pad admissions, net
    /// classes and via sizes; the length rules), set by the layout where
    /// either applies.
    #[serde(skip)]
    pub router_rules: Option<std::sync::Arc<KiCadBoardRouterConfig>>,
    /// The differential pairs' track widths and gaps by positive net, for
    /// the pair rows' pitch (`pair_rows`; set by the layout and the place
    /// check from the router's rules where the switch is on).
    #[serde(skip)]
    pub pair_geometry: std::collections::BTreeMap<String, PairGeometry>,
    /// Pads without a way out (off; docs/placer.md "Pads without a way
    /// out (2026-11-04)"): after each seed's placement, every pad's
    /// straight ways out are tested as the router's fan-out sees them (the
    /// statics at the class width, the placement lowered as the layout
    /// routes it); a part with a pad another part, a keepout or the board
    /// edge closes is moved or turned (it or the parts that close the pad,
    /// within `reach_mm`, the cheapest of `candidates` poses in wirelength
    /// that leaves fewer pads closed around it), and the pads left closed
    /// are reported (`pad_exits` in board-placer.json, the verdict's next
    /// actions). Needs the router's configuration (the layout and the
    /// place check give it). `PCB_PLACER_PAD_EXITS=1` turns it on with
    /// the defaults (`reach_mm` 1, `candidates` 16), `0` off.
    #[serde(default)]
    pub pad_exits: Option<KiCadPadExits>,
}

/// A differential pair's track width and gap where its coupled run is (the
/// custom rules' or the net class's, as the router takes them) and the
/// larger clearance of its two nets, millimetres.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PairGeometry {
    pub width: f64,
    pub gap: f64,
    pub clearance: f64,
}

impl KiCadBoardPlacerConfig {
    /// The decoupling capacitors' tolerance (`decoupling_mm`): 1 mm with
    /// `decoupling_near`, 2.5 mm without, unless set.
    pub fn decoupling_tolerance(&self) -> f64 {
        self.decoupling_mm
            .unwrap_or(if self.decoupling_near { 1.0 } else { 2.5 })
    }
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
            silk_over_parts_error: None,
            auto_decoupling: true,
            decoupling_mm: None,
            decoupling_near: false,
            keep_on_pieces: true,
            keep_copies: true,
            panel_copies: true,
            sub_boards: true,
            sub_board_reference: None,
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
            legality: KiCadPlacerLegality::Model,
            net_weight: KiCadNetWeight::Steep,
            keep_board_marks: true,
            attached_marks: None,
            global_guidance: None,
            escape_demand: default_escape_demand(),
            ratsnest_crossings: default_ratsnest_crossings(),
            bus_groups: None,
            align_rows: None,
            pair_rows: false,
            guidance_router: None,
            via_in_pad_far_side: true,
            bga_room: true,
            length_maxima: true,
            router_rules: None,
            pair_geometry: Default::default(),
            pad_exits: None,
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

fn default_escape_demand() -> Option<KiCadEscapeDemand> {
    Some(KiCadEscapeDemand::default())
}

/// `KiCadBoardPlacerConfig::escape_demand`.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct KiCadEscapeDemand {
    /// Millimetres of wirelength one track of channel overflow costs in the
    /// legaliser and the refinement.
    pub weight: f64,
    /// Factor on the global placement's inflation of a part's charge on a
    /// side (its demand there times the track pitch over the layers).
    pub inflate: f64,
    /// The rotation choice ranks the angles within this share of the
    /// shortest wirelength by how free their dense sides face.
    pub turn_tolerance: f64,
    /// The refinement corrects the estimate by the global route's overflow
    /// attribution per part (`pcb_global::Incremental`).
    pub measured: bool,
    /// Kept refinement moves between two measurements.
    pub measure_every: usize,
    /// The share of a boundary's capacity above which its usage counts in
    /// the measured attribution.
    pub threshold: f64,
    /// A guard (0.6 by default; `null` turns it off): where at least this
    /// share of the board's footprints is fixed, escape demand is not
    /// applied (the placement is the one without it). The movable parts of
    /// such boards are mostly passives among fixed parts, whose nets leave
    /// towards the fixed parts they belong to (docs/placer.md, "Per-board
    /// losers of the defaults (2026-10-25)" and "The 45 fixed-heavy boards
    /// (2026-10-26)").
    pub fixed_share: Option<f64>,
}

impl Default for KiCadEscapeDemand {
    fn default() -> Self {
        Self {
            weight: 2.0,
            inflate: 1.0,
            turn_tolerance: 0.05,
            measured: true,
            measure_every: 20,
            threshold: 0.5,
            fixed_share: Some(0.6),
        }
    }
}

fn default_ratsnest_crossings() -> Option<KiCadRatsnestCrossings> {
    Some(KiCadRatsnestCrossings::default())
}

/// `KiCadBoardPlacerConfig::ratsnest_crossings`.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct KiCadRatsnestCrossings {
    /// Millimetres of wirelength one crossing costs in the legaliser and
    /// the refinement (calibrated on the placement ablation's data: 2 to
    /// 3 mm order the variants and our placements best).
    pub weight: f64,
    /// The legaliser pays for the crossings a spot adds.
    pub legalise: bool,
    /// The refinement pays for them.
    pub refine: bool,
    /// The layout's race ranks the seeds' placements by the sum of their
    /// ranks by the congestion score and by the crossings.
    pub rank_seeds: bool,
    /// The anneal pays for them too (the nets of up to 16 pins, until the
    /// term's work reaches half the anneal's own), and half its swaps
    /// trade a part with one like it (the same footprint, or both with
    /// many pins): where the order of parts along a bus is decided. On
    /// by default since 2026-10-28 (the placer-default rule: -10.8 %
    /// weighted over five seed pairs on the dense boards, p 0.002; the
    /// short tier's one lost pass, framework_mobo_lefthalf at the tier's
    /// seed, checked at the five seeds: 4 strict passes on, 3 off;
    /// docs/placer.md).
    pub anneal: bool,
    /// After the global placement, the parts with 16 pins or more trade
    /// places and turn where that saves wirelength plus crossings. Off by
    /// default (no effect on the placed boards of MegaDrive and Glasgow).
    pub global: bool,
    /// A guard (off by default): the weight scales by two over the
    /// routing layers (unchanged on two layers, two thirds on three, half
    /// on four): with more layers a crossing costs a via and a layer
    /// change rather than a detour (docs/placer.md, "Per-board losers of
    /// the defaults (2026-10-25)").
    pub per_layer: bool,
}

impl Default for KiCadRatsnestCrossings {
    fn default() -> Self {
        Self {
            weight: 3.0,
            legalise: true,
            refine: true,
            rank_seeds: true,
            anneal: true,
            global: false,
            per_layer: false,
        }
    }
}

/// `KiCadBoardPlacerConfig::align_rows`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct KiCadAlignRows {
    /// Millimetres of wire one millimetre out of line costs.
    pub weight: f64,
    /// The most a part costs, in millimetres (farther counts as not in
    /// line).
    pub cap_mm: f64,
    /// Parts of a kind farther apart than this (body centres), or than
    /// one and a half times their larger body side where that is more, do
    /// not line up with each other.
    pub reach_mm: f64,
    /// After the refinement, parts line up where it costs next to nothing.
    pub finish: bool,
}

impl Default for KiCadAlignRows {
    fn default() -> Self {
        let settings = core::align::Settings::default();
        Self {
            weight: settings.weight,
            cap_mm: settings.cap,
            reach_mm: settings.reach,
            finish: settings.finish,
        }
    }
}

impl KiCadAlignRows {
    pub fn settings(&self) -> core::align::Settings {
        core::align::Settings {
            weight: self.weight,
            cap: self.cap_mm,
            reach: self.reach_mm,
            finish: self.finish,
        }
    }
}

/// `KiCadBoardPlacerConfig::bus_groups`.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct KiCadBusGroups {
    /// Two parts of one footprint form a bank when they share this many
    /// bus nets.
    pub min_shared: usize,
    /// Nets of up to this many pins are bus nets.
    pub bus_net_pins: usize,
    /// Passives: at most this many pins and this body area.
    pub passive_pins: usize,
    pub passive_area_mm2: f64,
    /// A passive goes with a part whose body lies within this gap.
    pub reach_mm: f64,
    /// After the global placement, each bank takes the order and
    /// arrangement that lowers its nets' wirelength plus crossings.
    pub arrange: bool,
    /// Group moves in the anneal and the refinement.
    pub anneal: bool,
    pub refine: bool,
    /// In the anneal a bank's members move only with their bank.
    pub rigid_banks: bool,
    /// The anneal's group moves per group a stage.
    pub anneal_moves: f64,
    /// Larger banks are not banks.
    pub max_bank: usize,
}

impl Default for KiCadBusGroups {
    fn default() -> Self {
        let settings = core::groups::Settings::default();
        Self {
            min_shared: settings.min_shared,
            bus_net_pins: settings.bus_net_pins,
            passive_pins: settings.passive_pins,
            passive_area_mm2: settings.passive_area,
            reach_mm: settings.reach,
            arrange: settings.arrange,
            anneal: settings.anneal,
            refine: settings.refine,
            rigid_banks: settings.rigid_banks,
            anneal_moves: settings.anneal_moves,
            max_bank: settings.max_bank,
        }
    }
}

impl KiCadBusGroups {
    pub fn settings(&self) -> core::groups::Settings {
        core::groups::Settings {
            min_shared: self.min_shared,
            bus_net_pins: self.bus_net_pins,
            passive_pins: self.passive_pins,
            passive_area: self.passive_area_mm2,
            reach: self.reach_mm,
            arrange: self.arrange,
            anneal: self.anneal,
            refine: self.refine,
            rigid_banks: self.rigid_banks,
            anneal_moves: self.anneal_moves,
            max_bank: self.max_bank,
        }
    }
}

/// The nets whose spanning trees count for the crossings (escape demand's:
/// two pins or more, not carried by a pour).
pub(crate) fn crossing_nets(lowered: &LoweredPlacement) -> Vec<bool> {
    let mut pins = vec![0usize; lowered.problem.net_weights.len()];
    for component in &lowered.problem.components {
        for pin in &component.pins {
            pins[pin.net] += 1;
        }
    }
    pins.iter()
        .zip(&lowered.poured)
        .map(|(pins, poured)| *pins >= 2 && !poured)
        .collect()
}

/// The lowering's escape fact (`core::escape::Escape`): the track pitch,
/// the routable layers, and the nets whose escapes count (two pins or
/// more, not carried by a pour).
pub(super) fn escape_fact(
    config: &KiCadEscapeDemand,
    pitch: f64,
    lowered: &LoweredPlacement,
    measured: Option<std::sync::Arc<core::escape::MeasuredFactory>>,
) -> core::escape::Escape {
    let mut pins = vec![0usize; lowered.problem.net_weights.len()];
    for component in &lowered.problem.components {
        for pin in &component.pins {
            pins[pin.net] += 1;
        }
    }
    core::escape::Escape {
        pitch,
        layers: lowered.routing_layers.max(1.0),
        counted: pins
            .iter()
            .zip(&lowered.poured)
            .map(|(pins, poured)| *pins >= 2 && !poured)
            .collect(),
        weight: config.weight,
        inflate: config.inflate,
        turn_tolerance: config.turn_tolerance,
        measured,
        measure_every: config.measure_every,
    }
}
