// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Whole-board negotiated-congestion routing (PathFinder style).
//!
//! Every net is routed on a shared lattice. Nets may overlap each other's
//! clearance zones at a price. The price of contested nodes rises from
//! iteration to iteration (present and historical congestion), and only
//! nets that are still in conflict are ripped up and rerouted, until no
//! conflicts remain. Pads, keepouts and the outline are never negotiable.

use rayon::prelude::*;
use std::cell::RefCell;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::sync::Arc;

use crate::board::{Board, NetId, NetRoute, RuleClass, Segment, Via};
use crate::grid::{DIRECTIONS, Grid, SAFETY, StaticMaps};
/// Read-only tile views of the negotiation, for the congestion model.
pub mod congestion;
mod fill;

#[derive(Clone, Debug)]
pub struct Config {
    /// Candidate lattice pitches; the one aligning best with the pads wins.
    pub pitches: Vec<f64>,
    /// Cost of a via, in millimetres of trace.
    pub via_cost: f64,
    /// Cost per 45 degrees of direction change, in millimetres.
    pub bend_cost: f64,
    /// Cost factor for running across a layer's preferred axis. Layers
    /// alternate between horizontal and vertical preference.
    pub against_direction: f64,
    pub present_factor: f64,
    pub present_growth: f64,
    /// Upper bound of the present-congestion factor. Beyond it contested
    /// nodes act as walls the heuristic knows nothing about, and searches
    /// flood; history cost keeps separating the nets instead.
    pub present_cap: f64,
    /// Connect pour nets' pads to their planes first (the pour nets
    /// negotiating among themselves) and keep those stubs and vias fixed
    /// through negotiation. Off: on ColdFire the pre-pass never settles and
    /// fixed stubs left clearance violations.
    pub fixed_plane_stubs: bool,
    /// Keep other nets' tracks off exclusive planes entirely (else they pay
    /// `plane_cut_cost` there, as designers do cut planes locally).
    pub exclusive_planes: bool,
    /// Within this distance of a net's own narrow pads (narrower than its
    /// track plus clearance) the net routes at the board's neck width: the
    /// fan-out of a fine-pitch part. 0 turns it off.
    pub neck_reach: f64,
    /// Plan every net on the tile graph first and confine its lattice
    /// search to that plan's corridor; conflicts teach the plan.
    pub global_routing: bool,
    /// Iterations without fewer conflicted nets that negotiation waits
    /// once the price of sharing is at its cap.
    pub stall_at_cap: usize,
    /// At the price cap, how much fewer conflicted nets count as progress
    /// (a share of the last low; at least two nets when above zero). Zero:
    /// any new low resets the stall count, which lets a board crawl down
    /// one net every few iterations for hundreds of seconds (Sisu: 27-32
    /// conflicted from iteration 6 to 38).
    pub stall_drop: f64,
    /// Iterations without fewer conflicted nets after which negotiation
    /// stops below the price cap (`stall_at_cap` at it).
    pub stall_patience: usize,
    /// Stop negotiating when a quarter of the nets have no path at all
    /// after the first iteration (see `Router::hopeless`).
    pub abandon_hopeless: bool,
    /// During negotiation, check every iteration which pads of a pour net
    /// the signals have cut off from the pour's main piece, and route those
    /// to it like off-pour pads (a via into the plane, mostly), so that the
    /// supply nets negotiate with the signals instead of being repaired
    /// after them, when the room is gone (ESC mini: 41 of GND's pads
    /// stranded after the stitching). Pads whose piece rejoins lose their
    /// stub again (a sticky first version left the early iterations' stubs
    /// in the signals' way). Connect rungs with it: ESC mini 22 open
    /// against 27, Hub 123 against 135; tracks mode unchanged; it costs
    /// iterations.
    pub pour_islands: bool,
    /// A pad's thermal spoke counts only where its strip keeps the
    /// clearance from other nets' copper all the way to the fill, as
    /// KiCad's does (`analyze_pours_full`).
    pub spoke_clearance: bool,
    /// Thermal spokes as KiCad draws and counts them (`kicad_spokes`): the
    /// spoke's end at the gap and half the minimum width, its width no
    /// more than the pad's, a piece that joins two pads is no island, and
    /// a pad's own track into the fill counts as a spoke.
    pub kicad_spokes: bool,
    /// A pad that the spoke model finds short of KiCad's two spokes gets a
    /// straight track into the main piece of its pour, which KiCad's DRC
    /// counts as a spoke (`add_manual_spokes`, after the stitching).
    pub manual_spokes: bool,
    /// A pad whose spokes on a layer reach only fill pieces nothing else
    /// of the net touches (KiCad's "spokes connected to isolated island")
    /// and that no straight manual spoke joins to a kept piece gets a
    /// stitching via from such an island to the net's main piece on
    /// another layer, where one fits (`add_manual_spokes`): the island
    /// then holds the pad and the via, and is part of the plane.
    pub island_vias: bool,
    /// A pad's own track joins the fill piece of one of its spokes only
    /// where the track's first run overlaps the spoke outside the pad's
    /// drill hole (`kicad_spokes`; without it, a routed track joins all
    /// of them, as if the spokes met at the pad's centre).
    pub track_meets_spokes: bool,
    /// Failing that, a straight track of the pad on the island's layer
    /// into the island (`add_manual_spokes`): KiCad's connectivity joins
    /// the track to the piece it overlaps, the piece has two items and is
    /// no island, and the DRC counts its spokes and the track. It changes
    /// no connection (the island was the pad's already); k30 and
    /// mackerel-30's tracks rungs, whose inner planes the signals cut into
    /// islands around through-hole pads.
    pub island_stubs: bool,
    /// Other pads' spokes across a pad's ring count as its spokes, as
    /// KiCad's DRC counts every crossing of the fill with the ring
    /// (`kicad_spokes`; `PCB_ROUTER_NO_FOREIGN_SPOKES` for A/B).
    pub foreign_spokes: bool,
    /// A track or via joins every pour piece its copper overlaps, not only
    /// those its nodes lie in (`PCB_ROUTER_OLD_TRACK_PIECES` for A/B).
    pub track_touch_fill: bool,
    /// A piece a spoke lands in, short of two items, joins the pieces next
    /// to it that KiCad's fill joins through a neck the lattice cuts
    /// (`PCB_ROUTER_NO_NECK_JOINS` for A/B).
    pub neck_joins: bool,
    /// Spokes in the pad's own frame (`Board::pad_spokes`) rather than
    /// along its box's axes (`PCB_ROUTER_OLD_SPOKE_FRAMES` for A/B).
    pub spoke_frames: bool,
    /// Pads merged into a larger pad of their number are checked inside
    /// its copper too, as KiCad builds and counts their spokes there (an
    /// exposed pad's thermal vias; `PCB_ROUTER_NO_INNER_PADS` for A/B).
    pub inner_pads: bool,
    /// The DRC's track-end test keeps the clearance to KiCad's DRC epsilon,
    /// half a micron (`fill::TRACK_END_TOLERANCE`), rather than the spokes'
    /// 3 microns (`PCB_ROUTER_OLD_TRACK_ENDS` for A/B).
    pub track_end_tolerance: bool,
    /// The clean-up keeps a change only if no pour net has more pads off
    /// its main piece after it.
    pub pour_aware_polish: bool,
    /// The same for a via reduction round.
    pub pour_aware_via_reduction: bool,
    /// Whether a pour net stuck in conflict is ripped up whole now and
    /// then like the other nets (`rip_up_whole`).
    pub pour_whole_rip_up: bool,
    /// During negotiation, a pad cut off from its pour's main piece first
    /// gets a stitching via from its piece to the main piece on another
    /// layer, where one fits, before it is routed as a track. Off: Sisu's
    /// connect rung got worse with it (68 open against 63, 74 against 56
    /// with narrow signals) though fewer pads were cut off (1313 against
    /// 1758).
    pub stitch_in_negotiation: bool,
    /// Fine-pitch pads (narrower than the track plus its clearance, on one
    /// layer, in a row of their kind) get a straight stub outward fixed
    /// before negotiation, up to this long in mm (0, the default, turns it
    /// off). The stubs keep every pad's only way out open: nothing else
    /// may run along the row inside the escape zone. Tried on Tiny Tapeout
    /// (82/108 either way) and ColdFire (205 vs 208 of 209): no gain yet.
    pub escape_stub_mm: f64,
    pub history_increment: f64,
    pub max_iterations: usize,
    /// Minimum search window margin around a net's terminals.
    pub window_margin: f64,
    /// Passes of per-net shortening after the board is conflict free.
    pub cleanup_passes: usize,
    /// Via cost while cleaning up. The existing route is always a fallback
    /// there, so a high value removes vias without risking completion.
    pub cleanup_via_cost: f64,
    /// Via cost when connecting to a copper pour: a short stub and a via is
    /// the preferred connection, not a long track to another pad.
    pub plane_via_cost: f64,
    /// Plan every connection on a coarse tile graph first and search the
    /// lattice only inside that corridor (widening it on failure).
    pub corridors: bool,
    /// Cost factor for a trace step on a node covered by another net's
    /// pour, and for a via through one: cutting a plane is expensive.
    pub plane_cut_cost: f64,
    /// Route every pour net first as a tree on its best-covered layer and
    /// keep that tree fixed: the pour then only adds copper, and signal
    /// routing cannot strand a pad on an island.
    pub plane_skeleton: bool,
    /// Cost multiplier for skeleton steps off the plane layer.
    pub skeleton_bias: f64,
    /// After the board is complete, renegotiate the nets that have vias with
    /// the via cost multiplied by `via_reduction_factor`, this many times,
    /// keeping a round only if nothing opens and vias go down.
    pub via_reduction_rounds: usize,
    pub via_reduction_factor: f64,
    /// Heuristic weight while reducing vias (searches there are long
    /// same-layer detours; a little greed keeps them cheap).
    pub via_reduction_weight: f64,
    /// Price of sharing when a via reduction round starts.
    pub via_reduction_present: f64,
    /// Via reduction stops once it has used this many times the main
    /// negotiation's time (at least 60 s).
    pub via_reduction_budget: f64,
    /// Seconds via reduction may take at most, whatever the negotiation
    /// took (default 300 s; the KiCad ladder sets it from what is left of
    /// its budget: a 210 x 170 mm board routed clean in 130 s and then
    /// spent the runner's remaining 770 s polishing).
    pub via_reduction_seconds: f64,
    /// The via reduction keeps its budget within a round too: a round
    /// whose cost (projected from the previous round) exceeds what is left
    /// does not start, and a round's negotiation and clean-up get only
    /// what is left. Without it a round started with budget left ran to
    /// its end (k30-SBC: a 60 s budget, one round of 182 s of work and a
    /// clean-up after it; CyberKeeb2040: 186-206 s for 87-117 s).
    pub strict_via_budget: bool,
    /// The whole run's work budget (the ladder's or the layout's, in work
    /// seconds) and what was left of it when this router started (its own
    /// work since counted against it). With more than half of the run's
    /// budget unspent when the via reduction starts, it may use up to a
    /// third of what is left, and at most 300 s (k30-SBC's round of 182 s
    /// of work, the one that took it from 414 to 354 vias, fits by right
    /// then); with less, its own budget applies strictly.
    pub run_work: Option<(f64, f64)>,
    /// A via-reduction round is kept only if it starves no more thermal
    /// pads (`starved_pads`) than before. Off: the router's count is not
    /// KiCad's (mackerel-08: 0 against 5); on the quick tier it took
    /// starved thermals from 13 to 10 but cost k30-SBC its 55-via round
    /// and 5 more starved pads in KiCad's count.
    pub via_reduction_keeps_spokes: bool,
    /// Negotiation batches are routed speculatively on all threads and kept
    /// where nothing they read changed (`route_batches`): the same result
    /// as routing them in turn. Off: measured, it keeps too few routes to
    /// pay (Castor 16 %, k30-SBC 8 %: the negotiation reroutes the nets of
    /// one congested area after one another, each reading what the one
    /// before changed).
    pub speculate: bool,
    /// The clean-up's trials of single nets run speculatively side by side
    /// (`clean_up_window`): the same result as in turn.
    pub speculate_cleanup: bool,
    /// Experimental: negotiation batches routed speculatively side by side
    /// (`route_batches`) keep their route unless the route itself (its new
    /// branches, a block around each node) meets copper an earlier batch of
    /// the window added or removed; what it read elsewhere may have
    /// changed (the next iteration prices any conflict left). Not the
    /// result of routing in turn, but reproducible: the window is a fixed
    /// 16 nets whatever the threads.
    pub speculate_paths: bool,
    /// Seconds the clean-up may take at most: no batch starts later than
    /// this, nor later than the negotiation took (at least 60 s). A
    /// 130 x 90 mm six-layer board spent 470 s in clean-up.
    pub cleanup_seconds: f64,
    /// Cost factor for another net's step on the ring around a pad that
    /// joins its pour by thermal spokes (copper there starves the spokes).
    pub thermal_guard_cost: f64,
    /// A pad with two spoke corridors or fewer that the fixed copper
    /// leaves open (a fine-pitch row: the neighbours' pads take the two
    /// along the row) keeps those closed to other nets' copper altogether,
    /// not just at a cost. Off: on k30-SBC it left the 8 one-spoke pads as
    /// they were and on Castor it cost six nets their route.
    pub hard_thermal_guards: bool,
    /// A negotiation that has not converged after this many seconds is
    /// handed to the resolution step as it is.
    pub negotiation_seconds: f64,
    /// Search expansions one negotiation may spend (0: no limit but the
    /// seconds). Unlike seconds, independent of the machine's load.
    pub negotiation_expansions: u64,
    /// Route spatially disjoint nets of one iteration in parallel.
    pub parallel: bool,
    /// Batch size when overlap is ignored (0 keeps batches disjoint). Nets in
    /// a batch then route against the same occupancy and settle their
    /// conflicts in later iterations.
    pub jacobi_batch: usize,
    /// Values above 1 trade path optimality for search speed.
    pub heuristic_weight: f64,
    /// Print one progress line per iteration to stderr.
    /// Perturbs the net order and seeds the history with noise of one
    /// history increment, so the same board can be routed several ways
    /// (`None`: the deterministic order).
    pub seed: Option<u64>,
    pub verbose: bool,
    /// A wall-clock time by which routing returns what it has, however
    /// much work its budgets still allow: every budget counts as spent
    /// from then on (an agent waits for the result).
    pub deadline: Option<std::time::Instant>,
    /// The work (this router's own, in seconds of an idle machine) at
    /// which routing returns what it has, as at `deadline`: the caller's
    /// deadline counted in work, so that where a run stops does not depend
    /// on how busy the machine is (the layout sets it from its own budget;
    /// `deadline` stays as the guard against load).
    pub deadline_work: Option<f64>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            pitches: vec![0.127, 0.125, 0.1],
            via_cost: 8.0,
            bend_cost: 0.1,
            against_direction: 1.6,
            present_factor: 0.5,
            present_growth: 1.5,
            present_cap: 1.0e4,
            // At the price cap, twelve iterations without a new low of
            // conflicted nets end a negotiation: after that the rest seldom
            // gains (Sisu stalled at 25 conflicted nets from iteration 21
            // and negotiated on to 47; 54 % of its expansions came after
            // iteration 20). Below the cap the rising price still moves
            // nets (a three-track channel needs more than twelve).
            stall_at_cap: 12,
            stall_drop: 0.0,
            stall_patience: 25,
            abandon_hopeless: true,
            pour_islands: true,
            spoke_clearance: true,
            // Experiment hooks for A/B runs of whole tiers: the old spoke
            // model, no manual spokes.
            kicad_spokes: std::env::var_os("PCB_ROUTER_OLD_SPOKES").is_none(),
            manual_spokes: std::env::var_os("PCB_ROUTER_NO_MANUAL_SPOKES").is_none(),
            island_vias: std::env::var_os("PCB_ROUTER_NO_ISLAND_VIAS").is_none(),
            track_meets_spokes: std::env::var_os("PCB_ROUTER_OLD_TRACKED").is_none(),
            island_stubs: std::env::var_os("PCB_ROUTER_NO_ISLAND_STUBS").is_none(),
            foreign_spokes: std::env::var_os("PCB_ROUTER_NO_FOREIGN_SPOKES").is_none(),
            track_touch_fill: std::env::var_os("PCB_ROUTER_OLD_TRACK_PIECES").is_none(),
            neck_joins: std::env::var_os("PCB_ROUTER_NO_NECK_JOINS").is_none(),
            spoke_frames: std::env::var_os("PCB_ROUTER_OLD_SPOKE_FRAMES").is_none(),
            inner_pads: std::env::var_os("PCB_ROUTER_NO_INNER_PADS").is_none(),
            track_end_tolerance: std::env::var_os("PCB_ROUTER_OLD_TRACK_ENDS").is_none(),
            pour_aware_polish: true,
            pour_aware_via_reduction: true,
            pour_whole_rip_up: true,
            stitch_in_negotiation: false,
            escape_stub_mm: 0.0,
            global_routing: false,
            neck_reach: 1.5,
            fixed_plane_stubs: false,
            exclusive_planes: false,
            history_increment: 0.3,
            max_iterations: 80,
            window_margin: 10.0,
            cleanup_passes: 4,
            cleanup_via_cost: 25.0,
            plane_via_cost: 2.0,
            corridors: true,
            parallel: true,
            jacobi_batch: 0,
            via_reduction_rounds: 3,
            via_reduction_factor: 2.0,
            via_reduction_weight: 1.0,
            via_reduction_present: 0.5,
            via_reduction_budget: 3.0,
            via_reduction_seconds: 300.0,
            strict_via_budget: true,
            run_work: None,
            via_reduction_keeps_spokes: false,
            speculate: false,
            speculate_cleanup: false,
            speculate_paths: false,
            cleanup_seconds: 180.0,
            thermal_guard_cost: 10.0,
            hard_thermal_guards: false,
            negotiation_seconds: 900.0,
            negotiation_expansions: 0,
            plane_cut_cost: 3.0,
            plane_skeleton: false,
            skeleton_bias: 6.0,
            heuristic_weight: 1.0,
            seed: None,
            verbose: false,
            deadline: None,
            deadline_work: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetStatus {
    /// Fewer than two terminals: nothing to do.
    Trivial,
    Routed,
    /// Some terminals could not be connected without violating a rule.
    Partial {
        unconnected_terminals: usize,
    },
    /// A terminal has no legal lattice node at all.
    Unreachable,
}

#[derive(Clone, Debug)]
pub struct RoutingResult {
    pub grid: Grid,
    /// Per lattice node: accumulated congestion history over all layers and
    /// vias. It is large where nets kept fighting for space.
    pub congestion: Vec<f32>,
    pub routes: Vec<NetRoute>,
    pub status: Vec<NetStatus>,
    pub iterations: usize,
    pub expansions: u64,
    pub searches: u64,
    /// Why the routing is not complete, when it is not.
    pub diagnostics: Diagnostics,
}

/// A machine-readable account of what stood in the way.
#[derive(Clone, Debug, Default)]
pub struct Diagnostics {
    /// Pads that no track of their net's class can enter, with the objects
    /// around them (candidates for what blocks them).
    pub dead_pads: Vec<DeadPad>,
    /// Nets still in conflict when negotiation stopped.
    pub conflicted: Vec<NetId>,
    /// The tiles where nets fought longest, most contested first.
    pub hot_spots: Vec<HotSpot>,
}

#[derive(Clone, Debug)]
pub struct DeadPad {
    pub net: NetId,
    pub label: String,
    pub anchor: crate::geometry::Point,
    pub near: Vec<String>,
}

/// Set on a `guard` entry whose corridor no other net may enter.
const HARD_GUARD: u32 = 1 << 31;

#[derive(Clone, Debug)]
pub struct HotSpot {
    pub layer: usize,
    pub center: crate::geometry::Point,
    pub history: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Node {
    layer: u8,
    cell: u32,
}

const NO_TERMINAL: u16 = u16::MAX;
/// Search expansions an idle 8-core machine does per second of negotiation
/// (ATAT1800: 52M in 29 s; Interf-U: 19M in 7.8 s), to turn a time budget
/// into work, and how much longer than that the clock may run before it
/// stops a negotiation anyway (a loaded machine).
/// How far an escape stub may come short of a clearance: floating point
/// noise, far below the verifier's tolerance and KiCad's DRC epsilon.
const STUB_FIT: f64 = 1.0e-6;

pub const EXPANSIONS_PER_SECOND: f64 = 2.0e6;
pub const GUARD: f64 = 4.0;
/// Lattice nodes of pour layers the pour analysis (`analyze_pours`, one
/// thread) takes in per second of an idle machine: the budgets of the
/// phases after negotiation count it as work beside the search
/// (`budget_work`; calibrated below a load of 20, see docs/router.md,
/// "Determinism and the wall clock").
pub const POUR_NODES_PER_SECOND: f64 = 20.0e6;
/// A branch end that lands on one of the net's copper pours.
const PLANE_TERMINAL: u16 = u16::MAX - 1;
/// A branch end that ends in the open, needing no host: the outer end of
/// a fixed escape stub. It can host junctions like any inner node.
const FREE_END: u16 = u16::MAX - 2;
const PARENT_SOURCE: u8 = 255;
/// Side of a coarse tile, in lattice nodes (a power of two).
const TILE: usize = 16;

#[derive(Clone, Debug)]
struct Branch {
    nodes: Vec<Node>,
    start_terminal: u16,
    end_terminal: u16,
}

/// What the pour model makes of one thermal spoke of a pad
/// (`analyze_pours_full`): no fill where the spoke would land, fill it
/// cannot reach for other nets' copper beside it, a piece without copper
/// of the net (an island to KiCad), or a piece that holds such copper.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SpokeOutcome {
    NoFill,
    Blocked,
    Island(u32),
    Good(u32),
    /// A spoke meeting a spoke of another pad of the net (that terminal).
    Pad(u32),
}

/// A pad's spokes on one layer as the pour model sees them, in the order
/// +x, -x, +y, -y (round pads: the diagonals +x+y, +x-y, -x+y, -x-y).
#[derive(Clone, Debug)]
pub struct SpokeRecord {
    pub terminal: usize,
    /// The pad (index into the board's obstacles) and the point its spokes
    /// leave from: the terminal's, or a pad merged into it
    /// (`Board::merged_pads`).
    pub pad: usize,
    pub anchor: crate::geometry::Point,
    /// Other pads' spokes that cross the ring half a gap around the pad,
    /// which KiCad's DRC counts as this pad's (`foreign_spokes`).
    pub crossings: usize,
    pub layer: usize,
    pub round: bool,
    pub outcomes: [SpokeOutcome; 4],
    /// The pad's own tracks that KiCad's DRC counts as spokes (a run
    /// leaving the pad and ending in fill), counted when the spokes alone
    /// are short of two (`kicad_spokes`).
    pub tracks: usize,
}

#[derive(Clone, Debug, Default)]
struct NetState {
    /// Branches made from a routed board's own tracks
    /// (`adopt_existing_copper`), by index: the track's exact ends.
    adopted: HashMap<usize, (crate::geometry::Point, crate::geometry::Point, f64)>,
    terminal_nodes: Vec<Vec<Node>>,
    branches: Vec<Branch>,
    connected: Vec<bool>,
    /// (map index, node) pairs this net currently contributes to.
    stamped: Vec<(u32, u32)>,
    /// Largest anchor-to-pad-node distance per terminal, in millimetres.
    terminal_reach: Vec<f32>,
    routable: bool,
    complete: bool,
    /// Even with every other net ignored no complete tree exists.
    blocked: bool,
    /// Per layer: the nodes covered by this net's pours (empty without).
    plane: Vec<Vec<bool>>,
    /// Terminals that touch a pour and therefore need no tracks. A terminal
    /// the signals cut off from the main piece leaves this (see
    /// `refresh_pour`) and keeps needing a branch from then on.
    on_plane: Vec<bool>,
    /// Terminal nodes outside their (too narrow) pad, with the lattice
    /// nodes sampled along the straight stub from the pad centre.
    escapes: HashMap<Node, (Vec<u32>, f64, Vec<crate::geometry::Point>)>,
    /// How often negotiation had to reroute this net; stubborn nets get
    /// wider corridors and finally none.
    reroutes: usize,
    /// Consecutive iterations in conflict. A net stuck this long is ripped
    /// up whole now and then: the branches it kept may be what boxes the
    /// other net in.
    stuck: usize,
    /// Connections of this net that found no path on the whole board in
    /// this negotiation. Only what never changes during it (pads, rule
    /// areas, the board's shape) stops an ordinary search, so after two the
    /// net's searches keep to their corridors: link spent a third of its
    /// search on such hopeless board-wide searches each iteration.
    hopeless_searches: u8,
    /// Per layer: the rule class describing the pour there.
    plane_class: Vec<usize>,
    /// When set, only these pour nodes (the main piece) are valid targets.
    plane_target: Vec<Vec<bool>>,
    /// Leading branches that are the plane skeleton; never ripped up.
    fixed: usize,
    /// Node ranges (x0, y0, x1, y1) around the net's narrow pads where it
    /// routes at the neck width.
    neck_zones: Vec<(usize, usize, usize, usize)>,
}

#[derive(Clone)]
struct Stamps {
    /// Like `trace_to_*`, grown by the error of snapping an off-lattice
    /// stub sample to its nearest node.
    stub_to_trace: Vec<(i32, i32)>,
    stub_to_via: Vec<(i32, i32)>,
    trace_to_trace: Vec<(i32, i32)>,
    trace_to_via: Vec<(i32, i32)>,
    via_to_trace: Vec<(i32, i32)>,
    via_to_via: Vec<(i32, i32)>,
    /// Cells a diagonal trace step from a node covers beyond the discs at
    /// its ends: the nodes on the step's perpendicular bisector closer to
    /// its middle than the clearance (relative to the step's start, for
    /// the step direction (+1, +1); mirrored for the others). No node of
    /// the lattice lies closer to an axis step than to its ends.
    diagonal_to_trace: Vec<(i32, i32)>,
    diagonal_to_via: Vec<(i32, i32)>,
    /// The reverse: the start cells (relative to a node, a via, a stub
    /// cell) of the diagonal steps of the querying class that pass it too
    /// closely, per orientation (see `Router::diagonal_map`).
    diagonal_block: [Vec<(i32, i32)>; 2],
    diagonal_block_via: [Vec<(i32, i32)>; 2],
    stub_diagonal_block: [Vec<(i32, i32)>; 2],
}

/// KiCad connects a track to a pad when the track ends inside the pad's
/// copper. Require a little depth so rounding cannot move it outside.
fn well_inside(shape: &crate::geometry::Shape, point: crate::geometry::Point) -> bool {
    const DEPTH: f64 = 0.02;
    shape.contains(point)
        && [(DEPTH, 0.0), (-DEPTH, 0.0), (0.0, DEPTH), (0.0, -DEPTH)]
            .iter()
            .all(|(dx, dy)| shape.contains([point[0] + dx, point[1] + dy]))
}

/// Maps every node that can host a junction to the kept branch owning it.
/// A branch's own junction end belongs to its host, not to itself.
/// A net's own stamps on a few occupancy maps, as bitsets: what the pour
/// analysis asks of every node ("is the one stamp here this net's?"),
/// without hashing the net's whole stamp list each time (a ground net
/// stamps millions of cells).
struct OwnStamps {
    slot: HashMap<usize, usize>,
    bits: Vec<Vec<u64>>,
}

impl OwnStamps {
    fn new(stamped: &[(u32, u32)], maps: &[usize], cells: usize) -> Self {
        let mut slot = HashMap::new();
        for map in maps {
            let next = slot.len();
            slot.entry(*map).or_insert(next);
        }
        let mut bits = vec![vec![0u64; cells.div_ceil(64)]; slot.len()];
        for (map, cell) in stamped {
            if let Some(index) = slot.get(&(*map as usize)) {
                bits[*index][*cell as usize / 64] |= 1 << (*cell % 64);
            }
        }
        Self { slot, bits }
    }

    /// Whether the net stamped `cell` of `map` (one of the maps given).
    fn contains(&self, map: usize, cell: usize) -> bool {
        let index = self.slot[&map];
        self.bits[index][cell / 64] & (1 << (cell % 64)) != 0
    }
}

fn junction_hosts(branches: &[Branch], keep: &[bool]) -> HashMap<Node, usize> {
    let mut hosts = HashMap::new();
    for (index, branch) in branches.iter().enumerate() {
        if !keep[index] {
            continue;
        }
        let first = (branch.start_terminal == NO_TERMINAL) as usize;
        let last = branch.nodes.len().saturating_sub((branch.end_terminal == NO_TERMINAL) as usize);
        // A one-node branch between two junctions hosts nothing (the Hub
        // board: "slice index starts at 1 but ends at 0").
        if first >= last {
            continue;
        }
        for node in &branch.nodes[first..last] {
            hosts.entry(*node).or_insert(index);
        }
    }
    hosts
}

/// Experiment hook: `PCB_ROUTER_SEED` perturbs the net order and seeds the
/// history with noise, so the same board can be routed many different ways.
fn experiment_seed() -> Option<u64> {
    std::env::var("PCB_ROUTER_SEED").ok()?.parse().ok()
}

/// A search heap entry: the estimate's bits (non-negative floats order as
/// their bits) above the state, compared as one word.
/// A min-heap of search keys (`heap_key`), four children a node: half the
/// levels of a binary heap, and a node's children share a cache line. It
/// lives in the search scratch, so that its buffer is not grown again for
/// every search (with the four-way layout 9-13 % less CPU per routed board,
/// k30-SBC and Sisu, identical routes). Keys order totally (a key holds
/// its state), so the pops are those of any other min-heap.
#[derive(Default)]
struct KeyHeap {
    keys: Vec<u64>,
}

impl KeyHeap {
    fn clear(&mut self) {
        self.keys.clear();
    }

    #[inline(always)]
    fn push(&mut self, key: u64) {
        let mut index = self.keys.len();
        self.keys.push(key);
        while index > 0 {
            let parent = (index - 1) / 4;
            let above = self.keys[parent];
            if above <= key {
                break;
            }
            self.keys[index] = above;
            index = parent;
        }
        self.keys[index] = key;
    }

    #[inline(always)]
    fn pop(&mut self) -> Option<u64> {
        let last = self.keys.pop()?;
        let len = self.keys.len();
        if len == 0 {
            return Some(last);
        }
        let top = self.keys[0];
        let mut index = 0;
        loop {
            let first = 4 * index + 1;
            if first >= len {
                break;
            }
            let end = (first + 4).min(len);
            let mut child = first;
            let mut least = self.keys[first];
            for other in first + 1..end {
                let key = self.keys[other];
                if key < least {
                    least = key;
                    child = other;
                }
            }
            if least >= last {
                break;
            }
            self.keys[index] = least;
            index = child;
        }
        self.keys[index] = last;
        Some(top)
    }
}

/// What the wall clock cut short in this process (`note_wall_cut`), one
/// bit per kind (`WALL_CUTS`).
static WALL_CUT_BITS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// The kinds of wall-clock cut, in bit order.
pub const WALL_CUTS: [&str; 6] = ["deadline", "negotiation guard", "budget guard", "ladder", "placement", "layout"];

/// Records that the wall clock, not the work, stopped `what` (one of
/// `WALL_CUTS`): the result then depends on how fast the machine ran, and
/// the run says so (its log once per kind, its report's `cut`). Everything
/// else in a run is decided by work and is the same for the same input
/// and thread count.
pub fn note_wall_cut(what: &str, detail: std::fmt::Arguments) {
    let Some(bit) = WALL_CUTS.iter().position(|kind| *kind == what) else {
        return;
    };
    let before = WALL_CUT_BITS.fetch_or(1 << bit, std::sync::atomic::Ordering::Relaxed);
    if before & (1 << bit) == 0 {
        eprintln!("wall guard: the wall clock cut the run short ({what}: {detail}); its result depends on the machine's load");
    }
}

/// The kinds of wall-clock cut this process saw (`note_wall_cut`).
pub fn wall_cuts() -> Vec<String> {
    let bits = WALL_CUT_BITS.load(std::sync::atomic::Ordering::Relaxed);
    WALL_CUTS.iter().enumerate().filter(|(bit, _)| bits & (1 << bit) != 0).map(|(_, kind)| kind.to_string()).collect()
}

/// Whether the wall clock guards the work budgets (always, unless
/// `PCB_ROUTER_NO_WALL_GUARD` is set: for measuring the router's speed
/// under valgrind, where the clock runs fifty times slower than the work,
/// so that a run's routes do not depend on how fast it ran; and for
/// checking that a run is deterministic on a machine too busy for its
/// budget: the layout then sets no wall-clock deadline either).
pub fn wall_guard() -> bool {
    static GUARDED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *GUARDED.get_or_init(|| std::env::var_os("PCB_ROUTER_NO_WALL_GUARD").is_none())
}

fn heap_key(estimate: f32, state: usize) -> u64 {
    ((estimate.to_bits() as u64) << 32) | state as u64
}

impl Scratch {
    /// The next search generation; when the 16-bit counter wraps, the
    /// marks it guards are cleared.
    fn next_generation(&mut self) -> u16 {
        if self.generation == u16::MAX {
            for node in &mut self.nodes {
                node.seen = 0;
                node.closed = 0;
                node.target_mark = 0;
            }
            self.own_via_near.fill(0);
            self.generation = 0;
        }
        self.generation += 1;
        self.generation
    }

    /// Clears the routed net's own-stamp mask (`Router::mask_own`).
    fn unmask_own(&mut self) {
        for bit in self.own_set.drain(..) {
            self.own_mark[bit / 64] &= !(1 << (bit % 64));
        }
        self.own_active = false;
    }
}

/// splitmix64 mapped to [0, 1).
fn unit_noise(state: &mut u64) -> f64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
}

fn find(parent: &mut [usize], mut element: usize) -> usize {
    while parent[element] != element {
        parent[element] = parent[parent[element]];
        element = parent[element];
    }
    element
}

/// Cells whose centre lies closer than `radius` to a node, in exact
/// nanometres: a node at exactly the radius (two tracks at exactly their
/// clearance, as a 0.4 mm pad pitch with 0.2 mm tracks and clearance
/// demands) is not covered, as KiCad accepts it.
fn disc(radius: f64, pitch: f64) -> Vec<(i32, i32)> {
    let reach = (radius / pitch).ceil() as i32 + 1;
    let pitch_nm = (pitch * 1.0e6).round() as i64;
    let radius_nm = (radius * 1.0e6).round() as i64;
    let mut offsets = Vec::new();
    for dy in -reach..=reach {
        for dx in -reach..=reach {
            let d2 = (dx as i64 * dx as i64 + dy as i64 * dy as i64) * pitch_nm * pitch_nm;
            if d2 < radius_nm * radius_nm {
                offsets.push((dx, dy));
            }
        }
    }
    offsets
}

/// Cells on the perpendicular bisector of the diagonal step from (0, 0) to
/// (1, 1) that are closer than `radius` to its middle: the only nodes a
/// diagonal step passes closer than its ends do (they lie at odd multiples
/// of half the diagonal from the middle).
fn diagonal_extra(radius: f64, pitch: f64) -> Vec<(i32, i32)> {
    let half = pitch * std::f64::consts::SQRT_2 / 2.0;
    let reach = (radius / half).ceil() as i32 + 1;
    let pitch_nm = (pitch * 1.0e6).round() as i64;
    let radius_nm = (radius * 1.0e6).round() as i64;
    let mut offsets = Vec::new();
    for k in -reach..=reach {
        // The cell (i, j) with i + j = 1 and i - j = 2k + 1.
        let (i, j) = (k + 1, -k);
        // Distance to the middle (0.5, 0.5): |i - j| * pitch / sqrt 2.
        let d2 = (2 * k as i64 + 1).pow(2) * pitch_nm * pitch_nm / 2;
        if d2 < radius_nm * radius_nm {
            offsets.push((i, j));
        }
    }
    offsets
}

/// `diagonal_extra` offsets turned to the diagonal direction (dx, dy).
fn turned(offsets: &[(i32, i32)], dx: i32, dy: i32) -> Vec<(i32, i32)> {
    offsets.iter().map(|(i, j)| (i * dx, j * dy)).collect()
}

/// The start cells, relative to a node, of the diagonal steps that pass it
/// closer than `radius`: the node is on their bisector. Per orientation
/// (0: the step (+1, +1), 1: the step (+1, -1)).
fn diagonal_block(radius: f64, pitch: f64) -> [Vec<(i32, i32)>; 2] {
    let extra = diagonal_extra(radius, pitch);
    [
        extra.iter().map(|(i, j)| (-i, -j)).collect(),
        extra.iter().map(|(i, j)| (-i, *j)).collect(),
    ]
}

/// What a search step reads of a node on one layer for one rule class
/// (see `Router::hot`). Twelve bytes: the guard and the cover name a net
/// in 16 bits (`Router::new` checks the board has few enough nets).
#[derive(Clone, Copy, Default)]
struct Hot {
    trace: u32,
    history: f32,
    /// `guard` as `owner` in 15 bits, `HOT_HARD` for a hard guard.
    guard: u16,
    /// `covered` (an `owner`).
    covered: u16,
}

/// `HARD_GUARD` in `Hot::guard`.
const HOT_HARD: u16 = 1 << 15;

/// A* state of one lattice node and layer, kept together: a search step
/// reads and writes them all, one cache line instead of five.
#[derive(Clone, Copy, Default)]
struct SearchNode {
    cost: f32,
    /// Search generation marks: 16 bits (a thread's scratch is the biggest
    /// part of a large board's footprint), cleared when the counter wraps.
    seen: u16,
    closed: u16,
    target_mark: u16,
    parent: u8,
}

/// Blocks of `READ_BLOCK` x `READ_BLOCK` cells: the grain at which the
/// speculative negotiation (`route_batches`) tracks what a net's routing
/// read of the board and what a net's new copper changed.
const READ_BLOCK_SHIFT: usize = 3;

/// What a net's routing read of the shared board: the blocks of the
/// cells (all layers; the neighbours a search step reads lie at most a
/// block further) and the coarse tiles whose fill the corridor planner
/// read. Recorded only while `active`.
#[derive(Default)]
struct ReadSet {
    active: bool,
    blocks_x: usize,
    block_mark: Vec<u64>,
    blocks: Vec<u32>,
    tile_mark: Vec<u64>,
    tiles: Vec<u32>,
}

impl ReadSet {
    fn start(&mut self, nx: usize, ny: usize, tiles: usize) {
        self.blocks_x = nx.div_ceil(1 << READ_BLOCK_SHIFT);
        let words = (self.blocks_x * ny.div_ceil(1 << READ_BLOCK_SHIFT)).div_ceil(64);
        if self.block_mark.len() != words {
            self.block_mark = vec![0; words];
            self.blocks.clear();
        }
        if self.tile_mark.len() != tiles.div_ceil(64) {
            self.tile_mark = vec![0; tiles.div_ceil(64)];
            self.tiles.clear();
        }
        for block in self.blocks.drain(..) {
            self.block_mark[block as usize / 64] = 0;
        }
        for tile in self.tiles.drain(..) {
            self.tile_mark[tile as usize / 64] = 0;
        }
        self.active = true;
    }

    #[inline(always)]
    fn note(&mut self, x: usize, y: usize) {
        if !self.active {
            return;
        }
        let block = (y >> READ_BLOCK_SHIFT) * self.blocks_x + (x >> READ_BLOCK_SHIFT);
        let bit = 1u64 << (block % 64);
        if self.block_mark[block / 64] & bit == 0 {
            self.block_mark[block / 64] |= bit;
            self.blocks.push(block as u32);
        }
    }

    fn note_cell(&mut self, cell: u32, nx: usize) {
        self.note(cell as usize % nx, cell as usize / nx);
    }

    fn note_tile(&mut self, tile: usize) {
        if !self.active {
            return;
        }
        let bit = 1u64 << (tile % 64);
        if self.tile_mark[tile / 64] & bit == 0 {
            self.tile_mark[tile / 64] |= bit;
            self.tiles.push(tile as u32);
        }
    }

    /// Stops recording; the blocks and tiles read.
    fn finish(&mut self) -> (Vec<u32>, Vec<u32>) {
        self.active = false;
        (self.blocks.clone(), self.tiles.clone())
    }
}

/// Hashes a lattice state index by one multiplication, folded so that the
/// low bits (the table's bucket) depend on all of them.
#[derive(Clone, Copy, Default)]
struct StateHasher(u64);

impl std::hash::Hasher for StateHasher {
    fn finish(&self) -> u64 {
        self.0 ^ (self.0 >> 32)
    }

    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 = (self.0.rotate_left(8) ^ *byte as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        }
    }

    fn write_u32(&mut self, value: u32) {
        self.0 = (value as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }
}

/// Lattice states (`Router::state`) of a few nodes, with a terminal each.
type StateMap = HashMap<u32, u16, std::hash::BuildHasherDefault<StateHasher>>;

/// Per-thread search state: A* arrays, generation marks and counters.
#[derive(Default)]
pub struct Scratch {
    nodes: Vec<SearchNode>,
    /// The current search's targets, with the element (terminal or
    /// component) each belongs to. A map, not an array per state: a
    /// thread's scratch is the biggest part of a large board's footprint,
    /// and targets and trees are a few thousand nodes of millions.
    targets: StateMap,
    /// The routed net's tree, which lives across the searches of one net:
    /// its nodes, with the terminal of each (`NO_TERMINAL` for a track).
    tree: StateMap,
    own_via_near: Vec<u16>,
    /// The search's open list, kept for its capacity.
    heap: KeyHeap,
    /// What the routing of a net read of the shared board, while the
    /// negotiation speculates (`route_batches`).
    read: ReadSet,
    /// Bits per (class or neck class, map kind, cell): set for the routed
    /// net's own stamps while its search is to ignore them (see `mask_own`;
    /// a bit per entry, cleared again from `own_set`, so a thread's mask
    /// costs megabytes, not a word per cell and map).
    own_mark: Vec<u64>,
    own_set: Vec<usize>,
    /// Per (layer, tile) of the routed net's class: how many of the tile's
    /// claimed cells are the net's own (masked like `own_mark`).
    own_claimed: Vec<u32>,
    /// Whether `own_mark` holds the routed net's stamps.
    own_active: bool,
    generation: u16,
    corridor: Option<Vec<bool>>,
    expansions: u64,
    searches: u64,
    /// Searches (and their expansions) that ran without a corridor.
    open_searches: u64,
    open_expansions: u64,
    /// Searches that found no path at all.
    failed_searches: u64,
    /// Lattice nodes the pour analyses took in (`pour_analysis_units`),
    /// counted from `&self` (the analysis does not mutate the router).
    pour_units: std::sync::atomic::AtomicU64,
    /// Wall nanoseconds of those analyses (for the profile only).
    pour_nanos: std::sync::atomic::AtomicU64,
}

impl Scratch {
    fn new(states: usize, cells: usize) -> Self {
        Self {
            nodes: vec![SearchNode::default(); states],
            targets: StateMap::default(),
            tree: StateMap::default(),
            own_via_near: vec![0; cells],
            heap: KeyHeap::default(),
            read: ReadSet::default(),
            own_mark: Vec::new(),
            own_claimed: Vec::new(),
            own_set: Vec::new(),
            own_active: false,
            generation: 0,
            corridor: None,
            expansions: 0,
            searches: 0,
            open_searches: 0,
            open_expansions: 0,
            failed_searches: 0,
            pour_units: std::sync::atomic::AtomicU64::new(0),
            pour_nanos: std::sync::atomic::AtomicU64::new(0),
        }
    }

    fn counters(&self) -> [u64; 5] {
        [
            self.searches,
            self.expansions,
            self.open_searches,
            self.open_expansions,
            self.failed_searches,
        ]
    }

    /// Bytes held by the per-node arrays.
    fn bytes(&self) -> usize {
        self.nodes.capacity() * std::mem::size_of::<SearchNode>()
            + (self.targets.capacity() + self.tree.capacity()) * 7
            + self.own_via_near.capacity() * 2
            + self.own_mark.capacity() * 8
            + self.corridor.as_ref().map_or(0, |corridor| corridor.capacity())
    }

    fn fits(&self, states: usize, cells: usize) -> bool {
        self.nodes.len() == states && self.own_via_near.len() == cells
    }

    /// Allocates the arrays for a lattice of `states` (a copy has none).
    fn ensure(&mut self, states: usize, cells: usize) {
        if !self.fits(states, cells) {
            *self = Self {
                nodes: vec![SearchNode::default(); states],
                own_via_near: vec![0; cells],
                ..self.clone()
            };
        }
    }
}

impl Clone for Scratch {
    /// A copy keeps the counters only: the arrays are the transient state
    /// of one search or one net and are allocated again on first use
    /// (`ensure`). Every copy of a router (a snapshot to go back to, the
    /// layout's trial moves) carried a search scratch of the board's size.
    fn clone(&self) -> Self {
        Self {
            expansions: self.expansions,
            searches: self.searches,
            open_searches: self.open_searches,
            open_expansions: self.open_expansions,
            failed_searches: self.failed_searches,
            pour_units: std::sync::atomic::AtomicU64::new(self.pour_units.load(std::sync::atomic::Ordering::Relaxed)),
            pour_nanos: std::sync::atomic::AtomicU64::new(self.pour_nanos.load(std::sync::atomic::Ordering::Relaxed)),
            ..Self::default()
        }
    }
}

/// Per rule class: whether some search runs with it (a net's class or the
/// neck class of one). The other classes are pour brushes, read only by
/// the pour model (their static trace, fill and edge maps, their trace and
/// diagonal occupancy).
fn searched_classes(board: &Board, neck_of: &[Option<usize>]) -> Vec<bool> {
    let mut searched = vec![false; board.classes.len()];
    for net in &board.nets {
        searched[net.class] = true;
        if let Some(neck) = neck_of[net.class] {
            searched[neck] = true;
        }
    }
    searched
}

thread_local! {
    static THREAD_SCRATCH: RefCell<Option<Scratch>> = const { RefCell::new(None) };
}

/// Runs `body` with this thread's scratch, allocated once per board size.
fn with_thread_scratch<T>(states: usize, cells: usize, body: impl FnOnce(&mut Scratch) -> T) -> T {
    THREAD_SCRATCH.with(|slot| {
        let mut slot = slot.borrow_mut();
        if !slot
            .as_ref()
            .is_some_and(|scratch| scratch.fits(states, cells))
        {
            *slot = Some(Scratch::new(states, cells));
        }
        body(slot.as_mut().unwrap())
    })
}

/// Wall seconds per phase of a run and how the negotiation's batches went,
/// for the verbose `profile:` line (where a board's time goes, and how
/// much of it uses one thread).
#[derive(Clone, Default)]
struct Profile {
    prepare: f64,
    negotiation: f64,
    conflicts: f64,
    refresh_pour: f64,
    replan: f64,
    resolve: f64,
    cleanup: f64,
    via_reduction: f64,
    thermal_spokes: f64,
    stitching: f64,
    emit: f64,
    /// Batches of one net (routed on this thread) and their seconds.
    single_batches: u64,
    single_seconds: f64,
    /// Batches of several nets (routed on the thread pool): how many, the
    /// nets in them, their wall seconds and the sum of the nets' own
    /// seconds (the work a single thread would have taken).
    parallel_batches: u64,
    parallel_nets: u64,
    parallel_seconds: f64,
    parallel_net_seconds: f64,
    /// Speculative routing (`route_batches`): seconds of the parallel
    /// part, nets whose route was kept, nets routed again, and (checking)
    /// kept routes that differed from the route in turn.
    speculative_seconds: f64,
    speculation_kept: u64,
    speculation_redone: u64,
    speculation_wrong: u64,
    speculation_block_conflicts: u64,
    speculation_tile_conflicts: u64,
    /// The pour analysis (`analyze_pours`, in nanoseconds: it runs on
    /// `&self`), wherever it is called from.
    analysis_nanos: Arc<std::sync::atomic::AtomicU64>,
    analysis_calls: Arc<std::sync::atomic::AtomicU64>,
    /// Per phase (`PHASES`): the search work, the pour analyses' work
    /// units and their wall seconds.
    phase_search: [f64; PHASES.len()],
    phase_pour_units: [u64; PHASES.len()],
    phase_pour_wall: [f64; PHASES.len()],
    phase_wall: [f64; PHASES.len()],
}

static BUILD_CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static BUILD_NANOS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Routers built (`Router::new`: lattice, obstacles, pours) in this
/// process and their seconds, summed over threads.
pub fn build_totals() -> (u64, f64) {
    use std::sync::atomic::Ordering::Relaxed;
    (BUILD_CALLS.load(Relaxed), BUILD_NANOS.load(Relaxed) as f64 / 1.0e9)
}

/// The phases the `profile work:` line splits the work into.
const PHASES: [&str; 6] = ["negotiation", "resolve", "clean-up", "via reduction", "thermal spokes", "stitching"];

/// A phase's start for the profile: search work, pour units, pour wall
/// nanoseconds, wall clock.
type PhaseMark = (f64, u64, u64, std::time::Instant);

impl Profile {
    /// Per phase: wall seconds, search work, pour analyses' wall seconds
    /// and their work units (lattice nodes, millions).
    fn work_line(&self) -> String {
        let phases: Vec<String> = PHASES
            .iter()
            .enumerate()
            .map(|(index, name)| {
                format!(
                    "{name} {:.1}s wall, {:.1}s search work, pour {:.1}s wall, {:.1}s work ({:.0}M nodes)",
                    self.phase_wall[index],
                    self.phase_search[index],
                    self.phase_pour_wall[index],
                    self.phase_pour_units[index] as f64 / POUR_NODES_PER_SECOND,
                    self.phase_pour_units[index] as f64 / 1.0e6
                )
            })
            .collect();
        format!("profile work: {}", phases.join("; "))
    }

    fn line(&self, search: f64, stamp: f64) -> String {
        let analysis = self.analysis_nanos.load(std::sync::atomic::Ordering::Relaxed) as f64 / 1.0e9;
        format!(
            "profile: prepare {:.1}s, negotiation {:.1}s (search {:.1}s, stamp {:.1}s, conflicts and history {:.1}s, pour refresh {:.1}s, replan {:.1}s), resolve {:.1}s, clean-up {:.1}s, via reduction {:.1}s, thermal spokes {:.1}s, stitching {:.1}s, emit {:.1}s; pour analysis {:.1}s in {} calls; batches: {} single ({:.1}s), {} parallel with {} nets ({:.1}s wall, {:.1}s of net time); speculation {:.1}s: {} kept, {} redone, {} wrong ({} batches read changed cells, {} changed tiles)",
            self.prepare,
            self.negotiation,
            search,
            stamp,
            self.conflicts,
            self.refresh_pour,
            self.replan,
            self.resolve,
            self.cleanup,
            self.via_reduction,
            self.thermal_spokes,
            self.stitching,
            self.emit,
            analysis,
            self.analysis_calls.load(std::sync::atomic::Ordering::Relaxed),
            self.single_batches,
            self.single_seconds,
            self.parallel_batches,
            self.parallel_nets,
            self.parallel_seconds,
            self.parallel_net_seconds,
            self.speculative_seconds,
            self.speculation_kept,
            self.speculation_redone,
            self.speculation_wrong,
            self.speculation_block_conflicts,
            self.speculation_tile_conflicts,
        )
    }
}

#[derive(Clone)]
pub struct Router {
    board: Board,
    config: Config,
    grid: Grid,
    /// Shared by the copies of a router (they change only with the board,
    /// in `update`, which replaces them).
    statics: Arc<Vec<StaticMaps>>,
    /// Dynamic occupancy, `[class * (layers + 1) + layer]`; index `layers`
    /// is the via map. A value counts the nets forbidding that class there.
    /// After the class maps follow the diagonal maps (see `diagonal_map`):
    /// the cells from which a diagonal step of that class would pass a
    /// node too closely.
    occupancy: Vec<Vec<u16>>,
    /// Bits per (map, cell): set while one net is stamped, so that a cell
    /// two offsets reach counts once; cleared again from the stamped list.
    stamp_mark: Vec<Vec<u64>>,
    history: Vec<Vec<f32>>,
    /// `stamps[net class][querying class]`
    stamps: Vec<Vec<Stamps>>,
    nets: Vec<NetState>,
    direction_cost: Vec<[f32; 8]>,
    /// Cleanup routes by pure geometry: no history, no preferred axes.
    cleanup: bool,
    /// Coarse tiles of `TILE` x `TILE` nodes, per occupancy map: how many
    /// nodes are currently claimed, and (per class and layer) how many are
    /// routable at all.
    tiles_x: usize,
    tiles_y: usize,
    tile_claimed: Vec<Vec<u32>>,
    tile_routable: Vec<Vec<u32>>,
    /// The global plan (the upper layer): per net a route on the tile
    /// graph whose corridor confines the lattice search.
    global: Option<crate::global::GlobalPlan>,
    /// Per layer (last entry: vias): where conflicts kept happening, so the
    /// corridor planner learns to send some nets another way.
    tile_history: Vec<Vec<f32>>,
    /// Per layer and node: the pour net (as `owner`) whose pad needs the
    /// surrounding copper for its thermal spokes; other nets pay to pass.
    guard: Vec<Vec<u32>>,
    /// Per node: bit 0 when any layer's `covered`, bit 1 when any layer's
    /// `guard` holds a net there (most nodes: neither, and a via step
    /// skips the per-layer maps).
    via_marks: Vec<u8>,
    /// Per layer and node: the pour (as `owner`) covering the node, if any.
    covered: Vec<Vec<u32>>,
    /// Per rule class and layer (index `class * layers + layer`), per
    /// node: what a search step reads of the static map, the thermal
    /// guard, the pour cover and the history, in one record. A big board's
    /// search is bound by memory (PolyKybd's 9.4M-state lattice ran at
    /// 0.65M expansions a second against 3M on a small one); one cache
    /// line per neighbour instead of four. Kept in step with `history`.
    hot: Vec<Vec<Hot>>,
    /// Per layer and tile: share of the tile under some pour.
    tile_covered: Vec<Vec<f32>>,
    /// Per layer: cost factor for cutting a pour there. Relative to the
    /// least covered layer, so a board poured on every layer has no penalty.
    layer_cut: Vec<f32>,
    /// Per layer: an exclusive plane there (other nets' tracks may not run
    /// on its covered nodes).
    exclusive: Vec<bool>,
    /// Per rule class: the class with the neck width instead of the track
    /// width (an added class), if the track is wider.
    neck_of: Vec<Option<usize>>,
    /// Per rule class: whether a search runs with it (see
    /// `searched_classes`). The others (pour brushes) have no `hot`
    /// records and no via owners or free-layer words in their static maps.
    searched: Vec<bool>,
    /// Per layer: step cost multiplier while a plane skeleton is routed
    /// (empty otherwise).
    layer_bias: Vec<f32>,

    scratch: Scratch,
    profile: Profile,
    /// While the via reduction runs (its negotiations are its own phase).
    in_via_reduction: bool,
    search_seconds: f64,
    /// Wall time of the last full negotiation.
    negotiation_seconds: f64,
    /// The price of sharing where the last negotiation stopped.
    present_reached: f32,
    /// Negotiation gave up early: a quarter of the nets found no path at
    /// all. The polish is skipped then; the ladder's next rung is what
    /// matters.
    hopeless: bool,
    stamp_seconds: f64,
    iterations: usize,
    /// Nets in conflict after the last negotiation iteration.
    last_conflicted: Vec<NetId>,
    /// Called after every negotiation iteration with the routing as it
    /// stands (animations, diagnostics).
    frame_hook: Option<Arc<dyn Fn(usize, &RoutingResult) + Send + Sync>>,
}

impl Router {
    /// `board` with a neck variant of every class wider than the neck (for
    /// routing next to fine-pitch pads), and per class its neck class.
    fn with_neck_classes(board: &Board, config: &Config) -> (Board, Vec<Option<usize>>) {
        let mut board = board.clone();
        let mut neck_of = vec![None; board.classes.len()];
        if config.neck_reach > 0.0 {
            for class in 0..neck_of.len() {
                let rules = board.classes[class];
                if rules.trace_width > board.neck_width + 1.0e-9 {
                    let neck = RuleClass { trace_width: board.neck_width, ..rules };
                    let index = board.classes.iter().position(|existing| *existing == neck).unwrap_or_else(|| {
                        board.classes.push(neck);
                        board.classes.len() - 1
                    });
                    neck_of[class] = Some(index);
                }
            }
        }
        neck_of.resize(board.classes.len(), None);
        (board, neck_of)
    }

    pub fn new(board: &Board, config: &Config) -> Self {
        let started = std::time::Instant::now();
        let router = Self::build(board, config);
        BUILD_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        BUILD_NANOS.fetch_add(started.elapsed().as_nanos() as u64, std::sync::atomic::Ordering::Relaxed);
        router
    }

    fn build(board: &Board, config: &Config) -> Self {
        let built = std::time::Instant::now();
        let (board, neck_of) = Self::with_neck_classes(board, config);
        let board = &board;
        let grid = Grid::choose(board, &config.pitches);
        if config.verbose {
            eprintln!(
                "lattice: pitch {} mm, origin {:?}, {} x {} nodes",
                grid.pitch, grid.origin, grid.nx, grid.ny
            );
        }
        let cells = grid.cells();
        let layers = board.layer_count;
        assert!(
            board.nets.len() + 1 < HOT_HARD as usize,
            "{} nets: the search records name nets in 15 bits",
            board.nets.len()
        );
        let searched = searched_classes(board, &neck_of);
        let statics: Vec<_> = (0..board.classes.len())
            .map(|class| StaticMaps::build(board, &grid, class, searched[class]))
            .collect();
        let maps = board.classes.len() * (layers + 1) + board.classes.len() * layers * 2;
        let step = grid.pitch * std::f64::consts::SQRT_2;
        let stamps = board
            .classes
            .iter()
            .map(|own| {
                board
                    .classes
                    .iter()
                    .map(|other| {
                        let clearance = own.clearance.max(other.clearance);
                        // Exact: a node at the radius is legal, and the
                        // steps between nodes are covered apart (see
                        // `diagonal_extra`; the search checks the cells a
                        // diagonal step cuts).
                        let with_margin = |radius: f64| radius;
                        let snap = grid.pitch * 0.75 + step * step / (4.0 * grid.pitch);
                        let trace_radius = own.trace_width / 2.0 + other.trace_width / 2.0 + clearance;
                        // Copper keeps the copper clearance from a via's
                        // ring and the hole clearance from its drill;
                        // with a thin ring the hole rule is the larger
                        // (the laptop motherboard: 154 KiCad findings).
                        let hole = board.hole_clearance;
                        let via_radius = (own.trace_width / 2.0 + other.via_diameter / 2.0 + clearance)
                            .max(own.trace_width / 2.0 + other.via_drill / 2.0 + hole);
                        let own_via_radius = (own.via_diameter / 2.0 + other.trace_width / 2.0 + clearance)
                            .max(own.via_drill / 2.0 + other.trace_width / 2.0 + hole);
                        Stamps {
                            diagonal_to_trace: diagonal_extra(trace_radius, grid.pitch),
                            diagonal_to_via: diagonal_extra(via_radius, grid.pitch),
                            diagonal_block: diagonal_block(trace_radius, grid.pitch),
                            // This net's via against the querying class's
                            // diagonal steps: its ring and their trace (Sisu:
                            // 0.3 power tracks passed 0.4 vias at 0.145).
                            diagonal_block_via: diagonal_block(own_via_radius, grid.pitch),
                            stub_diagonal_block: diagonal_block(trace_radius + snap, grid.pitch),
                            stub_to_trace: disc(
                                with_margin(
                                    own.trace_width / 2.0 + other.trace_width / 2.0 + clearance,
                                ) + snap,
                                grid.pitch,
                            ),
                            stub_to_via: disc(with_margin(via_radius) + snap, grid.pitch),
                            trace_to_trace: disc(
                                with_margin(
                                    own.trace_width / 2.0 + other.trace_width / 2.0 + clearance,
                                ),
                                grid.pitch,
                            ),
                            trace_to_via: disc(with_margin(via_radius), grid.pitch),
                            via_to_trace: disc(with_margin(own_via_radius), grid.pitch),
                            via_to_via: disc(
                                (own.via_diameter / 2.0 + other.via_diameter / 2.0 + clearance)
                                    .max(own.via_drill / 2.0 + other.via_drill / 2.0 + board.hole_to_hole)
                                    .max(own.via_drill / 2.0 + other.via_diameter / 2.0 + hole)
                                    .max(own.via_diameter / 2.0 + other.via_drill / 2.0 + hole),
                                grid.pitch,
                            ),
                        }
                    })
                    .collect()
            })
            .collect();
        let direction_cost = (0..layers)
            .map(|layer| {
                let against = config.against_direction as f32;
                let (horizontal, vertical) = if layer % 2 == 0 {
                    (1.0, against)
                } else {
                    (against, 1.0)
                };
                let diagonal = (horizontal + vertical) / 2.0 * std::f32::consts::SQRT_2;
                let mut costs = [0.0; 8];
                for (direction, (dx, dy)) in DIRECTIONS.iter().enumerate() {
                    costs[direction] = match (dx != &0, dy != &0) {
                        (true, true) => diagonal,
                        (true, false) => horizontal,
                        _ => vertical,
                    } * grid.pitch as f32;
                }
                costs
            })
            .collect();
        let states = cells * layers;
        let tiles_x = grid.nx.div_ceil(TILE);
        let tiles_y = grid.ny.div_ceil(TILE);
        let mut tile_routable = vec![vec![0u32; tiles_x * tiles_y]; board.classes.len() * layers];
        for class in 0..board.classes.len() {
            for layer in 0..layers {
                let counts = &mut tile_routable[class * layers + layer];
                for (cell, value) in statics[class].trace[layer].iter().enumerate() {
                    if *value != crate::grid::BLOCKED {
                        counts[(cell / grid.nx / TILE) * tiles_x + (cell % grid.nx) / TILE] += 1;
                    }
                }
            }
        }
        let mut router = Self {
            board: board.clone(),
            config: config.clone(),
            statics: Arc::new(statics),
            occupancy: vec![vec![0; cells]; maps],
            stamp_mark: vec![vec![0; cells.div_ceil(64)]; maps],
            history: vec![vec![0.0; cells]; layers + 1],
            stamps,
            nets: Vec::new(),
            direction_cost,
            cleanup: false,
            tiles_x,
            tiles_y,
            tile_claimed: vec![vec![0; tiles_x * tiles_y]; maps],
            tile_routable,
            global: None,
            tile_history: vec![vec![0.0; tiles_x * tiles_y]; layers + 1],
            guard: vec![Vec::new(); layers],
            via_marks: Vec::new(),
            covered: vec![Vec::new(); layers],
            hot: Vec::new(),
            tile_covered: vec![vec![0.0; tiles_x * tiles_y]; layers],
            layer_cut: vec![1.0; layers],
            neck_of,
            searched,
            exclusive: (0..layers).map(|layer| config.exclusive_planes && board.planes.iter().any(|plane| plane.layer == layer && plane.exclusive)).collect(),
            layer_bias: Vec::new(),
            scratch: Scratch::new(states, cells),
            profile: Profile::default(),
            in_via_reduction: false,
            frame_hook: None,
            search_seconds: 0.0,
            negotiation_seconds: 0.0,
            present_reached: 0.0,
            hopeless: false,
            stamp_seconds: 0.0,
            iterations: 0,
            last_conflicted: Vec::new(),
            grid,
        };
        if let Some(seed) = config.seed.or_else(experiment_seed) {
            let mut state = seed;
            let amplitude = config.history_increment as f32;
            for layer in router.history.iter_mut().chain(router.tile_history.iter_mut()) {
                for value in layer.iter_mut() {
                    *value = amplitude * unit_noise(&mut state) as f32;
                }
            }
        }
        router.nets = (0..board.nets.len())
            .map(|net| router.prepare_net(net as NetId))
            .collect();
        router.guard = router.thermal_guards();
        router.mark_covered();
        router.mark_via_cells();
        router.build_hot();
        if config.verbose {
            eprintln!("{} (built in {:.1}s)", router.memory_report(), built.elapsed().as_secs_f64());
        }
        router
    }

    /// What the router's lattice maps hold, in megabytes per part, and in
    /// bytes per node (`cells`, one layer) for the whole: where a board's
    /// memory goes. The per-thread search scratches of the parallel
    /// batches come on top, one per worker thread that routed a batch.
    pub fn memory_report(&self) -> String {
        fn bytes<T>(maps: &[Vec<T>]) -> usize {
            maps.iter().map(|map| map.capacity() * std::mem::size_of::<T>()).sum()
        }
        let cells = self.grid.cells().max(1);
        let statics: usize = self.statics.iter().map(StaticMaps::bytes).sum();
        let occupancy = bytes(&self.occupancy);
        let stamp_mark = bytes(&self.stamp_mark);
        let hot = bytes(&self.hot);
        let pour = bytes(&self.history) + bytes(&self.guard) + bytes(&self.covered) + self.via_marks.capacity();
        let planes: usize = self.nets.iter().map(|state| bytes(&state.plane) + bytes(&state.plane_target)).sum();
        let scratch = self.scratch.bytes();
        let tiles = bytes(&self.tile_claimed) + bytes(&self.tile_routable) + bytes(&self.tile_history) + bytes(&self.tile_covered);
        let total = statics + occupancy + stamp_mark + hot + pour + planes + scratch + tiles;
        let mb = |value: usize| value as f64 / (1024.0 * 1024.0);
        format!(
            "memory: {:.0} MB in maps ({:.0} bytes a node; {} x {} nodes, {} layers, {} classes, {} searched, {} track geometries): statics {:.0}, occupancy {:.0}, stamp marks {:.0}, hot {:.0}, history/guard/cover {:.0}, pour masks {:.0}, scratch {:.0} (+{:.0} per worker thread), tiles {:.0}",
            mb(total),
            total as f64 / cells as f64,
            self.grid.nx,
            self.grid.ny,
            self.board.layer_count,
            self.board.classes.len(),
            self.searched.iter().filter(|searched| **searched).count(),
            {
                let mut geometries: Vec<(u64, u64)> = (0..self.board.classes.len())
                    .filter(|class| self.searched[*class])
                    .map(|class| (self.board.classes[class].trace_width.to_bits(), self.board.classes[class].clearance.to_bits()))
                    .collect();
                geometries.sort();
                geometries.dedup();
                geometries.len()
            },
            mb(statics),
            mb(occupancy),
            mb(stamp_mark),
            mb(hot),
            mb(pour),
            mb(planes),
            mb(scratch),
            mb(self.scratch.bytes()),
            mb(tiles),
        )
    }

    /// Packs the static map, the guard, the pour cover and the history
    /// per class and layer (see `hot`); after any of them changed.
    fn build_hot(&mut self) {
        let layers = self.board.layer_count;
        let cells = self.grid.cells();
        self.hot = (0..self.statics.len() * layers)
            .map(|index| {
                let (class, layer) = (index / layers, index % layers);
                if !self.searched[class] {
                    return Vec::new();
                }
                let trace = &self.statics[class].trace[layer];
                let guard = &self.guard[layer];
                let covered = &self.covered[layer];
                let history = &self.history[layer];
                (0..cells)
                    .map(|cell| Hot {
                        trace: trace[cell],
                        guard: match guard.get(cell) {
                            Some(&value) if value != 0 => {
                                (value & !HARD_GUARD) as u16 | if value & HARD_GUARD != 0 { HOT_HARD } else { 0 }
                            }
                            _ => 0,
                        },
                        covered: covered.get(cell).map_or(0, |&value| value as u16),
                        history: history[cell],
                    })
                    .collect()
            })
            .collect();
    }

    pub fn grid(&self) -> &Grid {
        &self.grid
    }

    fn state(&self, node: Node) -> usize {
        node.layer as usize * self.grid.cells() + node.cell as usize
    }

    #[inline(always)]
    fn map_index(&self, class: usize, layer: usize) -> usize {
        class * (self.board.layer_count + 1) + layer
    }

    /// The map counting the nets whose copper a diagonal step of `class`
    /// on `layer` starting at a cell would pass too closely (`orientation`
    /// 0: the step towards +x +y, 1: towards +x -y; a step towards -x is
    /// the same step from its other end). A diagonal step passes the nodes
    /// on its bisector closer than its ends do; those nodes stamp here.
    #[inline(always)]
    fn diagonal_map(&self, class: usize, layer: usize, orientation: usize) -> usize {
        let layers = self.board.layer_count;
        self.board.classes.len() * (layers + 1) + (class * layers + layer) * 2 + orientation
    }

    /// Access for a pad too narrow to hold a lattice node: nearby legal
    /// nodes that a straight stub from the pad centre reaches without coming
    /// too close to any fixed copper of another net.
    fn escape_nodes(
        &self,
        net: NetId,
        terminal: &crate::board::Terminal,
        nodes: &mut Vec<Node>,
        escapes: &mut HashMap<Node, (Vec<u32>, f64, Vec<crate::geometry::Point>)>,
    ) {
        const REACH: f64 = 2.0;
        const WANTED: usize = 12;
        let class = self.board.classes[self.board.nets[net as usize].class];
        let statics = &self.statics[self.board.nets[net as usize].class];
        let widths = if self.board.neck_width < class.trace_width {
            vec![class.trace_width, self.board.neck_width]
        } else {
            vec![class.trace_width]
        };
        let bounds = crate::geometry::Aabb {
            minimum: terminal.anchor,
            maximum: terminal.anchor,
        }
        .inflated(REACH);
        let Some((x0, y0, x1, y1)) = self.grid.node_range(bounds) else {
            return;
        };
        for layer in 0..self.board.layer_count {
            if terminal.layers & (1 << layer) == 0 {
                continue;
            }
            let mut candidates: Vec<(f64, usize)> = Vec::new();
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let cell = self.grid.index(x, y);
                    let distance =
                        crate::geometry::distance(terminal.anchor, self.grid.center(x, y));
                    if distance <= REACH && statics.trace_allowed(layer, cell, net) {
                        candidates.push((distance, cell));
                    }
                }
            }
            candidates.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            let mut accepted = 0;
            // Exit points along the pad's long axis, for bent stubs.
            let pad = self.board.obstacles[terminal.pad].shape.aabb();
            let extent = [
                pad.maximum[0] - pad.minimum[0],
                pad.maximum[1] - pad.minimum[1],
            ];
            let axis = if extent[0] >= extent[1] {
                [1.0, 0.0]
            } else {
                [0.0, 1.0]
            };
            let half_length = extent[0].max(extent[1]) / 2.0;
            let exits: Vec<crate::geometry::Point> = [1.0, -1.0]
                .iter()
                .flat_map(|sign| {
                    (0..8).map(move |step| {
                        let d = half_length + 0.05 + step as f64 * 0.1;
                        [
                            terminal.anchor[0] + sign * axis[0] * d,
                            terminal.anchor[1] + sign * axis[1] * d,
                        ]
                    })
                })
                .collect();
            // A pad straddling the board edge (connector shell, card-edge
            // finger) cannot host copper at its centre: the stub then starts
            // at the nearest point along the pad's long axis that keeps the
            // edge clearance.
            let narrowest = *widths.last().unwrap();
            let mut starts = vec![terminal.anchor];
            if !self.stub_keeps_edge(terminal.anchor, terminal.anchor, narrowest) {
                let shape = &self.board.obstacles[terminal.pad].shape;
                let mut interior: Vec<crate::geometry::Point> = Vec::new();
                for step in 1.. {
                    let d = step as f64 * 0.1;
                    if d > half_length {
                        break;
                    }
                    for sign in [1.0, -1.0] {
                        let point = [
                            terminal.anchor[0] + sign * axis[0] * d,
                            terminal.anchor[1] + sign * axis[1] * d,
                        ];
                        if well_inside(shape, point)
                            && self.stub_keeps_edge(point, point, narrowest)
                        {
                            interior.push(point);
                        }
                    }
                }
                starts = interior;
                if std::env::var_os("PCB_ROUTER_DEBUG").is_some() {
                    eprintln!(
                        "edge escape for {} at {:?}: {} interior starts",
                        terminal.label,
                        terminal.anchor,
                        starts.len()
                    );
                }
            }
            let candidate_count = candidates.len();
            if std::env::var_os("PCB_ROUTER_DEBUG").is_some() && candidates.len() > 0 {
                // What blocks the narrowest stub to the nearest candidates.
                for (distance, cell) in candidates.iter().take(3) {
                    let center = self.grid.center_of(*cell);
                    let blockers: Vec<String> = self
                        .board
                        .obstacles
                        .iter()
                        .filter(|obstacle| {
                            obstacle.net != Some(net) && obstacle.layers & (1 << layer) != 0 && obstacle.blocks_tracks && {
                                let required = narrowest / 2.0
                                    + SAFETY
                                    + match obstacle.kind {
                                        crate::board::ObstacleKind::Copper => self.board.copper_clearance(&class, obstacle),
                                        crate::board::ObstacleKind::Keepout => obstacle.clearance,
                                        crate::board::ObstacleKind::Hole => self.board.hole_clearance.max(obstacle.clearance),
                                    };
                                obstacle.shape.distance_to_segment(terminal.anchor, center) < required
                            }
                        })
                        .map(|obstacle| format!("{} ({:.3})", obstacle.label, obstacle.shape.distance_to_segment(terminal.anchor, center)))
                        .collect();
                    eprintln!("  escape {} candidate at {:?} ({distance:.3} mm): stub of {narrowest} blocked by {blockers:?}", terminal.label, center);
                }
            }
            for (_, cell) in candidates.into_iter().take(600) {
                let center = self.grid.center_of(cell);
                let mut found: Option<(f64, Vec<crate::geometry::Point>)> = None;
                'widths: for width in widths.iter().copied() {
                    for start in &starts {
                        if self.stub_is_clear(net, layer, *start, center, width)
                            && self.stub_keeps_edge(*start, center, width)
                        {
                            found = Some((width, vec![*start, center]));
                            break 'widths;
                        }
                    }
                    for exit in &exits {
                        if crate::geometry::distance(*exit, center) > 1.5 * self.grid.pitch + 0.15 {
                            continue;
                        }
                        if self.stub_is_clear(net, layer, terminal.anchor, *exit, width)
                            && self.stub_keeps_edge(terminal.anchor, *exit, width)
                            && self.stub_is_clear(net, layer, *exit, center, width)
                            && self.stub_keeps_edge(*exit, center, width)
                        {
                            found = Some((width, vec![terminal.anchor, *exit, center]));
                            break 'widths;
                        }
                    }
                }
                let Some((width, polyline)) = found else {
                    continue;
                };
                let mut cells: Vec<u32> = Vec::new();
                for pair in polyline.windows(2) {
                    let length = crate::geometry::distance(pair[0], pair[1]);
                    let samples = (length / (self.grid.pitch / 2.0)).ceil().max(1.0) as usize;
                    for index in 0..=samples {
                        let t = index as f64 / samples as f64;
                        if let Some(cell) = self.grid.nearest_node([
                            pair[0][0] + t * (pair[1][0] - pair[0][0]),
                            pair[0][1] + t * (pair[1][1] - pair[0][1]),
                        ]) {
                            cells.push(cell as u32);
                        }
                    }
                }
                cells.dedup();
                let node = Node {
                    layer: layer as u8,
                    cell: cell as u32,
                };
                // One escape per node: two pads of a net side by side (a
                // QFN's two VIN pins) reaching the same node would share
                // it, and only the later stub would be kept; a route
                // through it then joined both pads with one pad's copper
                // missing (Sisu's U9 pad 11).
                if escapes.contains_key(&node) {
                    continue;
                }
                escapes.insert(node, (cells, width, polyline));
                nodes.push(node);
                accepted += 1;
                if accepted == WANTED {
                    break;
                }
            }
            if accepted == 0 && std::env::var_os("PCB_ROUTER_DEBUG").is_some() {
                eprintln!(
                    "no escape for {} at {:?} layer {layer}: {candidate_count} candidates, {} stub starts",
                    terminal.label,
                    terminal.anchor,
                    starts.len()
                );
            }
        }
    }

    /// Whether the stub behind escape node `node` is free of other nets.
    fn escape_is_free(&self, scratch: &Scratch, net_state: &NetState, net: NetId, node: Node) -> bool {
        let Some((cells, _, _)) = net_state.escapes.get(&node) else {
            return true;
        };
        let class = self.board.nets[net as usize].class;
        let map = self.map_index(class, node.layer as usize);
        let nx = self.grid.nx as i64;
        cells.iter().all(|cell| {
            (-1..=1).all(|dy| {
                (-1..=1).all(|dx| {
                    let target = *cell as i64 + dy * nx + dx;
                    target < 0
                        || target as usize >= self.grid.cells()
                        || self.occupancy_seen(scratch, class, map, target as usize) == 0
                })
            })
        })
    }

    /// Sets `via_marks` from `covered` and `guard`.
    fn mark_via_cells(&mut self) {
        let mut marks = vec![0u8; self.grid.cells()];
        for (bit, maps) in [(1u8, &self.covered), (2u8, &self.guard)] {
            for map in maps.iter().filter(|map| !map.is_empty()) {
                for (mark, value) in marks.iter_mut().zip(map) {
                    if *value != 0 {
                        *mark |= bit;
                    }
                }
            }
        }
        self.via_marks = marks;
    }

    /// Marks the nodes and tiles under pours, per layer.
    fn mark_covered(&mut self) {
        let tiles = self.tiles_x * self.tiles_y;
        for plane in &self.board.planes {
            let layer = plane.layer;
            if self.covered[layer].is_empty() {
                self.covered[layer] = vec![0; self.grid.cells()];
            }
            let mask = &self.nets[plane.net as usize].plane;
            if mask.is_empty() || mask[layer].is_empty() {
                continue;
            }
            let mut per_tile = vec![0u32; tiles];
            for (cell, inside) in mask[layer].iter().enumerate() {
                if *inside {
                    self.covered[layer][cell] = crate::grid::owner(plane.net);
                    per_tile[(cell / self.grid.nx / TILE) * self.tiles_x
                        + (cell % self.grid.nx) / TILE] += 1;
                }
            }
            for (tile, count) in per_tile.iter().enumerate() {
                let share = *count as f32 / (TILE * TILE) as f32;
                self.tile_covered[layer][tile] = self.tile_covered[layer][tile].max(share.min(1.0));
            }
        }
        let share: Vec<f32> = (0..self.board.layer_count)
            .map(|layer| {
                if self.covered[layer].is_empty() {
                    0.0
                } else {
                    self.covered[layer]
                        .iter()
                        .filter(|owner| **owner != 0)
                        .count() as f32
                        / self.grid.cells() as f32
                }
            })
            .collect();
        let least = share.iter().copied().fold(1.0f32, f32::min);
        for layer in 0..self.board.layer_count {
            let relative = (share[layer] - least).max(0.0) / (1.0 - least).max(1.0e-3);
            self.layer_cut[layer] = 1.0 + (self.config.plane_cut_cost as f32 - 1.0) * relative;
        }
    }

    /// Per pad connecting to a pour of its net: the layer, the net and the
    /// nodes of each of its four spokes' corridors, straight out from the
    /// pad along its axes (diagonally for a round pad, as KiCad draws them),
    /// across the gap and the spoke's length.
    /// Each corridor lists the spoke's own strip (the bridge's width) first;
    /// the fourth element says how many cells that is.
    fn spoke_corridors(&self) -> Vec<(usize, NetId, [Vec<usize>; 4], [usize; 4])> {
        let mut corridors = Vec::new();
        for plane in &self.board.planes {
            if plane.thermal_reach <= 0.0 {
                continue;
            }
            for terminal in &self.board.nets[plane.net as usize].terminals {
                if terminal.layers & (1 << plane.layer) == 0
                    || !crate::geometry::point_in_polygon(terminal.anchor, &plane.polygon)
                {
                    continue;
                }
                let shape = &self.board.obstacles[terminal.pad].shape;
                let bounds = shape.aabb();
                // The guards keep the corridors along the pad's box: in
                // the pad's own frame (`spoke_frames`) they cost Hub's
                // route 6 connections (29 to 35 open; turned parts guarded
                // diagonally), for no fewer starved pads.
                let (_, anchor, geometry, _) = self.spoke_geometry_in(terminal.pad, terminal.anchor, 0.0, false);
                let bridge = (plane.thermal_reach - plane.thermal_gap).max(0.0);
                let width = bridge / 2.0 + self.board.classes[self.board.nets[plane.net as usize].class].clearance + 0.1;
                let Some((x0, y0, x1, y1)) = self.grid.node_range(bounds.inflated(plane.thermal_reach + width)) else {
                    continue;
                };
                let mut cores: [Vec<usize>; 4] = Default::default();
                let mut margins: [Vec<usize>; 4] = Default::default();
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        let center = self.grid.center(x, y);
                        if shape.distance_to_point(center) >= plane.thermal_reach {
                            continue;
                        }
                        let offset = [center[0] - anchor[0], center[1] - anchor[1]];
                        for (index, (direction, edge, _)) in geometry.iter().enumerate() {
                            let edge = *edge;
                            let along = offset[0] * direction[0] + offset[1] * direction[1];
                            let across = (offset[0] * direction[1] - offset[1] * direction[0]).abs();
                            if along > edge - 1.0e-9 && along < edge + plane.thermal_reach && across < width {
                                if across <= bridge / 2.0 + self.grid.pitch / 2.0 {
                                    cores[index].push(self.grid.index(x, y));
                                } else {
                                    margins[index].push(self.grid.index(x, y));
                                }
                            }
                        }
                    }
                }
                let core_counts = [cores[0].len(), cores[1].len(), cores[2].len(), cores[3].len()];
                let mut cells: [Vec<usize>; 4] = Default::default();
                for index in 0..4 {
                    cells[index] = std::mem::take(&mut cores[index]);
                    cells[index].append(&mut margins[index]);
                }
                corridors.push((plane.layer, plane.net, cells, core_counts));
            }
        }
        corridors
    }

    /// Nodes around pads that connect to a pour of their net on that layer:
    /// their spokes' corridors (other nets may pass the pad's corners, not
    /// its spokes: Sisu's GND resistors starved at one spoke of two).
    fn thermal_guards(&self) -> Vec<Vec<u32>> {
        let mut guard = vec![Vec::new(); self.board.layer_count];
        for (layer, net, cells, cores) in self.spoke_corridors() {
            if guard[layer].is_empty() {
                guard[layer] = vec![0; self.grid.cells()];
            }
            // Corridors the fixed copper (other parts' pads) already
            // closes do not count; with two or fewer open the pad has
            // none to spare, and those are closed to other nets. Open
            // means the spoke's own strip has room for fill (the margin
            // beside it lies in the neighbours' clearance on a fine-pitch
            // row, and is no measure).
            let statics = &self.statics[self.board.nets[net as usize].class];
            let open = |corridor: &Vec<usize>, core: usize| {
                core > 0 && corridor[..core].iter().filter(|cell| statics.fill_allowed(layer, **cell, net)).count() * 2 >= core
            };
            let spare = cells.iter().zip(cores).filter(|(corridor, core)| open(corridor, *core)).count() > 2;
            let hard = self.config.hard_thermal_guards && !spare;
            for (corridor, core) in cells.iter().zip(cores) {
                let mark = if hard && open(corridor, core) {
                    crate::grid::owner(net) | HARD_GUARD
                } else {
                    crate::grid::owner(net)
                };
                for cell in corridor {
                    guard[layer][*cell] = mark;
                }
            }
        }
        // Planes without thermal reach still get their (empty) layer map.
        for plane in &self.board.planes {
            if plane.thermal_reach > 0.0 && guard[plane.layer].is_empty() {
                guard[plane.layer] = vec![0; self.grid.cells()];
            }
        }
        guard
    }

    /// Pads joined to a pour of their net by thermal spokes that have
    /// fewer than two of their spoke corridors free of other nets' copper
    /// (as `free_thermal_spokes` counts them): what KiCad reports as
    /// starved thermals.
    fn starved_pads(&self) -> usize {
        let corridors = self.spoke_corridors();
        if corridors.is_empty() {
            return 0;
        }
        let layers = self.board.layer_count;
        let mut occupants: HashMap<(usize, usize), Vec<NetId>> = HashMap::new();
        for (net, state) in self.nets.iter().enumerate() {
            for branch in &state.branches {
                for (index, node) in branch.nodes.iter().enumerate() {
                    let via = branch.nodes.get(index + 1).is_some_and(|next| next.cell == node.cell && next.layer != node.layer);
                    let on: Vec<usize> = if via { (0..layers).collect() } else { vec![node.layer as usize] };
                    for layer in on {
                        let entry = occupants.entry((layer, node.cell as usize)).or_default();
                        if !entry.contains(&(net as NetId)) {
                            entry.push(net as NetId);
                        }
                    }
                }
            }
        }
        corridors
            .iter()
            .filter(|(layer, net, lists, _)| {
                let blocked = lists
                    .iter()
                    .filter(|list| {
                        list.iter().any(|cell| {
                            occupants.get(&(*layer, *cell)).is_some_and(|nets| nets.iter().any(|other| other != net))
                        })
                    })
                    .count();
                lists.iter().filter(|list| !list.is_empty()).count() - blocked < 2
            })
            .count()
    }

    /// Pads left with fewer than two of their four spoke corridors free of
    /// other nets' copper are starved thermals to KiCad (Sisu: Y1 and R70
    /// at one spoke of two). The nets in their corridors are routed again,
    /// the guard made all but impassable; a new route is kept when it is
    /// complete and crosses fewer corridor nodes. Returns the nets moved.
    fn free_thermal_spokes(&mut self) -> usize {
        let corridors = self.spoke_corridors();
        if corridors.is_empty() {
            return 0;
        }
        let layers = self.board.layer_count;
        // Which nets' copper lies on each node (a via on every layer).
        let occupants = |router: &Self| -> HashMap<(usize, usize), Vec<NetId>> {
            let mut map: HashMap<(usize, usize), Vec<NetId>> = HashMap::new();
            for (net, state) in router.nets.iter().enumerate() {
                for branch in &state.branches {
                    for (index, node) in branch.nodes.iter().enumerate() {
                        let via = branch.nodes.get(index + 1).is_some_and(|next| next.cell == node.cell && next.layer != node.layer);
                        let on: Vec<usize> = if via { (0..layers).collect() } else { vec![node.layer as usize] };
                        for layer in on {
                            let entry = map.entry((layer, node.cell as usize)).or_default();
                            if !entry.contains(&(net as NetId)) {
                                entry.push(net as NetId);
                            }
                        }
                    }
                }
            }
            map
        };
        let starved = |router: &Self| -> (Vec<NetId>, Vec<(usize, usize)>) {
            let map = occupants(router);
            let mut offenders = Vec::new();
            let mut nodes = Vec::new();
            for (layer, net, lists, _) in &corridors {
                let blocked: Vec<&Vec<usize>> = lists
                    .iter()
                    .filter(|list| list.iter().any(|cell| map.get(&(*layer, *cell)).is_some_and(|nets| nets.iter().any(|other| other != net))))
                    .collect();
                if lists.iter().filter(|list| !list.is_empty()).count() - blocked.len() >= 2 {
                    continue;
                }
                for list in blocked {
                    for cell in list {
                        nodes.push((*layer, *cell));
                        for other in map.get(&(*layer, *cell)).into_iter().flatten() {
                            if other != net && !offenders.contains(other) {
                                offenders.push(*other);
                            }
                        }
                    }
                }
            }
            (offenders, nodes)
        };
        let (offenders, nodes) = starved(self);
        if offenders.is_empty() {
            return 0;
        }
        let corridor_nodes: std::collections::HashSet<(usize, usize)> = nodes.into_iter().collect();
        let crossings = |router: &Self, net: NetId| -> usize {
            router.nets[net as usize]
                .branches
                .iter()
                .flat_map(|branch| &branch.nodes)
                .filter(|node| corridor_nodes.contains(&(node.layer as usize, node.cell as usize)))
                .count()
        };
        let guard_cost = self.config.thermal_guard_cost;
        self.config.thermal_guard_cost = 1.0e4;
        self.cleanup = true;
        let mut moved = 0;
        // Budgeted like the clean-up: a hard reroute per offending net on a
        // big lattice adds up (PolyKybd right: 435 s and 369 s in one
        // layout, and the harness killed the board).
        let clock = self.clock();
        let budget = self.config.cleanup_seconds.min(self.negotiation_seconds.max(60.0));
        let mut skipped = 0;
        for net in offenders {
            if self.spent(&clock, budget) {
                skipped += 1;
                continue;
            }
            let state = &self.nets[net as usize];
            if !state.routable || !state.complete || !state.plane.is_empty() {
                continue;
            }
            let before = crossings(self, net);
            let saved = state.branches.clone();
            self.rip_up(net);
            self.route_net_seq(net, 0.0, true, 1.0);
            if self.nets[net as usize].complete && crossings(self, net) < before {
                moved += 1;
            } else {
                let state = &mut self.nets[net as usize];
                state.branches = saved;
                state.connected.fill(true);
                state.complete = true;
            }
            self.stamp(net);
        }
        self.cleanup = false;
        self.config.thermal_guard_cost = guard_cost;
        if self.config.verbose {
            eprintln!("thermal spokes: {moved} nets moved out of starved pads' spoke corridors ({skipped} left by the budget)");
        }
        moved
    }

    /// The legal lattice nodes inside each terminal's pad copper.
    fn prepare_net(&self, net: NetId) -> NetState {
        let description = &self.board.nets[net as usize];
        let statics = &self.statics[description.class];
        let mut terminal_nodes = Vec::new();
        let mut terminal_reach = Vec::new();
        let mut escapes: HashMap<Node, (Vec<u32>, f64, Vec<crate::geometry::Point>)> =
            HashMap::new();
        for terminal in &description.terminals {
            let pad = &self.board.obstacles[terminal.pad];
            let contact = terminal.contact.as_ref().unwrap_or(&pad.shape);
            let mut nodes = Vec::new();
            if let Some((x0, y0, x1, y1)) = self.grid.node_range(contact.aabb()) {
                for layer in 0..self.board.layer_count {
                    if terminal.layers & (1 << layer) == 0 {
                        continue;
                    }
                    for y in y0..=y1 {
                        for x in x0..=x1 {
                            let cell = self.grid.index(x, y);
                            if statics.trace_allowed(layer, cell, net)
                                && well_inside(contact, self.grid.center(x, y))
                            {
                                nodes.push(Node {
                                    layer: layer as u8,
                                    cell: cell as u32,
                                });
                            }
                        }
                    }
                }
            }
            if nodes.is_empty() {
                self.escape_nodes(net, terminal, &mut nodes, &mut escapes);
            }
            let reach = nodes
                .iter()
                .map(|node| {
                    crate::geometry::distance(
                        terminal.anchor,
                        self.grid.center_of(node.cell as usize),
                    ) as f32
                })
                .fold(0.0, f32::max);
            terminal_reach.push(reach);
            terminal_nodes.push(nodes);
        }
        // Neck zones around pads narrower than the net's track plus its
        // clearance (a fine-pitch part's), where the net routes at the neck
        // width.
        let mut neck_zones = Vec::new();
        if self.neck_of[description.class].is_some() {
            let rules = self.board.classes[description.class];
            for terminal in &description.terminals {
                let bounds = self.board.obstacles[terminal.pad].shape.aabb();
                let narrow = (bounds.maximum[0] - bounds.minimum[0]).min(bounds.maximum[1] - bounds.minimum[1]);
                if narrow < rules.trace_width + rules.clearance {
                    let reach = crate::geometry::Aabb { minimum: terminal.anchor, maximum: terminal.anchor }.inflated(self.config.neck_reach);
                    if let Some(range) = self.grid.node_range(reach) {
                        neck_zones.push(range);
                    }
                }
            }
        }
        let (plane, plane_class) = self.pour_masks(net, true);
        let on_plane: Vec<bool> = terminal_nodes
            .iter()
            .enumerate()
            .map(|(index, nodes)| {
                // A pad set to no zone connection never joins a pour.
                self.board.isolated_pads.binary_search(&description.terminals[index].label).is_err()
                    && nodes.iter().any(|node| {
                    plane
                        .get(node.layer as usize)
                        .is_some_and(|mask| !mask.is_empty() && mask[node.cell as usize])
                })
            })
            .collect();
        let has_plane = !plane.is_empty();
        if std::env::var_os("PCB_ROUTER_DEBUG").is_some() {
            for (index, nodes) in terminal_nodes.iter().enumerate() {
                if nodes.is_empty() && !on_plane[index] {
                    let terminal = &description.terminals[index];
                    let pad = &self.board.obstacles[terminal.pad];
                    let (mut inside, mut allowed) = (0, 0);
                    if let Some((x0, y0, x1, y1)) = self.grid.node_range(pad.shape.aabb()) {
                        for layer in 0..self.board.layer_count {
                            if terminal.layers & (1 << layer) == 0 {
                                continue;
                            }
                            for y in y0..=y1 {
                                for x in x0..=x1 {
                                    let cell = self.grid.index(x, y);
                                    inside += well_inside(&pad.shape, self.grid.center(x, y)) as usize;
                                    allowed += statics.trace_allowed(layer, cell, net) as usize;
                                }
                            }
                        }
                    }
                    let near = pad.shape.aabb().inflated(1.0);
                    for other in &self.board.obstacles {
                        if other.net != Some(net) && other.layers & terminal.layers != 0 && other.shape.aabb().intersects(near) {
                            eprintln!("  blocked by {:?} net {:?} {:?}", other.kind, other.net, other.shape.aabb());
                        }
                    }
                    eprintln!(
                        "dead pad {} of {} at {:?} layers {:#b}: shape {:?}, {inside} nodes well inside, {allowed} allowed",
                        terminal.label,
                        description.name,
                        terminal.anchor,
                        terminal.layers,
                        pad.shape.aabb(),
                    );
                }
            }
        }
        // Pads without any access are reported; the rest is still routed.
        let live = terminal_nodes
            .iter()
            .zip(&on_plane)
            .filter(|(nodes, on_plane)| **on_plane || !nodes.is_empty())
            .count();
        let routable = live >= 2 || (has_plane && live >= 1);
        NetState {
            escapes,
            plane,
            plane_class,
            on_plane,
            connected: vec![false; description.terminals.len()],
            terminal_nodes,
            terminal_reach,
            routable,
            neck_zones,
            ..Default::default()
        }
    }

    /// Per layer, the nodes where a pour of `net` may fill (its polygon
    /// less higher-priority pours, where its brush's static map lets it;
    /// empty for a layer without one; no layers without any pour), and
    /// the brush class per layer. `connected_only`: only the pours pads
    /// connect through (the routing's); else every pour of the net (what
    /// KiCad fills on a board whose pour nets are routed as tracks).
    fn pour_masks(&self, net: NetId, connected_only: bool) -> (Vec<Vec<bool>>, Vec<usize>) {
        let description = &self.board.nets[net as usize];
        let mut plane: Vec<Vec<bool>> = Vec::new();
        let mut plane_class = vec![description.class; self.board.layer_count];
        for pour in self
            .board
            .planes
            .iter()
            .filter(|pour| pour.net == net && (pour.connect || !connected_only))
        {
            if plane.is_empty() {
                plane = vec![Vec::new(); self.board.layer_count];
            }
            plane_class[pour.layer] = pour.class;
            let statics = &self.statics[pour.class];
            if plane[pour.layer].is_empty() {
                plane[pour.layer] = vec![false; self.grid.cells()];
            }
            let Some((x0, y0, x1, y1)) = self.grid.node_range(
                crate::geometry::Shape::Polygon {
                    points: pour.polygon.clone(),
                }
                .aabb(),
            ) else {
                continue;
            };
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let cell = self.grid.index(x, y);
                    let center = self.grid.center(x, y);
                    if statics.fill_allowed(pour.layer, cell, net)
                        && crate::geometry::point_in_polygon(center, &pour.polygon)
                        && !pour
                            .excluded
                            .iter()
                            .any(|other| crate::geometry::point_in_polygon(center, other))
                    {
                        plane[pour.layer][cell] = true;
                    }
                }
            }
        }
        (plane, plane_class)
    }

    /// The rule class `net` routes with at `cell`: its neck class inside
    /// its neck zones, else its own.
    fn class_at(&self, net: NetId, state: &NetState, cell: u32) -> usize {
        let class = self.board.nets[net as usize].class;
        match self.neck_of[class] {
            Some(neck) if !state.neck_zones.is_empty() => {
                let (x, y) = self.grid.xy(cell as usize);
                if state.neck_zones.iter().any(|&(x0, y0, x1, y1)| x >= x0 && x <= x1 && y >= y0 && y <= y1) { neck } else { class }
            }
            _ => class,
        }
    }

    fn node_class(&self, net: NetId, cell: u32) -> usize {
        self.class_at(net, &self.nets[net as usize], cell)
    }

    /// Track width of `net` at `node`.
    fn width_at(&self, net: NetId, cell: u32) -> f64 {
        self.board.classes[self.node_class(net, cell)].trace_width
    }

    fn unstamp(&mut self, net: NetId) {
        let state = &mut self.nets[net as usize];
        for (map, cell) in state.stamped.drain(..) {
            self.occupancy[map as usize][cell as usize] -= 1;
            if self.occupancy[map as usize][cell as usize] == 0 {
                let cell = cell as usize;
                let tile =
                    (cell / self.grid.nx / TILE) * self.tiles_x + (cell % self.grid.nx) / TILE;
                self.tile_claimed[map as usize][tile] -= 1;
            }
        }
    }

    /// Removes the net's copper but its fixed branches, which stay stamped:
    /// a stub or skeleton must hold its ground while other nets are routed
    /// against it, whether or not its net is ripped up at the time.
    fn rip_up(&mut self, net: NetId) {
        let state = &mut self.nets[net as usize];
        let fixed = state.fixed;
        state.branches.truncate(fixed);
        state.connected.fill(false);
        state.complete = false;
        self.stamp(net);
    }

    /// Whether a rip-up of `net` takes every branch: every `STUCK_WHOLE`th
    /// iteration of a net stuck in conflict. The branches a partial rip-up
    /// keeps may be what leaves the other net no way through.
    fn rip_up_whole(&self, net: NetId) -> bool {
        const STUCK_WHOLE: usize = 8;
        let stuck = self.nets[net as usize].stuck;
        // A pour net ripped up whole loses the vias that join its layers'
        // fill: the next refresh finds its pads scattered over a dozen
        // pieces, cuts them off from the main piece and routes them as
        // tracks (Sisu's GND every eighth iteration: 217 pads in the main
        // piece, then 35, then 261 again, about 100 pads cut off each time).
        if !self.nets[net as usize].plane.is_empty() && !self.config.pour_whole_rip_up {
            return false;
        }
        stuck > 0 && stuck % STUCK_WHOLE == 0
    }

    /// Removes only the branches of `net` that are in conflict, plus any
    /// branch left dangling at a junction on a removed branch.
    fn rip_up_conflicted(&mut self, net: NetId) {
        let keep = self.unconflicted_branches(net);
        let mut keep = keep.into_iter();
        let state = &mut self.nets[net as usize];
        state.branches.retain(|_| keep.next().unwrap());
        state.connected.fill(false);
        state.complete = false;
        // Kept branches stay stamped and are masked in the net's own search.
        self.stamp(net);
    }

    /// Which branches of `net` are free of conflict and not left dangling
    /// at a junction on a conflicted branch (the skeleton always is).
    fn unconflicted_branches(&self, net: NetId) -> Vec<bool> {
        let branches = &self.nets[net as usize].branches;
        let whole = self.rip_up_whole(net);
        let mut found = Vec::new();
        let mut keep: Vec<bool> = branches
            .iter()
            .map(|branch| {
                found.clear();
                self.branch_conflicts(net, branch, &mut found);
                found.is_empty() && !whole
            })
            .collect();
        for flag in keep.iter_mut().take(self.nets[net as usize].fixed) {
            *flag = true;
        }
        loop {
            let hosts = junction_hosts(branches, &keep);
            let mut changed = false;
            for (index, branch) in branches.iter().enumerate() {
                if !keep[index] {
                    continue;
                }
                let dangling = (branch.start_terminal == NO_TERMINAL
                    && !hosts.contains_key(&branch.nodes[0]))
                    || (branch.end_terminal == NO_TERMINAL
                        && !hosts.contains_key(branch.nodes.last().unwrap()));
                if dangling {
                    keep[index] = false;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        keep
    }

    fn stamp(&mut self, net: NetId) {
        // A full restamp: whatever was stamped before goes first.
        self.unstamp(net);
        let class = self.board.nets[net as usize].class;
        let layers = self.board.layer_count;
        let mut stamped = Vec::new();
        let nx = self.grid.nx as i64;
        let ny = self.grid.ny as i64;
        let branches = std::mem::take(&mut self.nets[net as usize].branches);
        let escapes = std::mem::take(&mut self.nets[net as usize].escapes);
        let node_classes: Vec<Vec<usize>> = branches
            .iter()
            .map(|branch| branch.nodes.iter().map(|node| self.node_class(net, node.cell)).collect())
            .collect();
        let diagonal_base = self.board.classes.len() * (layers + 1);
        for querying in 0..self.board.classes.len() {
            let stamps = &self.stamps[class][querying];
            let mut apply = |map: usize, cell: u32, offsets: &[(i32, i32)]| {
                let x = cell as i64 % nx;
                let y = cell as i64 / nx;
                for (dx, dy) in offsets {
                    let tx = x + *dx as i64;
                    let ty = y + *dy as i64;
                    if tx < 0 || ty < 0 || tx >= nx || ty >= ny {
                        continue;
                    }
                    let target = (ty * nx + tx) as usize;
                    if self.stamp_mark[map][target / 64] & (1 << (target % 64)) == 0 {
                        self.stamp_mark[map][target / 64] |= 1 << (target % 64);
                        self.occupancy[map][target] += 1;
                        if self.occupancy[map][target] == 1 {
                            let tile = (target / self.grid.nx / TILE) * self.tiles_x
                                + (target % self.grid.nx) / TILE;
                            self.tile_claimed[map][tile] += 1;
                        }
                        stamped.push((map as u32, target as u32));
                    }
                }
            };
            let base = querying * (layers + 1);
            for (which, branch) in branches.iter().enumerate() {
                for end in [branch.nodes[0], *branch.nodes.last().unwrap()] {
                    if let Some((cells, width, _)) = escapes.get(&end) {
                        // A stub at the neck width stamps the neck's footprint.
                        let stub_class = match self.neck_of[class] {
                            Some(neck) if *width < self.board.classes[class].trace_width - 1.0e-9 => neck,
                            _ => class,
                        };
                        let stub_stamps = &self.stamps[stub_class][querying];
                        for cell in cells {
                            apply(base + end.layer as usize, *cell, &stub_stamps.stub_to_trace);
                            apply(base + layers, *cell, &stub_stamps.stub_to_via);
                            for orientation in 0..2 {
                                apply(
                                    diagonal_base + (querying * layers + end.layer as usize) * 2 + orientation,
                                    *cell,
                                    &stub_stamps.stub_diagonal_block[orientation],
                                );
                            }
                        }
                    }
                }
                for (index, node) in branch.nodes.iter().enumerate() {
                    // A node in a neck zone stamps its narrower footprint.
                    let own = &self.stamps[node_classes[which][index]][querying];
                    apply(
                        base + node.layer as usize,
                        node.cell,
                        &own.trace_to_trace,
                    );
                    apply(base + layers, node.cell, &own.trace_to_via);
                    for orientation in 0..2 {
                        apply(
                            diagonal_base + (querying * layers + node.layer as usize) * 2 + orientation,
                            node.cell,
                            &own.diagonal_block[orientation],
                        );
                    }
                    let is_via = index > 0
                        && branch.nodes[index - 1].cell == node.cell
                        && branch.nodes[index - 1].layer != node.layer;
                    if is_via {
                        for layer in 0..layers {
                            apply(base + layer, node.cell, &stamps.via_to_trace);
                        }
                        apply(base + layers, node.cell, &stamps.via_to_via);
                        for layer in 0..layers {
                            for orientation in 0..2 {
                                apply(
                                    diagonal_base + (querying * layers + layer) * 2 + orientation,
                                    node.cell,
                                    &stamps.diagonal_block_via[orientation],
                                );
                            }
                        }
                    }
                    // A diagonal step covers the nodes on its bisector too.
                    if index > 0 && !is_via {
                        let previous = branch.nodes[index - 1];
                        let (dx, dy) = (
                            node.cell as i64 % nx - previous.cell as i64 % nx,
                            node.cell as i64 / nx - previous.cell as i64 / nx,
                        );
                        if dx != 0 && dy != 0 {
                            let own = &self.stamps[node_classes[which][index]][querying];
                            apply(base + node.layer as usize, previous.cell, &turned(&own.diagonal_to_trace, dx as i32, dy as i32));
                            apply(base + layers, previous.cell, &turned(&own.diagonal_to_via, dx as i32, dy as i32));
                        }
                    }
                }
            }
        }
        for (map, cell) in &stamped {
            self.stamp_mark[*map as usize][*cell as usize / 64] &= !(1 << (*cell % 64));
        }
        let state = &mut self.nets[net as usize];
        state.branches = branches;
        state.escapes = escapes;
        state.stamped = stamped;
    }

    /// Nodes of `net` that lie inside another net's clearance zone.
    fn conflicts(&self, net: NetId) -> Vec<(usize, u32)> {
        let mut result = Vec::new();
        // Skeleton branches are fixed: an overlap is the other net's conflict.
        for branch in self.nets[net as usize]
            .branches
            .iter()
            .skip(self.nets[net as usize].fixed)
        {
            self.branch_conflicts(net, branch, &mut result);
        }
        result
    }

    /// The cells of `branch` that another net's clearance zone covers
    /// (`(layer, cell)`, the layer count standing for the via map): its
    /// escape stubs, its nodes and its vias. Another net's diagonal step
    /// passing a node too closely shows here as well, through the stamps
    /// of that step's bisector. The one judge of a conflict: what rips a
    /// branch up must be what the negotiation counts.
    fn branch_conflicts(&self, net: NetId, branch: &Branch, result: &mut Vec<(usize, u32)>) {
        let class = self.board.nets[net as usize].class;
        let layers = self.board.layer_count;
        let state = &self.nets[net as usize];
        for end in [branch.nodes[0], *branch.nodes.last().unwrap()] {
            if let Some((cells, width, _)) = state.escapes.get(&end) {
                let stub_class = match self.neck_of[class] {
                    Some(neck) if *width < self.board.classes[class].trace_width - 1.0e-9 => neck,
                    _ => class,
                };
                let map = self.map_index(stub_class, end.layer as usize);
                for cell in cells {
                    if self.occupancy[map][*cell as usize] > 1 {
                        result.push((end.layer as usize, *cell));
                    }
                }
            }
        }
        for (index, node) in branch.nodes.iter().enumerate() {
            let map = self.map_index(self.node_class(net, node.cell), node.layer as usize);
            if self.occupancy[map][node.cell as usize] > 1 {
                result.push((node.layer as usize, node.cell));
            }
            let is_via = index > 0
                && branch.nodes[index - 1].cell == node.cell
                && branch.nodes[index - 1].layer != node.layer;
            if is_via && self.occupancy[self.map_index(class, layers)][node.cell as usize] > 1 {
                result.push((layers, node.cell));
            }
        }
    }

    /// Debug: with `PCB_ROUTER_DUMP=x0,y0,x1,y1` (mm), prints the lattice
    /// of that region per layer: `#` static block, `x` the nets' own
    /// pads, `.` free, a digit the occupancy, `*` an occupied node of one
    /// of `nets`, `v` where a via may not go.
    fn dump_region(&self, nets: &[NetId]) {
        let Some(spec) = std::env::var_os("PCB_ROUTER_DUMP") else {
            return;
        };
        let bounds: Vec<f64> = spec.to_string_lossy().split(',').filter_map(|v| v.parse().ok()).collect();
        if bounds.len() != 4 {
            return;
        }
        let Some((x0, y0, x1, y1)) = self.grid.node_range(crate::geometry::Aabb {
            minimum: [bounds[0], bounds[1]],
            maximum: [bounds[2], bounds[3]],
        }) else {
            return;
        };
        let layers = self.board.layer_count;
        let mut own_nodes: HashSet<(usize, u32)> = HashSet::new();
        for net in nets {
            for branch in &self.nets[*net as usize].branches {
                for node in &branch.nodes {
                    own_nodes.insert((node.layer as usize, node.cell));
                }
            }
        }
        for layer in 0..layers {
            eprintln!("  layer {layer}, x {:.2}..{:.2}, y {:.2}..{:.2}", bounds[0], bounds[2], bounds[1], bounds[3]);
            for y in y0..=y1 {
                let mut line = String::new();
                for x in x0..=x1 {
                    let cell = self.grid.index(x, y);
                    let class = self.board.nets[nets[0] as usize].class;
                    let allowed = self.statics[class].trace[layer][cell];
                    let occupancy = self.occupancy[self.map_index(class, layer)][cell];
                    line.push(if own_nodes.contains(&(layer, cell as u32)) {
                        '*'
                    } else if allowed == crate::grid::BLOCKED {
                        '#'
                    } else if allowed != crate::grid::FREE {
                        if nets.iter().any(|net| allowed == crate::grid::owner(*net)) { 'x' } else { 'o' }
                    } else if occupancy > 0 {
                        char::from_digit(occupancy.min(9) as u32, 10).unwrap()
                    } else if self.statics[class].via_blocked[cell] {
                        'v'
                    } else {
                        '.'
                    });
                }
                eprintln!("  {line}");
            }
        }
    }

    fn window(&self, net: NetId, growth: f64) -> (usize, usize, usize, usize) {
        let mut minimum = [f64::INFINITY; 2];
        let mut maximum = [f64::NEG_INFINITY; 2];
        for terminal in &self.board.nets[net as usize].terminals {
            for axis in 0..2 {
                minimum[axis] = minimum[axis].min(terminal.anchor[axis]);
                maximum[axis] = maximum[axis].max(terminal.anchor[axis]);
            }
        }
        let extent = (maximum[0] - minimum[0]).max(maximum[1] - minimum[1]);
        let margin = self.config.window_margin.max(0.35 * extent) * growth;
        let bounds = crate::geometry::Aabb { minimum, maximum }.inflated(margin);
        self.grid
            .node_range(bounds)
            .unwrap_or((0, 0, self.grid.nx - 1, self.grid.ny - 1))
    }

    /// Connects all terminals of `net` into one tree, reusing any branches
    /// the net still has. With `hard`, nodes claimed by other nets are
    /// impassable and a partial result may be kept.
    #[allow(clippy::too_many_arguments)]
    fn route_net(
        &self,
        scratch: &mut Scratch,
        net: NetId,
        net_state: &mut NetState,
        present: f32,
        hard: bool,
        window_growth: f64,
    ) {
        scratch.ensure(self.grid.cells() * self.board.layer_count, self.grid.cells());
        let terminal_count = self.board.nets[net as usize].terminals.len();
        let retained = net_state.branches.clone();
        let keep = vec![true; retained.len()];
        let hosts = junction_hosts(&retained, &keep);

        // Elements are terminals, retained branches, and finally the pours.
        let has_plane = !net_state.plane.is_empty();
        let plane_element = terminal_count + retained.len();
        let mut parent: Vec<usize> =
            (0..terminal_count + retained.len() + has_plane as usize).collect();
        if has_plane {
            for terminal in 0..terminal_count {
                if net_state.on_plane[terminal] {
                    parent[terminal] = plane_element;
                }
            }
        }
        for (index, branch) in retained.iter().enumerate() {
            let element = terminal_count + index;
            let ends = [
                (branch.start_terminal, branch.nodes[0]),
                (branch.end_terminal, *branch.nodes.last().unwrap()),
            ];
            for (terminal, node) in ends {
                let other = if terminal == PLANE_TERMINAL {
                    // While pads must reach the main piece, a stub joins the
                    // plane only where it still ends in that piece: the
                    // signals may have cut its end off since (k30's +5V: 22
                    // pads stranded on stubs into islands or no fill at all,
                    // which every reroute kept as connected).
                    if !net_state.plane_target.is_empty()
                        && !net_state
                            .plane_target
                            .get(node.layer as usize)
                            .is_some_and(|mask| mask.get(node.cell as usize).copied().unwrap_or(false))
                    {
                        continue;
                    }
                    plane_element
                } else if terminal == FREE_END {
                    continue;
                } else if terminal != NO_TERMINAL {
                    terminal as usize
                } else if let Some(host) = hosts.get(&node) {
                    terminal_count + host
                } else {
                    // A junction whose host is gone: the end is free.
                    continue;
                };
                let (a, b) = (find(&mut parent, element), find(&mut parent, other));
                parent[a] = b;
            }
        }
        let component: Vec<usize> = (0..parent.len())
            .map(|element| find(&mut parent, element))
            .collect();

        scratch.tree.clear();
        let mut tree: Vec<Node> = Vec::new();
        let mut in_tree = vec![false; parent.len()];
        let cells = self.grid.cells();
        let state_of = |node: Node| node.layer as usize * cells + node.cell as usize;
        let absorb = |scratch: &mut Scratch,
                      net_state: &mut NetState,
                      tree: &mut Vec<Node>,
                      in_tree: &mut Vec<bool>,
                      root: usize| {
            for element in 0..component.len() {
                if component[element] != root || in_tree[element] {
                    continue;
                }
                in_tree[element] = true;
                if has_plane && element == plane_element {
                    continue;
                }
                if element < terminal_count {
                    for node in net_state.terminal_nodes[element].clone() {
                        scratch.tree.insert(state_of(node) as u32, element as u16);
                        tree.push(node);
                    }
                    net_state.connected[element] = true;
                } else {
                    for node in &retained[element - terminal_count].nodes {
                        if let std::collections::hash_map::Entry::Vacant(entry) = scratch.tree.entry(state_of(*node) as u32) {
                            entry.insert(NO_TERMINAL);
                            tree.push(*node);
                        }
                    }
                }
            }
        };
        if has_plane {
            self.connect_to_plane(
                scratch,
                net,
                net_state,
                &retained,
                &component,
                plane_element,
                present,
                hard,
            );
            return;
        }
        let first_live = (0..terminal_count)
            .find(|terminal| !net_state.terminal_nodes[*terminal].is_empty())
            .unwrap_or(0);
        absorb(
            scratch,
            net_state,
            &mut tree,
            &mut in_tree,
            component[first_live],
        );

        let pitch = self.grid.pitch as f32;
        loop {
            let mut targets: Vec<(Node, u16)> = Vec::new();
            let mut points: Vec<(i32, i32, f32)> = Vec::new();
            let mut exact_points = true;
            for element in 0..component.len() {
                if in_tree[element] {
                    continue;
                }
                if element < terminal_count {
                    for node in &net_state.terminal_nodes[element] {
                        targets.push((*node, element as u16));
                    }
                    let anchor = self.board.nets[net as usize].terminals[element].anchor;
                    points.push((
                        ((anchor[0] - self.grid.origin[0]) / self.grid.pitch).round() as i32,
                        ((anchor[1] - self.grid.origin[1]) / self.grid.pitch).round() as i32,
                        // Rounding the anchor to a node costs at most one pitch.
                        net_state.terminal_reach[element] + pitch,
                    ));
                } else {
                    exact_points = false;
                    for node in &retained[element - terminal_count].nodes {
                        targets.push((*node, element as u16));
                    }
                }
            }
            if targets.is_empty() {
                break;
            }
            let points = (exact_points && points.len() <= 12).then_some(points);
            let windowed = self.window(net, window_growth);
            let full = (0, 0, self.grid.nx - 1, self.grid.ny - 1);
            // Two-level search: a corridor from the tile graph first, wider
            // on failure, and only then the plain window and the full board.
            let mut planned = None;
            let reroutes = net_state.reroutes;
            let global = self.global.as_ref().filter(|_| !self.cleanup && !hard);
            if let Some(plan) = global {
                // The global plan's corridor, wider for stubborn nets.
                let margin = 1 + reroutes / 4;
                for margin in [margin, margin + 2] {
                    let Some(corridor) = plan.corridor(net as usize, margin) else {
                        break;
                    };
                    scratch.corridor = Some(corridor);
                    planned = self.search(
                        scratch,
                        net,
                        net_state,
                        &tree,
                        &targets,
                        points.as_deref(),
                        false,
                        present,
                        hard,
                        full,
                    );
                    scratch.corridor = None;
                    if planned.is_some() {
                        break;
                    }
                }
            } else if self.config.corridors && !self.cleanup {
                // One tile more every six reroutes. Widened every three,
                // the corridors of the nets the negotiation kept rerouting
                // gave their searches room the paths never used (Sisu and
                // jetson: 73 % of a late iteration's expansions more than
                // three tiles from the path found). The slower growth: on
                // the quick tier 10 % less CPU, unconnected 32 to 30,
                // starved thermals 20 to 15, vias -6 %, no pass lost; in
                // route mode nonSNES SNSP-CPU-01 171 to 98 unconnected at
                // +6 % vias. A cap instead (at most 3 or 5 tiles) left
                // SNSP-CPU-01 unconverged: some nets need the wide detour.
                let margin = (1 + reroutes / 6).min(10);
                for margin in [margin, margin + 3] {
                    let mut read = std::mem::take(&mut scratch.read);
                    let corridor = self.plan_corridor(
                        net,
                        net_state,
                        &tree,
                        &targets,
                        hard,
                        margin,
                        scratch.own_active.then_some(scratch.own_claimed.as_slice()),
                        &mut read,
                    );
                    scratch.read = read;
                    let Some(corridor) = corridor
                    else {
                        break;
                    };
                    scratch.corridor = Some(corridor);
                    planned = self.search(
                        scratch,
                        net,
                        net_state,
                        &tree,
                        &targets,
                        points.as_deref(),
                        false,
                        present,
                        hard,
                        full,
                    );
                    scratch.corridor = None;
                    if planned.is_some() {
                        break;
                    }
                }
            }
            // Board-wide fallbacks, unless they keep failing where a
            // corridor was searched first.
            let corridor_tried = global.is_some() || (self.config.corridors && !self.cleanup);
            let fallback = hard || !corridor_tried || net_state.hopeless_searches < 2;
            let path = planned.or_else(|| {
                if !fallback {
                    return None;
                }
                self.search(
                    scratch,
                    net,
                    net_state,
                    &tree,
                    &targets,
                    points.as_deref(),
                    false,
                    present,
                    hard,
                    windowed,
                )
            });
            let path = path.or_else(|| {
                (fallback && windowed != full)
                    .then(|| {
                        self.search(
                            scratch,
                            net,
                            net_state,
                            &tree,
                            &targets,
                            points.as_deref(),
                            false,
                            present,
                            hard,
                            full,
                        )
                    })
                    .flatten()
            });
            let Some(path) = path else {
                if fallback && !hard {
                    net_state.hopeless_searches = net_state.hopeless_searches.saturating_add(1);
                }
                break;
            };
            let first = self.state(path[0]);
            let last = self.state(*path.last().unwrap());
            let start_terminal = scratch.tree.get(&(first as u32)).copied().unwrap_or(NO_TERMINAL);
            let reached = scratch.targets[&(last as u32)] as usize;
            for node in &path[1..] {
                if let std::collections::hash_map::Entry::Vacant(entry) = scratch.tree.entry(self.state(*node) as u32) {
                    entry.insert(NO_TERMINAL);
                    tree.push(*node);
                }
            }
            absorb(
                scratch,
                net_state,
                &mut tree,
                &mut in_tree,
                component[reached],
            );
            net_state.branches.push(Branch {
                nodes: path,
                start_terminal,
                end_terminal: if reached < terminal_count {
                    reached as u16
                } else {
                    NO_TERMINAL
                },
            });
        }
        let complete = in_tree.iter().all(|inside| *inside);
        net_state.complete = complete;
        if !hard && !complete {
            net_state.blocked = true;
        }
    }

    /// Routing for a net with copper pours: every group of terminals and
    /// retained branches that does not touch a pour is connected to the
    /// nearest free pour node, or to another group on the way.
    #[allow(clippy::too_many_arguments)]
    fn connect_to_plane(
        &self,
        scratch: &mut Scratch,
        net: NetId,
        net_state: &mut NetState,
        retained: &[Branch],
        component: &[usize],
        plane_element: usize,
        present: f32,
        hard: bool,
    ) {
        let terminal_count = self.board.nets[net as usize].terminals.len();
        let mut parent: Vec<usize> = component.to_vec();
        let full = (0, 0, self.grid.nx - 1, self.grid.ny - 1);
        let mut failed = vec![false; parent.len()];
        loop {
            // Done when all terminals are one group; the pour is only a
            // means to that end and need not be reached at all.
            let mut roots: Vec<usize> = (0..terminal_count)
                .map(|terminal| find(&mut parent, terminal))
                .collect();
            roots.sort_unstable();
            roots.dedup();
            if std::env::var_os("PCB_ROUTER_DEBUG").is_some() {
                eprintln!(
                    "connect_to_plane {}: {} groups, hard {hard}, off-pour terminals {:?}",
                    self.board.nets[net as usize].name,
                    roots.len(),
                    (0..terminal_count)
                        .filter(|terminal| !net_state.on_plane[*terminal])
                        .collect::<Vec<_>>()
                );
            }
            if roots.len() <= 1 {
                break;
            }
            let plane_root = find(&mut parent, plane_element);
            let Some(root) = roots
                .into_iter()
                .find(|root| *root != plane_root && !failed[*root])
            else {
                break;
            };
            scratch.tree.clear();
            let mut sources: Vec<Node> = Vec::new();
            let mut targets: Vec<(Node, u16)> = Vec::new();
            for element in (0..parent.len()).filter(|element| *element != plane_element) {
                let inside = find(&mut parent, element) == root;
                let nodes: Vec<Node> = if element < terminal_count {
                    net_state.terminal_nodes[element].clone()
                } else if element < plane_element {
                    retained[element - terminal_count].nodes.clone()
                } else {
                    // Branches added by this call follow the retained ones.
                    net_state.branches[retained.len() + element - plane_element - 1]
                        .nodes
                        .clone()
                };
                for node in nodes {
                    if inside {
                        if let std::collections::hash_map::Entry::Vacant(entry) = scratch.tree.entry(self.state(node) as u32) {
                            entry.insert(if element < terminal_count { element as u16 } else { NO_TERMINAL });
                            sources.push(node);
                        }
                    } else {
                        targets.push((node, element as u16));
                    }
                }
            }
            let Some(path) = self.search(
                scratch, net, net_state, &sources, &targets, None, true, present, hard, full,
            ) else {
                if std::env::var_os("PCB_ROUTER_DEBUG").is_some() {
                    let restricted = !net_state.plane_target.is_empty();
                    let plane_nodes: usize = if restricted {
                        net_state
                            .plane_target
                            .iter()
                            .map(|m| m.iter().filter(|b| **b).count())
                            .sum()
                    } else {
                        net_state
                            .plane
                            .iter()
                            .map(|m| m.iter().filter(|b| **b).count())
                            .sum()
                    };
                    eprintln!(
                        "  no path: group root {root}, {} sources, {} group targets, plane nodes {plane_nodes} (restricted {restricted}), hard {hard}",
                        sources.len(),
                        targets.len()
                    );
                }
                failed[root] = true;
                continue;
            };
            let first = self.state(path[0]);
            let last = self.state(*path.last().unwrap());
            let start_terminal = scratch.tree.get(&(first as u32)).copied().unwrap_or(NO_TERMINAL);
            let (reached, end_terminal) = if scratch.nodes[last].target_mark == scratch.generation {
                let element = scratch.targets[&(last as u32)] as usize;
                (
                    element,
                    if element < terminal_count {
                        element as u16
                    } else {
                        NO_TERMINAL
                    },
                )
            } else {
                (plane_element, PLANE_TERMINAL)
            };
            let (a, b) = (find(&mut parent, root), find(&mut parent, reached));
            parent[a] = b;
            net_state.branches.push(Branch {
                nodes: path,
                start_terminal,
                end_terminal,
            });
            // The new branch is an element of its own from now on.
            parent.push(b);
            failed.push(false);
        }
        let mut votes: HashMap<usize, usize> = HashMap::new();
        for terminal in 0..terminal_count {
            *votes.entry(find(&mut parent, terminal)).or_default() += 1;
        }
        let main = votes
            .into_iter()
            .max_by_key(|(root, count)| (*count, usize::MAX - *root))
            .map_or(0, |(root, _)| root);
        let mut complete = true;
        for terminal in 0..terminal_count {
            let connected = find(&mut parent, terminal) == main;
            net_state.connected[terminal] = connected;
            complete &= connected;
        }
        net_state.complete = complete;
        if !hard && !complete {
            net_state.blocked = true;
        }
    }

    /// Plans a connection on the tile graph: Dijkstra over (layer, tile)
    /// from the source tiles to any target tile, where a tile costs more the
    /// fuller it is. Returns the tiles on the path, grown by `margin` tiles.
    fn plan_corridor(
        &self,
        net: NetId,
        net_state: &NetState,
        sources: &[Node],
        targets: &[(Node, u16)],
        hard: bool,
        margin: usize,
        own_claimed: Option<&[u32]>,
        read: &mut ReadSet,
    ) -> Option<Vec<bool>> {
        let class = self.board.nets[net as usize].class;
        let layers = self.board.layer_count;
        let tiles = self.tiles_x * self.tiles_y;
        let tile_of = |cell: u32| {
            (cell as usize / self.grid.nx / TILE) * self.tiles_x
                + (cell as usize % self.grid.nx) / TILE
        };
        let mut is_target = vec![false; tiles * layers];
        let mut endpoint = vec![false; tiles];
        for (node, _) in targets {
            is_target[node.layer as usize * tiles + tile_of(node.cell)] = true;
            endpoint[tile_of(node.cell)] = true;
        }
        let tile_mm = (TILE as f64 * self.grid.pitch) as f32;
        let mut cost = vec![f32::INFINITY; tiles * layers];
        let mut from = vec![u32::MAX; tiles * layers];
        let mut heap: BinaryHeap<Reverse<(u32, u32)>> = BinaryHeap::new();
        for node in sources {
            let state = node.layer as usize * tiles + tile_of(node.cell);
            endpoint[tile_of(node.cell)] = true;
            if cost[state] > 0.0 {
                cost[state] = 0.0;
                heap.push(Reverse((0, state as u32)));
            }
        }
        // How expensive a tile is: free tiles cost their size; a full tile
        // is nearly a wall (a real wall when routing hard).
        let enter = |layer: usize, tile: usize| -> Option<f32> {
            let routable = self.tile_routable[class * layers + layer][tile];
            if routable == 0 && !endpoint[tile] {
                return None;
            }
            let mut claimed = self.tile_claimed[self.map_index(class, layer)][tile];
            if let Some(own) = own_claimed {
                claimed = claimed.saturating_sub(own[layer * tiles + tile]);
            }
            let fill = (claimed as f32 / routable.max(1) as f32).min(1.0);
            if hard && fill >= 1.0 && !endpoint[tile] {
                return None;
            }
            let covered = if net_state.plane.is_empty() {
                self.tile_covered[layer][tile]
            } else {
                0.0
            };
            Some(
                (1.0 + 12.0 * fill * fill * fill)
                    * (1.0 + self.tile_history[layer][tile])
                    * (1.0 + (self.layer_cut[layer] - 1.0) * covered),
            )
        };
        let mut reached = None;
        while let Some(Reverse((bits, state))) = heap.pop() {
            let state = state as usize;
            let here = f32::from_bits(bits);
            if here > cost[state] {
                continue;
            }
            if is_target[state] {
                reached = Some(state);
                break;
            }
            let (layer, tile) = (state / tiles, state % tiles);
            let (x, y) = (tile % self.tiles_x, tile / self.tiles_x);
            let mut relax = |next: usize, step: f32, heap: &mut BinaryHeap<Reverse<(u32, u32)>>| {
                let total = here + step;
                if total < cost[next] {
                    cost[next] = total;
                    from[next] = state as u32;
                    heap.push(Reverse((total.to_bits(), next as u32)));
                }
            };
            for (direction, (dx, dy)) in DIRECTIONS.iter().enumerate() {
                let (tx, ty) = (x as i64 + *dx as i64, y as i64 + *dy as i64);
                if tx < 0 || ty < 0 || tx >= self.tiles_x as i64 || ty >= self.tiles_y as i64 {
                    continue;
                }
                let next_tile = ty as usize * self.tiles_x + tx as usize;
                let Some(factor) = enter(layer, next_tile) else {
                    continue;
                };
                let step = self.direction_cost[layer][direction] / self.grid.pitch as f32 * tile_mm;
                relax(layer * tiles + next_tile, step * factor, &mut heap);
            }
            for other in (0..layers).filter(|other| *other != layer) {
                if let Some(factor) = enter(other, tile) {
                    relax(
                        other * tiles + tile,
                        self.config.via_cost as f32 * factor,
                        &mut heap,
                    );
                }
            }
        }
        // What the planner read: the tiles it reached and their
        // neighbours (each relaxation reads the neighbour's fill).
        if read.active {
            for state in 0..cost.len() {
                if cost[state].is_finite() {
                    read.note_tile(state % tiles);
                }
            }
        }
        let mut state = reached?;
        let mut corridor = vec![false; tiles];
        loop {
            let tile = state % tiles;
            let (x, y) = (tile % self.tiles_x, tile / self.tiles_x);
            for ty in y.saturating_sub(margin)..=(y + margin).min(self.tiles_y - 1) {
                for tx in x.saturating_sub(margin)..=(x + margin).min(self.tiles_x - 1) {
                    corridor[ty * self.tiles_x + tx] = true;
                }
            }
            if from[state] == u32::MAX {
                break;
            }
            state = from[state] as usize;
        }
        Some(corridor)
    }

    #[allow(clippy::too_many_arguments)]
    fn search(
        &self,
        scratch: &mut Scratch,
        net: NetId,
        net_state: &NetState,
        sources: &[Node],
        targets: &[(Node, u16)],
        points: Option<&[(i32, i32, f32)]>,
        plane_target: bool,
        present: f32,
        hard: bool,
        window: (usize, usize, usize, usize),
    ) -> Option<Vec<Node>> {
        scratch.searches += 1;
        let expansions_before = scratch.expansions;
        let generation = scratch.next_generation();
        let class = self.board.nets[net as usize].class;
        let layers = self.board.layer_count;
        let cells = self.grid.cells();
        let nx = self.grid.nx;
        let via_cost = if plane_target {
            self.config.plane_via_cost
        } else if self.cleanup {
            self.config.cleanup_via_cost
        } else {
            self.config.via_cost
        } as f32;
        let bend_cost = self.config.bend_cost as f32;
        // Cleanup and hard reinsertion decide final geometry: search exactly.
        let weight = if self.cleanup || hard {
            0.999
        } else {
            self.config.heuristic_weight as f32 * 0.999
        };

        {
            let stamps = &self.stamps[class][class];
            let nx = self.grid.nx as i64;
            let ny = self.grid.ny as i64;
            let mut near = std::mem::take(&mut scratch.own_via_near);
            for branch in &net_state.branches {
                for pair in branch.nodes.windows(2) {
                    if pair[0].cell != pair[1].cell {
                        continue;
                    }
                    let (x, y) = (pair[0].cell as i64 % nx, pair[0].cell as i64 / nx);
                    for (dx, dy) in &stamps.via_to_via {
                        let (tx, ty) = (x + *dx as i64, y + *dy as i64);
                        if tx >= 0 && ty >= 0 && tx < nx && ty < ny && (*dx != 0 || *dy != 0) {
                            near[(ty * nx + tx) as usize] = generation;
                        }
                    }
                    near[pair[0].cell as usize] = 0;
                }
            }
            // A via cell itself stays allowed even inside another own via's
            // ring, so mark exact via cells last.
            for branch in &net_state.branches {
                for pair in branch.nodes.windows(2) {
                    if pair[0].cell == pair[1].cell {
                        near[pair[0].cell as usize] = 0;
                    }
                }
            }
            scratch.own_via_near = near;
        }
        // What the sources and targets read (their escape stubs too).
        for node in sources.iter().chain(targets.iter().map(|(node, _)| node)) {
            scratch.read.note_cell(node.cell, nx);
            if let Some((cells, _, _)) = net_state.escapes.get(node) {
                for cell in cells {
                    scratch.read.note_cell(*cell, nx);
                }
            }
        }
        scratch.targets.clear();
        for (node, element) in targets {
            if hard && !self.escape_is_free(scratch, net_state, net, *node) {
                continue;
            }
            let state = self.state(*node);
            scratch.nodes[state].target_mark = generation;
            scratch.targets.insert(state as u32, *element);
        }

        // Without a short list of target points, guide the search with an
        // exact octile distance transform over coarse blocks.
        const BLOCK: usize = 8;
        let blocks_x = nx.div_ceil(BLOCK);
        let blocks_y = self.grid.ny.div_ceil(BLOCK);
        let coarse: Vec<f32> = if points.is_some() || plane_target {
            Vec::new()
        } else {
            let mut distance = vec![f32::INFINITY; blocks_x * blocks_y];
            for (node, _) in targets {
                let (x, y) = self.grid.xy(node.cell as usize);
                distance[(y / BLOCK) * blocks_x + x / BLOCK] = 0.0;
            }
            let diagonal = std::f32::consts::SQRT_2;
            let relax = |x: usize, y: usize, dx: i64, dy: i64, distance: &mut Vec<f32>| {
                let sx = x as i64 + dx;
                let sy = y as i64 + dy;
                if sx < 0 || sy < 0 || sx >= blocks_x as i64 || sy >= blocks_y as i64 {
                    return;
                }
                let step = if dx != 0 && dy != 0 { diagonal } else { 1.0 };
                let candidate = distance[sy as usize * blocks_x + sx as usize] + step;
                if candidate < distance[y * blocks_x + x] {
                    distance[y * blocks_x + x] = candidate;
                }
            };
            for y in 0..blocks_y {
                for x in 0..blocks_x {
                    for (dx, dy) in [(-1, 0), (-1, -1), (0, -1), (1, -1)] {
                        relax(x, y, dx, dy, &mut distance);
                    }
                }
            }
            for y in (0..blocks_y).rev() {
                for x in (0..blocks_x).rev() {
                    for (dx, dy) in [(1, 0), (1, 1), (0, 1), (-1, 1)] {
                        relax(x, y, dx, dy, &mut distance);
                    }
                }
            }
            distance
        };
        let direction_cost: Vec<[f32; 8]> = if self.cleanup {
            let pitch = self.grid.pitch as f32;
            let mut costs = [pitch; 8];
            for direction in [1, 3, 5, 7] {
                costs[direction] = pitch * std::f32::consts::SQRT_2;
            }
            vec![costs; layers]
        } else {
            self.direction_cost.clone()
        };
        let minimum = |direction: usize| {
            direction_cost
                .iter()
                .map(|costs| costs[direction])
                .fold(f32::INFINITY, f32::min)
        };
        let (any_horizontal, any_diagonal, any_vertical) = (minimum(0), minimum(1), minimum(2));
        let heuristic = |layer: usize, x: i32, y: i32| -> f32 {
            if plane_target {
                // A pour is everywhere; the nearest free spot is close.
                return 0.0;
            }
            let Some(points) = points else {
                let blocks = coarse[(y as usize / BLOCK) * blocks_x + x as usize / BLOCK];
                return (blocks - std::f32::consts::SQRT_2).max(0.0)
                    * BLOCK as f32
                    * any_horizontal.min(any_vertical)
                    * weight;
            };
            let costs = &direction_cost[layer];
            let mut best = f32::INFINITY;
            for (tx, ty, reach) in points {
                let dx = (x - tx).abs() as f32;
                let dy = (y - ty).abs() as f32;
                let both = dx.min(dy);
                // Stay on this layer, or pay a via for the cheapest mix.
                let same_layer = both * costs[1] + (dx - both) * costs[0] + (dy - both) * costs[2];
                let any_layer = via_cost
                    + both * any_diagonal
                    + (dx - both) * any_horizontal
                    + (dy - both) * any_vertical;
                best = best.min(same_layer.min(any_layer) - reach * 2.0);
            }
            best.max(0.0) * weight
        };

        // The scratch's heap, its capacity kept from search to search.
        let mut heap = std::mem::take(&mut scratch.heap);
        heap.clear();
        for node in sources {
            // A hard route may not even start inside another net's zone.
            if hard
                && (self.occupancy_seen(scratch, class, self.map_index(class, node.layer as usize), node.cell as usize)
                    > 0
                    || !self.escape_is_free(scratch, net_state, net, *node))
            {
                continue;
            }
            let state = self.state(*node);
            let (x, y) = self.grid.xy(node.cell as usize);
            scratch.nodes[state].seen = generation;
            scratch.nodes[state].cost = 0.0;
            scratch.nodes[state].parent = PARENT_SOURCE;
            heap.push(heap_key(heuristic(node.layer as usize, x as i32, y as i32), state));
        }

        let statics = &self.statics[class];
        let own = crate::grid::owner(net);
        let own16 = crate::grid::owner16(net);
        let trace_base = class * (layers + 1);
        let via_map = trace_base + layers;
        let necks = !net_state.neck_zones.is_empty();
        let mut found = None;
        while let Some(key) = heap.pop() {
            let state = (key & u32::MAX as u64) as usize;
            if scratch.nodes[state].closed == generation {
                continue;
            }
            scratch.nodes[state].closed = generation;
            scratch.expansions += 1;
            if scratch.nodes[state].target_mark == generation {
                found = Some(state);
                break;
            }
            if plane_target {
                let state_net = &net_state;
                let mask = if state_net.plane_target.is_empty() {
                    &state_net.plane[state / cells]
                } else {
                    &state_net.plane_target[state / cells]
                };
                let pour_map = state_net.plane_class[state / cells] * (layers + 1) + state / cells;
                if !mask.is_empty()
                    && mask[state % cells]
                    && !(hard && self.occupancy_seen(scratch, class, pour_map, state % cells) > 0)
                {
                    found = Some(state);
                    break;
                }
            }
            let layer = state / cells;
            let cell = state % cells;
            let x = cell % nx;
            let y = cell / nx;
            scratch.read.note(x, y);
            let here = scratch.nodes[state].cost;
            let arrived = scratch.nodes[state].parent;
            let here_statics = if necks { &self.statics[self.class_at(net, net_state, cell as u32)] } else { statics };
            let edge_block = here_statics.edge_block[layer][cell];
            let blocked_edges = if edge_block != 0 && here_statics.edge_owner[layer][cell] == own16 {
                0
            } else {
                edge_block
            };
            // This layer's packed maps, looked up once per expansion
            // rather than once per step.
            let layer_hot = &self.hot[class * layers + layer];
            let cleanup = self.cleanup;
            let layer_costs = &direction_cost[layer];
            let layer_factor = if self.layer_bias.is_empty() { 1.0 } else { self.layer_bias[layer] };
            let thermal_guard_cost = self.config.thermal_guard_cost as f32;

            for (direction, (dx, dy)) in DIRECTIONS.iter().enumerate() {
                let tx = x as i64 + *dx as i64;
                let ty = y as i64 + *dy as i64;
                if tx < window.0 as i64
                    || ty < window.1 as i64
                    || tx > window.2 as i64
                    || ty > window.3 as i64
                {
                    continue;
                }
                if blocked_edges & (1 << direction) != 0 {
                    continue;
                }
                if let Some(corridor) = &scratch.corridor
                    && !corridor[(ty as usize / TILE) * self.tiles_x + tx as usize / TILE]
                {
                    continue;
                }
                let target_cell = ty as usize * nx + tx as usize;
                let target_state = layer * cells + target_cell;
                // The search node is read for the cost update anyway: a
                // closed neighbour costs no other map's cache line.
                if scratch.nodes[target_state].closed == generation {
                    continue;
                }
                let target_class = if necks { self.class_at(net, net_state, target_cell as u32) } else { class };
                let hot = if necks { self.hot[target_class * layers + layer][target_cell] } else { layer_hot[target_cell] };
                if hot.trace != crate::grid::FREE && hot.trace != own {
                    continue;
                }
                let map = target_class * (layers + 1) + layer;
                let mut occupied = self.occupancy_seen(scratch, class, map, target_cell) as f32;
                // A diagonal step passes the nodes on its bisector closer
                // than its ends do: none of another net may lie there
                // within the clearance (exact clearance; an axis step
                // passes no node closer than its ends).
                if *dx != 0 && *dy != 0 {
                    let (start, orientation) = if *dx > 0 {
                        (cell, (*dy < 0) as usize)
                    } else {
                        (target_cell, (*dy > 0) as usize)
                    };
                    let blocked = self.occupancy_seen(scratch, class, self.diagonal_map(target_class, layer, orientation), start) as f32;
                    occupied = occupied.max(blocked);
                }
                if hard && occupied > 0.0 {
                    continue;
                }
                let history = if cleanup { 0.0 } else { hot.history };
                let mut step = layer_costs[direction] * (1.0 + history) * (1.0 + present * occupied);
                if hot.guard != 0 && (hot.guard & !HOT_HARD) as u32 != own {
                    if hot.guard & HOT_HARD != 0 {
                        continue;
                    }
                    step *= thermal_guard_cost;
                }
                if hot.covered != 0 && hot.covered as u32 != own {
                    if self.exclusive[layer] {
                        continue;
                    }
                    step *= self.layer_cut[layer];
                }
                step *= layer_factor;
                if (arrived as usize) < 8 {
                    let turn = (direction as i32 - arrived as i32).rem_euclid(8);
                    step += bend_cost * turn.min(8 - turn) as f32;
                }
                let total = here + step;
                if scratch.nodes[target_state].seen != generation || total < scratch.nodes[target_state].cost {
                    scratch.nodes[target_state].seen = generation;
                    scratch.nodes[target_state].cost = total;
                    scratch.nodes[target_state].parent = direction as u8;
                    let estimate = total + heuristic(layer, tx as i32, ty as i32);
                    heap.push(heap_key(estimate, target_state));
                }
            }

            let via_occupied = self.occupancy_seen(scratch, class, via_map, cell) as f32;
            if layers > 1
                && statics.via_allowed(cell, net)
                && !(hard && via_occupied > 0.0)
                && scratch.own_via_near[cell] != generation
            {
                let occupied = via_occupied;
                let history = if self.cleanup {
                    0.0
                } else {
                    self.history[layers][cell]
                };
                let mut cut = 1.0f32;
                let marks = self.via_marks.get(cell).copied().unwrap_or(3);
                if marks & 1 != 0 {
                    for (covered, factor) in self.covered.iter().zip(&self.layer_cut) {
                        if !covered.is_empty() && covered[cell] != 0 && covered[cell] != own {
                            cut *= factor;
                        }
                    }
                }
                // A via is copper on every layer: inside another net's
                // thermal ring on any of them it starves the spokes there.
                if marks & 2 != 0 {
                    let foreign = |guard: &Vec<u32>| !guard.is_empty() && guard[cell] != 0 && guard[cell] & !HARD_GUARD != own;
                    if self.guard.iter().any(|guard| foreign(guard) && guard[cell] & HARD_GUARD != 0) {
                        continue;
                    }
                    if self.guard.iter().any(foreign) {
                        cut *= self.config.thermal_guard_cost as f32;
                    }
                }
                let step = via_cost * cut * (1.0 + history) * (1.0 + present * occupied);
                let cell_class = if necks { self.class_at(net, net_state, cell as u32) } else { class };
                let cell_statics = &self.statics[cell_class];
                let free_layers = cell_statics.free_layers[cell];
                for target_layer in 0..layers {
                    if target_layer == layer {
                        continue;
                    }
                    if free_layers & (1 << target_layer) == 0 {
                        let allowed = cell_statics.trace[target_layer][cell];
                        if allowed != crate::grid::FREE && allowed != own {
                            continue;
                        }
                    }
                    if hard && self.occupancy_seen(scratch, class, cell_class * (layers + 1) + target_layer, cell) > 0 {
                        continue;
                    }
                    let target_state = target_layer * cells + cell;
                    if scratch.nodes[target_state].closed == generation {
                        continue;
                    }
                    let total = here + step;
                    if scratch.nodes[target_state].seen != generation
                        || total < scratch.nodes[target_state].cost
                    {
                        scratch.nodes[target_state].seen = generation;
                        scratch.nodes[target_state].cost = total;
                        scratch.nodes[target_state].parent = 8 + layer as u8;
                        let estimate = total + heuristic(target_layer, x as i32, y as i32);
                        heap.push(heap_key(estimate, target_state));
                    }
                }
            }
        }

        scratch.heap = heap;
        if scratch.corridor.is_none() {
            scratch.open_searches += 1;
            scratch.open_expansions += scratch.expansions - expansions_before;
        }
        if found.is_none() {
            scratch.failed_searches += 1;
        }
        let mut state = found?;
        let mut path = Vec::new();
        loop {
            let layer = state / cells;
            let cell = state % cells;
            path.push(Node {
                layer: layer as u8,
                cell: cell as u32,
            });
            let parent = scratch.nodes[state].parent;
            if parent == PARENT_SOURCE {
                break;
            }
            if parent < 8 {
                let (dx, dy) = DIRECTIONS[parent as usize];
                let x = (cell % nx) as i64 - dx as i64;
                let y = (cell / nx) as i64 - dy as i64;
                state = layer * cells + y as usize * nx + x as usize;
            } else {
                state = (parent as usize - 8) * cells + cell;
            }
        }
        path.reverse();
        Some(path)
    }

    /// Whether a retained branch is still legal against the fixed copper.
    fn branch_is_legal(&self, net: NetId, branch: &Branch) -> bool {
        let nx = self.grid.nx as i64;
        for (index, node) in branch.nodes.iter().enumerate() {
            let statics = &self.statics[self.node_class(net, node.cell)];
            if !statics.trace_allowed(node.layer as usize, node.cell as usize, net) {
                return false;
            }
            if index == 0 {
                continue;
            }
            let previous = branch.nodes[index - 1];
            if previous.cell == node.cell {
                if !statics.via_allowed(node.cell as usize, net) {
                    return false;
                }
                continue;
            }
            let (dx, dy) = (
                node.cell as i64 % nx - previous.cell as i64 % nx,
                node.cell as i64 / nx - previous.cell as i64 / nx,
            );
            let Some(direction) = DIRECTIONS
                .iter()
                .position(|(sx, sy)| *sx as i64 == dx && *sy as i64 == dy)
            else {
                return false;
            };
            if !statics.edge_allowed(
                previous.layer as usize,
                previous.cell as usize,
                direction,
                net,
            ) {
                return false;
            }
        }
        true
    }

    /// Replaces the board (same nets and rule classes, other fixed copper or
    /// terminal positions) and reroutes only what the change invalidated:
    /// nets whose pads moved and nets whose copper now collides with fixed
    /// objects. Returns the number of nets rerouted. Everything else keeps
    /// its copper, occupancy, and history.
    pub fn update(&mut self, board: &Board) -> Result<usize, String> {
        let (board, neck_of) = Self::with_neck_classes(board, &self.config);
        let board = &board;
        if neck_of != self.neck_of {
            return Err("incremental update needs the same nets and rules".into());
        }
        if board.nets.len() != self.board.nets.len()
            || board.classes != self.board.classes
            || board.layer_count != self.board.layer_count
            || board.nets.iter().zip(&self.board.nets).any(|(new, old)| {
                new.name != old.name || new.terminals.len() != old.terminals.len()
            })
        {
            return Err("incremental update needs the same nets and rules".into());
        }
        let previous: Vec<NetState> = std::mem::take(&mut self.nets);
        // Occupancy is rebuilt from the retained branches below.
        for map in &mut self.occupancy {
            map.fill(0);
        }
        for map in &mut self.tile_claimed {
            map.fill(0);
        }
        self.board = board.clone();
        self.searched = searched_classes(&self.board, &self.neck_of);
        self.statics = Arc::new(
            (0..self.board.classes.len())
                .map(|class| StaticMaps::build(&self.board, &self.grid, class, self.searched[class]))
                .collect(),
        );
        let layers = self.board.layer_count;
        let tiles = self.tiles_x * self.tiles_y;
        self.tile_routable = vec![vec![0u32; tiles]; self.board.classes.len() * layers];
        for class in 0..self.board.classes.len() {
            for layer in 0..layers {
                for (cell, value) in self.statics[class].trace[layer].iter().enumerate() {
                    if *value != crate::grid::BLOCKED {
                        self.tile_routable[class * layers + layer][(cell / self.grid.nx / TILE)
                            * self.tiles_x
                            + (cell % self.grid.nx) / TILE] += 1;
                    }
                }
            }
        }
        self.nets = (0..self.board.nets.len())
            .map(|net| self.prepare_net(net as NetId))
            .collect();
        self.guard = self.thermal_guards();
        self.covered = vec![Vec::new(); layers];
        self.tile_covered = vec![vec![0.0; tiles]; layers];
        self.mark_covered();
        self.mark_via_cells();
        self.build_hot();

        let mut rerouted = 0;
        for (net, old) in previous.into_iter().enumerate() {
            let id = net as NetId;
            let same_terminals = old.terminal_nodes == self.nets[net].terminal_nodes;
            let legal = same_terminals
                && old
                    .branches
                    .iter()
                    .all(|branch| self.branch_is_legal(id, branch));
            if legal && !old.branches.is_empty() {
                let state = &mut self.nets[net];
                state.branches = old.branches;
                state.connected = old.connected;
                state.complete = old.complete;
                state.reroutes = old.reroutes;
                state.fixed = old.fixed;
                self.stamp(id);
            } else if self.nets[net].routable {
                rerouted += 1;
            }
        }
        Ok(rerouted)
    }

    /// Routes whatever is unrouted or in conflict after `update`, then
    /// finishes as `run` does. Cheap when little changed. Without `polish`
    /// the per-net cleanup pass is skipped (for trials).
    pub fn reroute(&mut self, polish: bool) -> RoutingResult {
        let order = self.routing_order();
        self.fix_escapes(&order);
        self.route_skeletons(&order);
        let pending: Vec<NetId> = order
            .iter()
            .copied()
            .filter(|net| {
                let state = &self.nets[*net as usize];
                !state.complete || !self.conflicts(*net).is_empty()
            })
            .collect();
        // PCB_ROUTER_TIMING: where a reroute's time goes (layout moves).
        let timing = std::env::var_os("PCB_ROUTER_TIMING").is_some();
        let (started, pending_count, iterations_before) = (std::time::Instant::now(), pending.len(), self.iterations);
        // At the price of sharing the negotiation reached, as `resume`: a
        // price started low again undid what was settled (the layout's
        // polish turned eurorack's 21 open into 42, PolyKybd's 33 into
        // 108, without any deadline).
        let present = self.present_reached.max(self.config.present_factor as f32);
        self.negotiate_from(&order, pending, present);
        // The nets rerouted may cut a pour's fill where another net's plane
        // stub ends; negotiation refreshes the pours only from its third
        // iteration, which a reroute may not reach (USB_Cable_Tester: a
        // moved capacitor's signals ran over /SHLD's plane end, a dangling
        // track in KiCad). Pads cut off so are routed to the main piece.
        if self.config.pour_islands {
            let stranded: Vec<NetId> = order.iter().copied().filter(|net| self.refresh_pour(*net) > 0).collect();
            if !stranded.is_empty() {
                if self.config.verbose {
                    eprintln!("reroute: {} pour nets with pads cut off, routed again", stranded.len());
                }
                self.negotiate_from(&order, stranded, present);
            }
        }
        let negotiated = started.elapsed().as_secs_f64();
        let result = self.finish_polished(&order, polish);
        if timing {
            eprintln!(
                "reroute: {pending_count} pending, {} iterations, negotiation {negotiated:.1} s, finish {:.1} s",
                self.iterations - iterations_before,
                started.elapsed().as_secs_f64() - negotiated
            );
        }
        result
    }

    /// Routes one net with the router's own scratch (sequential callers).
    fn route_net_seq(&mut self, net: NetId, present: f32, hard: bool, window_growth: f64) {
        let mut scratch = std::mem::take(&mut self.scratch);
        let mut net_state = std::mem::take(&mut self.nets[net as usize]);
        // Fixed branches stay stamped through a rip-up; the search must
        // not take its own for another net's.
        if !net_state.stamped.is_empty() {
            self.mask_own(&mut scratch, net, &net_state);
        }
        self.route_net(
            &mut scratch,
            net,
            &mut net_state,
            present,
            hard,
            window_growth,
        );
        scratch.unmask_own();
        self.nets[net as usize] = net_state;
        self.scratch = scratch;
    }

    /// Where a map of `net`'s own class (0) or neck class (1) is marked in
    /// `own_mark`: `(which, kind)`, the kind being the layer (or the via
    /// map, or a diagonal map's layer and orientation) within that class.
    fn own_kind(&self, class: usize, map: usize) -> Option<(usize, usize)> {
        let layers = self.board.layer_count;
        let diagonal_base = self.board.classes.len() * (layers + 1);
        let (of_class, kind) = if map < diagonal_base {
            (map / (layers + 1), map % (layers + 1))
        } else {
            let relative = map - diagonal_base;
            (relative / (2 * layers), layers + 1 + relative % (2 * layers))
        };
        let which = if of_class == class {
            0
        } else if Some(of_class) == self.neck_of[class] {
            1
        } else {
            return None;
        };
        Some((which, kind))
    }

    /// The occupancy of a cell as `net`'s own search sees it: without the
    /// net's own stamps when they are still in place (`mask_own`).
    #[inline(always)]
    fn occupancy_seen(&self, scratch: &Scratch, class: usize, map: usize, cell: usize) -> u16 {
        let value = self.occupancy[map][cell];
        if !scratch.own_active || value == 0 {
            return value;
        }
        let Some((which, kind)) = self.own_kind(class, map) else {
            return value;
        };
        let kinds = 3 * self.board.layer_count + 1;
        let bit = (which * kinds + kind) * self.grid.cells() + cell;
        if scratch.own_mark[bit / 64] & (1 << (bit % 64)) != 0 {
            value - 1
        } else {
            value
        }
    }

    /// Marks the net's own stamps in `scratch.own_mark` so that its search
    /// subtracts them from the occupancy it sees (see `own_kind` for the
    /// layout). The caller calls `Scratch::unmask_own` afterwards.
    fn mask_own(&self, scratch: &mut Scratch, net: NetId, net_state: &NetState) {
        let layers = self.board.layer_count;
        let cells = self.grid.cells();
        let kinds = 3 * layers + 1;
        let words = (2 * kinds * cells).div_ceil(64);
        if scratch.own_mark.len() != words {
            scratch.own_mark = vec![0; words];
            scratch.own_set.clear();
        }
        scratch.unmask_own();
        scratch.own_active = true;
        let class = self.board.nets[net as usize].class;
        let tiles = self.tiles_x * self.tiles_y;
        scratch.own_claimed.clear();
        scratch.own_claimed.resize(layers * tiles, 0);
        let class_base = class * (layers + 1);
        for (map, cell) in &net_state.stamped {
            let map = *map as usize;
            // The corridor planner's tile fill: cells only this net claims.
            if map >= class_base && map < class_base + layers && self.occupancy[map][*cell as usize] == 1 {
                let tile = (*cell as usize / self.grid.nx / TILE) * self.tiles_x + (*cell as usize % self.grid.nx) / TILE;
                scratch.own_claimed[(map - class_base) * tiles + tile] += 1;
            }
            if let Some((which, kind)) = self.own_kind(class, map) {
                let bit = (which * kinds + kind) * cells + *cell as usize;
                scratch.own_mark[bit / 64] |= 1 << (bit % 64);
                scratch.own_set.push(bit);
            }
        }
    }

    /// Routes `batch` in parallel against the board as it is, every net
    /// ignoring its own old stamps, then replaces the old copper in order.
    fn route_jacobi(&mut self, batch: &[NetId], present: f32, growth: f64) {
        let states = self.grid.cells() * self.board.layer_count;
        let cells = self.grid.cells();
        let this: &Self = self;
        let routed: Vec<(NetId, NetState, [u64; 5])> = batch
            .par_iter()
            .map(|net| {
                with_thread_scratch(states, cells, |scratch| {
                    let before = scratch.counters();
                    let mut net_state = this.nets[*net as usize].clone();
                    let keep = this.unconflicted_branches(*net);
                    let mut keep = keep.into_iter();
                    net_state.branches.retain(|_| keep.next().unwrap());
                    net_state.connected.fill(false);
                    net_state.complete = false;
                    // The old stamps stay in the shared occupancy; mask them.
                    this.mask_own(scratch, *net, &net_state);
                    this.route_net(scratch, *net, &mut net_state, present, false, growth);
                    scratch.unmask_own();
                    let after = scratch.counters();
                    let mut delta = [0u64; 5];
                    for (index, value) in delta.iter_mut().enumerate() {
                        *value = after[index] - before[index];
                    }
                    (*net, net_state, delta)
                })
            })
            .collect();
        for (net, mut net_state, counters) in routed {
            self.unstamp(net);
            net_state.stamped = Vec::new();
            net_state.reroutes = self.nets[net as usize].reroutes + 1;
            self.nets[net as usize] = net_state;
            self.scratch.searches += counters[0];
            self.scratch.expansions += counters[1];
            self.scratch.open_searches += counters[2];
            self.scratch.open_expansions += counters[3];
            self.scratch.failed_searches += counters[4];
            let stamp_started = std::time::Instant::now();
            self.stamp(net);
            self.stamp_seconds += stamp_started.elapsed().as_secs_f64();
        }
    }

    /// Groups `pending` into batches of nets whose search windows (with a
    /// tile of margin) do not overlap, keeping the given order inside each
    /// batch. Pour nets and nets without a bounded window get a batch each.
    fn batches(&self, pending: &[NetId], growth: f64) -> Vec<(Vec<NetId>, bool)> {
        let full = (0, 0, self.grid.nx - 1, self.grid.ny - 1);
        let mut batches: Vec<(Vec<NetId>, Vec<(usize, usize, usize, usize)>)> = Vec::new();
        for net in pending {
            let window = self.window(*net, growth);
            let alone = window == full || !self.nets[*net as usize].plane.is_empty();
            let margin = TILE * 2;
            let rect = (
                window.0.saturating_sub(margin),
                window.1.saturating_sub(margin),
                window.2 + margin,
                window.3 + margin,
            );
            let overlaps = |other: &(usize, usize, usize, usize)| {
                rect.0 <= other.2 && rect.2 >= other.0 && rect.1 <= other.3 && rect.3 >= other.1
            };
            let slot = if alone {
                None
            } else {
                batches
                    .iter()
                    .position(|(members, rects)| members.len() < 64 && !rects.iter().any(overlaps))
            };
            match slot {
                Some(index) if !alone => {
                    batches[index].0.push(*net);
                    batches[index].1.push(rect);
                }
                _ => {
                    let rects = if alone { vec![full] } else { vec![rect] };
                    batches.push((vec![*net], rects));
                }
            }
        }
        // Nets left alone because their windows overlap everything are
        // still routed in parallel, Jacobi style: each against the copper
        // as it is now, its own included. Pour nets stay sequential.
        let mut result: Vec<(Vec<NetId>, bool)> = Vec::new();
        for (members, _) in batches {
            let single = members.len() == 1 && self.nets[members[0] as usize].plane.is_empty();
            if self.config.jacobi_batch > 1 && single {
                if let Some((group, true)) = result.last_mut()
                    && group.len() < self.config.jacobi_batch
                {
                    group.push(members[0]);
                    continue;
                }
                result.push((members, true));
            } else {
                result.push((members, false));
            }
        }
        result
    }

    /// Builds the tile graph and negotiates every net's plan on it (pour
    /// nets are left to the lattice).
    fn plan_globally(&mut self, order: &[NetId]) {
        let started = std::time::Instant::now();
        let layers = self.board.layer_count;
        let cells = self.grid.cells();
        // The reference class: the one most nets use.
        let mut counts = vec![0usize; self.board.classes.len()];
        for net in order {
            counts[self.board.nets[*net as usize].class] += 1;
        }
        let reference = (0..counts.len()).max_by_key(|&class| counts[class]).unwrap_or(0);
        let rules = self.board.classes[reference];
        let free: Vec<Vec<bool>> = (0..layers)
            .map(|layer| self.statics[reference].trace[layer].iter().map(|&value| value == crate::grid::FREE).collect())
            .collect();
        let via_free: Vec<bool> = self.statics[reference].via_blocked.iter().map(|&blocked| !blocked).collect();
        let tiles_x = self.tiles_x;
        let tiles = self.tiles_x * self.tiles_y;
        let tile_of = |cell: u32| (cell as usize / self.grid.nx / TILE) * tiles_x + (cell as usize % self.grid.nx) / TILE;
        let nets: Vec<crate::global::GlobalNet> = (0..self.board.nets.len())
            .map(|net| {
                let state = &self.nets[net];
                if !state.routable || !state.plane.is_empty() {
                    return crate::global::GlobalNet::default();
                }
                let class = self.board.classes[self.board.nets[net].class];
                crate::global::GlobalNet {
                    terminals: state
                        .terminal_nodes
                        .iter()
                        .map(|nodes| {
                            let mut list: Vec<usize> = nodes.iter().map(|node| node.layer as usize * tiles + tile_of(node.cell)).collect();
                            list.sort_unstable();
                            list.dedup();
                            list
                        })
                        .collect(),
                    demand: ((class.trace_width + class.clearance) / (rules.trace_width + rules.clearance)) as f32,
                    ..Default::default()
                }
            })
            .collect();
        let inputs = crate::global::Inputs {
            nx: self.grid.nx,
            ny: self.grid.ny,
            tile: TILE,
            pitch: self.grid.pitch,
            layers,
            free: free.iter().map(Vec::as_slice).collect(),
            via_free: &via_free,
            room: rules.trace_width + rules.clearance,
            via_cost: self.config.via_cost,
        };
        let mut plan = crate::global::GlobalPlan::new(&inputs, nets);
        let planned: Vec<usize> = order
            .iter()
            .map(|net| *net as usize)
            .filter(|&net| plan.nets[net].terminals.iter().filter(|terminal| !terminal.is_empty()).count() > 1)
            .collect();
        let overflow = plan.negotiate(&planned, 40);
        let _ = cells;
        if self.config.verbose {
            eprintln!(
                "global plan: {} nets on {} x {} tiles x {layers} layers, overflow {overflow:.1} tracks, {:.2}s",
                planned.len(),
                self.tiles_x,
                self.tiles_y,
                started.elapsed().as_secs_f64()
            );
        }
        self.global = Some(plan);
    }

    /// Teaches the plan where `conflicted` nets collide on the lattice and
    /// plans them again.
    fn replan(&mut self, conflicted: &[NetId]) {
        let tiles_x = self.tiles_x;
        let nx = self.grid.nx;
        let mut hits: Vec<(usize, usize)> = Vec::new();
        for net in conflicted {
            for (layer, cell) in self.conflicts(*net) {
                let cell = cell as usize;
                hits.push((layer, (cell / nx / TILE) * tiles_x + (cell % nx) / TILE));
            }
        }
        hits.sort_unstable();
        hits.dedup();
        let Some(plan) = self.global.as_mut() else { return };
        for (layer, tile) in hits {
            if layer < plan.layers {
                plan.learn(layer, tile, 0.5);
            }
        }
        for net in conflicted {
            let net = *net as usize;
            if plan.nets[net].terminals.is_empty() {
                continue;
            }
            plan.rip_up(net);
            plan.route(net, 4.0);
        }
    }

    /// Nets in routing order: pour nets first (their connections are local
    /// stubs and vias that must claim the sites next to their pads), then
    /// small nets by span, then the large ones.
    fn routing_order(&self) -> Vec<NetId> {
        let mut order: Vec<NetId> = (0..self.board.nets.len() as NetId)
            .filter(|net| self.nets[*net as usize].routable)
            .collect();
        let span = |router: &Self, net: NetId| {
            let (x0, y0, x1, y1) = router.window(net, 0.0);
            (x1 - x0) + (y1 - y0)
        };
        let seed = self.config.seed.or_else(experiment_seed);
        let mut state = seed.unwrap_or(0) ^ 0x5DEE_CE66;
        let jitter: Vec<f64> = (0..self.board.nets.len())
            .map(|_| match seed {
                Some(_) => 0.6 + 0.8 * unit_noise(&mut state),
                None => 1.0,
            })
            .collect();
        order.sort_by_key(|net| {
            (
                self.nets[*net as usize].plane.is_empty(),
                self.board.nets[*net as usize].terminals.len() > 8,
                (span(self, *net) as f64 * jitter[*net as usize]) as u64,
                *net,
            )
        });
        order
    }

    /// Routes everything from the current state and produces the result.
    pub fn run(mut self) -> RoutingResult {
        self.run_in_place()
    }

    /// Like `run`, keeping the router for later `update` calls.
    pub fn run_in_place(&mut self) -> RoutingResult {
        self.run_in_place_polished(true)
    }

    /// `run_in_place`; without `polish` the clean-up and the via reduction
    /// are skipped (for trials judged on what stays open, which the polish
    /// does not change; a later `reroute(true)` polishes).
    pub fn run_in_place_polished(&mut self, polish: bool) -> RoutingResult {
        let order = self.routing_order();
        let started = std::time::Instant::now();
        self.fix_escapes(&order);
        self.route_skeletons(&order);
        self.fix_plane_stubs(&order);
        if self.config.global_routing {
            self.plan_globally(&order);
        }
        self.profile.prepare += started.elapsed().as_secs_f64();
        self.negotiate(&order, order.clone());
        self.finish_polished(&order, polish)
    }

    /// The first part of `run_in_place`: prepares the nets and negotiates
    /// for at most `seconds`, keeping every bit of state, so that `resume`
    /// can go on from here. Returns how many nets are still conflicted or
    /// incomplete (the measure a caller ranks probes by).
    pub fn probe(&mut self, seconds: f64) -> usize {
        let order = self.routing_order();
        let started = std::time::Instant::now();
        self.fix_escapes(&order);
        self.route_skeletons(&order);
        self.fix_plane_stubs(&order);
        if self.config.global_routing {
            self.plan_globally(&order);
        }
        self.profile.prepare += started.elapsed().as_secs_f64();
        let limit = (self.config.negotiation_seconds, self.config.negotiation_expansions);
        self.config.negotiation_seconds = seconds * GUARD;
        self.config.negotiation_expansions = (seconds * EXPANSIONS_PER_SECOND) as u64;
        self.negotiate(&order, order.clone());
        (self.config.negotiation_seconds, self.config.negotiation_expansions) = limit;
        self.unfinished()
    }

    /// After `probe`: negotiates on for at most `seconds` where the probe
    /// stopped (its history and its price of sharing: starting the price
    /// low again undid the probe's progress, and a board a fresh run
    /// completes stayed at 20 conflicted nets), then finishes and polishes
    /// as `run` does.
    pub fn resume(&mut self, seconds: f64) -> RoutingResult {
        self.resume_polished(seconds, true)
    }

    /// The clean-up, the via reduction and the pour stitching on the
    /// routes as they are, without negotiating again (a renegotiation of
    /// the open nets at a short patience left jetson-nano with 28 open
    /// where it had 22).
    pub fn polish(&mut self) -> RoutingResult {
        let order = self.routing_order();
        self.finish_polished(&order, true)
    }

    /// `resume`; without `polish` as in `run_in_place_polished`.
    pub fn resume_polished(&mut self, seconds: f64, polish: bool) -> RoutingResult {
        let limit = (self.config.negotiation_seconds, self.config.negotiation_expansions);
        self.config.negotiation_seconds = seconds * GUARD;
        self.config.negotiation_expansions = (seconds * EXPANSIONS_PER_SECOND) as u64;
        self.hopeless = false;
        let order = self.routing_order();
        let pending: Vec<NetId> = order
            .iter()
            .copied()
            .filter(|net| {
                let state = &self.nets[*net as usize];
                !state.complete || !self.conflicts(*net).is_empty()
            })
            .collect();
        let present = self.present_reached.max(self.config.present_factor as f32);
        self.negotiate_from(&order, pending, present);
        let result = self.finish_polished(&order, polish);
        (self.config.negotiation_seconds, self.config.negotiation_expansions) = limit;
        result
    }

    /// The router's configuration, to change between runs (the layout
    /// trials at a short patience, the final resume at the full one).
    pub fn config_mut(&mut self) -> &mut Config {
        &mut self.config
    }

    /// Search expansions so far (all phases, all threads).
    pub fn expansions(&self) -> u64 {
        self.scratch.expansions
    }

    /// The work done so far, in seconds of an idle machine (search
    /// expansions at `EXPANSIONS_PER_SECOND`): budgets measured in it do
    /// not depend on how busy the machine is.
    fn work_seconds(&self) -> f64 {
        self.scratch.expansions as f64 / EXPANSIONS_PER_SECOND
    }

    fn phase_mark(&self) -> PhaseMark {
        (
            self.scratch.expansions as f64 / EXPANSIONS_PER_SECOND,
            self.scratch.pour_units.load(std::sync::atomic::Ordering::Relaxed),
            self.scratch.pour_nanos.load(std::sync::atomic::Ordering::Relaxed),
            std::time::Instant::now(),
        )
    }

    /// Adds what was done since `mark` to the profile's `phase`.
    fn phase_charge(&mut self, phase: usize, mark: PhaseMark) {
        let now = self.phase_mark();
        self.profile.phase_search[phase] += now.0 - mark.0;
        self.profile.phase_pour_units[phase] += now.1 - mark.1;
        self.profile.phase_pour_wall[phase] += (now.2 - mark.2) as f64 / 1.0e9;
        self.profile.phase_wall[phase] += mark.3.elapsed().as_secs_f64();
    }

    /// The pour analyses' work so far, in seconds of an idle machine
    /// (their lattice nodes at `POUR_NODES_PER_SECOND`).
    fn pour_work_seconds(&self) -> f64 {
        self.scratch.pour_units.load(std::sync::atomic::Ordering::Relaxed) as f64 / POUR_NODES_PER_SECOND
    }

    /// The work the phase budgets (`clock`, `spent`) count: the search's
    /// and the pour analyses'. A clean-up on a pour board spends much of
    /// its time checking that a batch strands no pour pad (Sisu: 21-35 s
    /// of a 61-66 s budget); counted as search alone, its budget ran on in
    /// wall clock with the load, and under load the wall guard ended it.
    fn budget_work(&self) -> f64 {
        self.work_seconds() + self.pour_work_seconds()
    }

    /// A budget's start: the work and the wall clock at this point.
    fn clock(&self) -> (f64, std::time::Instant) {
        (self.budget_work(), std::time::Instant::now())
    }

    /// Whether `budget` (work seconds) is spent since `clock`; the wall
    /// clock stops it too, at `GUARD` times the budget.
    fn spent(&self, clock: &(f64, std::time::Instant), budget: f64) -> bool {
        if self.budget_work() - clock.0 > budget || self.past_work_deadline() {
            return true;
        }
        if self.wall_guard() && clock.1.elapsed().as_secs_f64() > budget * GUARD {
            note_wall_cut(
                "budget guard",
                format_args!("{:.0} s of wall clock for a budget of {budget:.0} s of work, {:.0} s of it done", clock.1.elapsed().as_secs_f64(), self.budget_work() - clock.0),
            );
            return true;
        }
        self.past_wall_deadline()
    }

    /// Whether the wall clock guards the work budgets at `GUARD` times
    /// them: only without a caller's deadline (route mode), which is
    /// otherwise the one wall guard, so that the work alone decides the
    /// routes; and not under `PCB_ROUTER_NO_WALL_GUARD` (`wall_guard`).
    fn wall_guard(&self) -> bool {
        self.config.deadline.is_none() && wall_guard()
    }

    /// Whether the caller's deadline has passed: its work
    /// (`deadline_work`) or, on a machine too busy for it, its wall clock.
    pub fn past_deadline(&self) -> bool {
        self.past_work_deadline() || self.past_wall_deadline()
    }

    /// Whether this router's work has reached the caller's `deadline_work`.
    fn past_work_deadline(&self) -> bool {
        self.config.deadline_work.is_some_and(|limit| self.work_seconds() >= limit)
    }

    /// Whether the caller's wall-clock deadline has passed (noted as a cut:
    /// the work deadline, when there is one, did not stop it first).
    fn past_wall_deadline(&self) -> bool {
        let past = self.config.deadline.is_some_and(|deadline| std::time::Instant::now() >= deadline);
        if past {
            note_wall_cut("deadline", format_args!("{:.0} s of this router's work done", self.work_seconds()));
        }
        past
    }

    /// The work this router has done, in seconds of an idle machine
    /// (search expansions at `EXPANSIONS_PER_SECOND`).
    pub fn work(&self) -> f64 {
        self.work_seconds()
    }

    /// Routable nets in conflict or not complete, plus the pads of pour
    /// nets that their pour does not reach yet (a pour net counts as
    /// complete while its pads touch the pour anywhere; whether the pieces
    /// they touch hang together only shows when it is stitched, and a probe
    /// that ignores it ranks a rung that ends with 120 pads stranded first).
    pub fn unfinished(&self) -> usize {
        let mut count = 0;
        for net in 0..self.nets.len() as NetId {
            let state = &self.nets[net as usize];
            if !state.routable {
                continue;
            }
            if !state.complete || !self.conflicts(net).is_empty() {
                count += 1;
            }
            if !state.plane.is_empty() {
                let (pours, mut parent, main) = self.analyze_pours(net);
                let terminal_base = pours.pieces + 1;
                count += (0..state.terminal_nodes.len())
                    .filter(|terminal| find(&mut parent, terminal_base + terminal) != main)
                    .count();
            }
        }
        count
    }

    /// `finish`, skipping the clean-up and the via reduction without
    /// `polish`.
    fn finish_polished(&mut self, order: &[NetId], polish: bool) -> RoutingResult {
        let (cleanup, rounds) = (self.config.cleanup_passes, self.config.via_reduction_rounds);
        if !polish {
            self.config.cleanup_passes = 0;
            self.config.via_reduction_rounds = 0;
        }
        let result = self.finish(order);
        self.config.cleanup_passes = cleanup;
        self.config.via_reduction_rounds = rounds;
        result
    }

    /// Registers a hook called after every negotiation iteration with the
    /// iteration count and the routing as it stands (see `snapshot`).
    pub fn set_frame_hook(&mut self, hook: Arc<dyn Fn(usize, &RoutingResult) + Send + Sync>) {
        self.frame_hook = Some(hook);
    }

    /// The routing as it stands: every net materialized as it is, conflicts
    /// and all, without cleanup, stub checks or congestion.
    pub fn snapshot(&self) -> RoutingResult {
        let routes = (0..self.board.nets.len() as NetId)
            .map(|net| self.materialize(net).0)
            .collect();
        RoutingResult {
            grid: self.grid.clone(),
            congestion: Vec::new(),
            routes,
            status: self.statuses(),
            iterations: self.iterations,
            expansions: self.scratch.expansions,
            searches: self.scratch.searches,
            diagnostics: Diagnostics::default(),
        }
    }

    /// Dead pads, the last conflicted nets and the hottest tiles.
    fn diagnostics(&self) -> Diagnostics {
        let mut dead_pads = Vec::new();
        for (net, state) in self.nets.iter().enumerate() {
            let description = &self.board.nets[net];
            let class = self.board.classes[description.class];
            for (index, nodes) in state.terminal_nodes.iter().enumerate() {
                if !nodes.is_empty() || state.on_plane.get(index).copied().unwrap_or(false) {
                    continue;
                }
                let terminal = &description.terminals[index];
                let pad = &self.board.obstacles[terminal.pad];
                let reach = pad
                    .shape
                    .aabb()
                    .inflated(class.trace_width / 2.0 + class.clearance + self.grid.pitch);
                let mut near: Vec<String> = self
                    .board
                    .obstacles
                    .iter()
                    .filter(|other| {
                        other.net != Some(net as NetId)
                            && other.layers & terminal.layers != 0
                            && other.shape.aabb().intersects(reach)
                    })
                    .map(|other| other.label.clone())
                    .collect();
                near.sort();
                near.dedup();
                near.truncate(8);
                dead_pads.push(DeadPad {
                    net: net as NetId,
                    label: terminal.label.clone(),
                    anchor: terminal.anchor,
                    near,
                });
            }
        }
        let mut hot_spots: Vec<HotSpot> = Vec::new();
        for (layer, history) in self.tile_history.iter().enumerate() {
            for (tile, value) in history.iter().enumerate() {
                if *value > 0.0 {
                    let x = (tile % self.tiles_x) as f64 * TILE as f64 + TILE as f64 / 2.0;
                    let y = (tile / self.tiles_x) as f64 * TILE as f64 + TILE as f64 / 2.0;
                    hot_spots.push(HotSpot {
                        layer,
                        center: [
                            self.grid.origin[0] + x * self.grid.pitch,
                            self.grid.origin[1] + y * self.grid.pitch,
                        ],
                        history: *value,
                    });
                }
            }
        }
        hot_spots.sort_by(|a, b| b.history.total_cmp(&a.history));
        hot_spots.truncate(10);
        Diagnostics {
            dead_pads,
            conflicted: self.last_conflicted.clone(),
            hot_spots,
        }
    }

    fn statuses(&self) -> Vec<NetStatus> {
        (0..self.board.nets.len())
            .map(|net| {
                let state = &self.nets[net];
                if self.board.nets[net].terminals.len() < 2 && state.plane.is_empty() {
                    NetStatus::Trivial
                } else if !state.routable {
                    NetStatus::Unreachable
                } else if state.complete {
                    NetStatus::Routed
                } else {
                    NetStatus::Partial {
                        unconnected_terminals: state
                            .connected
                            .iter()
                            .filter(|connected| !**connected)
                            .count(),
                    }
                }
            })
            .collect()
    }

    /// Routes every pour net that has no copper yet as a plain tree, biased
    /// onto the layer its pours cover most, against all copper already on
    /// the board. The tree stays fixed for the rest of the run.
    /// Fixes a straight escape stub outward for every fine-pitch pad of a
    /// dense row (see `Config::escape_stub_mm`). The stubs are leading
    /// fixed branches, like a plane skeleton; a net that has branches
    /// already (kept through an update) is left alone.
    fn fix_escapes(&mut self, order: &[NetId]) {
        let length = self.config.escape_stub_mm;
        if length <= 0.0 {
            return;
        }
        let pitch = self.grid.pitch;
        let steps = (length / pitch).round() as i64;
        let nx = self.grid.nx as i64;
        let ny = self.grid.ny as i64;
        // Fine-pitch pads on one layer, with the direction of their long
        // side: (net, terminal, layer, axis, aabb).
        let mut narrow: Vec<(NetId, usize, usize, usize, crate::geometry::Aabb)> = Vec::new();
        for net in order {
            let description = &self.board.nets[*net as usize];
            let rules = self.board.classes[description.class];
            let state = &self.nets[*net as usize];
            if !state.routable || !state.branches.is_empty() {
                continue;
            }
            for (index, terminal) in description.terminals.iter().enumerate() {
                if terminal.layers.count_ones() != 1 || state.terminal_nodes[index].is_empty() {
                    continue;
                }
                let bounds = self.board.obstacles[terminal.pad].shape.aabb();
                let size = [bounds.maximum[0] - bounds.minimum[0], bounds.maximum[1] - bounds.minimum[1]];
                let short = size[0].min(size[1]);
                if short >= rules.trace_width + rules.clearance || size[0].max(size[1]) < 1.5 * short {
                    continue;
                }
                let axis = usize::from(size[1] > size[0]);
                narrow.push((*net, index, terminal.layers.trailing_zeros() as usize, axis, bounds));
            }
        }
        // A row: at least four such pads of the same direction whose
        // centres line up across the long side and follow each other along
        // it within three pad pitches.
        let mut in_row = vec![false; narrow.len()];
        for (i, (_, _, layer, axis, bounds)) in narrow.iter().enumerate() {
            let across = 1 - *axis;
            let centre = [(bounds.minimum[0] + bounds.maximum[0]) / 2.0, (bounds.minimum[1] + bounds.maximum[1]) / 2.0];
            let short = (bounds.maximum[across] - bounds.minimum[across]).max(1.0e-6);
            let mut alike = 0;
            for (j, (_, _, other_layer, other_axis, other)) in narrow.iter().enumerate() {
                if i == j || other_layer != layer || other_axis != axis {
                    continue;
                }
                let other_centre = [(other.minimum[0] + other.maximum[0]) / 2.0, (other.minimum[1] + other.maximum[1]) / 2.0];
                if (other_centre[*axis] - centre[*axis]).abs() < short
                    && (other_centre[across] - centre[across]).abs() <= 3.0 * (2.0 * short + 0.05)
                {
                    alike += 1;
                }
            }
            in_row[i] = alike >= 3;
            if std::env::var_os("PCB_ROUTER_DEBUG").is_some() && !in_row[i] {
                eprintln!("  narrow pad {} not in a row ({alike} alike)", self.board.nets[narrow[i].0 as usize].terminals[narrow[i].1].label);
            }
        }
        if std::env::var_os("PCB_ROUTER_DEBUG").is_some() {
            eprintln!("  {} narrow pads, {} in rows", narrow.len(), in_row.iter().filter(|r| **r).count());
        }
        let mut fixed = 0;
        let mut by_net: HashMap<NetId, Vec<Branch>> = HashMap::new();
        for (i, (net, index, layer, axis, bounds)) in narrow.iter().enumerate() {
            if !in_row[i] {
                continue;
            }
            let net = *net;
            let nodes = &self.nets[net as usize].terminal_nodes[*index];
            let across = 1 - *axis;
            let middle = (bounds.minimum[across] + bounds.maximum[across]) / 2.0;
            let centre = (bounds.minimum[*axis] + bounds.maximum[*axis]) / 2.0;
            // Outward is the way with more room along the long side, from
            // the pad node nearest that end (a pad too narrow for a node
            // has escape nodes outside it: the straightest one that way).
            // Away from the part, when the part's other pads say where it
            // is (a QFN's inside is free of statics too).
            let (mut sum, mut count) = (0.0, 0usize);
            for (_, _, _, _, other) in &narrow {
                let other_centre = [(other.minimum[0] + other.maximum[0]) / 2.0, (other.minimum[1] + other.maximum[1]) / 2.0];
                if (other_centre[0] - (bounds.minimum[0] + bounds.maximum[0]) / 2.0).abs() < 8.0
                    && (other_centre[1] - (bounds.minimum[1] + bounds.maximum[1]) / 2.0).abs() < 8.0
                {
                    sum += other_centre[*axis];
                    count += 1;
                }
            }
            let offset = centre - sum / count.max(1) as f64;
            let signs: &[i64] = if offset > 1.0 { &[1] } else if offset < -1.0 { &[-1] } else { &[-1, 1] };
            let mut best: Option<(i64, Vec<Node>)> = None;
            for sign in signs.iter().copied() {
                let end = if sign > 0 { bounds.maximum[*axis] } else { bounds.minimum[*axis] };
                let Some(start) = nodes
                    .iter()
                    .filter(|node| node.layer as usize == *layer)
                    .filter(|node| {
                        let at = self.grid.center_of(node.cell as usize);
                        !self.nets[net as usize].escapes.contains_key(node) || (at[*axis] - centre) * sign as f64 > 0.0
                    })
                    .min_by(|a, b| {
                        let key = |node: &Node| {
                            let at = self.grid.center_of(node.cell as usize);
                            ((at[across] - middle).abs() * 4.0 + (at[*axis] - end).abs(), 0)
                        };
                        key(a).partial_cmp(&key(b)).unwrap()
                    })
                else {
                    continue;
                };
                let mut path = vec![*start];
                let (mut x, mut y) = (start.cell as i64 % nx, start.cell as i64 / nx);
                let inside = |cell: i64| {
                    let at = self.grid.center_of(cell as usize);
                    at[*axis] >= bounds.minimum[*axis] - 1.0e-6 && at[*axis] <= bounds.maximum[*axis] + 1.0e-6
                };
                let mut beyond = 0;
                while beyond < steps {
                    if *axis == 0 { x += sign } else { y += sign }
                    if x < 0 || y < 0 || x >= nx || y >= ny {
                        break;
                    }
                    let cell = y * nx + x;
                    let class = self.node_class(net, cell as u32);
                    if !self.statics[class].trace_allowed(*layer, cell as usize, net)
                        || self.occupancy[self.map_index(class, *layer)][cell as usize] > 0
                    {
                        break;
                    }
                    path.push(Node { layer: *layer as u8, cell: cell as u32 });
                    if !inside(cell) {
                        beyond += 1;
                    }
                }
                if beyond >= 3 && best.as_ref().is_none_or(|(length, _)| beyond > *length) {
                    best = Some((beyond, path));
                }
            }
            if let Some((_, path)) = best {
                if std::env::var_os("PCB_ROUTER_DEBUG").is_some() {
                    let from = self.grid.center_of(path[0].cell as usize);
                    let to = self.grid.center_of(path.last().unwrap().cell as usize);
                    eprintln!(
                        "  escape stub {} ({}): ({:.2},{:.2}) -> ({:.2},{:.2}) on layer {layer}",
                        self.board.nets[net as usize].terminals[*index].label,
                        self.board.nets[net as usize].name,
                        from[0], from[1], to[0], to[1]
                    );
                }
                by_net.entry(net).or_default().push(Branch {
                    nodes: path,
                    start_terminal: *index as u16,
                    end_terminal: FREE_END,
                });
                fixed += 1;
                // Stamp now so the next pad's stub sees this one.
                let branches = by_net.get(&net).unwrap().clone();
                let state = &mut self.nets[net as usize];
                state.branches = branches;
                state.fixed = state.branches.len();
                self.stamp(net);
            }
        }
        if self.config.verbose && fixed > 0 {
            eprintln!("escape stubs fixed for {fixed} fine-pitch pads of {} nets", by_net.len());
        }
    }

    fn route_skeletons(&mut self, order: &[NetId]) {
        if !self.config.plane_skeleton {
            return;
        }
        let layers = self.board.layer_count;
        for net in order {
            let state = &self.nets[*net as usize];
            if state.plane.is_empty()
                || state.fixed > 0
                || !state.branches.is_empty()
                || self.board.nets[*net as usize].terminals.len() < 2
            {
                continue;
            }
            let started = std::time::Instant::now();
            let layer = (0..layers)
                .max_by_key(|layer| state.plane[*layer].iter().filter(|inside| **inside).count())
                .unwrap_or(0);
            self.layer_bias = vec![self.config.skeleton_bias as f32; layers];
            self.layer_bias[layer] = 1.0;
            let mut scratch = std::mem::take(&mut self.scratch);
            let mut net_state = std::mem::take(&mut self.nets[*net as usize]);
            let plane = std::mem::take(&mut net_state.plane);
            self.route_net(&mut scratch, *net, &mut net_state, 0.0, true, 4.0);
            net_state.plane = plane;
            net_state.fixed = net_state.branches.len();
            net_state.blocked = false;
            self.nets[*net as usize] = net_state;
            self.scratch = scratch;
            self.layer_bias.clear();
            self.stamp(*net);
            if self.config.verbose {
                let state = &self.nets[*net as usize];
                let vias = state
                    .branches
                    .iter()
                    .map(|branch| {
                        branch
                            .nodes
                            .windows(2)
                            .filter(|pair| pair[0].cell == pair[1].cell)
                            .count()
                    })
                    .sum::<usize>();
                eprintln!(
                    "skeleton {} on layer {layer}: {} branches, {vias} vias, complete {}, {:.2}s",
                    self.board.nets[*net as usize].name,
                    state.fixed,
                    state.complete,
                    started.elapsed().as_secs_f64()
                );
            }
        }
    }

    /// Connects every pad of a pour net to its plane on the empty board (a
    /// stub and a via next to the pad, as a designer places them) and keeps
    /// those connections fixed: signals route around them instead of
    /// pushing them off the via sites next to the pads.
    fn fix_plane_stubs(&mut self, order: &[NetId]) {
        if !self.config.fixed_plane_stubs {
            return;
        }
        let started = std::time::Instant::now();
        let pour_nets: Vec<NetId> = order
            .iter()
            .copied()
            .filter(|net| {
                let state = &self.nets[*net as usize];
                !state.plane.is_empty() && state.fixed == 0 && state.branches.is_empty()
            })
            .collect();
        if pour_nets.is_empty() {
            return;
        }
        // On the empty board every pad touches its layer's pour; the pours
        // on the outer layers are what the signals cut to pieces later. So
        // only the inner planes count here: a pad on an outer layer has to
        // reach one through a via, as a designer places one next to every
        // ground pad. Boards without an inner plane are left alone.
        let layers = self.board.layer_count;
        let mut saved = Vec::new();
        for net in &pour_nets {
            let state = &mut self.nets[*net as usize];
            let inner = (1..layers.saturating_sub(1)).any(|layer| !state.plane[layer].is_empty());
            if std::env::var_os("PCB_ROUTER_DEBUG").is_some() {
                let covered: Vec<usize> = (0..layers).filter(|layer| !state.plane[*layer].is_empty()).collect();
                eprintln!("  plane stubs {}: pour layers {covered:?}, inner {inner}", self.board.nets[*net as usize].name);
            }
            if !inner {
                continue;
            }
            saved.push((*net, state.plane.clone(), state.on_plane.clone()));
            state.plane[0] = Vec::new();
            state.plane[layers - 1] = Vec::new();
            for (index, nodes) in state.terminal_nodes.iter().enumerate() {
                state.on_plane[index] = nodes.iter().any(|node| {
                    state.plane.get(node.layer as usize).is_some_and(|mask| !mask.is_empty() && mask[node.cell as usize])
                });
            }
        }
        if saved.is_empty() {
            return;
        }
        let pour_nets: Vec<NetId> = saved.iter().map(|(net, _, _)| *net).collect();
        // The pour nets negotiate among themselves (their vias compete for
        // the same spots next to alternating pads), then stay.
        // At most a minute: of work where the wall clock guards (route
        // mode; a minute of wall clock made it depend on the load), as
        // before under a caller's deadline (the layout: stall alone).
        let limits = (self.config.negotiation_seconds, self.config.negotiation_expansions);
        if self.wall_guard() {
            let minute = (60.0 * EXPANSIONS_PER_SECOND) as u64;
            self.config.negotiation_expansions =
                if limits.1 == 0 { minute } else { limits.1.min(minute) };
            self.config.negotiation_seconds = limits.0.min(60.0 * GUARD);
        } else {
            self.config.negotiation_seconds = limits.0.min(60.0);
        }
        self.negotiate(&pour_nets, pour_nets.clone());
        (self.config.negotiation_seconds, self.config.negotiation_expansions) = limits;
        for (net, plane, on_plane) in saved {
            let state = &mut self.nets[net as usize];
            state.plane = plane;
            state.on_plane = on_plane;
        }
        for net in &pour_nets {
            // Only what is in conflict with nothing stays fixed: the
            // branches still contested (ColdFire's alternating GND and
            // +3.3V pads, whose vias compete for the same spots) go, and
            // those pads are left to the stitching at the end.
            let keep = self.unconflicted_branches(*net);
            let mut keep = keep.into_iter();
            let state = &mut self.nets[*net as usize];
            state.branches.retain(|_| keep.next().unwrap());
            state.fixed = state.branches.len();
            state.blocked = false;
            state.complete = false;
            self.stamp(*net);
            let state = &mut self.nets[*net as usize];
            if self.config.verbose {
                eprintln!(
                    "plane stubs {}: {} fixed branches, {} of {} terminals connected, {:.2}s",
                    self.board.nets[*net as usize].name,
                    state.fixed,
                    state.connected.iter().filter(|done| **done).count(),
                    state.connected.len(),
                    started.elapsed().as_secs_f64()
                );
            }
        }
    }

    /// Negotiated congestion over `pending` (which grows to whatever those
    /// nets conflict with) until the board is conflict free or stalls.
    fn negotiate(&mut self, order: &[NetId], pending: Vec<NetId>) {
        self.negotiate_from(order, pending, self.config.present_factor as f32);
    }

    /// `negotiate`, with sharing priced at `present` from the start.
    /// Routes the batches of one negotiation iteration in turn, with the
    /// result of routing them one after the other, but on all threads:
    /// the nets of the next few batches are routed speculatively side by
    /// side against the board as it is; then, batch by batch in order, a
    /// net's speculative route is kept when no cell an earlier batch of the
    /// window changed (where a net's stamps came or went) lies where the
    /// net's routing read the board (its searches' expanded nodes and their
    /// neighbours, its sources and targets with their escape stubs, its own
    /// copper for the conflict check, the corridor planner's tiles), and a
    /// batch with any net that read changed cells is routed again as
    /// usual. A route is a function of what it read, so the result is the
    /// one of routing in turn; only the work of kept and rerouted nets
    /// counts. `PCB_ROUTER_SPECULATION_CHECK` routes every batch in turn
    /// and counts kept routes that differ (none may).
    fn route_batches(&mut self, batches: &[(Vec<NetId>, bool)], present: f32, growth: f64) {
        let threads = rayon::current_num_threads();
        let paths = self.config.speculate_paths || std::env::var_os("PCB_ROUTER_SPECULATE_PATHS").is_some();
        let speculate = self.config.parallel
            && (self.config.speculate || paths)
            && (threads > 1 || paths)
            && std::env::var_os("PCB_ROUTER_NO_SPECULATION").is_none();
        let check = std::env::var_os("PCB_ROUTER_SPECULATION_CHECK").is_some() && !paths;
        let window = if paths { 16 } else { 2 * threads };
        let nx = self.grid.nx;
        let ny = self.grid.ny;
        let tiles = self.tiles_x * self.tiles_y;
        let blocks_x = nx.div_ceil(1 << READ_BLOCK_SHIFT);
        let blocks_y = ny.div_ceil(1 << READ_BLOCK_SHIFT);
        let mut dirty_blocks = vec![false; if speculate { blocks_x * blocks_y } else { 0 }];
        let mut dirty_tiles = vec![false; if speculate { tiles } else { 0 }];
        let mut dirty_cells = vec![false; if speculate && paths { self.grid.cells() } else { 0 }];
        let mut position = 0;
        while position < batches.len() {
            if !speculate || batches[position].1 {
                self.route_batch(&batches[position].0, batches[position].1, present, growth);
                position += 1;
                continue;
            }
            // The window: the next batches, up to twice as many nets as
            // there are threads (a later net is more likely to read what an
            // earlier one changed).
            let mut end = position;
            let mut count = 0;
            while end < batches.len() && !batches[end].1 && count < window {
                count += batches[end].0.len();
                end += 1;
            }
            if count <= 1 {
                self.route_batch(&batches[position].0, false, present, growth);
                position += 1;
                continue;
            }
            let started = std::time::Instant::now();
            let nets: Vec<NetId> = batches[position..end].iter().flat_map(|(members, _)| members.iter().copied()).collect();
            let states = self.grid.cells() * self.board.layer_count;
            let cells = self.grid.cells();
            let this: &Self = self;
            let speculative: Vec<(NetState, [u64; 5], Vec<u32>, Vec<u32>, Vec<u32>)> = nets
                .par_iter()
                .map(|net| {
                    with_thread_scratch(states, cells, |scratch| {
                        scratch.read.start(nx, ny, tiles);
                        let before = scratch.counters();
                        let mut net_state = this.nets[*net as usize].clone();
                        // The conflict check reads the board at the net's
                        // own copper (`rip_up_conflicted`).
                        for branch in &net_state.branches {
                            for node in &branch.nodes {
                                scratch.read.note_cell(node.cell, nx);
                            }
                        }
                        for (escape_cells, _, _) in net_state.escapes.values() {
                            for cell in escape_cells {
                                scratch.read.note_cell(*cell, nx);
                            }
                        }
                        let keep = this.unconflicted_branches(*net);
                        let mut keep = keep.into_iter();
                        net_state.branches.retain(|_| keep.next().unwrap());
                        let kept = net_state.branches.len();
                        net_state.connected.fill(false);
                        net_state.complete = false;
                        // The old stamps stay on the board; masked, the
                        // search sees what it would see with the conflicted
                        // ones ripped up and the kept ones masked.
                        if !net_state.stamped.is_empty() {
                            this.mask_own(scratch, *net, &net_state);
                        }
                        this.route_net(scratch, *net, &mut net_state, present, false, growth);
                        // The cells the new branches run through.
                        let mut path_blocks: Vec<u32> = net_state.branches[kept.min(net_state.branches.len())..]
                            .iter()
                            .flat_map(|branch| &branch.nodes)
                            .map(|node| node.cell)
                            .collect();
                        path_blocks.sort_unstable();
                        path_blocks.dedup();
                        scratch.unmask_own();
                        let after = scratch.counters();
                        let mut delta = [0u64; 5];
                        for (index, value) in delta.iter_mut().enumerate() {
                            *value = after[index] - before[index];
                        }
                        let (blocks, read_tiles) = scratch.read.finish();
                        (net_state, delta, blocks, read_tiles, path_blocks)
                    })
                })
                .collect();
            self.profile.speculative_seconds += started.elapsed().as_secs_f64();
            let mut speculative = speculative.into_iter();
            let mut marked_blocks: Vec<usize> = Vec::new();
            let mut marked_tiles: Vec<usize> = Vec::new();
            let mut marked_cells: Vec<usize> = Vec::new();
            for (members, _) in &batches[position..end] {
                let routes: Vec<(NetState, [u64; 5], Vec<u32>, Vec<u32>, Vec<u32>)> =
                    speculative.by_ref().take(members.len()).collect();
                let block_valid = if paths {
                    routes.iter().all(|(_, _, _, _, path)| !path.iter().any(|cell| dirty_cells[*cell as usize]))
                } else {
                    routes.iter().all(|(_, _, blocks, _, _)| !blocks.iter().any(|block| dirty_blocks[*block as usize]))
                };
                let tile_valid = paths
                    || routes.iter().all(|(_, _, _, read_tiles, _)| !read_tiles.iter().any(|tile| dirty_tiles[*tile as usize]));
                if !block_valid {
                    self.profile.speculation_block_conflicts += 1;
                }
                if !tile_valid {
                    self.profile.speculation_tile_conflicts += 1;
                }
                // A pour net's search reads its brush class's maps, where its
                // own old stamps stay unmasked: it routes in turn.
                let valid = block_valid
                    && (tile_valid || std::env::var_os("PCB_ROUTER_SPECULATION_NO_TILES").is_some())
                    && members.iter().all(|net| self.nets[*net as usize].plane.is_empty());
                let old: Vec<Vec<(u32, u32)>> = members.iter().map(|net| self.nets[*net as usize].stamped.clone()).collect();
                if valid && !check {
                    let commit_started = std::time::Instant::now();
                    for (net, (mut state, counters, _, _, _)) in members.iter().zip(routes) {
                        // As `route_batch` leaves it: the old stamps go,
                        // the new route is stamped.
                        state.stamped = std::mem::take(&mut self.nets[*net as usize].stamped);
                        state.reroutes += 1;
                        self.nets[*net as usize] = state;
                        self.scratch.searches += counters[0];
                        self.scratch.expansions += counters[1];
                        self.scratch.open_searches += counters[2];
                        self.scratch.open_expansions += counters[3];
                        self.scratch.failed_searches += counters[4];
                        self.stamp(*net);
                    }
                    self.stamp_seconds += commit_started.elapsed().as_secs_f64();
                    self.profile.speculation_kept += members.len() as u64;
                } else {
                    self.route_batch(members, false, present, growth);
                    if valid {
                        // Checking: a kept route must be the one routed in turn.
                        for (net, (state, _, _, _, _)) in members.iter().zip(&routes) {
                            let same = state.branches.len() == self.nets[*net as usize].branches.len()
                                && state.branches.iter().zip(&self.nets[*net as usize].branches).all(|(a, b)| a.nodes == b.nodes);
                            if same {
                                self.profile.speculation_kept += 1;
                            } else {
                                self.profile.speculation_wrong += 1;
                                eprintln!("speculation check: {} routed differently", self.board.nets[*net as usize].name);
                            }
                        }
                    } else {
                        self.profile.speculation_redone += members.len() as u64;
                    }
                }
                // What changed: the stamps that came or went (a stamp the
                // net had before and after leaves its cell's count as it
                // was). Its block and the blocks around it (a search step
                // reads its neighbours), and its tile and the tiles around
                // it (the planner reads the neighbours of the tiles it
                // reached), are dirty for the rest of the window.
                for (net, old) in members.iter().zip(old) {
                    let mut before: HashSet<(u32, u32)> = old.into_iter().collect();
                    let mut changed: Vec<u32> = Vec::new();
                    for entry in &self.nets[*net as usize].stamped {
                        if !before.remove(entry) {
                            changed.push(entry.1);
                            if paths && !dirty_cells[entry.1 as usize] {
                                // Copper this batch added: a later route
                                // through its zone conflicts.
                                dirty_cells[entry.1 as usize] = true;
                                marked_cells.push(entry.1 as usize);
                            }
                        }
                    }
                    changed.extend(before.into_iter().map(|(_, cell)| cell));
                    for cell in changed {
                        let (x, y) = (cell as usize % nx, cell as usize / nx);
                        let (bx, by) = (x >> READ_BLOCK_SHIFT, y >> READ_BLOCK_SHIFT);
                        for dy in by.saturating_sub(1)..=(by + 1).min(blocks_y - 1) {
                            for dx in bx.saturating_sub(1)..=(bx + 1).min(blocks_x - 1) {
                                let block = dy * blocks_x + dx;
                                if !dirty_blocks[block] {
                                    dirty_blocks[block] = true;
                                    marked_blocks.push(block);
                                }
                            }
                        }
                        let (tx, ty) = (x / TILE, y / TILE);
                        for dy in ty.saturating_sub(1)..=(ty + 1).min(self.tiles_y - 1) {
                            for dx in tx.saturating_sub(1)..=(tx + 1).min(self.tiles_x - 1) {
                                let tile = dy * self.tiles_x + dx;
                                if !dirty_tiles[tile] {
                                    dirty_tiles[tile] = true;
                                    marked_tiles.push(tile);
                                }
                            }
                        }
                    }
                }
            }
            for block in marked_blocks {
                dirty_blocks[block] = false;
            }
            for tile in marked_tiles {
                dirty_tiles[tile] = false;
            }
            for cell in marked_cells {
                dirty_cells[cell] = false;
            }
            self.search_seconds += started.elapsed().as_secs_f64();
            position = end;
        }
    }

    /// Routes one batch of the negotiation (see `batches`): rips up the
    /// conflicted branches of its nets, routes them (on the thread pool
    /// when there are several: their windows do not overlap), and stamps
    /// them.
    fn route_batch(&mut self, batch: &[NetId], jacobi: bool, present: f32, growth: f64) {
        let search_started = std::time::Instant::now();
        if jacobi && self.config.parallel {
            self.route_jacobi(batch, present, growth);
            self.search_seconds += search_started.elapsed().as_secs_f64();
            return;
        }
        for net in batch {
            self.rip_up_conflicted(*net);
        }
        if batch.len() == 1 || !self.config.parallel {
            let single_started = std::time::Instant::now();
            for net in batch {
                self.route_net_seq(*net, present, false, growth);
            }
            self.profile.single_batches += 1;
            self.profile.single_seconds += single_started.elapsed().as_secs_f64();
        } else {
            // The nets of a batch do not overlap: routing them against
            // the same occupancy is the same as routing them in turn.
            let parallel_started = std::time::Instant::now();
            let states = self.grid.cells() * self.board.layer_count;
            let cells = self.grid.cells();
            let this: &Self = self;
            let routed: Vec<(NetId, NetState, [u64; 5], f64)> = batch
                .par_iter()
                .map(|net| {
                    with_thread_scratch(states, cells, |scratch| {
                        let net_started = std::time::Instant::now();
                        let before = scratch.counters();
                        let mut net_state = this.nets[*net as usize].clone();
                        if !net_state.stamped.is_empty() {
                            this.mask_own(scratch, *net, &net_state);
                        }
                        this.route_net(
                            scratch,
                            *net,
                            &mut net_state,
                            present,
                            false,
                            growth,
                        );
                        scratch.unmask_own();
                        let after = scratch.counters();
                        let mut delta = [0u64; 5];
                        for (index, value) in delta.iter_mut().enumerate() {
                            *value = after[index] - before[index];
                        }
                        (*net, net_state, delta, net_started.elapsed().as_secs_f64())
                    })
                })
                .collect();
            self.profile.parallel_batches += 1;
            self.profile.parallel_nets += batch.len() as u64;
            self.profile.parallel_seconds += parallel_started.elapsed().as_secs_f64();
            for (net, net_state, counters, seconds) in routed {
                self.profile.parallel_net_seconds += seconds;
                self.nets[net as usize] = net_state;
                self.scratch.searches += counters[0];
                self.scratch.expansions += counters[1];
                self.scratch.open_searches += counters[2];
                self.scratch.open_expansions += counters[3];
                self.scratch.failed_searches += counters[4];
            }
        }
        for net in batch {
            self.nets[*net as usize].reroutes += 1;
        }
        self.search_seconds += search_started.elapsed().as_secs_f64();
        let stamp_started = std::time::Instant::now();
        for net in batch {
            self.stamp(*net);
        }
        self.stamp_seconds += stamp_started.elapsed().as_secs_f64();
    }

    fn negotiate_from(&mut self, order: &[NetId], mut pending: Vec<NetId>, present: f32) {
        let started = std::time::Instant::now();
        let profile_started = started;
        // The via reduction negotiates too: its time is its own phase.
        let nested = self.in_via_reduction;
        let expansions_before = self.scratch.expansions;
        let work_before = self.work_seconds();
        let phase_mark = self.phase_mark();
        let mut present = present;
        let mut best_conflicted = usize::MAX;
        let mut stalled = 0;
        for state in &mut self.nets {
            state.hopeless_searches = 0;
        }
        for iteration in 0..self.config.max_iterations {
            self.iterations += 1;
            let growth = 1.0 + iteration as f64 / 6.0;
            let batches = self.batches(&pending, growth);
            if std::env::var_os("PCB_ROUTER_DEBUG").is_some() {
                let sizes: Vec<String> = batches
                    .iter()
                    .map(|(members, jacobi)| {
                        format!("{}{}", members.len(), if *jacobi { "j" } else { "" })
                    })
                    .collect();
                eprintln!(
                    "  {} nets in {} batches: {:?}",
                    pending.len(),
                    batches.len(),
                    sizes
                );
            }
            self.route_batches(&batches, present, growth);
            let conflicts_started = std::time::Instant::now();
            let mut conflicted = Vec::new();
            for net in order {
                let conflicts = self.conflicts(*net);
                if std::env::var_os("PCB_ROUTER_DEBUG").is_some() && (order.len() <= 3 || iteration % 10 == 9) && !conflicts.is_empty() {
                    let spots: Vec<String> = conflicts
                        .iter()
                        .take(8)
                        .map(|(layer, cell)| {
                            let at = self.grid.center_of(*cell as usize);
                            let map = self.map_index(self.node_class(*net, *cell), *layer) as u32;
                            let others: Vec<&str> = (0..self.nets.len())
                                .filter(|other| *other != *net as usize)
                                .filter(|other| self.nets[*other].stamped.contains(&(map, *cell)))
                                .map(|other| self.board.nets[other].name.as_str())
                                .collect();
                            format!("L{layer}({:.2},{:.2})x{}[{}]", at[0], at[1], self.occupancy[map as usize][*cell as usize], others.join(","))
                        })
                        .collect();
                    eprintln!("  {} conflicts at {}", self.board.nets[*net as usize].name, spots.join(" "));
                }
                if conflicts.is_empty() {
                    self.nets[*net as usize].stuck = 0;
                } else {
                    self.nets[*net as usize].stuck += 1;
                }
                if conflicts.is_empty()
                    && (self.nets[*net as usize].complete || self.nets[*net as usize].blocked)
                {
                    continue;
                }
                let mut tiles_hit: HashSet<(usize, usize)> = HashSet::new();
                let layers = self.board.layer_count;
                for (layer, cell) in conflicts {
                    self.history[layer][cell as usize] += self.config.history_increment as f32;
                    if layer < layers {
                        for class in 0..self.statics.len() {
                            if let Some(hot) = self.hot[class * layers + layer].get_mut(cell as usize) {
                                hot.history += self.config.history_increment as f32;
                            }
                        }
                    }
                    let cell = cell as usize;
                    tiles_hit.insert((
                        layer,
                        (cell / self.grid.nx / TILE) * self.tiles_x + (cell % self.grid.nx) / TILE,
                    ));
                }
                for (layer, tile) in tiles_hit {
                    self.tile_history[layer][tile] += self.config.history_increment as f32;
                }
                conflicted.push(*net);
            }
            self.profile.conflicts += conflicts_started.elapsed().as_secs_f64();
            if let Some(hook) = self.frame_hook.clone() {
                hook(self.iterations, &self.snapshot());
            }
            if self.config.verbose {
                eprintln!(
                    "iteration {iteration}: rerouted {}, conflicted {}, present {present:.2}, {:.2}s (search {:.2}s, stamp {:.2}s, {} searches, {}M expansions; {} open searches with {}M, {} failed)",
                    pending.len(),
                    conflicted.len(),
                    started.elapsed().as_secs_f64(),
                    self.search_seconds,
                    self.stamp_seconds,
                    self.scratch.searches,
                    self.scratch.expansions / 1_000_000,
                    self.scratch.open_searches,
                    self.scratch.open_expansions / 1_000_000,
                    self.scratch.failed_searches
                );
            }
            // Pads of a pour net that the signals cut off from the main
            // piece this iteration are routed to it from the next one on.
            // Progress is measured without them: the pads come and go with
            // the signals, and a new low by their count kept ESC mini's
            // connect rung negotiating 80 iterations instead of 53.
            let really_conflicted = conflicted.len();
            if self.config.pour_islands && iteration >= 2 {
                let refresh_started = std::time::Instant::now();
                let mut stranded = 0;
                for net in order {
                    let newly = self.refresh_pour(*net);
                    stranded += newly;
                    if newly > 0 && !conflicted.contains(net) {
                        conflicted.push(*net);
                    }
                }
                self.profile.refresh_pour += refresh_started.elapsed().as_secs_f64();
                if self.config.verbose && stranded > 0 {
                    eprintln!(
                        "  {stranded} pour pads cut off from their main piece, routed from now on ({:.2}s)",
                        refresh_started.elapsed().as_secs_f64()
                    );
                }
            }
            self.last_conflicted = conflicted.clone();
            if conflicted.is_empty() {
                break;
            }
            // The plan did not survive the lattice where nets conflict:
            // those tiles lose room on the graph and the nets are planned
            // again around them.
            if self.global.is_some() && iteration % 2 == 1 {
                let replan_started = std::time::Instant::now();
                self.replan(&conflicted);
                self.profile.replan += replan_started.elapsed().as_secs_f64();
            }
            if std::env::var_os("PCB_ROUTER_DEBUG").is_some() && iteration % 10 == 9 {
                // The most contested tiles: where history piled up.
                let mut tiles: Vec<(f32, usize, usize)> = Vec::new();
                for (layer, history) in self.tile_history.iter().enumerate() {
                    for (tile, value) in history.iter().enumerate() {
                        if *value > 0.0 {
                            tiles.push((*value, layer, tile));
                        }
                    }
                }
                tiles.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
                let hot: Vec<String> = tiles
                    .iter()
                    .take(8)
                    .map(|(value, layer, tile)| {
                        let x = (tile % self.tiles_x) as f64 * TILE as f64 * self.grid.pitch
                            + self.grid.origin[0];
                        let y = (tile / self.tiles_x) as f64 * TILE as f64 * self.grid.pitch
                            + self.grid.origin[1];
                        format!("L{layer}({x:.0},{y:.0}):{value:.0}")
                    })
                    .collect();
                eprintln!("  hottest tiles: {}", hot.join(" "));
                let names: Vec<String> = conflicted
                    .iter()
                    .take(24)
                    .map(|net| {
                        let state = &self.nets[*net as usize];
                        format!(
                            "{}{}",
                            self.board.nets[*net as usize].name,
                            if state.complete { "" } else { "!" }
                        )
                    })
                    .collect();
                eprintln!("  conflicted (! = incomplete): {}", names.join(" "));
                if conflicted.len() <= 4 || iteration % 10 == 9 {
                    for net in conflicted.iter().take(4) {
                        let terminals: Vec<String> = self.board.nets[*net as usize]
                            .terminals
                            .iter()
                            .map(|t| format!("({:.2},{:.2})", t.anchor[0], t.anchor[1]))
                            .collect();
                        eprintln!("    {} terminals {}", self.board.nets[*net as usize].name, terminals.join(" "));
                    }
                    self.dump_region(&conflicted);
                }
            }
            let at_cap = present >= self.config.present_cap as f32;
            let progress = if at_cap && self.config.stall_drop > 0.0 && best_conflicted != usize::MAX {
                let needed = ((best_conflicted as f64 * self.config.stall_drop).ceil() as usize).max(2);
                really_conflicted + needed <= best_conflicted
            } else {
                really_conflicted < best_conflicted
            };
            if progress {
                best_conflicted = really_conflicted;
                stalled = 0;
            } else {
                // The low to beat stays where progress was last made: small
                // steps add up until they make a real drop.
                stalled += 1;
            }
            // Experiment hook: reroute only a random fraction of the
            // conflicted nets per iteration (damped Jacobi).
            if let Some(fraction) = std::env::var("PCB_ROUTER_REROUTE_FRACTION")
                .ok()
                .and_then(|value| value.parse::<f64>().ok())
            {
                let mut state = self.config.seed.or_else(experiment_seed).unwrap_or(0)
                    ^ (iteration as u64).wrapping_mul(0x2545_F491);
                let kept: Vec<NetId> = conflicted
                    .iter()
                    .copied()
                    .filter(|_| unit_noise(&mut state) < fraction)
                    .collect();
                if !kept.is_empty() {
                    conflicted = kept;
                }
            }
            pending = conflicted;
            present =
                (present * self.config.present_growth as f32).min(self.config.present_cap as f32);
            // At the price cap only history still moves anything: give up
            // sooner there.
            let patience =
                if present >= self.config.present_cap as f32 { self.config.stall_at_cap } else { self.config.stall_patience };
            // Work, not seconds, when the caller gives it: the same board
            // then routes the same way however busy the machine is; the
            // seconds stay as the last guard.
            let worked_out = self.config.negotiation_expansions > 0
                && self.scratch.expansions - expansions_before > self.config.negotiation_expansions;
            if stalled > patience || worked_out || self.past_work_deadline() {
                break;
            }
            if self.wall_guard() && started.elapsed().as_secs_f64() > self.config.negotiation_seconds {
                note_wall_cut(
                    "negotiation guard",
                    format_args!("{:.0} s of wall clock, {:.0} s of work", started.elapsed().as_secs_f64(), (self.scratch.expansions - expansions_before) as f64 / EXPANSIONS_PER_SECOND),
                );
                break;
            }
            if self.past_wall_deadline() {
                break;
            }
            // Nets without any path are not congestion; when a quarter of
            // the board has none after the first iterations, this rung of
            // the ladder cannot succeed and the next one should get the
            // time (video's exclusive-plane rung: 275 open searches from
            // iteration 0, 895 s).
            if iteration >= 1 && self.config.abandon_hopeless {
                let (routable, pathless) = self.nets.iter().filter(|state| state.routable).fold(
                    (0usize, 0usize),
                    |(routable, pathless), state| (routable + 1, pathless + usize::from(!state.complete && state.blocked)),
                );
                if pathless * 4 > routable {
                    if self.config.verbose {
                        eprintln!("hopeless: {pathless} of {routable} nets have no path at all");
                    }
                    self.hopeless = true;
                    break;
                }
            }
        }
        // In work seconds: the budgets of the repair, the clean-up and the
        // via reduction follow from it.
        self.negotiation_seconds = self.work_seconds() - work_before;
        self.present_reached = present;
        if !nested {
            self.profile.negotiation += profile_started.elapsed().as_secs_f64();
            self.phase_charge(0, phase_mark);
        }
    }

    /// Resolves what negotiation left, cleans up, stitches pours and
    /// materializes the copper.
    /// (open terminals, vias, lattice length) of the current state.
    fn quality(&self) -> (usize, usize, f64) {
        let mut open = 0;
        let mut vias = 0;
        let mut length = 0.0;
        for (net, state) in self.nets.iter().enumerate() {
            if !state.routable {
                continue;
            }
            open += state.connected.iter().filter(|c| !**c).count();
            if !self.conflicts(net as NetId).is_empty() {
                open += 1;
            }
            let nx = self.grid.nx as i64;
            for branch in &state.branches {
                for pair in branch.nodes.windows(2) {
                    if pair[0].cell == pair[1].cell {
                        vias += 1;
                    } else {
                        let dx = (pair[0].cell as i64 % nx - pair[1].cell as i64 % nx).abs();
                        let dy = (pair[0].cell as i64 / nx - pair[1].cell as i64 / nx).abs();
                        length += self.grid.pitch * ((dx * dx + dy * dy) as f64).sqrt();
                    }
                }
            }
        }
        (open, vias, length)
    }

    /// Per pour net (in net order): how many of its pads the pour and the
    /// net's copper leave off the main piece. The polish keeps a change
    /// only if none of these grows (`pour_aware_polish`).
    fn pour_stranded(&self) -> Vec<usize> {
        self.pour_net_ids().into_iter().map(|net| self.pour_stranded_of(net)).collect()
    }

    /// The pour nets `pour_stranded` counts, in its order.
    fn pour_net_ids(&self) -> Vec<NetId> {
        (0..self.nets.len() as NetId)
            .filter(|net| self.nets[*net as usize].routable && !self.nets[*net as usize].plane.is_empty())
            .collect()
    }

    fn pour_stranded_of(&self, net: NetId) -> usize {
        let (pours, mut parent, main) = self.analyze_pours(net);
        let terminal_base = pours.pieces + 1;
        (0..self.nets[net as usize].terminal_nodes.len())
            .filter(|terminal| find(&mut parent, terminal_base + terminal) != main)
            .count()
    }

    /// Node bounds (x0, y0, x1, y1) of a pour net's planes on all layers.
    fn plane_bounds(&self, net: NetId) -> Option<(usize, usize, usize, usize)> {
        let mut bounds: Option<(usize, usize, usize, usize)> = None;
        for mask in &self.nets[net as usize].plane {
            for (cell, inside) in mask.iter().enumerate() {
                if *inside {
                    let (x, y) = self.grid.xy(cell);
                    bounds = Some(bounds.map_or((x, y, x, y), |(x0, y0, x1, y1)| (x0.min(x), y0.min(y), x1.max(x), y1.max(y))));
                }
            }
        }
        bounds
    }

    /// Which pads of pour net `net` its pour no longer joins to the main
    /// piece (the signals cut the pour between them): those leave
    /// `on_plane`, so the net routes them to the main piece (only its
    /// nodes are targets from then on). A pad whose piece the pour joins
    /// again (the signals moved) is back on the plane and loses its stub:
    /// the vias of the early, crowded iterations must not stay (ESC mini's
    /// exclusive-plane rung: 113 open with them against 81). Returns how
    /// many pads were cut off newly; the net then needs rerouting.
    fn refresh_pour(&mut self, net: NetId) -> usize {
        let state = &self.nets[net as usize];
        if state.plane.is_empty() || !state.routable {
            return 0;
        }
        let (mut pours, mut parent, mut main) = self.analyze_pours(net);
        // Pads the signals cut off join the main piece again through a via
        // where their piece lies over the main piece on another layer: a
        // stitch now is cheaper than routing each pad as a track to it,
        // which displaces the signals (Sisu: GND left negotiation with
        // about 200 of 264 pads off the main piece).
        if self.config.stitch_in_negotiation && self.board.layer_count >= 2 {
            let terminal_base = pours.pieces + 1;
            let mut anchors: HashMap<usize, crate::geometry::Point> = HashMap::new();
            for terminal in 0..state.terminal_nodes.len() {
                if !state.on_plane[terminal] {
                    continue;
                }
                let root = find(&mut parent, terminal_base + terminal);
                if root != main {
                    anchors.entry(root).or_insert(self.board.nets[net as usize].terminals[terminal].anchor);
                }
            }
            if !anchors.is_empty() {
                let vias = self.stitching_vias(net, &pours, &mut parent, main, &anchors, true);
                if !vias.is_empty() {
                    for (layer, other, cell) in &vias {
                        self.nets[net as usize].branches.push(Branch {
                            nodes: vec![
                                Node { layer: *layer as u8, cell: *cell as u32 },
                                Node { layer: *other as u8, cell: *cell as u32 },
                            ],
                            start_terminal: PLANE_TERMINAL,
                            end_terminal: PLANE_TERMINAL,
                        });
                    }
                    self.stamp(net);
                    if self.config.verbose {
                        eprintln!("  pour {}: {} stitching vias for cut-off pads", self.board.nets[net as usize].name, vias.len());
                    }
                    (pours, parent, main) = self.analyze_pours(net);
                }
            }
        }
        let state = &self.nets[net as usize];
        let terminal_base = pours.pieces + 1;
        let terminal_count = state.terminal_nodes.len();
        let touches = |state: &NetState, terminal: usize| {
            state.terminal_nodes[terminal].iter().any(|node| {
                state.plane.get(node.layer as usize).is_some_and(|mask| !mask.is_empty() && mask[node.cell as usize])
            })
        };
        let newly: Vec<usize> = (0..terminal_count)
            .filter(|terminal| state.on_plane[*terminal] && find(&mut parent, terminal_base + terminal) != main)
            .collect();
        // Pads cut off before whose own piece joins the main one again
        // without any stub (a pad's branch to the plane): through the pour
        // and the net's other branches only.
        let is_stub = |index: usize| {
            let branch = &state.branches[index];
            (branch.end_terminal == PLANE_TERMINAL && branch.start_terminal < FREE_END)
                || (branch.start_terminal == PLANE_TERMINAL && branch.end_terminal < FREE_END)
        };
        let rejoined: Vec<usize> = if (0..terminal_count).any(|terminal| !state.on_plane[terminal] && touches(state, terminal)) {
            // Without the stubs the pour has pieces of its own (the stubs'
            // nodes are fill no more), so the terminals follow its pieces,
            // not the full analysis's (ASH-ART Qfwfq: index 143 of 143).
            let (pours_without, mut without, main_without) = self.analyze_pours_skipping(net, &is_stub);
            let base_without = pours_without.pieces + 1;
            (0..terminal_count)
                .filter(|terminal| !state.on_plane[*terminal] && touches(state, *terminal))
                .filter(|terminal| find(&mut without, base_without + terminal) == main_without)
                .collect()
        } else {
            Vec::new()
        };
        // While any pad has to reach the main piece by a branch, only the
        // main piece is a target (the nearest pour node may be an island);
        // the piece moves with the signals, so it is found again each time.
        let any_off = !newly.is_empty()
            || state.on_plane.iter().enumerate().any(|(terminal, on)| !on && !rejoined.contains(&terminal));
        let target: Vec<Vec<bool>> = if any_off {
            (0..self.board.layer_count)
                .map(|layer| {
                    pours.label[layer]
                        .iter()
                        .map(|piece| *piece != 0 && find(&mut parent, *piece as usize) == main)
                        .collect()
                })
                .collect()
        } else {
            Vec::new()
        };
        let state = &mut self.nets[net as usize];
        state.plane_target = target;
        if !rejoined.is_empty() {
            for terminal in &rejoined {
                state.on_plane[*terminal] = true;
            }
            // The stubs those pads needed go, and with them any branch
            // left dangling at a junction on one of them (katia and the
            // Telemetry board panicked on such a junction without its
            // host); what remains is restamped.
            let fixed = state.fixed;
            let mut keep: Vec<bool> = state
                .branches
                .iter()
                .enumerate()
                .map(|(index, branch)| {
                    index < fixed
                        || !((branch.end_terminal == PLANE_TERMINAL && rejoined.contains(&(branch.start_terminal as usize)))
                            || (branch.start_terminal == PLANE_TERMINAL && rejoined.contains(&(branch.end_terminal as usize))))
                })
                .collect();
            loop {
                let hosts = junction_hosts(&state.branches, &keep);
                let mut changed = false;
                for (index, branch) in state.branches.iter().enumerate() {
                    if !keep[index] || index < fixed {
                        continue;
                    }
                    let dangling = (branch.start_terminal == NO_TERMINAL && !hosts.contains_key(&branch.nodes[0]))
                        || (branch.end_terminal == NO_TERMINAL && !hosts.contains_key(branch.nodes.last().unwrap()));
                    if dangling {
                        keep[index] = false;
                        changed = true;
                    }
                }
                if !changed {
                    break;
                }
            }
            let mut keep = keep.into_iter();
            state.branches.retain(|_| keep.next().unwrap());
            self.stamp(net);
        }
        let state = &mut self.nets[net as usize];
        if newly.is_empty() {
            return 0;
        }
        for terminal in &newly {
            state.on_plane[*terminal] = false;
            state.connected[*terminal] = false;
        }
        state.complete = false;
        newly.len()
    }

    /// Renegotiates the nets that have vias with a higher via cost, from the
    /// converged state. A round is kept only if nothing opens and the via
    /// count drops.
    fn reduce_vias(&mut self, order: &[NetId]) {
        // Fewer vias are worth nothing while connections are open, and the
        // renegotiation would take as long as the one that failed.
        if self.quality().0 > 0 {
            if std::env::var_os("PCB_ROUTER_DEBUG").is_some() {
                for net in order {
                    let conflicts = self.conflicts(*net);
                    if !conflicts.is_empty() {
                        let spots: Vec<String> = conflicts.iter().take(6).map(|(layer, cell)| {
                            let at = self.grid.center_of(*cell as usize);
                            format!("L{layer}({:.2},{:.2})", at[0], at[1])
                        }).collect();
                        eprintln!("no via reduction: {} conflicts at {}", self.board.nets[*net as usize].name, spots.join(" "));
                    }
                }
            }
            return;
        }
        let mut budget = (self.config.via_reduction_budget * self.negotiation_seconds)
            .max(60.0)
            .min(self.config.via_reduction_seconds);
        if let Some((total, left_at_start)) = self.config.run_work {
            let left = left_at_start - self.work_seconds();
            if left > total / 2.0 {
                // At most 300 s: doubling the quick tier's via-reduction
                // time for 100 vias was too much; k30-SBC's 182 s round fits.
                let mut allowance = (left / 3.0).min(300.0);
                // A third of what is left before the deadline: of its work
                // when the caller counts it (the wall clock left would make
                // the budget depend on the machine's load), else of its
                // wall clock.
                if let Some(limit) = self.config.deadline_work {
                    allowance = allowance.min((limit - self.work_seconds()).max(0.0) / 3.0);
                } else if let Some(deadline) = self.config.deadline {
                    allowance = allowance.min(deadline.saturating_duration_since(std::time::Instant::now()).as_secs_f64() / 3.0);
                }
                if allowance > budget && self.config.verbose {
                    eprintln!("via reduction: {left:.0}s of the run's {total:.0}s left, a budget of {allowance:.0}s");
                }
                budget = budget.max(allowance);
            }
        }
        let started = self.clock();
        let strict = self.config.strict_via_budget;
        let mut last_round: Option<f64> = None;
        for round in 0..self.config.via_reduction_rounds {
            if self.spent(&started, budget) {
                if self.config.verbose {
                    eprintln!(
                        "via reduction: budget of {budget:.0}s used, stopping before round {round}"
                    );
                }
                break;
            }
            let left = budget - (self.budget_work() - started.0);
            if strict && let Some(cost) = last_round.filter(|cost| *cost > left) {
                if self.config.verbose {
                    eprintln!(
                        "via reduction: round {round} would take about {cost:.0}s of the {left:.0}s left of {budget:.0}s: stopping"
                    );
                }
                break;
            }
            let round_started = self.budget_work();
            let before = self.quality();
            let stranded_before = if self.config.pour_aware_via_reduction { self.pour_stranded() } else { Vec::new() };
            let starved_before = if self.config.via_reduction_keeps_spokes { self.starved_pads() } else { 0 };
            let pending: Vec<NetId> = order
                .iter()
                .copied()
                .filter(|net| {
                    self.nets[*net as usize].branches.iter().any(|branch| {
                        branch
                            .nodes
                            .windows(2)
                            .any(|pair| pair[0].cell == pair[1].cell)
                    })
                })
                .collect();
            if pending.is_empty() {
                break;
            }
            let snapshot = self.lean_snapshot();
            // After the snapshot: a revert restores the configuration too.
            let limits = (self.config.negotiation_seconds, self.config.negotiation_expansions, self.config.cleanup_seconds);
            if strict {
                // The round's negotiation stops where the budget ends (a
                // cut round that leaves something open is reverted below).
                self.config.negotiation_expansions = ((left * EXPANSIONS_PER_SECOND) as u64).max(1);
                self.config.negotiation_seconds = self.config.negotiation_seconds.min(left * GUARD);
            }
            let via_cost = self.config.via_cost;
            let weight = self.config.heuristic_weight;
            self.config.via_cost =
                via_cost * self.config.via_reduction_factor.powi(round as i32 + 1);
            self.config.heuristic_weight = weight.max(self.config.via_reduction_weight);
            for net in &pending {
                self.rip_up(*net);
            }
            // The rerouted nets find their way around the settled ones
            // rather than through them.
            self.negotiate_from(order, pending.clone(), self.config.via_reduction_present as f32);
            self.resolve_remaining(order);
            self.config.heuristic_weight = weight;
            if strict {
                // The clean-up gets what the negotiation left.
                let left = (budget - (self.budget_work() - started.0)).max(0.0);
                self.config.cleanup_seconds = self.config.cleanup_seconds.min(left);
            }
            // Only the nets this round touched can have got worse.
            let touched: Vec<NetId> = order
                .iter()
                .copied()
                .filter(|net| {
                    self.nets[*net as usize].reroutes > snapshot.nets[*net as usize].reroutes
                        || pending.contains(net)
                })
                .collect();
            self.clean_up(&touched);
            (self.config.negotiation_seconds, self.config.negotiation_expansions, self.config.cleanup_seconds) = limits;
            last_round = Some(self.budget_work() - round_started);
            self.config.via_cost = via_cost;
            let after = self.quality();
            let stranded_after = if self.config.pour_aware_via_reduction { self.pour_stranded() } else { Vec::new() };
            let starved_after = if self.config.via_reduction_keeps_spokes { self.starved_pads() } else { 0 };
            let keep = after.0 <= before.0
                && after.1 < before.1
                && stranded_after.iter().zip(&stranded_before).all(|(after, before)| after <= before)
                && starved_after <= starved_before;
            if self.config.verbose {
                eprintln!(
                    "via reduction round {round}: {:?} -> {:?}, {starved_before} -> {starved_after} starved pads, {}",
                    before,
                    after,
                    if keep { "kept" } else { "reverted" }
                );
            }
            if !keep {
                // Restore, but still try the next (more expensive) round.
                self.restore_lean(snapshot);
            }
        }
    }

    /// A copy of the router to go back to, without what a negotiation
    /// leaves as it is (the static maps, the search scratch, the stamp
    /// marks, which are clear between stamps) or what is rebuilt from the
    /// rest (`hot`): a whole copy doubled the peak memory of every board
    /// (Castor: 1.39 GB for 0.51 GB of maps).
    fn lean_snapshot(&mut self) -> Router {
        let statics = std::mem::take(&mut self.statics);
        let scratch = std::mem::take(&mut self.scratch);
        let hot = std::mem::take(&mut self.hot);
        let stamp_mark = std::mem::take(&mut self.stamp_mark);
        let snapshot = self.clone();
        self.statics = statics;
        self.scratch = scratch;
        self.hot = hot;
        self.stamp_mark = stamp_mark;
        snapshot
    }

    /// Goes back to a `lean_snapshot` taken since the static maps last
    /// changed.
    fn restore_lean(&mut self, snapshot: Router) {
        let statics = std::mem::take(&mut self.statics);
        let scratch = std::mem::take(&mut self.scratch);
        let stamp_mark = std::mem::take(&mut self.stamp_mark);
        // The time spent stays spent.
        let profile = std::mem::take(&mut self.profile);
        let (search_seconds, stamp_seconds) = (self.search_seconds, self.stamp_seconds);
        *self = snapshot;
        self.profile = profile;
        (self.search_seconds, self.stamp_seconds) = (search_seconds, stamp_seconds);
        self.statics = statics;
        self.scratch = scratch;
        self.stamp_mark = stamp_mark;
        self.build_hot();
    }

    /// Debug: a fingerprint of every net's branches and stamps.
    fn fingerprint(&self) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        let mut mix = |value: u64| {
            hash ^= value;
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        };
        for state in &self.nets {
            for branch in &state.branches {
                for node in &branch.nodes {
                    mix(((node.layer as u64) << 32) | node.cell as u64);
                }
                mix(u64::MAX);
            }
            for (map, cell) in &state.stamped {
                mix(((*map as u64) << 32) | *cell as u64);
            }
        }
        for history in &self.history {
            for value in history {
                mix(value.to_bits() as u64);
            }
        }
        if std::env::var_os("PCB_ROUTER_FINGERPRINT_ALL").is_some() {
            for map in &self.occupancy {
                for value in map {
                    mix(*value as u64);
                }
            }
            for state in &self.nets {
                for layer in &state.plane {
                    for value in layer {
                        mix(*value as u64);
                    }
                }
                for value in &state.on_plane {
                    mix(*value as u64);
                }
                mix(state.complete as u64);
                for value in &state.connected {
                    mix(*value as u64);
                }
            }
        }
        hash
    }

    fn finish(&mut self, order: &[NetId]) -> RoutingResult {
        if std::env::var_os("PCB_ROUTER_FINGERPRINT").is_some() {
            eprintln!("fingerprint before resolve: {:016x}", self.fingerprint());
        }
        let resolve_started = std::time::Instant::now();
        let mark = self.phase_mark();
        self.resolve_remaining(order);
        self.phase_charge(1, mark);
        self.profile.resolve += resolve_started.elapsed().as_secs_f64();
        let cleanup_started = std::time::Instant::now();
        let mark = self.phase_mark();
        let improved = if self.hopeless { 0 } else { self.clean_up(order) };
        self.phase_charge(2, mark);
        let cleaned = cleanup_started.elapsed().as_secs_f64();
        self.profile.cleanup += cleaned;
        if std::env::var_os("PCB_ROUTER_FINGERPRINT").is_some() {
            eprintln!("fingerprint after clean-up: {:016x}", self.fingerprint());
        }
        let reduction_started = std::time::Instant::now();
        let mark = self.phase_mark();
        self.in_via_reduction = true;
        self.reduce_vias(order);
        self.in_via_reduction = false;
        self.phase_charge(3, mark);
        self.profile.via_reduction += reduction_started.elapsed().as_secs_f64();
        if std::env::var_os("PCB_ROUTER_FINGERPRINT").is_some() {
            eprintln!("fingerprint after via reduction: {:016x}", self.fingerprint());
        }
        // A repair for the final board, like the via reduction: a trial
        // (clean-up and via reduction off) skips it (PolyKybd left: 313 s
        // in the first route, mostly hard searches flooding the lattice).
        let spokes_started = std::time::Instant::now();
        let mark = self.phase_mark();
        if self.config.cleanup_passes > 0 || self.config.via_reduction_rounds > 0 {
            self.free_thermal_spokes();
        }
        self.phase_charge(4, mark);
        self.profile.thermal_spokes += spokes_started.elapsed().as_secs_f64();
        if std::env::var_os("PCB_ROUTER_FINGERPRINT").is_some() {
            eprintln!("fingerprint after thermal spokes: {:016x}", self.fingerprint());
        }
        if self.config.verbose {
            eprintln!(
                "clean up {cleaned:.2}s, via reduction {:.2}s",
                cleanup_started.elapsed().as_secs_f64() - cleaned
            );
        }
        let plane_nets: Vec<NetId> = order
            .iter()
            .copied()
            .filter(|net| !self.nets[*net as usize].plane.is_empty())
            .collect();
        for net in plane_nets {
            // The skeleton is trimmed first so that stitching only adds the
            // vias the pour pieces really need.
            let stitching_started = std::time::Instant::now();
            let mark = self.phase_mark();
            let trimmed = self.trim_skeleton(net);
            if std::env::var_os("PCB_ROUTER_FINGERPRINT").is_some() {
                let (pours, _, main, spoke_islands) = self.analyze_pours_full(net, &|_| false);
                eprintln!(
                    "fingerprint before stitching {}: {:016x}, {} pieces, main {main}, spoke islands {:?}",
                    self.board.nets[net as usize].name,
                    self.fingerprint(),
                    pours.pieces,
                    spoke_islands
                );
            }
            let stitches = self.stitch_pours(net, order);
            let tails = self.trim_dangling_plane_ends(net);
            if self.config.verbose && tails > 0 {
                eprintln!("pour {}: {tails} nodes of plane ends without fill trimmed", self.board.nets[net as usize].name);
            }
            self.profile.stitching += stitching_started.elapsed().as_secs_f64();
            self.phase_charge(5, mark);
            if self.config.verbose {
                eprintln!(
                    "pour {}: {stitches} stitching vias, {trimmed} skeleton nodes trimmed, complete {}",
                    self.board.nets[net as usize].name, self.nets[net as usize].complete
                );
            }
        }
        if self.config.manual_spokes && self.config.kicad_spokes && (self.config.cleanup_passes > 0 || self.config.via_reduction_rounds > 0) {
            let started = std::time::Instant::now();
            let added = self.add_manual_spokes();
            self.profile.thermal_spokes += started.elapsed().as_secs_f64();
            if self.config.verbose && added > 0 {
                eprintln!("manual spokes: {added} tracks from starved pads into their pours");
            }
        }
        if self.config.verbose {
            eprintln!(
                "cleanup: {improved} improvements, {:.2}s",
                cleanup_started.elapsed().as_secs_f64()
            );
        }

        if let Some(directory) = std::env::var_os("PCB_ROUTER_POUR_DUMP") {
            static DUMPS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let number = DUMPS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let directory = std::path::Path::new(&directory).join(format!("finish-{number:03}"));
            if let Err(error) = self.dump_pours(&directory) {
                eprintln!("pour dump {}: {error}", directory.display());
            } else {
                let vias: usize = (0..self.nets.len()).map(|net| self.nets[net].branches.iter().map(|branch| branch.nodes.windows(2).filter(|pair| pair[0].cell == pair[1].cell && pair[0].layer != pair[1].layer).count()).sum::<usize>()).sum();
                eprintln!("pour dump {}: {vias} via steps", directory.display());
            }
        }
        let emit_started = std::time::Instant::now();
        let status = self.statuses();
        let (mut routes, mut stubs): (Vec<NetRoute>, Vec<Vec<usize>>) = (0..self.board.nets.len()
            as NetId)
            .map(|net| self.materialize(net))
            .unzip();
        // Stubs were only checked against fixed objects; drop any that comes
        // too close to routed copper of another net.
        loop {
            let mut offending: Vec<(usize, usize)> = Vec::new();
            for violation in crate::verify::verify(&self.board, &routes) {
                let involved = violation
                    .segment
                    .map(|segment| (violation.net as usize, segment))
                    .into_iter()
                    .chain(
                        violation
                            .other_segment
                            .map(|(net, segment)| (net as usize, segment)),
                    );
                for (net, segment) in involved {
                    if stubs[net].contains(&segment) && !offending.contains(&(net, segment)) {
                        offending.push((net, segment));
                    }
                }
            }
            if offending.is_empty() {
                break;
            }
            offending.sort_by(|left, right| right.cmp(left));
            for (net, segment) in offending {
                routes[net].segments.remove(segment);
                stubs[net].retain(|stub| *stub != segment);
                for stub in &mut stubs[net] {
                    if *stub > segment {
                        *stub -= 1;
                    }
                }
            }
        }
        let mut congestion = vec![0.0f32; self.grid.cells()];
        for layer in &self.history {
            for (total, value) in congestion.iter_mut().zip(layer) {
                *total += value;
            }
        }
        let diagnostics = self.diagnostics();
        self.profile.emit += emit_started.elapsed().as_secs_f64();
        if self.config.verbose {
            eprintln!("{}", self.profile.line(self.search_seconds, self.stamp_seconds));
            eprintln!("{}", self.profile.work_line());
        }
        RoutingResult {
            diagnostics,
            congestion,
            routes,
            status,
            iterations: self.iterations,
            expansions: self.scratch.expansions,
            searches: self.scratch.searches,
            grid: self.grid.clone(),
        }
    }

    /// Completes `net` by force: it is routed straight through whatever is
    /// in its way, and exactly the nets it collides with are rerouted around
    /// it. The result is kept only if every net involved ends up complete
    /// and conflict free; otherwise everything is restored.
    fn force_connect(&mut self, net: NetId, order: &[NetId]) -> bool {
        type Saved = (NetId, Vec<Branch>, Vec<bool>, bool);
        let save = |router: &Self, net: NetId| -> Saved {
            let state = &router.nets[net as usize];
            (
                net,
                state.branches.clone(),
                state.connected.clone(),
                state.complete,
            )
        };
        let mut saved = vec![save(self, net)];
        let mut involved: Vec<NetId> = vec![net];
        self.unstamp(net);
        self.route_net_seq(net, 50.0, false, 4.0);
        self.nets[net as usize].blocked = false;
        self.stamp(net);
        let mut ok = self.nets[net as usize].complete;
        // A small negotiation among the nets this connection displaces: they
        // may push each other around, but the forced net itself stays put.
        let mut present = 2.0f32;
        for _ in 0..16 {
            if !ok {
                break;
            }
            let conflicted: Vec<NetId> = order
                .iter()
                .copied()
                .filter(|other| *other != net && !self.conflicts(*other).is_empty())
                .collect();
            if conflicted.is_empty() {
                break;
            }
            // A forced connection is a local repair; when it cascades
            // through the board the copper is simply not there.
            if involved.len() + conflicted.len() > 24 {
                ok = false;
                break;
            }
            for victim in &conflicted {
                if !involved.contains(victim) {
                    involved.push(*victim);
                    saved.push(save(self, *victim));
                }
                self.rip_up_conflicted(*victim);
                self.route_net_seq(*victim, present, false, 4.0);
                self.stamp(*victim);
            }
            present = (present * 1.5).min(self.config.present_cap as f32);
        }
        if ok {
            ok = involved.iter().all(|member| {
                self.nets[*member as usize].complete && self.conflicts(*member).is_empty()
            }) && self.conflicts(net).is_empty();
        }
        if std::env::var_os("PCB_ROUTER_DEBUG").is_some() {
            let incomplete: Vec<&str> = involved
                .iter()
                .filter(|member| !self.nets[**member as usize].complete)
                .map(|member| self.board.nets[*member as usize].name.as_str())
                .collect();
            let conflicted: Vec<&str> = involved
                .iter()
                .filter(|member| !self.conflicts(**member).is_empty())
                .map(|member| self.board.nets[*member as usize].name.as_str())
                .collect();
            eprintln!(
                "force_connect {}: ok {ok}, {} involved, incomplete {incomplete:?}, conflicted {conflicted:?}",
                self.board.nets[net as usize].name,
                involved.len()
            );
        }
        if !ok {
            for (restored, ..) in &saved {
                self.unstamp(*restored);
            }
            for (restored, branches, connected, complete) in saved {
                let state = &mut self.nets[restored as usize];
                state.branches = branches;
                state.connected = connected;
                state.complete = complete;
                self.stamp(restored);
            }
        }
        ok
    }

    /// Drops the parts of the plane skeleton that lie in solid pour copper:
    /// the pour provides that connection itself. What remains (hops to pads
    /// off the pour, runs squeezed between foreign copper) ends on the pour.
    /// Returns the number of nodes removed.
    fn trim_skeleton(&mut self, net: NetId) -> usize {
        let fixed = self.nets[net as usize].fixed;
        if fixed == 0 {
            return 0;
        }
        let (pours, _, _) = self.analyze_pours(net);
        let grid = self.grid.clone();
        let redundant =
            |node: &Node| pours.solid_around(&grid, node.layer as usize, node.cell as usize, 1);
        self.unstamp(net);
        let state = &mut self.nets[net as usize];
        let branches = std::mem::take(&mut state.branches);
        let mut kept: Vec<Branch> = Vec::new();
        let mut removed = 0;
        for (index, branch) in branches.into_iter().enumerate() {
            if index >= fixed {
                kept.push(branch);
                continue;
            }
            let drop: Vec<bool> = branch.nodes.iter().map(redundant).collect();
            let mut start = 0;
            while start < branch.nodes.len() {
                if drop[start] {
                    start += 1;
                    continue;
                }
                let mut end = start;
                while end + 1 < branch.nodes.len() && !drop[end + 1] {
                    end += 1;
                }
                // One pour node on each trimmed side keeps the contact.
                let first = if start > 0 { start - 1 } else { 0 };
                let last = if end + 1 < branch.nodes.len() {
                    end + 1
                } else {
                    end
                };
                kept.push(Branch {
                    nodes: branch.nodes[first..=last].to_vec(),
                    start_terminal: if first == 0 {
                        branch.start_terminal
                    } else {
                        PLANE_TERMINAL
                    },
                    end_terminal: if last + 1 == branch.nodes.len() {
                        branch.end_terminal
                    } else {
                        PLANE_TERMINAL
                    },
                });
                removed += branch.nodes.len() - (last + 1 - first);
                start = last + 1;
            }
            if drop.iter().all(|flag| *flag) {
                removed += branch.nodes.len();
            }
        }
        // Junctions that sat on a removed node now sit on the pour.
        let keep = vec![true; kept.len()];
        let hosts = junction_hosts(&kept, &keep);
        for branch in &mut kept {
            if branch.start_terminal == NO_TERMINAL && !hosts.contains_key(&branch.nodes[0]) {
                branch.start_terminal = PLANE_TERMINAL;
            }
            if branch.end_terminal == NO_TERMINAL
                && !hosts.contains_key(branch.nodes.last().unwrap())
            {
                branch.end_terminal = PLANE_TERMINAL;
            }
        }
        state.branches = kept;
        state.fixed = 0;
        self.stamp(net);
        removed
    }

    /// Which pour piece every terminal and branch of `net` belongs to.
    /// Returns the pour map, a union-find over `pieces + terminals +
    /// branches` elements (piece labels start at 1; element 0 is unused),
    /// and the root of the main piece (the one holding most terminals).
    fn analyze_pours(&self, net: NetId) -> (crate::pour::PourMap, Vec<usize>, usize) {
        self.analyze_pours_skipping(net, &|_| false)
    }

    /// `analyze_pours` leaving out the branches `skip` names (by index):
    /// what the pour and the other branches join without them.
    fn analyze_pours_skipping(&self, net: NetId, skip: &dyn Fn(usize) -> bool) -> (crate::pour::PourMap, Vec<usize>, usize) {
        let started = std::time::Instant::now();
        let result = self.analyze_pours_skipping_untimed(net, skip);
        self.profile.analysis_nanos.fetch_add(started.elapsed().as_nanos() as u64, std::sync::atomic::Ordering::Relaxed);
        self.profile.analysis_calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        result
    }

    fn analyze_pours_skipping_untimed(&self, net: NetId, skip: &dyn Fn(usize) -> bool) -> (crate::pour::PourMap, Vec<usize>, usize) {
        let (pours, parent, main, _) = self.analyze_pours_full(net, skip);
        (pours, parent, main)
    }

    /// `analyze_pours_skipping` with the spoke islands too: the pieces
    /// without copper of the net in them that a pad's thermal spoke
    /// reaches, each with one such pad. KiCad removes the piece and does
    /// not count the spoke (Castor: RV8 with one spoke into the fill and
    /// one into a 15 mm² piece, "1 spokes connected to isolated island").
    fn analyze_pours_full(
        &self,
        net: NetId,
        skip: &dyn Fn(usize) -> bool,
    ) -> (crate::pour::PourMap, Vec<usize>, usize, Vec<(usize, usize)>) {
        self.analyze_pours_reporting(net, skip, None)
    }

    /// `analyze_pours_full`, recording each pad's spokes in `report` when
    /// given (for `dump_pours`).
    ///
    /// Every analysis counts as work (`pour_analysis_units`).
    fn analyze_pours_reporting(
        &self,
        net: NetId,
        skip: &dyn Fn(usize) -> bool,
        report: Option<&mut Vec<SpokeRecord>>,
    ) -> (crate::pour::PourMap, Vec<usize>, usize, Vec<(usize, usize)>) {
        let started = std::time::Instant::now();
        let result = self.analyze_pours_uncounted(net, skip, report);
        let units = self.pour_analysis_units(net);
        self.scratch.pour_units.fetch_add(units, std::sync::atomic::Ordering::Relaxed);
        self.scratch.pour_nanos.fetch_add(started.elapsed().as_nanos() as u64, std::sync::atomic::Ordering::Relaxed);
        if std::env::var_os("PCB_ROUTER_POUR_TRACE").is_some() {
            let state = &self.nets[net as usize];
            let planes = state.plane.iter().filter(|mask| !mask.is_empty()).count();
            let nodes: usize = state.branches.iter().map(|branch| branch.nodes.len()).sum();
            let nanos = started.elapsed().as_nanos();
            let solid: usize = result.0.solid.iter().map(|layer| layer.iter().filter(|free| **free).count()).sum();
            let inside: usize = state.plane.iter().map(|layer| layer.iter().filter(|inside| **inside).count()).sum();
            eprintln!(
                "pour trace: cells {} planes {planes} terminals {} nodes {nodes} stamped {} units {units} nanos {nanos} solid {solid} inside {inside} pieces {}",
                self.grid.cells(),
                state.terminal_nodes.len(),
                state.stamped.len(),
                result.0.pieces
            );
        }
        result
    }

    /// The size of a pour analysis of `net`, in lattice nodes: its pour's
    /// layers times the lattice (the fill mask and its labelling run over
    /// every node of a pour layer).
    fn pour_analysis_units(&self, net: NetId) -> u64 {
        let planes = self.nets[net as usize].plane.iter().filter(|mask| !mask.is_empty()).count();
        (planes * self.grid.cells()) as u64
    }

    fn analyze_pours_uncounted(
        &self,
        net: NetId,
        skip: &dyn Fn(usize) -> bool,
        mut report: Option<&mut Vec<SpokeRecord>>,
    ) -> (crate::pour::PourMap, Vec<usize>, usize, Vec<(usize, usize)>) {
        let layers = self.board.layer_count;
        let state = &self.nets[net as usize];
        let own = {
            let maps: Vec<usize> = (0..layers)
                .filter(|layer| !state.plane[*layer].is_empty())
                .flat_map(|layer| {
                    let class = state.plane_class[layer];
                    [self.map_index(class, layer), self.diagonal_map(class, layer, 0), self.diagonal_map(class, layer, 1)]
                })
                .collect();
            OwnStamps::new(&state.stamped, &maps, self.grid.cells())
        };
        let mut free: Vec<Vec<bool>> = (0..layers)
            .map(|layer| {
                let mask = &state.plane[layer];
                if mask.is_empty() {
                    return Vec::new();
                }
                let map = self.map_index(state.plane_class[layer], layer);
                (0..mask.len())
                    .map(|cell| {
                        mask[cell]
                            && self.occupancy[map][cell] as usize
                                == own.contains(map, cell) as usize
                    })
                    .collect()
            })
            .collect();
        let rings = self.thermal_ring_pads(net);
        self.cut_thermal_rings(&rings, &mut free);
        // The net's own tracks are copper of the net, and the fill joins
        // them anywhere, inside a pad's thermal ring too (a plane stub ends
        // in the ring: Castor's U7.3 read as stranded with "1 branch, 0
        // pieces" and was rerouted and stitched for nothing).
        for (index, branch) in state.branches.iter().enumerate() {
            if skip(index) {
                continue;
            }
            for node in &branch.nodes {
                let layer = node.layer as usize;
                if free[layer].is_empty() {
                    continue;
                }
                let cell = node.cell as usize;
                let map = self.map_index(state.plane_class[layer], layer);
                if state.plane[layer][cell]
                    && self.occupancy[map][cell] as usize == own.contains(map, cell) as usize
                {
                    free[layer][cell] = true;
                }
            }
        }
        // Two free nodes diagonal to each other whose common neighbours are
        // both taken join when the brush can go straight from one to the
        // other: no other net's copper, fixed or routed, and no thermal ring
        // comes closer to the step than to the nodes. A neck of KiCad's
        // fill running diagonally to the lattice holds no two nodes side by
        // side; labelled 4-connected, k30-SBC's +5V plane on In1 fell into
        // 674 pieces where KiCad's fill has 178 outlines (with the diagonal
        // steps 183; GND on F.Cu 420, 298 and 306).
        let diagonal = |layer: usize, start: usize, orientation: usize| {
            let class = state.plane_class[layer];
            let direction = if orientation == 0 { 1 } else { 7 };
            if !self.statics[class].edge_allowed(layer, start, direction, net) {
                return false;
            }
            let map = self.diagonal_map(class, layer, orientation);
            if self.occupancy[map][start] as usize != own.contains(map, start) as usize {
                return false;
            }
            let Some(end) = self.grid.neighbor(start, direction) else {
                return false;
            };
            let (a, b) = (self.grid.center_of(start), self.grid.center_of(end));
            let middle = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
            !rings[layer].iter().any(|(pad, reach)| {
                let shape = &self.board.obstacles[*pad].shape;
                shape.aabb().inflated(*reach + self.grid.pitch).intersects(crate::geometry::Aabb { minimum: middle, maximum: middle })
                    && shape.distance_to_segment(a, b) < *reach
            })
        };
        let pours = crate::pour::PourMap::build(&self.grid, &free, &diagonal);
        let branch_pieces = self.branch_pieces(net, &pours, skip);
        let terminal_count = state.terminal_nodes.len();
        let terminal_base = pours.pieces + 1;
        let branch_base = terminal_base + terminal_count;
        let mut parent: Vec<usize> = (0..branch_base + state.branches.len()).collect();
        let reach = (0.8 / self.grid.pitch).ceil() as usize;
        let union = |parent: &mut Vec<usize>, a: usize, b: usize| {
            let (a, b) = (find(parent, a), find(parent, b));
            parent[a] = b;
        };
        // A pad joins a pour piece the way KiCad joins it: through a
        // thermal spoke, straight out from the pad (along its axes, or
        // diagonally for a round pad), across the thermal gap into fill. A
        // radius around the pad counted fill that only lies diagonally
        // beside a fine-pitch pad row, which KiCad leaves unconnected.
        // A solid pour (KiCad's `connect_pads yes`) joins a pad wherever
        // its fill comes close.
        let solid: Vec<bool> = (0..layers)
            .map(|layer| self.board.planes.iter().any(|plane| plane.net == net && plane.layer == layer && plane.solid))
            .collect();
        let description = &self.board.nets[net as usize];
        // Per pad, KiCad's own zone connection overrides the pour's.
        let isolated = |terminal: usize| self.board.isolated_pads.binary_search(&description.terminals[terminal].label).is_ok();
        let solid_pad = |terminal: usize| self.board.solid_pads.binary_search(&description.terminals[terminal].label).is_ok();
        // Pieces with copper of the net in them: a track or via, or a pad
        // the fill touches. KiCad keeps a fill piece only with such an item
        // in it; a piece joined to the net by thermal spokes alone is an
        // island, removed with its spokes (k30-SBC: 8 pads with "spokes
        // connected to isolated island", each in a 1-35 mm² piece of its own
        // and connected by a track; a via put in the piece cures it). Such
        // a piece gets no connectivity from the pads it spokes to; the pads
        // get none from it.
        let mut touched: Vec<usize> = Vec::new();
        for (terminal, nodes) in state.terminal_nodes.iter().enumerate() {
            if isolated(terminal) {
                continue;
            }
            for node in nodes.iter().filter(|node| solid[node.layer as usize] || solid_pad(terminal)) {
                if let Some(piece) = pours.piece_near(&self.grid, node.layer as usize, node.cell as usize, reach) {
                    union(&mut parent, terminal_base + terminal, piece as usize);
                    touched.push(piece as usize);
                }
            }
        }
        let keep = vec![true; state.branches.len()];
        let hosts = junction_hosts(&state.branches, &keep);
        for (index, branch) in state.branches.iter().enumerate() {
            if skip(index) {
                continue;
            }
            for (piece, _) in &branch_pieces[index] {
                union(&mut parent, branch_base + index, *piece as usize);
                touched.push(*piece as usize);
            }
            for (terminal, node) in [
                (branch.start_terminal, branch.nodes[0]),
                (branch.end_terminal, *branch.nodes.last().unwrap()),
            ] {
                if terminal == NO_TERMINAL {
                    if let Some(host) = hosts.get(&node)
                        && !skip(*host)
                    {
                        union(&mut parent, branch_base + index, branch_base + host);
                    }
                } else if terminal != PLANE_TERMINAL && terminal != FREE_END {
                    union(
                        &mut parent,
                        branch_base + index,
                        terminal_base + terminal as usize,
                    );
                }
            }
        }
        let spoke_islands = if self.config.kicad_spokes {
            self.kicad_spokes(net, &pours, &mut parent, terminal_base, skip, &rings, &branch_pieces, report)
        } else {
            let with_items: HashSet<usize> = touched.iter().map(|piece| find(&mut parent, *piece)).collect();
            let has_item = |parent: &mut Vec<usize>, piece: usize| with_items.contains(&find(parent, piece));
            let mut spoke_islands: Vec<(usize, usize)> = Vec::new();
            for (terminal, nodes) in state.terminal_nodes.iter().enumerate() {
                if isolated(terminal) || solid_pad(terminal) {
                    continue;
                }
                let pad = &self.board.obstacles[description.terminals[terminal].pad];
                let anchor = description.terminals[terminal].anchor;
                let bounds = pad.shape.aabb();
                let round = matches!(pad.shape, crate::geometry::Shape::Circle { .. });
                // In layer order: a hash set's order made the union order, and
                // so the roots, the main piece's tie-break and the stitching
                // vias' anchors, differ from run to run.
                let layers_of_nodes: std::collections::BTreeSet<usize> = nodes.iter().map(|node| node.layer as usize).collect();
                for layer in layers_of_nodes {
                    if pours.label[layer].is_empty() || solid[layer] {
                        continue;
                    }
                    let gap = self
                        .board
                        .planes
                        .iter()
                        .filter(|plane| plane.net == net && plane.layer == layer)
                        .map(|plane| plane.thermal_reach)
                        .fold(0.0f64, f64::max)
                        .max(self.grid.pitch);
                    let half = [(bounds.maximum[0] - bounds.minimum[0]) / 2.0, (bounds.maximum[1] - bounds.minimum[1]) / 2.0];
                    let directions: [[f64; 2]; 4] = if round {
                        let d = std::f64::consts::FRAC_1_SQRT_2;
                        [[d, d], [d, -d], [-d, d], [-d, -d]]
                    } else {
                        [[1.0, 0.0], [-1.0, 0.0], [0.0, 1.0], [0.0, -1.0]]
                    };
                    // KiCad wants two spokes into kept fill on the layer; a
                    // spoke into an island matters only to a pad short of them.
                    let mut good = 0;
                    let mut islands: Vec<usize> = Vec::new();
                    let mut outcomes = [SpokeOutcome::NoFill; 4];
                    // The spoke's strip, as far as the brush's map tells: its
                    // middle and the lines half a spoke less half a brush to
                    // either side (a brush centre there keeps the clearance
                    // the spoke's edge needs).
                    let bridge = self
                        .board
                        .planes
                        .iter()
                        .filter(|plane| plane.net == net && plane.layer == layer)
                        .map(|plane| (plane.thermal_reach - plane.thermal_gap).max(0.0))
                        .fold(0.0f64, f64::max);
                    let side = ((bridge - self.board.classes[state.plane_class[layer]].trace_width) / 2.0).max(0.0);
                    let plane_map = self.map_index(state.plane_class[layer], layer);
                    let clear = |cell: usize| {
                        state.plane[layer][cell]
                            && self.occupancy[plane_map][cell] as usize == own.contains(plane_map, cell) as usize
                    };
                    for (direction_index, direction) in directions.into_iter().enumerate() {
                        // From the pad's edge along the spoke, the fill must
                        // begin within the gap and a little more.
                        let edge = if round { half[0] } else { (half[0] * direction[0]).abs() + (half[1] * direction[1]).abs() };
                        for extra in [gap, gap + self.grid.pitch] {
                            let distance = edge + extra;
                            let point = [anchor[0] + direction[0] * distance, anchor[1] + direction[1] * distance];
                            let Some(cell) = self.grid.nearest_node(point) else {
                                continue;
                            };
                            let piece = pours.label[layer][cell];
                            // KiCad keeps a spoke only where it keeps the
                            // clearance all the way from the pad to the fill:
                            // on a fine-pitch row the neighbours' clearance
                            // cuts it (k30's U2.7: fill 0.7 mm right of the pad
                            // counted as a spoke past pad 6's corner, 0.2 mm
                            // from the spoke; KiCad left the pad unconnected).
                            if piece != 0 && self.config.spoke_clearance {
                                let steps = ((extra / (self.grid.pitch / 2.0)).ceil() as usize).max(1);
                                let blocked = (1..=steps).any(|step| {
                                    let along = edge + extra * step as f64 / steps as f64;
                                    [-side, 0.0, side].iter().any(|offset| {
                                        let at = [
                                            anchor[0] + direction[0] * along - direction[1] * offset,
                                            anchor[1] + direction[1] * along + direction[0] * offset,
                                        ];
                                        self.grid.nearest_node(at).is_none_or(|cell| !clear(cell))
                                    })
                                });
                                if blocked {
                                    outcomes[direction_index] = SpokeOutcome::Blocked;
                                    break;
                                }
                            }
                            if piece != 0 {
                                if has_item(&mut parent, piece as usize) {
                                    union(&mut parent, terminal_base + terminal, piece as usize);
                                    good += 1;
                                    outcomes[direction_index] = SpokeOutcome::Good(piece);
                                } else {
                                    islands.push(piece as usize);
                                    outcomes[direction_index] = SpokeOutcome::Island(piece);
                                }
                                break;
                            }
                        }
                    }
                    if let Some(report) = report.as_deref_mut() {
                        report.push(SpokeRecord { terminal, pad: description.terminals[terminal].pad, anchor, crossings: 0, layer, round, outcomes, tracks: 0 });
                    }
                    if good < 2 {
                        for piece in islands {
                            if !spoke_islands.iter().any(|(island, _)| *island == piece) {
                                spoke_islands.push((piece, terminal));
                            }
                        }
                    }
                }
            }
            spoke_islands
        };
        let mut votes: HashMap<usize, usize> = HashMap::new();
        for terminal in 0..terminal_count {
            *votes
                .entry(find(&mut parent, terminal_base + terminal))
                .or_default() += 1;
        }
        let most = votes.values().copied().max().unwrap_or(0);
        let mut leaders: Vec<usize> = votes.iter().filter(|(_, count)| **count == most).map(|(root, _)| *root).collect();
        leaders.sort_unstable();
        // Equal votes: the component with the most fill.
        let largest = if leaders.len() > 1 {
            let mut area: HashMap<usize, usize> = HashMap::new();
            let mut root_of: HashMap<u32, usize> = HashMap::new();
            for labels in &pours.label {
                for label in labels.iter().filter(|label| **label != 0) {
                    let root = *root_of.entry(*label).or_insert_with(|| find(&mut parent, *label as usize));
                    *area.entry(root).or_default() += 1;
                }
            }
            leaders.iter().copied().max_by_key(|root| (area.get(root).copied().unwrap_or(0), usize::MAX - *root)).unwrap_or(0)
        } else {
            leaders.first().copied().unwrap_or(0)
        };
        (pours, parent, largest, spoke_islands)
    }

    /// The pads' thermal spokes as KiCad makes and counts them (the
    /// `kicad_spokes` model), joining each pad to the pieces its counted
    /// spokes reach. Returns the spoke islands (piece, terminal) of pads
    /// short of two spokes.
    ///
    /// KiCad (zone_filler.cpp, buildThermalSpokes and fillCopperZone)
    /// draws a spoke from the pad's centre to its box grown by the thermal
    /// gap and half the zone's minimum width, and keeps it when that end
    /// lies in the fill (the fill before spokes, opened by the minimum
    /// width); the spoke is as wide as the zone's spoke width but no wider
    /// than the pad, and none is drawn when that is narrower than the
    /// minimum width. Other nets' clearances then trim it, and the
    /// opening removes what is left narrower than the minimum width. The
    /// DRC (drc_test_provider_zone_connections.cpp) counts a spoke where
    /// the fill crosses the ring half a gap around the pad, ignores those
    /// into a fill piece connected to nothing but the pad, and, while a
    /// pad is short, counts a track that leaves the pad and ends in fill.
    /// A piece reached by two pads' spokes connects them (KiCad's
    /// connectivity joins every item a fill piece touches); a piece that
    /// only the one pad touches is the "isolated island" of its report.
    ///
    /// On the lattice a spoke lands where a brush centre (a free node)
    /// lies within half the minimum width and half a pitch of its end, and
    /// it is blocked where, along it from the middle of the gap to its end,
    /// no node across its width leaves the brush room.
    #[allow(clippy::too_many_arguments)]
    fn kicad_spokes(
        &self,
        net: NetId,
        pours: &crate::pour::PourMap,
        parent: &mut Vec<usize>,
        terminal_base: usize,
        skip: &dyn Fn(usize) -> bool,
        rings: &[Vec<(usize, f64)>],
        branch_pieces: &[Vec<(u32, usize)>],
        mut report: Option<&mut Vec<SpokeRecord>>,
    ) -> Vec<(usize, usize)> {
        let layers = self.board.layer_count;
        let state = &self.nets[net as usize];
        let description = &self.board.nets[net as usize];
        let pitch = self.grid.pitch;
        let union = |parent: &mut Vec<usize>, a: usize, b: usize| {
            let (a, b) = (find(parent, a), find(parent, b));
            parent[a] = b;
        };
        let isolated = |terminal: usize| self.board.isolated_pads.binary_search(&description.terminals[terminal].label).is_ok();
        let solid_pad = |terminal: usize| self.board.solid_pads.binary_search(&description.terminals[terminal].label).is_ok();
        let solid: Vec<bool> = (0..layers)
            .map(|layer| self.board.planes.iter().any(|plane| plane.net == net && plane.layer == layer && plane.solid))
            .collect();
        let label_at = |layer: usize, cell: usize| pours.label[layer].get(cell).copied().unwrap_or(0);
        // Items per piece (KiCad's CN_ZONE_LAYER::HasSingleConnection):
        // each branch with a node in it, each pad joined solidly, and each
        // pad whose spoke lands in it.
        let mut items: HashMap<u32, usize> = HashMap::new();
        for (index, pieces) in branch_pieces.iter().enumerate() {
            if skip(index) {
                continue;
            }
            for (piece, runs) in pieces {
                *items.entry(*piece).or_default() += runs;
            }
        }
        let reach = (0.8 / pitch).ceil() as usize;
        for (terminal, nodes) in state.terminal_nodes.iter().enumerate() {
            if isolated(terminal) {
                continue;
            }
            let mut seen: Vec<u32> = Vec::new();
            for node in nodes.iter().filter(|node| solid[node.layer as usize] || solid_pad(terminal)) {
                if let Some(piece) = pours.piece_near(&self.grid, node.layer as usize, node.cell as usize, reach)
                    && !seen.contains(&piece)
                {
                    seen.push(piece);
                }
            }
            for piece in seen {
                *items.entry(piece).or_default() += 1;
            }
        }
        let probe = fill::FillProbe::new(self, net, rings);
        // Pass one: the spokes' geometry, then where each lands (Good holds
        // the piece until the items are counted).
        struct Spoke {
            record: usize,
            index: usize,
            pad: usize,
            layer: usize,
            anchor: crate::geometry::Point,
            direction: crate::geometry::Point,
            edge: f64,
            tip: f64,
            gap: f64,
            width: f64,
            side: f64,
        }
        let mut records: Vec<SpokeRecord> = Vec::new();
        let mut spokes: Vec<Spoke> = Vec::new();
        // The terminals' pads on the layers they are reached on, and the
        // pads merged into them on their layers: where they are copper of
        // their own (U1.24's through-hole pad on F.Cu beside its B.Cu pad)
        // and, with `inner_pads`, inside their terminal's pad, where KiCad
        // builds and counts their spokes as any pad's (an exposed pad's
        // thermal vias: in a hatched zone each has its ring, which its
        // spokes reach; in a solid one the pad's relief takes them, and
        // the DRC counts what crosses their rings, the pad's own spokes).
        // Per pad, whether it lies inside its terminal's pad on the layer.
        let mut spoke_pads: Vec<(usize, usize, crate::geometry::Point, std::collections::BTreeSet<usize>, std::collections::BTreeSet<usize>)> = Vec::new();
        for (terminal, nodes) in state.terminal_nodes.iter().enumerate() {
            let layers_of_nodes: std::collections::BTreeSet<usize> = nodes.iter().map(|node| node.layer as usize).collect();
            spoke_pads.push((terminal, description.terminals[terminal].pad, description.terminals[terminal].anchor, layers_of_nodes, Default::default()));
        }
        for merged in self.board.merged_pads.iter().filter(|merged| merged.net == net && merged.host < state.terminal_nodes.len()) {
            let host = &self.board.obstacles[description.terminals[merged.host].pad];
            let inside = |layer: &usize| host.layers & (1 << layer) != 0 && host.shape.contains(merged.anchor);
            let layers: std::collections::BTreeSet<usize> = (0..layers)
                .filter(|layer| merged.layers & (1 << layer) != 0)
                .filter(|layer| self.config.inner_pads || !inside(layer))
                .collect();
            let inner = layers.iter().copied().filter(inside).collect();
            spoke_pads.push((merged.host, merged.pad, merged.anchor, layers, inner));
        }
        // Per record, whether its pad lies inside its terminal's pad.
        let mut record_inner: Vec<bool> = Vec::new();
        for (terminal, pad_index, pad_anchor, layers_of_nodes, inner) in spoke_pads {
            if isolated(terminal) || solid_pad(terminal) {
                continue;
            }
            for layer in layers_of_nodes {
                if pours.label[layer].is_empty() || solid[layer] {
                    continue;
                }
                let planes = || self.board.planes.iter().filter(|plane| plane.net == net && plane.layer == layer && !plane.solid);
                let gap = planes().map(|plane| plane.thermal_gap).fold(0.0f64, f64::max);
                let bridge = planes().map(|plane| (plane.thermal_reach - plane.thermal_gap).max(0.0)).fold(0.0f64, f64::max);
                let minimum = self.board.classes[state.plane_class[layer]].trace_width;
                // The end KiCad tests: half the minimum width past the
                // gap, half the web's width in a hatched zone.
                let (round, anchor, geometry, minor) = self.spoke_geometry(pad_index, pad_anchor, gap + fill::KICAD_MAX_ERROR + probe.spoke_half_width(layer));
                let width = bridge.min(minor);
                // No spoke narrower than the minimum width.
                if width + 1.0e-9 >= minimum {
                    for (index, (direction, edge, tip)) in geometry.into_iter().enumerate() {
                        spokes.push(Spoke {
                            record: records.len(),
                            index,
                            pad: pad_index,
                            layer,
                            anchor,
                            direction,
                            edge,
                            tip,
                            gap,
                            width,
                            side: ((width - minimum) / 2.0).max(0.0),
                        });
                    }
                }
                records.push(SpokeRecord { terminal, pad: pad_index, anchor: pad_anchor, crossings: 0, layer, round, outcomes: [SpokeOutcome::NoFill; 4], tracks: 0 });
                record_inner.push(inner.contains(&layer));
            }
        }
        let inside = |spoke: &Spoke, point: crate::geometry::Point| {
            let offset = [point[0] - spoke.anchor[0], point[1] - spoke.anchor[1]];
            let along = offset[0] * spoke.direction[0] + offset[1] * spoke.direction[1];
            let across = (offset[0] * spoke.direction[1] - offset[1] * spoke.direction[0]).abs();
            along >= -1.0e-6 && along <= spoke.tip + 1.0e-6 && across <= spoke.width / 2.0 + 1.0e-6
        };
        let end_of = |spoke: &Spoke| [spoke.anchor[0] + spoke.direction[0] * spoke.tip, spoke.anchor[1] + spoke.direction[1] * spoke.tip];
        // Per record, the pads whose spokes met its own.
        let mut met_pads: Vec<Vec<usize>> = vec![Vec::new(); records.len()];
        // Per spoke, whether what is left of it joins the pad to the ring
        // half a gap out: neighbours' clearances may cut a spoke between the
        // pad and the ring (k30's U14.12 in a row of 0.8 mm pitch: its
        // spoke reaches a pocket of fill past the row, cut off at the pad's
        // end). The part beyond still crosses the ring, but the pad is no
        // item of the piece it lands in (KiCad removes the pocket, an
        // island with nothing in it).
        let mut attached = vec![true; spokes.len()];
        for (position, spoke) in spokes.iter().enumerate() {
            let layer = spoke.layer;
            let minimum = self.board.classes[state.plane_class[layer]].trace_width;
            let end = end_of(spoke);
            // The DRC counts the spoke where the fill crosses the ring half a
            // gap around the pad: a brush centre within half the minimum
            // width of it, joined to the spoke's end (centres all the way).
            let from = (spoke.edge + spoke.gap / 2.0 + minimum / 2.0).min(spoke.tip);
            // KiCad keeps the spoke when its end lies in the fill, or in
            // a spoke of another pad whose end lies in this one (two pads
            // side by side: mackerel-08's J10 pins on In1, Castor's J2
            // shield).
            if !probe.spoke_end_lands(layer, end, spoke.direction) {
                if std::env::var_os("PCB_ROUTER_SPOKE_DEBUG").is_some() {
                    eprintln!(
                        "no fill at spoke end {} layer {layer} dir {:?} end {end:?}: {:?}",
                        description.terminals[records[spoke.record].terminal].label,
                        spoke.direction,
                        probe.blocker(layer, end, Some(&[]))
                    );
                }
                // Spokes of pads apart, whatever their directions (KiCad
                // asks only that each end lies in the other spoke: Hub's
                // J3.A1 and B12, 1.35 mm apart, meet with diagonal spokes at
                // right angles). A pad doubled at one spot (a USB-C
                // receptacle's A1 and B12) has one set of spokes in KiCad,
                // which dedups coincident pads: its spokes do not meet.
                // Pads of one terminal meet too where they are apart (an
                // exposed pad's thermal vias, `inner_pads`).
                let meets = spokes.iter().find(|other| {
                    other.layer == layer
                        && (records[other.record].terminal != records[spoke.record].terminal || (self.config.inner_pads && other.pad != spoke.pad))
                        && crate::geometry::distance(other.anchor, spoke.anchor) > 1.0e-6
                        && crate::geometry::distance(other.anchor, spoke.anchor) <= other.tip + spoke.tip
                        && inside(other, end)
                        && inside(spoke, end_of(other))
                });
                if let Some(other) = meets
                    && (!self.config.spoke_clearance || probe.spoke_clear(layer, spoke.anchor, spoke.direction, from, spoke.tip, spoke.side))
                {
                    records[spoke.record].outcomes[spoke.index] = SpokeOutcome::Pad(records[other.record].terminal as u32);
                    met_pads[spoke.record].push(other.pad);
                }
                continue;
            }
            // What is left of it after the clearances, from the middle of
            // the gap to its end, keeps the brush's width (k30's U2.7: a neighbour's clearance cuts it on
            // a fine-pitch row).
            if self.config.spoke_clearance && !probe.spoke_clear(layer, spoke.anchor, spoke.direction, from, spoke.tip, spoke.side) {
                if std::env::var_os("PCB_ROUTER_SPOKE_DEBUG").is_some() {
                    let steps = 8;
                    for step in 0..=steps {
                        let along = from + (spoke.tip - from) * step as f64 / steps as f64;
                        let at = [spoke.anchor[0] + spoke.direction[0] * along, spoke.anchor[1] + spoke.direction[1] * along];
                        if let Some(why) = probe.blocker(layer, at, None) {
                            eprintln!(
                                "blocked spoke {} layer {layer} dir {:?} width {:.3} side {:.3} at {along:.3} ({:.3}..{:.3}): {why}",
                                description.terminals[records[spoke.record].terminal].label,
                                spoke.direction, spoke.width, spoke.side, from, spoke.tip
                            );
                            break;
                        }
                    }
                }
                records[spoke.record].outcomes[spoke.index] = SpokeOutcome::Blocked;
                continue;
            }
            // The piece it joins: the nearest of the pour map's (none for
            // a sliver the lattice misses: a piece of its own).
            let radius = minimum / 2.0 + pitch;
            let mut best: Option<(f64, u32)> = None;
            if let Some((x0, y0, x1, y1)) = self.grid.node_range(crate::geometry::Aabb { minimum: end, maximum: end }.inflated(radius)) {
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        let piece = label_at(layer, self.grid.index(x, y));
                        let distance = crate::geometry::distance(end, self.grid.center(x, y));
                        if piece != 0 && distance <= radius && best.is_none_or(|(best, _)| distance < best) {
                            best = Some((distance, piece));
                        }
                    }
                }
            }
            // None that near: the end may lie in the narrow tip of a wedge
            // of fill whose brush centres the lattice holds only further in
            // (bit-preserve's IC11.1: its spoke ends where a track's
            // clearance and the relief leave a wedge, the nearest labelled
            // node 0.3 mm away). The nearest labelled node within the
            // minimum width and four pitches that a straight run of fill
            // joins to the end.
            if best.is_none() {
                let far = minimum + 4.0 * pitch;
                let mut candidates: Vec<(f64, usize)> = Vec::new();
                if let Some((x0, y0, x1, y1)) = self.grid.node_range(crate::geometry::Aabb { minimum: end, maximum: end }.inflated(far)) {
                    for y in y0..=y1 {
                        for x in x0..=x1 {
                            let cell = self.grid.index(x, y);
                            let distance = crate::geometry::distance(end, self.grid.center(x, y));
                            if label_at(layer, cell) != 0 && distance <= far {
                                candidates.push((distance, cell));
                            }
                        }
                    }
                }
                candidates.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
                let step = (pitch.min(minimum) / 2.0).max(0.01);
                best = candidates.into_iter().take(8).find_map(|(distance, cell)| {
                    let centre = self.grid.center_of(cell);
                    let steps = ((distance / step).ceil() as usize).max(1);
                    (1..steps)
                        .all(|index| {
                            let t = index as f64 / steps as f64;
                            probe.test_area_at(layer, [end[0] + (centre[0] - end[0]) * t, end[1] + (centre[1] - end[1]) * t])
                        })
                        .then(|| (distance, label_at(layer, cell)))
                });
            }
            // Piece 0 stands for a sliver the lattice misses: the spoke
            // counts only where a track of the pad touches it too.
            records[spoke.record].outcomes[spoke.index] = SpokeOutcome::Good(best.map_or(0, |(_, piece)| piece));
            // Only a kept spoke's join to the pad matters.
            if self.config.spoke_clearance {
                // A brush centre within half the minimum width of the pad
                // touches it.
                attached[position] = probe.spoke_clear(layer, spoke.anchor, spoke.direction, (spoke.edge + minimum / 2.0 - 1.0e-3).min(from), from, spoke.side);
                if std::env::var_os("PCB_ROUTER_SPOKE_DEBUG").is_some() {
                    let at = [spoke.anchor[0] + spoke.direction[0] * spoke.edge, spoke.anchor[1] + spoke.direction[1] * spoke.edge];
                    eprintln!(
                        "spoke {} layer {layer} dir {:?} attached {} (edge {:.3} from {:.3} side {:.3}; at the edge: {:?})",
                        description.terminals[records[spoke.record].terminal].label,
                        spoke.direction,
                        attached[position],
                        spoke.edge,
                        from,
                        spoke.side,
                        probe.blocker(layer, at, None)
                    );
                }
            }

        }
        // The pads' drill holes: KiCad's fill keeps out of a plated hole,
        // so a spoke reaches into a through-hole pad only up to its hole.
        let mut record_attached = vec![[true; 4]; records.len()];
        for (position, spoke) in spokes.iter().enumerate() {
            record_attached[spoke.record][spoke.index] = attached[position];
        }
        // Per label, each plated hole's centre and radius (pads may share
        // a number: the hole is the one at the pad).
        let mut holes: HashMap<&str, Vec<(crate::geometry::Point, f64)>> = HashMap::new();
        for obstacle in self.board.obstacles.iter().filter(|obstacle| obstacle.kind == crate::board::ObstacleKind::Hole) {
            if let Some(label) = obstacle.label.strip_suffix(" plated hole") {
                let bounds = obstacle.shape.aabb();
                let centre = [(bounds.minimum[0] + bounds.maximum[0]) / 2.0, (bounds.minimum[1] + bounds.maximum[1]) / 2.0];
                let radius = ((bounds.maximum[0] - bounds.minimum[0]).min(bounds.maximum[1] - bounds.minimum[1]) / 2.0).max(0.0);
                holes.entry(label).or_default().push((centre, radius));
            }
        }
        let hole_at = |label: &str, at: crate::geometry::Point| {
            holes
                .get(label)
                .and_then(|holes| holes.iter().min_by(|a, b| crate::geometry::distance(a.0, at).total_cmp(&crate::geometry::distance(b.0, at))))
                .map_or(0.0, |(_, radius)| *radius)
        };
        // Per record, which of its spokes the pad's own tracks on the layer
        // touch, and so join to the piece the spoke lands in (a second
        // item: Castor's D11.3, KiCad counts its one spoke, its exit stub
        // in the piece). With `track_meets_spokes` a track touches a spoke
        // only where its first run overlaps the spoke's strip outside the
        // pad's hole (mackerel-30's C50.2: a track leaving a round pad
        // along an axis misses its diagonal spokes, which reach the pad
        // only up to its drill, and KiCad finds the piece they land in an
        // isolated island); without it a routed track touches them all.
        let mut touched_spokes: Vec<[bool; 4]> = Vec::new();
        // Per piece, the pads joined to it by a spoke (one item each,
        // however many pieces of one outline they reach), and the pairs of
        // pieces a pad's overlapping spokes join.
        let mut pad_items: HashMap<u32, Vec<usize>> = HashMap::new();
        let mut spoke_joins: Vec<(u32, u32)> = Vec::new();
        for (record_index, record) in records.iter().enumerate() {
            let (terminal, layer, outcomes) = (record.terminal, record.layer, record.outcomes);
            let kept: Vec<&Spoke> = spokes
                .iter()
                .filter(|spoke| spoke.record == record_index && matches!(outcomes[spoke.index], SpokeOutcome::Good(_) | SpokeOutcome::Pad(_)))
                .collect();
            let hole = if self.config.track_meets_spokes { hole_at(description.terminals[terminal].label.as_str(), record.anchor) } else { 0.0 };
            let strip_of = |spoke: &Spoke| {
                let (d, w) = (spoke.direction, spoke.width / 2.0);
                let normal = [-d[1] * w, d[0] * w];
                // A spoke cut off from the pad begins where the ring is
                // (a track touching it over the pad touches nothing).
                let start = if record_attached[record_index][spoke.index] {
                    hole
                } else {
                    spoke.edge + spoke.gap / 2.0
                };
                let near = [spoke.anchor[0] + d[0] * start.min(spoke.tip), spoke.anchor[1] + d[1] * start.min(spoke.tip)];
                let far = [spoke.anchor[0] + d[0] * spoke.tip, spoke.anchor[1] + d[1] * spoke.tip];
                crate::geometry::Shape::Polygon {
                    points: vec![
                        [near[0] + normal[0], near[1] + normal[1]],
                        [far[0] + normal[0], far[1] + normal[1]],
                        [far[0] - normal[0], far[1] - normal[1]],
                        [near[0] - normal[0], near[1] - normal[1]],
                    ],
                }
            };
            let mut touched = [false; 4];
            for (index, branch) in state.branches.iter().enumerate() {
                if skip(index) || branch.nodes.is_empty() {
                    continue;
                }
                let ends_here = (branch.start_terminal as usize == terminal && branch.nodes.first().is_some_and(|node| node.layer as usize == layer))
                    || (branch.end_terminal as usize == terminal && branch.nodes.last().is_some_and(|node| node.layer as usize == layer));
                if !ends_here {
                    continue;
                }
                let segments: Vec<(crate::geometry::Point, crate::geometry::Point, f64)> = if let Some((start, end, radius)) = state.adopted.get(&index) {
                    vec![(*start, *end, *radius)]
                } else if self.config.track_meets_spokes {
                    self.first_run(net, terminal, branch)
                } else {
                    for spoke in &kept {
                        touched[spoke.index] = true;
                    }
                    continue;
                };
                for spoke in &kept {
                    let strip = strip_of(spoke);
                    if segments.iter().any(|(a, b, radius)| strip.distance_to_segment(*a, *b) <= *radius) {
                        touched[spoke.index] = true;
                    }
                }
            }
            // Per piece: whether a spoke joins the pad to it, and whether
            // the pad's own track touches one (each an item of it).
            let mut seen: Vec<(u32, bool, bool)> = Vec::new();
            for (index, outcome) in outcomes.iter().enumerate() {
                if let SpokeOutcome::Good(piece) = outcome
                    && *piece != 0
                {
                    let joined = record_attached[record_index][index];
                    match seen.iter_mut().find(|(seen, _, _)| seen == piece) {
                        Some(entry) => {
                            entry.1 |= joined;
                            entry.2 |= touched[index];
                        }
                        None => seen.push((*piece, joined, touched[index])),
                    }
                }
            }
            for (piece, joined, tracked) in seen {
                *items.entry(piece).or_default() += tracked as usize;
                if joined {
                    let pads = pad_items.entry(piece).or_default();
                    if !pads.contains(&record.pad) {
                        pads.push(record.pad);
                    }
                }
            }
            // Kept spokes of one pad are one polygon where they overlap:
            // all of them on a pad without a hole (they start at its
            // centre), on a plated pad where they still overlap outside its
            // drill. The pieces they land in are then one outline to
            // KiCad, the pad one item of it.
            let joined_spokes: Vec<&&Spoke> = kept
                .iter()
                .filter(|spoke| record_attached[record_index][spoke.index] && matches!(outcomes[spoke.index], SpokeOutcome::Good(piece) if piece != 0))
                .collect();
            if self.config.neck_joins && joined_spokes.len() > 1 {
                let hole = hole_at(description.terminals[terminal].label.as_str(), record.anchor);
                let piece_of = |spoke: &Spoke| if let SpokeOutcome::Good(piece) = outcomes[spoke.index] { piece } else { 0 };
                for (position, a) in joined_spokes.iter().enumerate() {
                    for b in &joined_spokes[position + 1..] {
                        let (pa, pb) = (piece_of(a), piece_of(b));
                        if pa == pb {
                            continue;
                        }
                        let facing = a.direction[0] * b.direction[0] + a.direction[1] * b.direction[1] < -0.5;
                        let reach = a.width.min(b.width) / 2.0 * if facing { 1.0 } else { std::f64::consts::SQRT_2 };
                        if hole < reach - 1.0e-6 {
                            spoke_joins.push((pa, pb));
                        }
                    }
                }
            }
            touched_spokes.push(touched);
        }
        // Necks the lattice cuts: a piece a kept spoke lands in, short of
        // two items, joins a piece next to it where KiCad's fill runs
        // between their nodes (a neck narrower than the lattice resolves;
        // census of 7 boards: 23 island spokes in KiCad outlines our map
        // splits, wrong verdicts on 4 pads such as Castor's U8.12).
        let mut groups: HashMap<u32, Vec<u32>> = HashMap::new();
        let merge = |groups: &mut HashMap<u32, Vec<u32>>, parent: &mut Vec<usize>, a: u32, b: u32| {
            union(parent, a as usize, b as usize);
            let mut group = groups.get(&a).cloned().unwrap_or_else(|| vec![a]);
            group.extend(groups.get(&b).cloned().unwrap_or_else(|| vec![b]));
            group.sort_unstable();
            group.dedup();
            for member in &group {
                groups.insert(*member, group.clone());
            }
        };
        let count_of = |groups: &HashMap<u32, Vec<u32>>, piece: u32| -> usize {
            let single = [piece];
            let members: &[u32] = groups.get(&piece).map_or(&single[..], |group| &group[..]);
            let mut pads: Vec<usize> = Vec::new();
            let mut count = 0;
            for member in members {
                count += items.get(member).copied().unwrap_or(0);
                for pad in pad_items.get(member).map_or(&[][..], |pads| &pads[..]) {
                    if !pads.contains(pad) {
                        pads.push(*pad);
                    }
                }
            }
            count + pads.len()
        };
        for (a, b) in std::mem::take(&mut spoke_joins) {
            merge(&mut groups, parent, a, b);
        }
        if self.config.neck_joins {
            let nx = self.grid.nx as i64;
            let ny = self.grid.ny as i64;
            let mut done: HashSet<(usize, u32)> = HashSet::new();
            for spoke in &spokes {
                let SpokeOutcome::Good(piece) = records[spoke.record].outcomes[spoke.index] else {
                    continue;
                };
                let layer = spoke.layer;
                if piece == 0 || count_of(&groups, piece) >= 2 || !done.insert((layer, piece)) {
                    continue;
                }
                // The piece's nodes on the layer, from the node nearest the
                // spoke's end (a bounded walk).
                let end = end_of(spoke);
                let Some((x0, y0, x1, y1)) = self.grid.node_range(crate::geometry::Aabb { minimum: end, maximum: end }.inflated(2.0 * pitch + spoke.width)) else {
                    continue;
                };
                let mut queue: Vec<usize> = Vec::new();
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        let cell = self.grid.index(x, y);
                        if label_at(layer, cell) == piece {
                            queue.push(cell);
                        }
                    }
                }
                let mut seen: HashSet<usize> = queue.iter().copied().collect();
                let mut neighbours: Vec<u32> = Vec::new();
                while let Some(cell) = queue.pop() {
                    if seen.len() > 4000 {
                        break;
                    }
                    let (x, y) = self.grid.xy(cell);
                    for dy in -2i64..=2 {
                        for dx in -2i64..=2 {
                            let (ox, oy) = (x as i64 + dx, y as i64 + dy);
                            if (dx == 0 && dy == 0) || ox < 0 || oy < 0 || ox >= nx || oy >= ny {
                                continue;
                            }
                            let other = self.grid.index(ox as usize, oy as usize);
                            let label = label_at(layer, other);
                            if label == piece {
                                if dx.abs() <= 1 && dy.abs() <= 1 && seen.insert(other) {
                                    queue.push(other);
                                }
                            } else if label != 0 && !neighbours.contains(&label) {
                                let (a, b) = (self.grid.center_of(cell), self.grid.center_of(other));
                                let steps = 8;
                                if (0..=steps).all(|step| {
                                    let t = step as f64 / steps as f64;
                                    probe.fill_at(layer, [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t])
                                }) {
                                    neighbours.push(label);
                                }
                            }
                        }
                    }
                }
                for label in neighbours {
                    merge(&mut groups, parent, piece, label);
                }
            }
        }
        // Pass two: spokes into pieces with another item count and join.
        let mut tallies: Vec<(usize, Vec<usize>)> = Vec::new();
        for (((record, touched), joined), inner) in records.iter_mut().zip(&touched_spokes).zip(&record_attached).zip(&record_inner) {
            let mut good = 0;
            let mut islands: Vec<usize> = Vec::new();
            for (index, outcome) in record.outcomes.iter_mut().enumerate() {
                let tracked = &touched[index];
                if *inner && matches!(*outcome, SpokeOutcome::Good(_)) {
                    // Inside its terminal's pad a spoke lands in fill that
                    // overlaps that pad (in a hatched zone, its own ring):
                    // the outline holds both pads, two items.
                    if let SpokeOutcome::Good(piece) = *outcome
                        && piece != 0
                        && joined[index]
                    {
                        union(parent, terminal_base + record.terminal, piece as usize);
                    }
                    good += 1;
                } else if let SpokeOutcome::Pad(other) = *outcome {
                    // One outline holds both pads: the spoke counts.
                    union(parent, terminal_base + record.terminal, terminal_base + other as usize);
                    good += 1;
                } else if let SpokeOutcome::Good(0) = *outcome {
                    if *tracked {
                        good += 1;
                    } else {
                        *outcome = SpokeOutcome::Island(0);
                    }
                } else if let SpokeOutcome::Good(piece) = *outcome {
                    let count = count_of(&groups, piece);
                    if !joined[index] && !tracked {
                        // Cut off from the pad: the crossing counts where
                        // the piece holds two items, is an island's with
                        // one, and is gone with none.
                        if count >= 2 {
                            good += 1;
                        } else if count == 1 {
                            *outcome = SpokeOutcome::Island(piece);
                        } else {
                            *outcome = SpokeOutcome::NoFill;
                        }
                    } else if count >= 2 {
                        union(parent, terminal_base + record.terminal, piece as usize);
                        good += 1;
                    } else {
                        *outcome = SpokeOutcome::Island(piece);
                        islands.push(piece as usize);
                    }
                }
            }
            tallies.push((good, islands));
        }
        // Pass three, with every join made: the DRC's track spokes. KiCad
        // removes a fill piece no pad's cluster reaches (an orphan) before
        // the DRC looks, so a track ending there counts for nothing
        // (Castor's RV10.1: its F.Cu track ends over an orphan of B.Cu).
        // KiCad's clusters join a piece to the pads whose spokes reach it
        // whether the DRC counts them or not (BAGEL's J102.1: its In1 track
        // ends over a piece of In2 that pads spoke into as islands).
        let mut terminal_roots: HashSet<usize> = (0..state.terminal_nodes.len()).map(|terminal| find(parent, terminal_base + terminal)).collect();
        for record in &records {
            for outcome in &record.outcomes {
                if let SpokeOutcome::Island(piece) = outcome
                    && *piece != 0
                {
                    terminal_roots.insert(find(parent, *piece as usize));
                }
            }
        }
        let kept_piece = |parent: &mut Vec<usize>, layer: usize, point: crate::geometry::Point| {
            let radius = self.board.classes[state.plane_class[layer]].trace_width / 2.0 + pitch;
            let mut best: Option<(f64, u32)> = None;
            if let Some((x0, y0, x1, y1)) = self.grid.node_range(crate::geometry::Aabb { minimum: point, maximum: point }.inflated(radius)) {
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        let piece = label_at(layer, self.grid.index(x, y));
                        let distance = crate::geometry::distance(point, self.grid.center(x, y));
                        if piece != 0 && distance <= radius && best.is_none_or(|(best, _)| distance < best) {
                            best = Some((distance, piece));
                        }
                    }
                }
            }
            best.is_some_and(|(_, piece)| terminal_roots.contains(&find(parent, piece as usize)))
        };
        // Per layer, the plated holes of the net's thermal pads there.
        let thermal_holes: Vec<Vec<usize>> = (0..layers)
            .map(|layer| {
                self.board
                    .obstacles
                    .iter()
                    .enumerate()
                    .filter(|(_, obstacle)| obstacle.kind == crate::board::ObstacleKind::Hole && obstacle.net == Some(net) && obstacle.label.ends_with(" plated hole"))
                    .filter(|(_, obstacle)| {
                        let bounds = obstacle.shape.aabb();
                        let centre = [(bounds.minimum[0] + bounds.maximum[0]) / 2.0, (bounds.minimum[1] + bounds.maximum[1]) / 2.0];
                        rings[layer].iter().any(|(pad, _)| self.board.obstacles[*pad].shape.contains(centre))
                    })
                    .map(|(index, _)| index)
                    .collect()
            })
            .collect();
        let records_outcomes: Vec<[SpokeOutcome; 4]> = records.iter().map(|record| record.outcomes).collect();
        let records_pads: Vec<(usize, usize, crate::geometry::Point)> = records.iter().map(|record| (record.terminal, record.pad, record.anchor)).collect();
        let mut spoke_islands: Vec<(usize, usize)> = Vec::new();
        for (record_index, (record, (good, islands))) in records.iter_mut().zip(tallies).enumerate() {
            // Other pads' spokes across this pad's ring half a gap out: the
            // DRC counts every crossing of the fill with the ring, and KiCad
            // adds spokes after cutting the reliefs (Castor's U6.11 between
            // U6.12's spoke and its own in a 0.65 mm row; Hub's J3.S1, which
            // a round neighbour's diagonal spoke crosses). Spokes meeting
            // this pad's own are counted with it already.
            if self.config.foreign_spokes {
                let gap = self.board.planes.iter().filter(|plane| plane.net == net && plane.layer == record.layer).map(|plane| plane.thermal_gap).fold(0.0f64, f64::max);
                let pad = &self.board.obstacles[record.pad];
                let minimum = self.board.classes[state.plane_class[record.layer]].trace_width;
                let met = &met_pads[record_index];
                let bounds = pad.shape.aabb();
                let radius = 0.5 * (bounds.maximum[0] - bounds.minimum[0]).hypot(bounds.maximum[1] - bounds.minimum[1]) + gap;
                let centre = [(bounds.minimum[0] + bounds.maximum[0]) / 2.0, (bounds.minimum[1] + bounds.maximum[1]) / 2.0];
                for other in spokes.iter().filter(|other| {
                    other.layer == record.layer && other.pad != record.pad && crate::geometry::distance(other.anchor, centre) <= other.tip + other.width + radius
                }) {
                    let (other_terminal, _, other_anchor) = records_pads[other.record];
                    if crate::geometry::distance(other_anchor, record.anchor) < 1.0e-6 || met.contains(&other.pad) {
                        continue;
                    }
                    let counted = match records_outcomes[other.record][other.index] {
                        SpokeOutcome::Pad(_) => true,
                        SpokeOutcome::Good(piece) => piece != 0,
                        _ => false,
                    };
                    if !counted || pad.shape.distance_to_segment(other.anchor, end_of(other)) > gap / 2.0 + other.width / 2.0 {
                        continue;
                    }
                    // Where its strip comes within half a gap of the pad,
                    // the brush keeps room in it.
                    // Inside its terminal's pad, the strip over that pad
                    // (spokes start at their pad's centre).
                    let start = if record_inner[record_index] { 0.0 } else { other.edge };
                    let steps = (((other.tip - start) / (minimum / 4.0)).ceil() as usize).max(1);
                    let crosses = (0..=steps).any(|step| {
                        let along = start + (other.tip - start) * step as f64 / steps as f64;
                        [-other.width / 2.0, -other.side, 0.0, other.side, other.width / 2.0].iter().any(|offset| {
                            let at = [
                                other.anchor[0] + other.direction[0] * along - other.direction[1] * offset,
                                other.anchor[1] + other.direction[1] * along + other.direction[0] * offset,
                            ];
                            pad.shape.distance_to_point(at) < gap / 2.0
                        }) && [-other.side, 0.0, other.side].iter().any(|offset| {
                            let at = [
                                other.anchor[0] + other.direction[0] * along - other.direction[1] * offset,
                                other.anchor[1] + other.direction[1] * along + other.direction[0] * offset,
                            ];
                            pad.shape.distance_to_point(at) < gap / 2.0 + minimum / 2.0 && probe.blocker(record.layer, at, None).is_none()
                        })
                    });
                    if crosses {
                        // Inside its terminal's pad a strip may run right
                        // through the pad's ring (the pad's own spokes from
                        // its centre across a thermal via): KiCad counts the
                        // crossings on both sides, two spokes.
                        let through = record_inner[record_index]
                            && pad.shape.distance_to_point(other.anchor) > gap / 2.0 + other.width / 2.0
                            && pad.shape.distance_to_point(end_of(other)) > gap / 2.0 + other.width / 2.0;
                        record.crossings += if through { 2 } else { 1 };
                        // A strip over the pad's copper joins the two pads.
                        if pad.shape.distance_to_segment(other.anchor, end_of(other)) <= other.width / 2.0 {
                            union(parent, terminal_base + record.terminal, terminal_base + other_terminal);
                        }
                    }
                }
            }
            let good = good + record.crossings;
            if good < 2 {
                // The pad's own tracks (KiCad's DRC: each track segment
                // connected to the pad that starts within half a gap of it
                // and ends in this layer's fill, on any layer; else, one
                // that ends there and starts in the fill). A routed branch
                // is its stub from the pad's anchor and its first straight
                // run; an adopted one its track.
                let terminal = &description.terminals[record.terminal];
                let pad = &self.board.obstacles[record.pad];
                let gap = self.board.planes.iter().filter(|plane| plane.net == net && plane.layer == record.layer).map(|plane| plane.thermal_gap).fold(0.0f64, f64::max);
                let near_pad = |point: crate::geometry::Point| pad.shape.distance_to_point(point) <= gap / 2.0;
                let minimum = self.board.classes[state.plane_class[record.layer]].trace_width;
                let in_fill = |parent: &mut Vec<usize>, point: crate::geometry::Point| {
                    // A kept spoke is fill too, as far as other nets'
                    // clearances leave the brush room in it (they start at
                    // their pads' centres: a track to the pad's edge ends on
                    // its own spoke, Castor's C12.1; U11.10's spoke is cut
                    // over the pad by its neighbours in the row).
                    spokes.iter().any(|spoke| {
                        if spoke.layer != record.layer
                            || matches!(records_outcomes[spoke.record][spoke.index], SpokeOutcome::NoFill | SpokeOutcome::Blocked)
                            || !inside(spoke, point)
                        {
                            return false;
                        }
                        let offset = [point[0] - spoke.anchor[0], point[1] - spoke.anchor[1]];
                        let along = offset[0] * spoke.direction[0] + offset[1] * spoke.direction[1];
                        let across = offset[0] * spoke.direction[1] - offset[1] * spoke.direction[0];
                        // A brush centre in the spoke (within `side` of its
                        // axis) within half the minimum width of the point,
                        // on a grid of an eighth of the minimum width: the
                        // point may lie in what the clearances leave of the
                        // spoke only near its far edge (PixelWave's C4.1:
                        // its track ends 0.01 mm inside a spoke its
                        // neighbour's clearance trims to 0.43 mm).
                        let step = minimum / 8.0;
                        let mut tried: Vec<(i64, i64)> = Vec::new();
                        (-4..=4).any(|i: i64| {
                            (-4..=4).any(|j: i64| {
                                let along = (along + i as f64 * step).clamp(0.0, spoke.tip);
                                let across = (across + j as f64 * step).clamp(-spoke.side, spoke.side);
                                let key = ((along / 1.0e-6).round() as i64, (across / 1.0e-6).round() as i64);
                                if tried.contains(&key) {
                                    return false;
                                }
                                tried.push(key);
                                let centre = [
                                    spoke.anchor[0] + spoke.direction[0] * along + spoke.direction[1] * across,
                                    spoke.anchor[1] + spoke.direction[1] * along - spoke.direction[0] * across,
                                ];
                                // No relief cuts a spoke (KiCad adds them after
                                // the reliefs: Hub's C35.1, its track ends in
                                // its spoke where it meets C30.1's, inside
                                // C30.1's relief).
                                crate::geometry::distance(centre, point) <= minimum / 2.0 + 1.0e-9 && probe.blocker(record.layer, centre, None).is_none()
                            })
                        })
                    }) || probe.in_ring(record.layer, point)
                        || (probe.final_fill_at(record.layer, point) && kept_piece(parent, record.layer, point))
                };
                // KiCad's fill keeps out of a thermal pad's plated hole (an
                // exposed pad's thermal via: a track to its centre ends in
                // no fill, 0xCB's U3.45).
                // The test is at a point, which KiCad's fill covers only
                // where it keeps the clearance to a micron or so
                // (`TRACK_END_TOLERANCE`).
                let in_fill = |parent: &mut Vec<usize>, point: crate::geometry::Point| {
                    !(self.config.inner_pads && thermal_holes[record.layer].iter().any(|hole| self.board.obstacles[*hole].shape.contains(point)))
                        && if self.config.track_end_tolerance { probe.for_track_end(|| in_fill(parent, point)) } else { in_fill(parent, point) }
                };
                // A pad merged into its terminal has the tracks whose copper
                // overlaps its own (KiCad's connectivity: 0xCB's thermal
                // vias, which the exposed pad's tracks cross), not all its
                // terminal's.
                let merged_pad = record.pad != terminal.pad && self.config.inner_pads;
                let half_width = self.board.classes[description.class].trace_width / 2.0;
                let connected = |a: crate::geometry::Point, b: crate::geometry::Point, radius: f64| !merged_pad || pad.shape.distance_to_segment(a, b) <= radius + 1.0e-6;
                // A branch ends at this pad, or at a pad of the net at its
                // very spot (a USB-C receptacle's A1 and B12: KiCad connects
                // a track to both).
                let at_pad = |end: u16| {
                    end as usize == record.terminal
                        || description.terminals.get(end as usize).is_some_and(|other| {
                            end != NO_TERMINAL && end != PLANE_TERMINAL && end != FREE_END && crate::geometry::distance(other.anchor, record.anchor) < 1.0e-6
                        })
                };
                for (index, branch) in state.branches.iter().enumerate() {
                    if skip(index) || branch.nodes.is_empty() {
                        continue;
                    }
                    let mut segments: Vec<(crate::geometry::Point, crate::geometry::Point)> = Vec::new();
                    let mut radius = half_width;
                    if let Some((start, end, adopted)) = state.adopted.get(&index) {
                        radius = *adopted;
                        if (at_pad(branch.start_terminal) || at_pad(branch.end_terminal))
                            && pad.layers & (1 << branch.nodes[0].layer) != 0
                        {
                            segments.push((*start, *end));
                        }
                    } else {
                        for (end, forward) in [(branch.start_terminal, true), (branch.end_terminal, false)] {
                            if !at_pad(end) {
                                continue;
                            }
                            let nodes: Vec<Node> = if forward { branch.nodes.clone() } else { branch.nodes.iter().rev().copied().collect() };
                            // Connected to the pad: on one of its layers.
                            if pad.layers & (1 << nodes[0].layer) == 0 {
                                continue;
                            }
                            let first = self.grid.center_of(nodes[0].cell as usize);
                            match state.escapes.get(&nodes[0]) {
                                Some((_, _, polyline)) => segments.extend(polyline.windows(2).map(|pair| (pair[0], pair[1]))),
                                None if crate::geometry::distance(terminal.anchor, first) > 1.0e-7 => segments.push((terminal.anchor, first)),
                                None => {}
                            }
                            let step = |a: Node, b: Node| {
                                let (ax, ay) = self.grid.xy(a.cell as usize);
                                let (bx, by) = self.grid.xy(b.cell as usize);
                                (a.layer == b.layer).then_some((bx as i64 - ax as i64, by as i64 - ay as i64))
                            };
                            if nodes.len() >= 2
                                && let Some(direction) = step(nodes[0], nodes[1])
                            {
                                let mut last = 1;
                                while last + 1 < nodes.len() && step(nodes[last], nodes[last + 1]) == Some(direction) {
                                    last += 1;
                                }
                                segments.push((first, self.grid.center_of(nodes[last].cell as usize)));
                            }
                        }
                    }
                    for (a, b) in segments {
                        let counted = connected(a, b, radius) && if near_pad(a) { in_fill(parent, b) } else { near_pad(b) && in_fill(parent, a) };
                        if std::env::var_os("PCB_ROUTER_SPOKE_DEBUG").is_some() && !counted {
                            eprintln!(
                                "track not a spoke {} layer {}: {a:?}..{b:?} near {} {} fill {} {} kept {} {}",
                                terminal.label, record.layer, near_pad(a), near_pad(b),
                                probe.fill_at(record.layer, a), probe.fill_at(record.layer, b),
                                kept_piece(parent, record.layer, a), kept_piece(parent, record.layer, b)
                            );
                        }
                        if counted {
                            record.tracks += 1;
                            if std::env::var_os("PCB_ROUTER_SPOKE_DEBUG").is_some() {
                                eprintln!("track spoke {} layer {}: branch {index} {a:?}..{b:?}", terminal.label, record.layer);
                            }
                        }
                    }
                }
            }
            if good + record.tracks < 2 {
                for piece in islands {
                    if !spoke_islands.iter().any(|(island, _)| *island == piece) {
                        spoke_islands.push((piece, record.terminal));
                    }
                }
            }
        }
        if let Some(report) = report.as_deref_mut() {
            report.extend(records);
        }
        spoke_islands
    }

    /// Per branch of `net` (none for those `skip` names), the pieces of
    /// `pours` its copper touches: those its nodes lie in and, with
    /// `track_touch_fill`, those with a brush centre within half its width
    /// (a via's radius) and half the minimum width of a node, as KiCad's
    /// connectivity joins a track to every fill outline its copper overlaps
    /// (Castor's U6.5: its exit track and via run along a piece through
    /// nodes the neighbours' clearances keep from the fill, and the piece
    /// held them as two items).
    fn branch_pieces(&self, net: NetId, pours: &crate::pour::PourMap, skip: &dyn Fn(usize) -> bool) -> Vec<Vec<(u32, usize)>> {
        let state = &self.nets[net as usize];
        let pitch = self.grid.pitch;
        let class = self.board.classes[self.board.nets[net as usize].class];
        state
            .branches
            .iter()
            .enumerate()
            .map(|(index, branch)| {
                // Per piece, the straight runs (KiCad's track segments, each
                // an item of the fill outline it touches) that touch it.
                let mut seen: Vec<(u32, usize)> = Vec::new();
                if skip(index) {
                    return seen;
                }
                let mut run = 0usize;
                let mut last: Option<(i64, i64, u8)> = None;
                let add = |seen: &mut Vec<(u32, usize)>, piece: u32, run: usize, marks: &mut Vec<(u32, usize)>| {
                    if marks.contains(&(piece, run)) {
                        return;
                    }
                    marks.push((piece, run));
                    match seen.iter_mut().find(|(seen, _)| *seen == piece) {
                        Some(entry) => entry.1 += 1,
                        None => seen.push((piece, 1)),
                    }
                };
                let mut marks: Vec<(u32, usize)> = Vec::new();
                for (position, node) in branch.nodes.iter().enumerate() {
                    if position > 0 {
                        let previous = branch.nodes[position - 1];
                        let (ax, ay) = self.grid.xy(previous.cell as usize);
                        let (bx, by) = self.grid.xy(node.cell as usize);
                        let step = (bx as i64 - ax as i64, by as i64 - ay as i64, node.layer);
                        if previous.layer != node.layer || last.is_some_and(|last| last != step) {
                            run += 1;
                        }
                        last = (previous.layer == node.layer).then_some(step);
                    }
                    let layer = node.layer as usize;
                    let labels = &pours.label[layer];
                    if labels.is_empty() {
                        continue;
                    }
                    let piece = labels[node.cell as usize];
                    if piece != 0 {
                        add(&mut seen, piece, run, &mut marks);
                        continue;
                    }
                    if !self.config.track_touch_fill {
                        continue;
                    }
                    let via = [position.wrapping_sub(1), position + 1]
                        .iter()
                        .filter_map(|other| branch.nodes.get(*other))
                        .any(|other| other.cell == node.cell && other.layer != node.layer);
                    let copper = if via {
                        class.via_diameter / 2.0
                    } else if let Some((_, _, radius)) = state.adopted.get(&index) {
                        *radius
                    } else {
                        self.width_at(net, node.cell) / 2.0
                    };
                    let reach = copper + self.board.classes[state.plane_class[layer]].trace_width / 2.0;
                    let cells = (reach / pitch).floor() as i64;
                    let (x, y) = self.grid.xy(node.cell as usize);
                    for dy in -cells..=cells {
                        for dx in -cells..=cells {
                            if ((dx * dx + dy * dy) as f64) * pitch * pitch > reach * reach {
                                continue;
                            }
                            let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                            if nx < 0 || ny < 0 || nx >= self.grid.nx as i64 || ny >= self.grid.ny as i64 {
                                continue;
                            }
                            let piece = labels[self.grid.index(nx as usize, ny as usize)];
                            if piece != 0 {
                                add(&mut seen, piece, run, &mut marks);
                            }
                        }
                    }
                }
                seen
            })
            .collect()
    }

    /// A pad's four thermal spokes as KiCad builds them (`PadSpokes` where
    /// the lowering gave the pad's frame; else along the axes of its box,
    /// diagonally for a circle): whether they run diagonally, the point
    /// they leave from, per spoke (in the order of `SpokeRecord::outcomes`:
    /// +x, -x, +y, -y, or +x+y, +x-y, -x+y, -x-y; a turned pad's in the
    /// nearest of those) its direction, the distance from that point to
    /// the pad's edge along it and to the edge grown by `reach`; and the
    /// pad's smaller side (no spoke is wider). 0xCB's 1337 has its parts
    /// at 45 degrees, their spokes too.
    fn spoke_geometry(&self, pad: usize, anchor: crate::geometry::Point, reach: f64) -> (bool, crate::geometry::Point, [(crate::geometry::Point, f64, f64); 4], f64) {
        self.spoke_geometry_in(pad, anchor, reach, self.config.spoke_frames)
    }

    /// `spoke_geometry`, in the pad's own frame only with `frames`.
    fn spoke_geometry_in(&self, pad: usize, anchor: crate::geometry::Point, reach: f64, frames: bool) -> (bool, crate::geometry::Point, [(crate::geometry::Point, f64, f64); 4], f64) {
        let d = std::f64::consts::FRAC_1_SQRT_2;
        let diagonals = [[d, d], [d, -d], [-d, d], [-d, -d]];
        let axes = [[1.0, 0.0], [-1.0, 0.0], [0.0, 1.0], [0.0, -1.0]];
        if let Some(frame) = self.board.pad_spokes.get(&pad).filter(|_| frames) {
            let spokes = frame.spokes(reach);
            let first = spokes[0].0;
            let round = first[0].abs() > 0.3 && first[1].abs() > 0.3;
            let canonical = if round { diagonals } else { axes };
            let mut taken = [false; 4];
            let mut result = [([0.0, 0.0], 0.0, 0.0); 4];
            for (slot, direction) in canonical.iter().enumerate() {
                let best = (0..4)
                    .filter(|index| !taken[*index])
                    .max_by(|a, b| {
                        let dot = |index: usize| spokes[index].0[0] * direction[0] + spokes[index].0[1] * direction[1];
                        dot(*a).total_cmp(&dot(*b)).then(b.cmp(a))
                    })
                    .unwrap();
                taken[best] = true;
                result[slot] = spokes[best];
            }
            return (round, frame.center, result, 2.0 * frame.half[0].min(frame.half[1]));
        }
        let shape = &self.board.obstacles[pad].shape;
        let bounds = shape.aabb();
        let round = matches!(shape, crate::geometry::Shape::Circle { .. });
        let half = [(bounds.maximum[0] - bounds.minimum[0]) / 2.0, (bounds.maximum[1] - bounds.minimum[1]) / 2.0];
        let directions = if round { diagonals } else { axes };
        let mut result = [([0.0, 0.0], 0.0, 0.0); 4];
        for (slot, direction) in directions.into_iter().enumerate() {
            let edge = if round { half[0] } else { (half[0] * direction[0]).abs() + (half[1] * direction[1]).abs() };
            result[slot] = (direction, edge, edge + reach);
        }
        (round, anchor, result, 2.0 * half[0].min(half[1]))
    }

    /// The copper of `branch` leaving terminal `terminal` (one of its
    /// ends) on its first layer: the stub from the pad's anchor to the
    /// lattice (an escape's polyline) and the first straight run, each
    /// segment with half its width. Empty when the branch does not end at
    /// the terminal.
    fn first_run(&self, net: NetId, terminal: usize, branch: &Branch) -> Vec<(crate::geometry::Point, crate::geometry::Point, f64)> {
        let state = &self.nets[net as usize];
        let anchor = self.board.nets[net as usize].terminals[terminal].anchor;
        let mut segments = Vec::new();
        for (end, forward) in [(branch.start_terminal, true), (branch.end_terminal, false)] {
            if end as usize != terminal || branch.nodes.is_empty() {
                continue;
            }
            let nodes: Vec<Node> = if forward { branch.nodes.clone() } else { branch.nodes.iter().rev().copied().collect() };
            let first = self.grid.center_of(nodes[0].cell as usize);
            let half = self.width_at(net, nodes[0].cell) / 2.0;
            match state.escapes.get(&nodes[0]) {
                Some((_, width, polyline)) => segments.extend(polyline.windows(2).map(|pair| (pair[0], pair[1], width / 2.0))),
                None if crate::geometry::distance(anchor, first) > 1.0e-7 => segments.push((anchor, first, half)),
                None => {}
            }
            let step = |a: Node, b: Node| {
                let (ax, ay) = self.grid.xy(a.cell as usize);
                let (bx, by) = self.grid.xy(b.cell as usize);
                (a.layer == b.layer).then_some((bx as i64 - ax as i64, by as i64 - ay as i64))
            };
            if nodes.len() >= 2
                && let Some(direction) = step(nodes[0], nodes[1])
            {
                let mut last = 1;
                while last + 1 < nodes.len() && step(nodes[last], nodes[last + 1]) == Some(direction) {
                    last += 1;
                }
                segments.push((first, self.grid.center_of(nodes[last].cell as usize), half));
            }
        }
        segments
    }

    /// Whether the spokes of a pad on a layer leave it a starved thermal to
    /// KiCad's DRC, and how many more counted spokes it needs: none counted
    /// and none into islands is no thermal connection (not starved); with
    /// tracks counted, the islands no longer stand in for spokes.
    fn spokes_short(record: &SpokeRecord) -> usize {
        let good = record.outcomes.iter().filter(|outcome| matches!(outcome, SpokeOutcome::Good(_) | SpokeOutcome::Pad(_))).count();
        let islands = record.outcomes.iter().filter(|outcome| matches!(outcome, SpokeOutcome::Island(_))).count();
        if good == 0 && islands == 0 && record.crossings == 0 {
            return 0;
        }
        let counted = good + record.crossings + record.tracks;
        if counted == 0 && islands >= 2 {
            return 0;
        }
        2usize.saturating_sub(counted)
    }

    /// For each pad the spoke model finds a starved thermal (fewer than
    /// two spokes as KiCad counts them), straight tracks from the pad into
    /// the main piece of its pour, as many as it is short, each on free
    /// lattice nodes (the net's own rules, no other net's copper): KiCad's
    /// DRC counts a track that starts within half a gap of the pad and
    /// ends in fill as a spoke, and a track keeps the net class's
    /// clearance, often less than the zone's (Castor's C49.2 and U14.2,
    /// in a field of other nets' pads that the fill keeps 0.5 mm from).
    /// Returns the tracks added.
    pub fn add_manual_spokes(&mut self) -> usize {
        let mut added = 0;
        // Every net with a pour, those routed as tracks too (the tracks
        // rung): KiCad fills their zones all the same, with the masks
        // set for the analysis only.
        let nets: Vec<NetId> = (0..self.nets.len() as NetId)
            .filter(|net| self.nets[*net as usize].routable && self.board.planes.iter().any(|plane| plane.net == *net && plane.thermal_gap > 0.0))
            .collect();
        for net in nets {
            let lent = self.nets[net as usize].plane.is_empty();
            if lent {
                let (plane, plane_class) = self.pour_masks(net, false);
                let state = &mut self.nets[net as usize];
                state.plane = plane;
                state.plane_class = plane_class;
            }
            added += self.add_manual_spokes_of(net);
            if lent {
                let class = self.board.nets[net as usize].class;
                let state = &mut self.nets[net as usize];
                state.plane = Vec::new();
                state.plane_class = vec![class; self.board.layer_count];
            }
        }
        added
    }

    /// `add_manual_spokes` for one net whose pour masks are set.
    fn add_manual_spokes_of(&mut self, net: NetId) -> usize {
        let cells = self.grid.cells();
        let nx = self.grid.nx as i64;
        let ny = self.grid.ny as i64;
        let reach = (2.5 / self.grid.pitch).ceil() as i64;
        let mut added = 0;
        {
            // Per pad and layer: tracks tried; a failed try ends its tries.
            let mut tried: HashMap<(usize, usize), usize> = HashMap::new();
            loop {
                let mut report = Vec::new();
                let (pours, mut parent, main, _) = self.analyze_pours_reporting(net, &|_| false, Some(&mut report));
                // A pad merged into its terminal has no nodes of its own to
                // start a track from.
                let own = |record: &SpokeRecord| record.pad == self.board.nets[net as usize].terminals[record.terminal].pad;
                let Some(record) = report.iter().find(|record| own(record) && Self::spokes_short(record) > 0 && tried.get(&(record.terminal, record.layer)).copied().unwrap_or(0) < 2) else {
                    break;
                };
                *tried.entry((record.terminal, record.layer)).or_default() += 1;
                let layer = record.layer;
                // The pieces only this pad's spokes reach (KiCad's isolated
                // islands).
                let islands: Vec<u32> = record
                    .outcomes
                    .iter()
                    .filter_map(|outcome| match outcome {
                        SpokeOutcome::Island(piece) if *piece != 0 => Some(*piece),
                        _ => None,
                    })
                    .collect();
                // The directions of its spokes into an island the lattice
                // misses (a sliver: piece 0), and how far out of the pad's
                // centre the fill begins along them (its edge and the gap).
                let slivers: Vec<(crate::geometry::Point, f64)> = {
                    let terminal = &self.board.nets[net as usize].terminals[record.terminal];
                    let gap = self.board.planes.iter().filter(|plane| plane.net == net && plane.layer == layer).map(|plane| plane.thermal_gap).fold(0.0f64, f64::max);
                    let (_, _, geometry, _) = self.spoke_geometry(terminal.pad, terminal.anchor, gap);
                    record
                        .outcomes
                        .iter()
                        .enumerate()
                        .filter(|(_, outcome)| **outcome == SpokeOutcome::Island(0))
                        .map(|(index, _)| (geometry[index].0, geometry[index].2))
                        .collect()
                };
                let class = self.board.nets[net as usize].class;
                let state = &self.nets[net as usize];
                let own = OwnStamps::new(
                    &state.stamped,
                    &[self.map_index(class, layer), self.diagonal_map(class, layer, 0), self.diagonal_map(class, layer, 1)],
                    cells,
                );
                let free = |map: usize, cell: usize| self.occupancy[map][cell] as usize == own.contains(map, cell) as usize;
                let statics = &self.statics[class];
                let trace = self.map_index(class, layer);
                // The shortest straight track into a kept piece, and into
                // one of the pad's islands.
                let mut best: Option<Vec<Node>> = None;
                let mut best_island: Option<Vec<Node>> = None;
                // The track ends in fill, not on a node the net's own
                // copper made free (Castor's C10.2: a step onto its own
                // track, which KiCad's DRC does not count).
                let rings = self.thermal_ring_pads(net);
                let probe = fill::FillProbe::new(self, net, &rings);
                // Into a piece joined to a pad of the net (KiCad keeps it),
                // the main piece first.
                let terminal_base = pours.pieces + 1;
                let kept: HashSet<usize> = (0..state.terminal_nodes.len()).map(|terminal| find(&mut parent, terminal_base + terminal)).collect();
                let starts: Vec<Node> = state.terminal_nodes[record.terminal].iter().copied().filter(|node| node.layer as usize == layer).collect();
                for start in starts {
                    if !statics.trace_allowed(layer, start.cell as usize, net) || !free(trace, start.cell as usize) {
                        continue;
                    }
                    for (direction, (dx, dy)) in DIRECTIONS.iter().enumerate() {
                        let mut path = vec![start];
                        let (mut x, mut y) = ((start.cell as i64) % nx, (start.cell as i64) / nx);
                        for _ in 0..reach {
                            let previous = (y * nx + x) as usize;
                            let (next_x, next_y) = (x + *dx as i64, y + *dy as i64);
                            if next_x < 0 || next_y < 0 || next_x >= nx || next_y >= ny {
                                break;
                            }
                            let cell = (next_y * nx + next_x) as usize;
                            let diagonal_ok = if *dx != 0 && *dy != 0 {
                                let (from, orientation) = if *dx > 0 { (previous, (*dy < 0) as usize) } else { (cell, (*dy > 0) as usize) };
                                free(self.diagonal_map(class, layer, orientation), from)
                            } else {
                                true
                            };
                            if !statics.trace_allowed(layer, cell, net)
                                || !statics.edge_allowed(layer, previous, direction, net)
                                || !free(trace, cell)
                                || !diagonal_ok
                            {
                                break;
                            }
                            path.push(Node { layer: layer as u8, cell: cell as u32 });
                            let piece = pours.label[layer][cell];
                            if piece != 0 && kept.contains(&find(&mut parent, piece as usize)) && probe.fill_at(layer, self.grid.center_of(cell)) {
                                if best.as_ref().is_none_or(|best| path.len() < best.len()) {
                                    best = Some(path.clone());
                                }
                                break;
                            }
                            // Into an island: one of its pieces, or the
                            // fill along a spoke into a sliver.
                            let along_sliver = || {
                                let (from, to) = (self.grid.center_of(start.cell as usize), self.grid.center_of(cell));
                                let length = crate::geometry::distance(from, to);
                                length > 0.0
                                    && slivers.iter().any(|(direction, reach)| {
                                        ((to[0] - from[0]) * direction[0] + (to[1] - from[1]) * direction[1]) / length > 0.99 && length >= *reach
                                    })
                            };
                            if ((piece != 0 && islands.contains(&piece)) || along_sliver()) && probe.fill_at(layer, self.grid.center_of(cell)) {
                                if best_island.as_ref().is_none_or(|best| path.len() < best.len()) {
                                    best_island = Some(path.clone());
                                }
                                break;
                            }
                            (x, y) = (next_x, next_y);
                        }
                    }
                }
                let label = &self.board.nets[net as usize].terminals[record.terminal].label;
                if let Some(nodes) = best {
                    if self.config.verbose {
                        eprintln!("manual spoke: {label} on layer {layer}, {} nodes", nodes.len());
                    }
                    self.nets[net as usize].branches.push(Branch { nodes, start_terminal: record.terminal as u16, end_terminal: PLANE_TERMINAL });
                    self.stamp(net);
                    added += 1;
                    continue;
                }
                // No track reaches kept fill: a via from an island to the
                // main piece on another layer.
                if self.config.island_vias && !islands.is_empty() && self.board.layer_count >= 2 {
                    let anchor = self.board.nets[net as usize].terminals[record.terminal].anchor;
                    let mut anchors: HashMap<usize, crate::geometry::Point> = HashMap::new();
                    for piece in &islands {
                        anchors.entry(find(&mut parent, *piece as usize)).or_insert(anchor);
                    }
                    let chosen = self.stitching_vias(net, &pours, &mut parent, main, &anchors, true);
                    if let Some((layer, other, cell)) = chosen.first().copied() {
                        if self.config.verbose {
                            eprintln!("island via: {label} on layer {layer} to layer {other}");
                        }
                        self.nets[net as usize].branches.push(Branch {
                            nodes: vec![Node { layer: layer as u8, cell: cell as u32 }, Node { layer: other as u8, cell: cell as u32 }],
                            start_terminal: PLANE_TERMINAL,
                            end_terminal: PLANE_TERMINAL,
                        });
                        self.stamp(net);
                        added += 1;
                        continue;
                    }
                }
                // Else the pad's own track into its island: two items in it.
                if self.config.island_stubs
                    && let Some(nodes) = best_island
                {
                    if self.config.verbose {
                        eprintln!("island stub: {label} on layer {layer}, {} nodes", nodes.len());
                    }
                    self.nets[net as usize].branches.push(Branch { nodes, start_terminal: record.terminal as u16, end_terminal: PLANE_TERMINAL });
                    self.stamp(net);
                    added += 1;
                    continue;
                }
                if std::env::var_os("PCB_ROUTER_SPOKE_DEBUG").is_some() {
                    eprintln!(
                        "manual spoke: none for {label} on layer {layer} ({} start nodes, {} islands)",
                        self.nets[net as usize].terminal_nodes[record.terminal].iter().filter(|node| node.layer as usize == layer).count(),
                        islands.len()
                    );
                }
                tried.insert((record.terminal, record.layer), 2);
            }
        }
        added
    }

    /// Per layer, the pads of `net` whose thermal relief keeps the pour's
    /// fill away, each with the distance the brush centre keeps from it
    /// (the gap and half the brush). A thermal relief keeps its gap around
    /// the pads it connects, all but the spokes: fill does not flow past
    /// such a pad, it joins the pad only through the spokes
    /// (`analyze_pours_full`). PolyKybd's GND ran across a connector's
    /// 0.3 mm pins in this model and fell into islands in KiCad's.
    fn thermal_ring_pads(&self, net: NetId) -> Vec<Vec<(usize, f64)>> {
        let layers = self.board.layer_count;
        let state = &self.nets[net as usize];
        let description = &self.board.nets[net as usize];
        let mut result = vec![Vec::new(); layers];
        for layer in 0..layers {
            if state.plane[layer].is_empty() {
                continue;
            }
            let Some(gap) = self
                .board
                .planes
                .iter()
                .filter(|plane| plane.net == net && plane.layer == layer && !plane.solid)
                .map(|plane| plane.thermal_gap)
                .reduce(f64::max)
                .filter(|gap| *gap > 0.0)
            else {
                continue;
            };
            let reach = gap + self.board.classes[state.plane_class[layer]].trace_width / 2.0;
            let thermal = |label: &str| self.board.isolated_pads.binary_search(&label.to_string()).is_err() && self.board.solid_pads.binary_search(&label.to_string()).is_err();
            for terminal in &description.terminals {
                if !thermal(&terminal.label) {
                    continue;
                }
                if self.board.obstacles[terminal.pad].layers & (1 << layer) == 0 {
                    continue;
                }
                result[layer].push((terminal.pad, reach));
            }
            // KiCad cuts a relief (and in a hatched zone adds a ring) around
            // every thermal pad, a pad merged into its terminal's larger pad
            // too (an exposed pad's thermal vias, on the far side or inside
            // the pad).
            if self.config.inner_pads {
                for merged in self.board.merged_pads.iter().filter(|merged| merged.net == net && merged.layers & (1 << layer) != 0) {
                    if description.terminals.get(merged.host).is_some_and(|host| thermal(&host.label)) {
                        result[layer].push((merged.pad, reach));
                    }
                }
            }
        }
        result
    }

    /// Clears the nodes of `free` (per layer, empty for none) that the
    /// brush may not take inside a thermal ring (`thermal_ring_pads`).
    fn cut_thermal_rings(&self, rings: &[Vec<(usize, f64)>], free: &mut [Vec<bool>]) {
        for (layer, pads) in rings.iter().enumerate() {
            if free[layer].is_empty() {
                continue;
            }
            for (pad, reach) in pads {
                let pad = &self.board.obstacles[*pad];
                let Some((x0, y0, x1, y1)) = self.grid.node_range(pad.shape.aabb().inflated(*reach)) else {
                    continue;
                };
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        if pad.shape.distance_to_point(self.grid.center(x, y)) < *reach {
                            free[layer][self.grid.index(x, y)] = false;
                        }
                    }
                }
            }
        }
    }

    /// Writes the pour map of every pour net into `directory`, one file
    /// per net and layer (`<net>-L<layer>.pour`): a JSON header line (net,
    /// layer, nx, ny, origin, pitch, pieces), then one flag byte per node
    /// (row-major, x fastest) and one little-endian u32 piece label per
    /// node. Flags: 1 inside the pour's polygon (less higher-priority
    /// pours), 2 the static map lets the fill in, 4 no other net's routed
    /// copper stamps the node, 8 inside a thermal ring, 16 free (what the
    /// pieces are labelled from), 32 a node of the net's own branches.
    /// For overlaying KiCad's fill (experiments/pour-fidelity).
    /// For `dump_pours` on a routed board: the pour nets' existing tracks
    /// and vias (fixed obstacles of their own net) become branches, so the
    /// pour model sees them as copper of the net in its pieces, as it sees
    /// its own routes. Branch ends join the branches they meet; no stamping (the obstacles
    /// already hold their nodes for the net). Analysis only.
    pub fn adopt_existing_copper(&mut self) {
        let layers = self.board.layer_count;
        for obstacle in &self.board.obstacles {
            let Some(net) = obstacle.net else {
                continue;
            };
            if obstacle.kind != crate::board::ObstacleKind::Copper
                || !(obstacle.label == "existing track" || obstacle.label == "existing via")
                || self.nets[net as usize].plane.is_empty()
            {
                continue;
            }
            let mut nodes: Vec<Node> = Vec::new();
            match &obstacle.shape {
                crate::geometry::Shape::Capsule { start, end, .. } => {
                    let length = crate::geometry::distance(*start, *end);
                    let steps = ((length / (self.grid.pitch / 4.0)).ceil() as usize).max(1);
                    for layer in (0..layers).filter(|layer| obstacle.layers & (1 << layer) != 0) {
                        for step in 0..=steps {
                            let t = step as f64 / steps as f64;
                            let at = [start[0] + (end[0] - start[0]) * t, start[1] + (end[1] - start[1]) * t];
                            if let Some(cell) = self.grid.nearest_node(at) {
                                let node = Node { layer: layer as u8, cell: cell as u32 };
                                if nodes.last() != Some(&node) {
                                    nodes.push(node);
                                }
                            }
                        }
                    }
                }
                shape => {
                    let bounds = shape.aabb();
                    let center = [(bounds.minimum[0] + bounds.maximum[0]) / 2.0, (bounds.minimum[1] + bounds.maximum[1]) / 2.0];
                    if let Some(cell) = self.grid.nearest_node(center) {
                        for layer in (0..layers).filter(|layer| obstacle.layers & (1 << layer) != 0) {
                            nodes.push(Node { layer: layer as u8, cell: cell as u32 });
                        }
                    }
                }
            }
            if nodes.is_empty() {
                continue;
            }
            // A track's end on a pad of the net ends on that pad: KiCad's
            // connectivity joins a track to a pad where the copper at its
            // end touches the pad's, and its DRC counts only such tracks as spokes
            // (Castor's U11.5: a segment starting 0.15 mm off the pad's end,
            // within half a gap of it, hangs on the pad's exit stub and does
            // not count). Other ends join the branches they meet (KiCad's
            // tracks and vias connect where they touch).
            let mut ends = [NO_TERMINAL, NO_TERMINAL];
            if let crate::geometry::Shape::Capsule { start, end, radius } = &obstacle.shape {
                for (slot, point) in [*start, *end].into_iter().enumerate() {
                    if let Some(terminal) = self.board.nets[net as usize]
                        .terminals
                        .iter()
                        .position(|terminal| {
                            let pad = &self.board.obstacles[terminal.pad];
                            pad.layers & obstacle.layers != 0 && pad.shape.distance_to_point(point) <= *radius + 1.0e-6
                        })
                    {
                        ends[slot] = terminal as u16;
                    }
                }
            }
            if let crate::geometry::Shape::Capsule { start, end, radius } = &obstacle.shape {
                let index = self.nets[net as usize].branches.len();
                self.nets[net as usize].adopted.insert(index, (*start, *end, *radius));
            }
            self.nets[net as usize].branches.push(Branch { nodes, start_terminal: ends[0], end_terminal: ends[1] });
        }
    }

    pub fn dump_pours(&self, directory: &std::path::Path) -> std::io::Result<()> {
        use std::io::Write;
        std::fs::create_dir_all(directory)?;
        let layers = self.board.layer_count;
        let cells = self.grid.cells();
        for net in 0..self.nets.len() as NetId {
            let state = &self.nets[net as usize];
            if state.plane.is_empty() {
                continue;
            }
            let mut spokes = Vec::new();
            let (pours, _, _, _) = self.analyze_pours_reporting(net, &|_| false, Some(&mut spokes));
            let mut rings: Vec<Vec<bool>> = (0..layers)
                .map(|layer| if state.plane[layer].is_empty() { Vec::new() } else { vec![true; cells] })
                .collect();
            self.cut_thermal_rings(&self.thermal_ring_pads(net), &mut rings);
            let own = OwnStamps::new(
                &state.stamped,
                &(0..layers).filter(|layer| !state.plane[*layer].is_empty()).map(|layer| self.map_index(state.plane_class[layer], layer)).collect::<Vec<_>>(),
                cells,
            );
            let name = &self.board.nets[net as usize].name;
            let file_name: String = name
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '_' { c } else { '_' })
                .collect();
            // Per pad and layer, the spokes as the model sees them: one
            // JSON object a line.
            {
                let mut file = std::io::BufWriter::new(std::fs::File::create(directory.join(format!("{file_name}.spokes")))?);
                for record in &spokes {
                    let terminal = &self.board.nets[net as usize].terminals[record.terminal];
                    let outcomes: Vec<String> = record
                        .outcomes
                        .iter()
                        .map(|outcome| match outcome {
                            SpokeOutcome::NoFill => "\"none\"".to_string(),
                            SpokeOutcome::Blocked => "\"blocked\"".to_string(),
                            SpokeOutcome::Island(piece) => format!("\"island {piece}\""),
                            SpokeOutcome::Good(piece) => format!("\"good {piece}\""),
                            SpokeOutcome::Pad(other) => format!("\"good pad {other}\""),
                        })
                        .collect();
                    writeln!(
                        file,
                        "{{\"net\": \"{}\", \"pad\": \"{}\", \"layer\": {}, \"anchor\": [{}, {}], \"round\": {}, \"spokes\": [{}], \"tracks\": {}, \"crossings\": {}}}",
                        name.replace('\\', "\\\\").replace('"', "\\\""),
                        terminal.label.replace('\\', "\\\\").replace('"', "\\\""),
                        record.layer,
                        record.anchor[0],
                        record.anchor[1],
                        record.round,
                        outcomes.join(", "),
                        record.tracks,
                        record.crossings,
                    )?;
                }
            }
            for layer in 0..layers {
                if state.plane[layer].is_empty() {
                    continue;
                }
                let mut flags = vec![0u8; cells];
                for plane in self.board.planes.iter().filter(|plane| plane.net == net && plane.layer == layer && plane.connect) {
                    let Some((x0, y0, x1, y1)) = self.grid.node_range(
                        crate::geometry::Shape::Polygon { points: plane.polygon.clone() }.aabb(),
                    ) else {
                        continue;
                    };
                    for y in y0..=y1 {
                        for x in x0..=x1 {
                            let center = self.grid.center(x, y);
                            if crate::geometry::point_in_polygon(center, &plane.polygon)
                                && !plane.excluded.iter().any(|other| crate::geometry::point_in_polygon(center, other))
                            {
                                flags[self.grid.index(x, y)] |= 1;
                            }
                        }
                    }
                }
                let statics = &self.statics[state.plane_class[layer]];
                let map = self.map_index(state.plane_class[layer], layer);
                for cell in 0..cells {
                    if statics.fill_allowed(layer, cell, net) {
                        flags[cell] |= 2;
                    }
                    if self.occupancy[map][cell] as usize == own.contains(map, cell) as usize {
                        flags[cell] |= 4;
                    }
                    if !rings[layer][cell] {
                        flags[cell] |= 8;
                    }
                    if pours.solid[layer][cell] {
                        flags[cell] |= 16;
                    }
                }
                for branch in &state.branches {
                    for node in branch.nodes.iter().filter(|node| node.layer as usize == layer) {
                        flags[node.cell as usize] |= 32;
                    }
                }
                let path = directory.join(format!("{file_name}-L{layer}.pour"));
                let mut file = std::io::BufWriter::new(std::fs::File::create(&path)?);
                writeln!(
                    file,
                    "{{\"net\": \"{}\", \"layer\": {layer}, \"nx\": {}, \"ny\": {}, \"origin\": [{}, {}], \"pitch\": {}, \"pieces\": {}}}",
                    name.replace('\\', "\\\\").replace('"', "\\\""),
                    self.grid.nx,
                    self.grid.ny,
                    self.grid.origin[0],
                    self.grid.origin[1],
                    self.grid.pitch,
                    pours.pieces,
                )?;
                file.write_all(&flags)?;
                for label in &pours.label[layer] {
                    file.write_all(&label.to_le_bytes())?;
                }
            }
        }
        Ok(())
    }

    /// Stitching vias for pour net `net`: per island in `anchors` (a root
    /// of `parent`, with the point it should be near), the free via spot
    /// closest to that point that joins the island to another piece on
    /// another layer (with `main_only`, to the main piece only; else the
    /// main piece is worth a 5 mm detour), keeping the hole-to-hole
    /// distance from the net's vias and from each other. Returns (layer,
    /// other layer, cell) per via.
    fn stitching_vias(
        &self,
        net: NetId,
        pours: &crate::pour::PourMap,
        parent: &mut Vec<usize>,
        main: usize,
        anchors: &HashMap<usize, crate::geometry::Point>,
        main_only: bool,
    ) -> Vec<(usize, usize, usize)> {
        let class_index = self.board.nets[net as usize].class;
        let class = self.board.classes[class_index];
        let layers = self.board.layer_count;
        let via_reach = 0;
        let via_map = self.map_index(class_index, layers);
        let own = OwnStamps::new(&self.nets[net as usize].stamped, &[via_map], self.grid.cells());
        // A via keeps the hole-to-hole distance from the net's vias
        // already there too: KiCad's rule holds within a net
        // (PolyKybd's stitches landed 0.1 mm from earlier ones).
        let spacing = class.via_drill + self.board.hole_to_hole;
        let mut crowded = vec![false; self.grid.cells()];
        for branch in &self.nets[net as usize].branches {
            for pair in branch.nodes.windows(2) {
                if pair[0].cell != pair[1].cell || pair[0].layer == pair[1].layer {
                    continue;
                }
                let center = self.grid.center_of(pair[0].cell as usize);
                let keep = spacing.max(class.via_diameter);
                let bounds = crate::geometry::Aabb { minimum: center, maximum: center }.inflated(keep);
                if let Some((x0, y0, x1, y1)) = self.grid.node_range(bounds) {
                    for y in y0..=y1 {
                        for x in x0..=x1 {
                            if crate::geometry::distance(center, self.grid.center(x, y)) < keep {
                                crowded[self.grid.index(x, y)] = true;
                            }
                        }
                    }
                }
            }
        }
        let mut best: HashMap<usize, (f64, usize, usize, usize)> = HashMap::new();
        if layers >= 2 {
            let roots: Vec<Vec<usize>> = (0..layers)
                .map(|layer| {
                    pours.label[layer]
                        .iter()
                        .map(|piece| if *piece == 0 { 0 } else { find(parent, *piece as usize) })
                        .collect()
                })
                .collect();
            for layer in 0..layers {
                for (cell, island) in roots[layer].iter().enumerate() {
                    let Some(anchor) = anchors.get(island) else {
                        continue;
                    };
                    if crowded[cell]
                        || !self.statics[class_index].via_allowed(cell, net)
                        || self.occupancy[via_map][cell] as usize
                            != own.contains(via_map, cell) as usize
                        || !pours.solid_around(&self.grid, layer, cell, via_reach)
                    {
                        continue;
                    }
                    for other in (0..layers).filter(|other| *other != layer) {
                        let target = roots[other].get(cell).copied().unwrap_or(0);
                        if target == 0 || target == *island || (main_only && target != main) || !pours.solid_around(&self.grid, other, cell, via_reach) {
                            continue;
                        }
                        let bonus = if target == main { 0.0 } else { 5.0 };
                        let distance = bonus + crate::geometry::distance(*anchor, self.grid.center_of(cell));
                        let entry = best.entry(*island).or_insert((f64::INFINITY, 0, 0, 0));
                        if distance < entry.0 {
                            *entry = (distance, layer, other, cell);
                        }
                    }
                }
            }
        }
        // Two new vias keep the hole-to-hole distance between them.
        let mut chosen: Vec<(usize, usize, usize)> = Vec::new();
        let mut candidates: Vec<(f64, usize, usize, usize, usize)> = best
            .into_iter()
            .filter(|(_, entry)| entry.0.is_finite())
            .map(|(island, (distance, layer, other, cell))| (distance, layer, other, cell, island))
            .collect();
        // Ties broken by the cell, then the island (two islands of one pad
        // on two layers share an anchor and a best cell): a hash map's
        // order must not decide.
        candidates.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap().then(a.3.cmp(&b.3)).then(a.4.cmp(&b.4)));
        for (_, layer, other, cell, _) in candidates {
            let at = self.grid.center_of(cell);
            if chosen.iter().any(|(_, _, placed)| {
                *placed == cell || crate::geometry::distance(at, self.grid.center_of(*placed)) < spacing.max(class.via_diameter)
            }) {
                continue;
            }
            chosen.push((layer, other, cell));
        }
        chosen
    }

    /// Connects pour islands that hold terminals to the main piece with
    /// vias; terminals that cannot be reached that way are routed to the
    /// main piece like off-pour pads. Returns the number of stitching vias.
    /// Plane ends of `net`'s branches that lie in no pour piece any more
    /// (other nets' copper took the fill after the branch was routed) and
    /// hang past a junction with another branch of the net: the tail is
    /// trimmed back to the junction, which now ends the branch
    /// (USB_Cable_Tester: a /SHLD branch ran 0.1 mm past a junction into
    /// fill that signals rerouted after a part moved took; KiCad reports
    /// the stub as a dangling track). Returns the nodes trimmed.
    fn trim_dangling_plane_ends(&mut self, net: NetId) -> usize {
        if self.nets[net as usize].plane.is_empty() {
            return 0;
        }
        let (pours, _, _) = self.analyze_pours(net);
        let state = &self.nets[net as usize];
        let mut uses: HashMap<Node, usize> = HashMap::new();
        for branch in &state.branches {
            for node in &branch.nodes {
                *uses.entry(*node).or_default() += 1;
            }
        }
        let label = |node: &Node| pours.label[node.layer as usize].get(node.cell as usize).copied().unwrap_or(0);
        // (branch, from its end, nodes to drop)
        let mut cuts: Vec<(usize, bool, usize)> = Vec::new();
        for (index, branch) in state.branches.iter().enumerate().skip(state.fixed) {
            for at_end in [true, false] {
                let terminal = if at_end { branch.end_terminal } else { branch.start_terminal };
                if terminal != PLANE_TERMINAL || branch.nodes.len() < 3 {
                    continue;
                }
                let nodes: Vec<&Node> = if at_end { branch.nodes.iter().rev().collect() } else { branch.nodes.iter().collect() };
                if label(nodes[0]) != 0 {
                    continue;
                }
                let Some(stop) = nodes.iter().position(|node| label(node) != 0 || uses.get(node).copied().unwrap_or(0) > 1) else {
                    continue;
                };
                // Only back to a junction, the branch keeping some length.
                if stop > 0 && uses.get(nodes[stop]).copied().unwrap_or(0) > 1 && label(nodes[stop]) == 0 && stop + 1 < nodes.len() && nodes[stop].layer == nodes[stop - 1].layer {
                    cuts.push((index, at_end, stop));
                }
            }
        }
        if cuts.is_empty() {
            return 0;
        }
        self.unstamp(net);
        let state = &mut self.nets[net as usize];
        let mut trimmed = 0;
        for (index, at_end, drop) in cuts {
            let branch = &mut state.branches[index];
            if at_end {
                branch.nodes.truncate(branch.nodes.len() - drop);
                branch.end_terminal = NO_TERMINAL;
            } else {
                branch.nodes.drain(..drop);
                branch.start_terminal = NO_TERMINAL;
            }
            trimmed += drop;
        }
        self.stamp(net);
        trimmed
    }

    fn stitch_pours(&mut self, net: NetId, order: &[NetId]) -> usize {
        let class = self.board.classes[self.board.nets[net as usize].class];
        let layers = self.board.layer_count;
        let terminal_count = self.nets[net as usize].terminal_nodes.len();
        let _ = class;
        let mut stitches = 0;
        let mut rerouted = false;
        let mut previous_islands = usize::MAX;
        let mut stalled = 0;
        for _ in 0..1500 {
            let (pours, mut parent, main, spoke_islands) = self.analyze_pours_full(net, &|_| false);
            let terminal_base = pours.pieces + 1;
            let islands: Vec<usize> = (0..terminal_count)
                .filter(|terminal| find(&mut parent, terminal_base + terminal) != main)
                .collect();
            if islands.is_empty() && spoke_islands.is_empty() {
                break;
            }
            // Vias that do not reduce the stranded count are not progress.
            let island_count = islands.len() + spoke_islands.len();
            if island_count >= previous_islands {
                stalled += 1;
                if stalled > 3 && rerouted {
                    break;
                }
            } else {
                stalled = 0;
            }
            previous_islands = island_count;
            if std::env::var_os("PCB_ROUTER_DEBUG").is_some() {
                for terminal in islands.iter().take(6) {
                    let root = find(&mut parent, terminal_base + terminal);
                    let branches = (0..self.nets[net as usize].branches.len())
                        .filter(|index| {
                            find(&mut parent, terminal_base + terminal_count + index) == root
                        })
                        .count();
                    let pieces: Vec<usize> = (1..=pours.pieces)
                        .filter(|piece| find(&mut parent, *piece) == root)
                        .collect();
                    let sizes: Vec<usize> = pieces
                        .iter()
                        .take(4)
                        .map(|piece| {
                            pours
                                .label
                                .iter()
                                .map(|layer| layer.iter().filter(|l| **l == *piece as u32).count())
                                .sum()
                        })
                        .collect();
                    for (index, branch) in self.nets[net as usize].branches.iter().enumerate() {
                        if find(&mut parent, terminal_base + terminal_count + index) != root {
                            continue;
                        }
                        let ends = |terminal: u16| match terminal {
                            NO_TERMINAL => "junction".to_string(),
                            PLANE_TERMINAL => "plane".to_string(),
                            FREE_END => "free".to_string(),
                            other => format!("pad {other}"),
                        };
                        let first = branch.nodes[0];
                        let last = *branch.nodes.last().unwrap();
                        eprintln!(
                            "    branch {index}: {} -> {}, {} nodes, first {:?} label {}, last {:?} label {}, on_plane {:?}",
                            ends(branch.start_terminal),
                            ends(branch.end_terminal),
                            branch.nodes.len(),
                            self.grid.center_of(first.cell as usize),
                            pours.label[first.layer as usize].get(first.cell as usize).copied().unwrap_or(0),
                            self.grid.center_of(last.cell as usize),
                            pours.label[last.layer as usize].get(last.cell as usize).copied().unwrap_or(0),
                            self.nets[net as usize].on_plane.get(*terminal).copied()
                        );
                    }
                    eprintln!(
                        "  stranded {} at {:?} layers {:#b}: {branches} branches, {} pieces (sizes {sizes:?}), terminal nodes {}",
                        self.board.nets[net as usize].terminals[*terminal].label,
                        self.board.nets[net as usize].terminals[*terminal].anchor,
                        self.board.nets[net as usize].terminals[*terminal].layers,
                        pieces.len(),
                        self.nets[net as usize].terminal_nodes[*terminal].len()
                    );
                }
            }
            if self.config.verbose {
                let attached = islands
                    .iter()
                    .filter(|terminal| {
                        let root = find(&mut parent, terminal_base + **terminal);
                        (1..=pours.pieces).any(|piece| find(&mut parent, piece) == root)
                    })
                    .count();
                eprintln!(
                    "pour {}: {} pieces, {} stranded terminals ({} touch a piece), {} spoke islands",
                    self.board.nets[net as usize].name,
                    pours.pieces,
                    islands.len(),
                    attached,
                    spoke_islands.len()
                );
            }
            // Per stranded island, the via closest to its terminal that joins
            // it to another piece (the main piece is worth a detour), all
            // found in one pass over the board and added together: one via
            // per pass and island made a ground net with 300 stranded pads
            // cost 300 board-wide floods. Islands without room stay for
            // rerouting.
            let mut anchors: HashMap<usize, crate::geometry::Point> = HashMap::new();
            for terminal in &islands {
                let island = find(&mut parent, terminal_base + terminal);
                anchors.entry(island).or_insert(self.board.nets[net as usize].terminals[*terminal].anchor);
            }
            // A spoke island gets a via too: with copper of the net in it
            // KiCad keeps it, and the spoke counts.
            for (piece, terminal) in &spoke_islands {
                let island = find(&mut parent, *piece);
                anchors.entry(island).or_insert(self.board.nets[net as usize].terminals[*terminal].anchor);
            }
            let chosen = self.stitching_vias(net, &pours, &mut parent, main, &anchors, false);
            if self.config.verbose && !spoke_islands.is_empty() {
                let into_spoke_islands = chosen
                    .iter()
                    .filter(|(layer, _, cell)| {
                        let root = find(&mut parent, pours.label[*layer][*cell] as usize);
                        spoke_islands.iter().any(|(piece, _)| find(&mut parent, *piece) == root)
                    })
                    .count();
                eprintln!(
                    "pour {}: {} vias into the {} spoke islands",
                    self.board.nets[net as usize].name,
                    into_spoke_islands,
                    spoke_islands.len()
                );
            }
            if !chosen.is_empty() {
                for (layer, other, cell) in &chosen {
                    self.nets[net as usize].branches.push(Branch {
                        nodes: vec![
                            Node {
                                layer: *layer as u8,
                                cell: *cell as u32,
                            },
                            Node {
                                layer: *other as u8,
                                cell: *cell as u32,
                            },
                        ],
                        start_terminal: PLANE_TERMINAL,
                        end_terminal: PLANE_TERMINAL,
                    });
                }
                self.stamp(net);
                stitches += chosen.len();
                continue;
            }
            // Spoke islands without room for a via stay as they are.
            if rerouted || islands.is_empty() {
                break;
            }
            // No via fits: route the stranded terminals to the main piece.
            rerouted = true;
            stalled = 0;
            let mut target: Vec<Vec<bool>> = Vec::new();
            for layer in 0..layers {
                target.push(
                    pours.label[layer]
                        .iter()
                        .map(|piece| *piece != 0 && find(&mut parent, *piece as usize) == main)
                        .collect(),
                );
            }
            for terminal in &islands {
                self.nets[net as usize].on_plane[*terminal] = false;
            }
            self.nets[net as usize].plane_target = target;
            self.unstamp(net);
            self.route_net_seq(net, 0.0, true, 1.0);
            self.stamp(net);
            let hard_complete = self.nets[net as usize].complete;
            let mut forced = None;
            if !hard_complete {
                forced = Some(self.force_connect(net, order));
            }
            if self.config.verbose {
                eprintln!(
                    "pour {}: reroute of {} stranded terminals: hard complete {hard_complete}, forced {forced:?}, {} branches",
                    self.board.nets[net as usize].name,
                    islands.len(),
                    self.nets[net as usize].branches.len()
                );
            }
            self.nets[net as usize].plane_target = Vec::new();
        }
        let (pours, mut parent, main) = self.analyze_pours(net);
        let terminal_base = pours.pieces + 1;
        let mut complete = true;
        for terminal in 0..terminal_count {
            let connected = find(&mut parent, terminal_base + terminal) == main;
            self.nets[net as usize].connected[terminal] = connected;
            complete &= connected;
        }
        self.nets[net as usize].complete = complete;
        stitches
    }

    /// Length plus via cost of a net's branches, in millimetres.
    fn geometric_cost(&self, net: NetId) -> f64 {
        self.geometric_cost_of(net, &self.nets[net as usize].branches)
    }

    /// `geometric_cost` of `net` with these branches.
    fn geometric_cost_of(&self, net: NetId, branches: &[Branch]) -> f64 {
        let nx = self.grid.nx as i64;
        let mut cost = 0.0;
        for branch in branches {
            for pair in branch.nodes.windows(2) {
                if pair[0].cell == pair[1].cell {
                    cost += if self.nets[net as usize].plane.is_empty() {
                        self.config.cleanup_via_cost
                    } else {
                        self.config.plane_via_cost
                    };
                } else {
                    let dx = (pair[0].cell as i64 % nx - pair[1].cell as i64 % nx).abs();
                    let dy = (pair[0].cell as i64 / nx - pair[1].cell as i64 / nx).abs();
                    cost += self.grid.pitch * ((dx * dx + dy * dy) as f64).sqrt();
                }
            }
        }
        cost
    }

    /// The cells where `net`'s stamps came or went since `old` (a stamp the
    /// net had before and after leaves its cell's count as it was).
    fn changed_cells(&self, net: NetId, old: Vec<(u32, u32)>) -> Vec<u32> {
        let mut before: HashSet<(u32, u32)> = old.into_iter().collect();
        let mut changed: Vec<u32> = Vec::new();
        for entry in &self.nets[net as usize].stamped {
            if !before.remove(entry) {
                changed.push(entry.1);
            }
        }
        changed.extend(before.into_iter().map(|(_, cell)| cell));
        changed
    }

    /// Marks the read blocks around `cells` (and the block next to each:
    /// a search step reads its neighbours) dirty, listing what it marked.
    fn mark_dirty_blocks(&self, cells: &[u32], dirty: &mut [bool], marked: &mut Vec<usize>) {
        let nx = self.grid.nx;
        let blocks_x = nx.div_ceil(1 << READ_BLOCK_SHIFT);
        let blocks_y = self.grid.ny.div_ceil(1 << READ_BLOCK_SHIFT);
        for cell in cells {
            let (x, y) = (*cell as usize % nx, *cell as usize / nx);
            let (bx, by) = (x >> READ_BLOCK_SHIFT, y >> READ_BLOCK_SHIFT);
            for dy in by.saturating_sub(1)..=(by + 1).min(blocks_y - 1) {
                for dx in bx.saturating_sub(1)..=(bx + 1).min(blocks_x - 1) {
                    let block = dy * blocks_x + dx;
                    if !dirty[block] {
                        dirty[block] = true;
                        marked.push(block);
                    }
                }
            }
        }
    }

    /// The clean-up of a run of single-net batches, with the result of
    /// cleaning them up one after the other, on all threads: every net is
    /// rerouted speculatively side by side against the board as it is;
    /// then, in order, a net's trial stands when no net before it in the
    /// run changed the board where the trial read it (only an improvement
    /// changes the board), and is redone in turn otherwise. Returns the
    /// improvements, or `None` when the budget ran out (checked before
    /// every net, as in turn).
    fn clean_up_window(
        &mut self,
        nets: &[NetId],
        started: &(f64, std::time::Instant),
        budget: f64,
        check: bool,
    ) -> Option<usize> {
        let (nx, ny) = (self.grid.nx, self.grid.ny);
        let tiles = self.tiles_x * self.tiles_y;
        let states = self.grid.cells() * self.board.layer_count;
        let cells = self.grid.cells();
        let speculation_started = std::time::Instant::now();
        let this: &Self = self;
        // A pour net's search reads its brush class's maps, where its own
        // old stamps would stay unmasked: it is cleaned up in turn.
        let trials: Vec<Option<(NetState, [u64; 5], Vec<u32>)>> = nets
            .par_iter()
            .map(|net| {
                if !this.nets[*net as usize].plane.is_empty() {
                    return None;
                }
                Some(with_thread_scratch(states, cells, |scratch| {
                    scratch.read.start(nx, ny, tiles);
                    let mut net_state = this.nets[*net as usize].clone();
                    // As `rip_up` leaves it; the old stamps stay on the
                    // board, masked.
                    net_state.branches.truncate(net_state.fixed);
                    net_state.connected.fill(false);
                    net_state.complete = false;
                    if !net_state.stamped.is_empty() {
                        this.mask_own(scratch, *net, &net_state);
                    }
                    let before = scratch.counters();
                    this.route_net(scratch, *net, &mut net_state, 0.0, true, 1.0);
                    let after = scratch.counters();
                    scratch.unmask_own();
                    let mut delta = [0u64; 5];
                    for (index, value) in delta.iter_mut().enumerate() {
                        *value = after[index] - before[index];
                    }
                    let (blocks, _) = scratch.read.finish();
                    (net_state, delta, blocks)
                }))
            })
            .collect();
        self.profile.speculative_seconds += speculation_started.elapsed().as_secs_f64();
        let blocks = nx.div_ceil(1 << READ_BLOCK_SHIFT) * ny.div_ceil(1 << READ_BLOCK_SHIFT);
        let mut dirty = vec![false; blocks];
        let mut marked = Vec::new();
        let mut improvements = 0;
        for (net, trial) in nets.iter().copied().zip(trials) {
            if self.spent(started, budget) {
                return None;
            }
            let old = self.nets[net as usize].stamped.clone();
            let Some((mut state, counters, read)) = trial else {
                if !self.clean_up_batch(&[net]).is_empty() {
                    improvements += 1;
                    let changed = self.changed_cells(net, old);
                    self.mark_dirty_blocks(&changed, &mut dirty, &mut marked);
                }
                continue;
            };
            let valid = !read.iter().any(|block| dirty[*block as usize]);
            let improved = if valid && !check {
                self.profile.speculation_kept += 1;
                self.scratch.searches += counters[0];
                self.scratch.expansions += counters[1];
                self.scratch.open_searches += counters[2];
                self.scratch.open_expansions += counters[3];
                self.scratch.failed_searches += counters[4];
                let before = self.geometric_cost(net);
                let better = state.complete && self.geometric_cost_of(net, &state.branches) < before - 1.0e-6;
                if !better {
                    state.branches = self.nets[net as usize].branches.clone();
                    state.connected.fill(true);
                    state.complete = true;
                }
                state.stamped = std::mem::take(&mut self.nets[net as usize].stamped);
                self.nets[net as usize] = state;
                self.stamp(net);
                better
            } else {
                let speculative_better =
                    state.complete && self.geometric_cost_of(net, &state.branches) < self.geometric_cost(net) - 1.0e-6;
                let better = !self.clean_up_batch(&[net]).is_empty();
                if valid {
                    // Checking: a trial that stands must be the one in turn.
                    let same = speculative_better == better
                        && (!better
                            || state.branches.len() == self.nets[net as usize].branches.len()
                                && state.branches.iter().zip(&self.nets[net as usize].branches).all(|(a, b)| a.nodes == b.nodes));
                    if same {
                        self.profile.speculation_kept += 1;
                    } else {
                        self.profile.speculation_wrong += 1;
                        eprintln!("speculation check: clean-up of {} differs", self.board.nets[net as usize].name);
                    }
                } else {
                    self.profile.speculation_redone += 1;
                }
                better
            };
            if improved {
                improvements += 1;
                let changed = self.changed_cells(net, old);
                self.mark_dirty_blocks(&changed, &mut dirty, &mut marked);
            }
        }
        Some(improvements)
    }

    /// One batch of the clean-up (see `clean_up`): its nets rerouted alone
    /// against all other copper, each new route kept when it is complete,
    /// inside its window and strictly cheaper. Returns the nets improved.
    fn clean_up_batch(&mut self, batch: &[NetId]) -> Vec<NetId> {
        let mut improved = Vec::new();
        let before: Vec<f64> = batch.iter().map(|net| self.geometric_cost(*net)).collect();
        let saved: Vec<Vec<Branch>> = batch.iter().map(|net| self.nets[*net as usize].branches.clone()).collect();
        for net in batch {
            self.rip_up(*net);
        }
        let mut inside = vec![true; batch.len()];
        if batch.len() == 1 {
            self.route_net_seq(batch[0], 0.0, true, 1.0);
        } else {
            let states = self.grid.cells() * self.board.layer_count;
            let cells = self.grid.cells();
            let this: &Self = self;
            let routed: Vec<(NetState, bool, u64)> = batch
                .par_iter()
                .map(|net| {
                    with_thread_scratch(states, cells, |scratch| {
                        let mut net_state = this.nets[*net as usize].clone();
                        if !net_state.stamped.is_empty() {
                            this.mask_own(scratch, *net, &net_state);
                        }
                        let before = scratch.expansions;
                        this.route_net(scratch, *net, &mut net_state, 0.0, true, 1.0);
                        let expansions = scratch.expansions - before;
                        scratch.unmask_own();
                        // Batch windows are kept two tiles apart:
                        // routes inside their own window stay clear
                        // of each other.
                        let window = this.window(*net, 1.0);
                        let within = net_state.branches.iter().flat_map(|branch| &branch.nodes).all(|node| {
                            let (x, y) = this.grid.xy(node.cell as usize);
                            x >= window.0 && x <= window.2 && y >= window.1 && y <= window.3
                        });
                        (net_state, within, expansions)
                    })
                })
                .collect();
            for (index, (net_state, within, expansions)) in routed.into_iter().enumerate() {
                self.nets[batch[index] as usize] = net_state;
                inside[index] = within;
                // The work of the threads counts on the shared clock.
                self.scratch.expansions += expansions;
            }
        }
        for (index, net) in batch.iter().enumerate() {
            let state = &self.nets[*net as usize];
            if inside[index] && state.complete && self.geometric_cost(*net) < before[index] - 1.0e-6 {
                improved.push(*net);
            } else {
                let state = &mut self.nets[*net as usize];
                state.branches = saved[index].clone();
                state.connected.fill(true);
                state.complete = true;
            }
            self.stamp(*net);
        }
        improved
    }

    /// Reroutes each complete net alone against all other copper and keeps
    /// the result when it is strictly cheaper. Returns the improvements.
    fn clean_up(&mut self, order: &[NetId]) -> usize {
        self.cleanup = true;
        let mut improvements = 0;
        let mut pour_stranded = self.config.pour_aware_polish.then(|| self.pour_stranded()).filter(|counts| !counts.is_empty());
        let mut pour_check_seconds = 0.0;
        let mut pour_rejected = 0;
        let mut pour_analyses = 0usize;
        let (pour_nets, pour_bounds) = if pour_stranded.is_some() {
            let nets = self.pour_net_ids();
            let bounds: Vec<_> = nets.iter().map(|net| self.plane_bounds(*net)).collect();
            (nets, bounds)
        } else {
            (Vec::new(), Vec::new())
        };
        // How far a stamp reaches from its node, generously: the widest
        // via and track of any class with the largest clearances, twice.
        let pour_reach = {
            let widest = self
                .board
                .classes
                .iter()
                .map(|class| class.via_diameter + class.trace_width + class.clearance)
                .fold(0.0f64, f64::max)
                + self.board.hole_clearance.max(self.board.hole_to_hole);
            (2.0 * widest / self.grid.pitch).ceil() as usize + 3
        };
        let started = self.clock();
        let pour_work_before = self.pour_work_seconds();
        let budget = self.config.cleanup_seconds.min(self.negotiation_seconds.max(60.0));
        'passes: for _ in 0..self.config.cleanup_passes {
            let mut improved = false;
            let complete: Vec<NetId> = order.iter().copied().filter(|net| self.nets[*net as usize].complete).collect();
            // Nets whose windows do not overlap are cleaned up at once; a
            // new route that leaves its window is not kept.
            let batches: Vec<Vec<NetId>> = if self.config.parallel {
                self.batches(&complete, 1.0).into_iter().map(|(members, _)| members).collect()
            } else {
                complete.iter().map(|net| vec![*net]).collect()
            };
            let threads = rayon::current_num_threads();
            // The speculative clean-up has no pour check: off with it.
            let speculate = self.config.parallel
                && self.config.speculate_cleanup
                && pour_stranded.is_none()
                && threads > 1
                && std::env::var_os("PCB_ROUTER_NO_SPECULATION").is_none();
            let check = std::env::var_os("PCB_ROUTER_SPECULATION_CHECK").is_some();
            let mut index = 0;
            while index < batches.len() {
                // Single-net batches side by side, speculatively (see
                // `clean_up_window`).
                let mut end = index;
                let window = std::env::var("PCB_ROUTER_CLEANUP_WINDOW").ok().and_then(|value| value.parse().ok()).unwrap_or(2 * threads);
                while speculate && end < batches.len() && batches[end].len() == 1 && end - index < window {
                    end += 1;
                }
                if end - index > 1 {
                    let nets: Vec<NetId> = batches[index..end].iter().map(|batch| batch[0]).collect();
                    match self.clean_up_window(&nets, &started, budget, check) {
                        Some(found) => {
                            improvements += found;
                            improved |= found > 0;
                        }
                        None => {
                            if self.config.verbose {
                                eprintln!("clean-up budget of {budget:.0} s used");
                            }
                            break 'passes;
                        }
                    }
                    index = end;
                    continue;
                }
                let batch = batches[index].clone();
                index += 1;
                if self.spent(&started, budget) {
                    if self.config.verbose {
                        eprintln!("clean-up budget of {budget:.0} s used");
                    }
                    break 'passes;
                }
                let before: Vec<f64> = batch.iter().map(|net| self.geometric_cost(*net)).collect();
                let saved: Vec<Vec<Branch>> = batch.iter().map(|net| self.nets[*net as usize].branches.clone()).collect();
                for net in &batch {
                    self.rip_up(*net);
                }
                let mut inside = vec![true; batch.len()];
                if batch.len() == 1 {
                    self.route_net_seq(batch[0], 0.0, true, 1.0);
                } else {
                    let states = self.grid.cells() * self.board.layer_count;
                    let cells = self.grid.cells();
                    let this: &Self = self;
                    let routed: Vec<(NetState, bool, u64)> = batch
                        .par_iter()
                        .map(|net| {
                            with_thread_scratch(states, cells, |scratch| {
                                let mut net_state = this.nets[*net as usize].clone();
                                if !net_state.stamped.is_empty() {
                                    this.mask_own(scratch, *net, &net_state);
                                }
                                let before = scratch.expansions;
                                this.route_net(scratch, *net, &mut net_state, 0.0, true, 1.0);
                                let expansions = scratch.expansions - before;
                                scratch.unmask_own();
                                // Batch windows are kept two tiles apart:
                                // routes inside their own window stay clear
                                // of each other.
                                let window = this.window(*net, 1.0);
                                let within = net_state.branches.iter().flat_map(|branch| &branch.nodes).all(|node| {
                                    let (x, y) = this.grid.xy(node.cell as usize);
                                    x >= window.0 && x <= window.2 && y >= window.1 && y <= window.3
                                });
                                (net_state, within, expansions)
                            })
                        })
                        .collect();
                    for (index, (net_state, within, expansions)) in routed.into_iter().enumerate() {
                        self.nets[batch[index] as usize] = net_state;
                        inside[index] = within;
                        // The work of the threads counts on the shared clock.
                        self.scratch.expansions += expansions;
                    }
                }
                let mut accepted: Vec<bool> = Vec::with_capacity(batch.len());
                for (index, net) in batch.iter().enumerate() {
                    let state = &self.nets[*net as usize];
                    let better = inside[index] && state.complete && self.geometric_cost(*net) < before[index] - 1.0e-6;
                    if !better {
                        let state = &mut self.nets[*net as usize];
                        state.branches = saved[index].clone();
                        state.connected.fill(true);
                        state.complete = true;
                    }
                    accepted.push(better);
                    self.stamp(*net);
                }
                // A shorter route that cuts a pour's pad off its main piece
                // is no improvement (k30's +5V: 54 pads stranded at the
                // stitching after the polish, 3 without it).
                if let Some(stranded) = pour_stranded.as_mut()
                    && accepted.iter().any(|kept| *kept)
                {
                    let checked = std::time::Instant::now();
                    // Only pour nets whose planes come within a stamp's
                    // reach of the copper that changed can count otherwise
                    // than before; the others keep their count.
                    let mut changed: Option<(usize, usize, usize, usize)> = None;
                    for (index, net) in batch.iter().enumerate() {
                        if !accepted[index] {
                            continue;
                        }
                        for node in saved[index].iter().chain(&self.nets[*net as usize].branches).flat_map(|branch| &branch.nodes) {
                            let (x, y) = self.grid.xy(node.cell as usize);
                            changed = Some(changed.map_or((x, y, x, y), |(x0, y0, x1, y1)| (x0.min(x), y0.min(y), x1.max(x), y1.max(y))));
                        }
                    }
                    let now: Vec<usize> = pour_nets
                        .iter()
                        .zip(&pour_bounds)
                        .zip(stranded.iter())
                        .map(|((net, bounds), before)| {
                            let near = batch.iter().enumerate().any(|(index, other)| accepted[index] && other == net)
                                || match (changed, bounds) {
                                    (Some((x0, y0, x1, y1)), &Some((px0, py0, px1, py1))) => {
                                        x0 <= px1 + pour_reach && px0 <= x1 + pour_reach && y0 <= py1 + pour_reach && py0 <= y1 + pour_reach
                                    }
                                    _ => false,
                                };
                            if near {
                                pour_analyses += 1;
                                self.pour_stranded_of(*net)
                            } else {
                                *before
                            }
                        })
                        .collect();
                    pour_check_seconds += checked.elapsed().as_secs_f64();
                    if now.iter().zip(stranded.iter()).any(|(now, before)| now > before) {
                        for (index, net) in batch.iter().enumerate() {
                            if accepted[index] {
                                self.unstamp(*net);
                                let state = &mut self.nets[*net as usize];
                                state.branches = saved[index].clone();
                                state.connected.fill(true);
                                state.complete = true;
                                self.stamp(*net);
                                accepted[index] = false;
                            }
                        }
                        pour_rejected += 1;
                    } else {
                        *stranded = now;
                    }
                }
                for kept in accepted {
                    if kept {
                        improved = true;
                        improvements += 1;
                    }
                }
            }
            if !improved {
                break;
            }
        }
        self.cleanup = false;
        if self.config.verbose && pour_stranded.is_some() {
            eprintln!(
                "clean-up: {pour_rejected} batches kept out for stranding pour pads, {pour_check_seconds:.1} s of pour checks ({pour_analyses} analyses); {:.1} s of work of a budget of {budget:.0} s, {:.1} s of it pour analyses, in {:.1} s of wall clock",
                self.budget_work() - started.0,
                self.pour_work_seconds() - pour_work_before,
                started.1.elapsed().as_secs_f64()
            );
        }
        improvements
    }

    /// Makes the board conflict free: nets still in conflict are ripped up
    /// (most contested first) and reinserted treating all other copper as
    /// hard obstacles.
    fn resolve_remaining(&mut self, order: &[NetId]) {
        let mut removed = Vec::new();
        loop {
            let worst = order
                .iter()
                .filter(|net| !removed.contains(*net))
                .map(|net| (self.conflicts(*net).len(), *net))
                .filter(|(count, _)| *count > 0)
                .max();
            let Some((_, net)) = worst else {
                break;
            };
            self.rip_up(net);
            removed.push(net);
        }
        for net in order {
            if !removed.contains(net) && !self.nets[*net as usize].complete {
                self.rip_up(*net);
                removed.push(*net);
            }
        }
        removed.sort_by_key(|net| order.iter().position(|other| other == net));
        // The hard reroute is a repair after negotiation: on a rung judged
        // hopeless it is not worth its time (the ladder moves on), and it
        // never takes longer than the negotiation did (at least 60 s): a
        // failing hard search sweeps four ever larger windows, 9 s a net on
        // a 4.8M-node board, 960 s for 107 nets. Nets it does not reach stay
        // ripped up (open, as they were in conflict).
        let budget = if self.hopeless { 0.0 } else { self.negotiation_seconds.max(60.0) };
        let started = std::time::Instant::now();
        let clock = self.clock();
        let mut skipped = 0;
        for net in &removed {
            if self.spent(&clock, budget) {
                skipped += 1;
                continue;
            }
            self.route_net_seq(*net, 0.0, true, 4.0);
            self.stamp(*net);
        }
        let rerouted = started.elapsed().as_secs_f64();
        let mut forced = 0;
        for net in removed.iter().copied() {
            if !self.nets[net as usize].complete && !self.spent(&clock, 2.0 * budget) {
                forced += 1;
                if self.force_connect(net, order) {
                    continue;
                }
            }
        }
        if self.config.verbose && !removed.is_empty() {
            eprintln!(
                "resolve: {} nets rerouted hard in {rerouted:.1} s ({skipped} left open by the budget), {forced} forced in {:.1} s",
                removed.len() - skipped,
                started.elapsed().as_secs_f64() - rerouted
            );
        }
    }

    /// A pad-centre stub is cosmetic (the track already ends inside the pad),
    /// so it is only emitted when it is exactly legal against fixed objects.
    fn stub_is_clear(
        &self,
        net: NetId,
        layer: usize,
        start: crate::geometry::Point,
        end: crate::geometry::Point,
        width: f64,
    ) -> bool {
        let class = self.board.classes[self.board.nets[net as usize].class];
        let half_width = width / 2.0;
        self.board.obstacles.iter().all(|obstacle| {
            if obstacle.net == Some(net)
                || obstacle.layers & (1 << layer) == 0
                || !obstacle.blocks_tracks
            {
                return true;
            }
            // A pad inside a mask opening leaves it by its stub: the only
            // way out, as the designer's tracks took (Sisu's J_LCD1 opening
            // over its eight pads keeps everything else out). A rule area
            // forbids tracks outright (Sisu's module keepout).
            if obstacle.kind == crate::board::ObstacleKind::Keepout
                && obstacle.label.starts_with("solder mask opening")
                && obstacle.shape.contains(start)
            {
                return true;
            }
            // Stubs are exact geometry: an exact fit passes, as in KiCad
            // (a 0.2 track out of a 0.4 pitch QFN's 0.2 pad keeps exactly
            // 0.2 from both neighbours, RP2040's U15).
            let required = half_width - STUB_FIT
                + match obstacle.kind {
                    crate::board::ObstacleKind::Copper => {
                        self.board.copper_clearance(&class, obstacle)
                    }
                    crate::board::ObstacleKind::Keepout => obstacle.clearance,
                    crate::board::ObstacleKind::Hole => {
                        self.board.hole_clearance.max(obstacle.clearance)
                    }
                };
            let stub = crate::geometry::Aabb {
                minimum: [start[0].min(end[0]), start[1].min(end[1])],
                maximum: [start[0].max(end[0]), start[1].max(end[1])],
            };
            !stub.intersects(obstacle.shape.aabb().inflated(required))
                || obstacle.shape.distance_to_segment(start, end) >= required
        })
    }

    /// Whether a stub of `width` from `start` to `end` lies inside the board
    /// and keeps the edge clearance from the outline. Cutouts are obstacles
    /// and are checked by `stub_is_clear`.
    fn stub_keeps_edge(
        &self,
        start: crate::geometry::Point,
        end: crate::geometry::Point,
        width: f64,
    ) -> bool {
        let outline = &self.board.outline;
        if outline.is_empty() {
            return true;
        }
        if !crate::geometry::point_in_polygon(start, outline)
            || !crate::geometry::point_in_polygon(end, outline)
        {
            return false;
        }
        let required = width / 2.0 + self.board.edge_clearance + SAFETY;
        let bounds = crate::geometry::Aabb {
            minimum: [start[0].min(end[0]), start[1].min(end[1])],
            maximum: [start[0].max(end[0]), start[1].max(end[1])],
        }
        .inflated(required);
        crate::geometry::polygon_edges(outline).all(|(a, b)| {
            let edge = crate::geometry::Aabb {
                minimum: [a[0].min(b[0]), a[1].min(b[1])],
                maximum: [a[0].max(b[0]), a[1].max(b[1])],
            };
            !bounds.intersects(edge)
                || crate::geometry::segment_segment_distance(start, end, a, b) >= required
        })
    }

    /// The copper of `net` and the indices of its cosmetic pad-centre stubs.
    fn materialize(&self, net: NetId) -> (NetRoute, Vec<usize>) {
        let description = &self.board.nets[net as usize];
        let rules = self.board.classes[description.class];
        let state = &self.nets[net as usize];
        let mut route = NetRoute::default();
        let mut stubs = Vec::new();
        let junctions: HashSet<Node> = state
            .branches
            .iter()
            .flat_map(|branch| {
                let start = (branch.start_terminal == NO_TERMINAL).then(|| branch.nodes[0]);
                let end =
                    (branch.end_terminal == NO_TERMINAL).then(|| *branch.nodes.last().unwrap());
                start.into_iter().chain(end)
            })
            .collect();
        for branch in &state.branches {
            let first = branch.nodes[0];
            let last = *branch.nodes.last().unwrap();
            let mut stub = |node: Node, terminal: u16| {
                if terminal == NO_TERMINAL || terminal == PLANE_TERMINAL || terminal == FREE_END {
                    return;
                }
                let anchor = description.terminals[terminal as usize].anchor;
                let center = self.grid.center_of(node.cell as usize);
                if let Some((_, width, polyline)) = state.escapes.get(&node) {
                    for pair in polyline.windows(2) {
                        route.segments.push(Segment {
                            layer: node.layer as usize,
                            start: pair[0],
                            end: pair[1],
                            width: *width,
                        });
                    }
                } else if crate::geometry::distance(anchor, center) > 1.0e-7 {
                    // The pad stub at the node's width, or the neck if the
                    // full width does not fit.
                    let mut widths = vec![self.width_at(net, node.cell)];
                    if self.board.neck_width < widths[0] {
                        widths.push(self.board.neck_width);
                    }
                    for width in widths {
                        if self.stub_is_clear(net, node.layer as usize, anchor, center, width)
                            && self.stub_keeps_edge(anchor, center, width)
                        {
                            stubs.push(route.segments.len());
                            route.segments.push(Segment { layer: node.layer as usize, start: anchor, end: center, width });
                            break;
                        }
                    }
                }
            };
            stub(first, branch.start_terminal);
            stub(last, branch.end_terminal);

            let mut run_start = 0;
            for index in 1..=branch.nodes.len() {
                let layer_change = index < branch.nodes.len()
                    && branch.nodes[index].layer != branch.nodes[index - 1].layer;
                if index < branch.nodes.len() && !layer_change {
                    continue;
                }
                self.emit_run(
                    net,
                    &branch.nodes[run_start..index],
                    &junctions,
                    &rules,
                    &mut route,
                );
                if layer_change {
                    // Through vias at one spot are one hole.
                    let at = self.grid.center_of(branch.nodes[index].cell as usize);
                    if !route.vias.iter().any(|via| via.at == at) {
                        route.vias.push(Via {
                            at,
                            diameter: rules.via_diameter,
                            drill: rules.via_drill,
                        });
                    }
                }
                run_start = index;
            }
        }
        (route, stubs)
    }

    fn emit_run(
        &self,
        net: NetId,
        nodes: &[Node],
        junctions: &HashSet<Node>,
        rules: &crate::board::RuleClass,
        route: &mut NetRoute,
    ) {
        if nodes.len() < 2 {
            return;
        }
        let nx = self.grid.nx as i64;
        let delta = |a: Node, b: Node| {
            (
                b.cell as i64 % nx - a.cell as i64 % nx,
                b.cell as i64 / nx - a.cell as i64 / nx,
            )
        };
        let widths: Vec<f64> = if self.nets[net as usize].neck_zones.is_empty() {
            vec![rules.trace_width; nodes.len()]
        } else {
            nodes.iter().map(|node| self.width_at(net, node.cell)).collect()
        };
        // A segment is as wide as its narrowest node: a run is cut at a
        // wide node next to a narrow one, so the wide segment ends at a
        // node checked at the full width and the narrow one reaches it.
        let wide = rules.trace_width;
        let mut start = 0;
        for index in 1..nodes.len() {
            let turning = index + 1 < nodes.len()
                && delta(nodes[index - 1], nodes[index]) != delta(nodes[index], nodes[index + 1]);
            let last = index + 1 == nodes.len();
            let width_edge = widths[index] >= wide
                && ((index + 1 < nodes.len() && widths[index + 1] < wide) || widths[index - 1] < wide);
            if turning || last || width_edge || junctions.contains(&nodes[index]) {
                let width = widths[start..=index].iter().copied().fold(f64::INFINITY, f64::min);
                route.segments.push(Segment {
                    layer: nodes[start].layer as usize,
                    start: self.grid.center_of(nodes[start].cell as usize),
                    end: self.grid.center_of(nodes[index].cell as usize),
                    width,
                });
                start = index;
            }
        }
    }
}

pub fn route(board: &Board, config: &Config) -> RoutingResult {
    Router::new(board, config).run()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_heap_pops_in_order() {
        let mut heap = KeyHeap::default();
        let mut reference = std::collections::BinaryHeap::new();
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        for round in 0..20_000 {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            if round % 3 == 2 {
                assert_eq!(heap.pop(), reference.pop().map(|std::cmp::Reverse(key)| key));
            } else {
                let key = state >> 40;
                heap.push(key);
                reference.push(std::cmp::Reverse(key));
            }
        }
        while let Some(std::cmp::Reverse(key)) = reference.pop() {
            assert_eq!(heap.pop(), Some(key));
        }
        assert_eq!(heap.pop(), None);
    }
    use crate::board::{Net, Obstacle, ObstacleKind, Plane, RuleClass, Terminal};
    use crate::geometry::Shape;

    fn copper(shape: Shape, net: NetId, label: &str) -> Obstacle {
        Obstacle {
            shape,
            layers: 1,
            kind: ObstacleKind::Copper,
            net: Some(net),
            clearance: 0.0,
            clearance_override: None,
            blocks_tracks: true,
            blocks_vias: true,
            label: label.into(),
        }
    }

    /// A pour on one layer, cut by two parallel walls of another net
    /// running diagonally to the lattice: between them only the nodes on
    /// one diagonal keep the brush's clearance (0.45 mm from both walls
    /// where 0.425 is needed; the next diagonals are 0.071 mm closer).
    /// `notch` adds a small disc of the other net beside one diagonal step
    /// that passes it closer than the clearance while both of the step's
    /// nodes keep it.
    fn diagonal_channel(notch: bool) -> (Router, [f64; 2]) {
        let class = RuleClass { trace_width: 0.25, clearance: 0.2, via_diameter: 0.6, via_drill: 0.3 };
        let mut obstacles = vec![
            copper(Shape::Circle { center: [0.5, 7.5], radius: 0.3 }, 0, "P.1"),
            copper(Shape::Circle { center: [7.5, 0.5], radius: 0.3 }, 1, "Q.1"),
        ];
        // Walls with radius 0.1 along y = x +- 0.6364 (0.45 mm from the
        // diagonal y = x on both sides, perpendicular).
        let offset = 0.45 * std::f64::consts::SQRT_2;
        for sign in [-1.0, 1.0] {
            obstacles.push(copper(
                Shape::Capsule { start: [-1.0, -1.0 + sign * offset], end: [9.0, 9.0 + sign * offset], radius: 0.1 },
                1,
                "wall",
            ));
        }
        // The step from (3.0, 3.0) to (3.1, 3.1) has its middle at (3.05,
        // 3.05); a disc of radius 0.075 at 0.397 mm from it across the
        // diagonal comes within 0.397 - 0.075 = 0.322 < 0.325 of the step
        // and 0.328 of either node.
        let middle = [3.05, 3.05];
        let across = [-std::f64::consts::FRAC_1_SQRT_2, std::f64::consts::FRAC_1_SQRT_2];
        if notch {
            obstacles.push(copper(
                Shape::Circle { center: [middle[0] + across[0] * 0.397, middle[1] + across[1] * 0.397], radius: 0.075 },
                1,
                "notch",
            ));
        }
        let terminal = |pad: usize, at: [f64; 2], label: &str| Terminal { anchor: at, layers: 1, pad, contact: None, label: label.into() };
        let board = Board {
            layer_count: 1,
            outline: vec![[0.0, 0.0], [8.0, 0.0], [8.0, 8.0], [0.0, 8.0]],
            edge_clearance: 0.1,
            hole_clearance: 0.25,
            hole_to_hole: 0.25,
            classes: vec![class],
            neck_width: 0.25,
            obstacles,
            nets: vec![
                Net { name: "P".into(), class: 0, terminals: vec![terminal(0, [0.5, 7.5], "P.1")] },
                Net { name: "Q".into(), class: 0, terminals: vec![terminal(1, [7.5, 0.5], "Q.1")] },
            ],
            planes: vec![Plane {
                net: 0,
                class: 0,
                layer: 0,
                polygon: vec![[0.0, 0.0], [8.0, 0.0], [8.0, 8.0], [0.0, 8.0]],
                excluded: Vec::new(),
                connect: true,
                thermal_reach: 0.0,
                thermal_gap: 0.0,
                exclusive: false,
                solid: false,
                hatch: None,
            }],
            solid_pads: Vec::new(),
            isolated_pads: Vec::new(),
            pad_spokes: Default::default(),
            merged_pads: Vec::new(),
        };
        let config = Config { pitches: vec![0.1], ..Config::default() };
        (Router::new(&board, &config), middle)
    }

    #[test]
    fn pour_analyses_count_as_budget_work() {
        // Each analysis counts its pour's layers times the lattice, at
        // `POUR_NODES_PER_SECOND`, in the phase budgets' work and not in the
        // search's; a copy of the router keeps the count.
        let (router, _) = diagonal_channel(false);
        let clock = router.clock();
        let search = router.work_seconds();
        router.analyze_pours(0);
        router.analyze_pours_full(0, &|_| false);
        let nodes = router.grid().cells() as f64;
        assert_eq!(router.pour_analysis_units(0), router.grid().cells() as u64);
        let expected = 2.0 * nodes / POUR_NODES_PER_SECOND;
        assert!((router.budget_work() - clock.0 - expected).abs() < 1.0e-12);
        assert_eq!(router.work_seconds(), search);
        let copy = router.clone();
        assert!((copy.budget_work() - router.budget_work()).abs() < 1.0e-12);
        // A budget smaller than that work is spent by it alone.
        assert!(router.spent(&clock, 0.5 * expected));
        assert!(!router.spent(&clock, 2.0 * expected));
        // Another net without a pour counts nothing.
        assert_eq!(router.pour_analysis_units(1), 0);
    }

    #[test]
    fn a_diagonal_neck_of_the_fill_holds_together() {
        let (router, _) = diagonal_channel(false);
        let grid = router.grid().clone();
        let (pours, _, _) = router.analyze_pours(0);
        let piece = |x: f64, y: f64| pours.label[0][grid.nearest_node([x, y]).unwrap()];
        // The lattice lies on the diagonal: its nodes are free, their side
        // neighbours are not.
        let on = grid.nearest_node([2.0, 2.0]).unwrap();
        assert!((grid.center_of(on)[0] - grid.center_of(on)[1]).abs() < 1.0e-9);
        assert!(pours.solid[0][on]);
        assert!(!pours.solid[0][grid.nearest_node([2.1, 2.0]).unwrap()]);
        assert!(!pours.solid[0][grid.nearest_node([2.0, 2.1]).unwrap()]);
        // The channel is one piece from end to end, and the fill on either
        // side of the walls is another each.
        assert_ne!(piece(2.0, 2.0), 0);
        assert_eq!(piece(2.0, 2.0), piece(6.0, 6.0));
        assert_ne!(piece(1.0, 6.0), piece(6.0, 1.0));
        assert_ne!(piece(1.0, 6.0), piece(2.0, 2.0));
        assert_eq!(pours.pieces, 3);
    }

    #[test]
    fn a_diagonal_step_passing_other_copper_too_closely_does_not_join() {
        let (router, middle) = diagonal_channel(true);
        let grid = router.grid().clone();
        let (pours, _, _) = router.analyze_pours(0);
        let piece = |x: f64, y: f64| pours.label[0][grid.nearest_node([x, y]).unwrap()];
        // Both nodes of the step stay free; the step between them does not
        // keep the clearance, so the channel falls into two pieces there.
        let (before, after) = ([middle[0] - 0.05, middle[1] - 0.05], [middle[0] + 0.05, middle[1] + 0.05]);
        assert!(pours.solid[0][grid.nearest_node(before).unwrap()]);
        assert!(pours.solid[0][grid.nearest_node(after).unwrap()]);
        assert_ne!(piece(before[0], before[1]), piece(after[0], after[1]));
        assert_eq!(piece(2.0, 2.0), piece(before[0], before[1]));
        assert_eq!(piece(6.0, 6.0), piece(after[0], after[1]));
        assert_eq!(pours.pieces, 4);
    }

    fn rect(center: [f64; 2], half: [f64; 2]) -> Shape {
        Shape::Polygon {
            points: vec![
                [center[0] - half[0], center[1] - half[1]],
                [center[0] + half[0], center[1] - half[1]],
                [center[0] + half[0], center[1] + half[1]],
                [center[0] - half[0], center[1] + half[1]],
            ],
        }
    }

    fn plane(net: NetId, layer: usize, thermal: Option<(f64, f64)>) -> Plane {
        Plane {
            net,
            class: 0,
            layer,
            polygon: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
            excluded: Vec::new(),
            connect: true,
            thermal_reach: thermal.map_or(0.0, |(gap, bridge)| gap + bridge),
            thermal_gap: thermal.map_or(0.0, |(gap, _)| gap),
            exclusive: false,
            solid: false,
            hatch: None,
        }
    }

    fn board(layer_count: usize, obstacles: Vec<Obstacle>, nets: Vec<Net>, planes: Vec<Plane>, solid_pads: Vec<String>) -> Board {
        Board {
            layer_count,
            outline: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
            edge_clearance: 0.1,
            hole_clearance: 0.25,
            hole_to_hole: 0.25,
            classes: vec![RuleClass { trace_width: 0.25, clearance: 0.2, via_diameter: 0.6, via_drill: 0.3 }],
            neck_width: 0.25,
            obstacles,
            nets,
            planes,
            solid_pads,
            isolated_pads: Vec::new(),
            pad_spokes: Default::default(),
            merged_pads: Vec::new(),
        }
    }

    #[test]
    fn a_plane_stub_cut_off_from_the_main_piece_is_routed_again() {
        // Two front pads of P reach P's plane on the back through a stub
        // and a via each. While pads must reach the main piece, the stub
        // whose via no longer lands in it (the signals cut it off) does not
        // join its pad to the plane: the pad is routed again, to the main
        // piece; the other pad keeps its stub.
        let mut obstacles = vec![
            copper(Shape::Circle { center: [2.0, 5.0], radius: 0.3 }, 0, "P.1"),
            copper(Shape::Circle { center: [8.0, 5.0], radius: 0.3 }, 0, "P.2"),
        ];
        for obstacle in &mut obstacles {
            obstacle.layers = 0b01;
        }
        let terminal = |pad: usize, at: [f64; 2], label: &str| Terminal { anchor: at, layers: 0b01, pad, contact: None, label: label.into() };
        let board = board(
            2,
            obstacles,
            vec![Net { name: "P".into(), class: 0, terminals: vec![terminal(0, [2.0, 5.0], "P.1"), terminal(1, [8.0, 5.0], "P.2")] }],
            vec![plane(0, 1, None)],
            Vec::new(),
        );
        let mut router = Router::new(&board, &Config { pitches: vec![0.1], ..Config::default() });
        let grid = router.grid().clone();
        let node = |layer: u8, at: [f64; 2]| Node { layer, cell: grid.nearest_node(at).unwrap() as u32 };
        let stub = |pad: u16, from: [f64; 2], to: [f64; 2]| {
            let steps = ((to[0] - from[0]) / 0.1).round() as usize;
            let mut nodes: Vec<Node> = (0..=steps).map(|step| node(0, [from[0] + 0.1 * step as f64, from[1]])).collect();
            nodes.push(node(1, to));
            Branch { nodes, start_terminal: pad, end_terminal: PLANE_TERMINAL }
        };
        let cut_off = [3.0, 5.0];
        {
            let state = &mut router.nets[0];
            assert!(!state.plane[1].is_empty());
            assert_eq!(state.on_plane, vec![false, false]);
            state.branches = vec![stub(0, [2.0, 5.0], cut_off), stub(1, [8.0, 5.0], [9.0, 5.0])];
            // The main piece: the plane but around the first stub's via.
            let target: Vec<bool> = (0..grid.cells())
                .map(|cell| crate::geometry::distance(grid.center_of(cell), cut_off) > 0.5)
                .collect();
            state.plane_target = vec![Vec::new(), target];
        }
        router.stamp(0);
        router.route_net_seq(0, 0.0, true, 1.0);
        let state = &router.nets[0];
        assert!(state.complete);
        assert_eq!(state.branches.len(), 3, "{:?}", state.branches.iter().map(|branch| (branch.start_terminal, branch.end_terminal)).collect::<Vec<_>>());
        // The new branch leaves pad 1's copper and ends on the main piece.
        let added = &state.branches[2];
        let end = *added.nodes.last().unwrap();
        assert_eq!(added.end_terminal, PLANE_TERMINAL);
        assert!(state.plane_target[end.layer as usize][end.cell as usize]);
        assert!(added.nodes.iter().any(|node| state.branches[0].nodes.contains(node)) || added.start_terminal == 0);
    }

    #[test]
    fn a_spoke_passing_another_nets_copper_too_closely_does_not_count() {
        // P.1 sits in a row between two pads of Q that reach 0.5 mm past
        // it, 0.7 mm apart. Its spokes land in fill that P.2 (a solid pad)
        // holds, but on the way there a brush between the Q pads would come
        // closer than the zone's clearance (0.3 mm, more than the tracks'
        // 0.2) to them: it keeps 0.425 mm from each. KiCad keeps none.
        let obstacles = vec![
            copper(rect([5.0, 5.0], [0.75, 0.15]), 0, "P.1"),
            copper(Shape::Circle { center: [8.5, 8.5], radius: 0.4 }, 0, "P.2"),
            copper(rect([5.0, 4.5], [1.25, 0.15]), 1, "Q.1"),
            copper(rect([5.0, 5.5], [1.25, 0.15]), 1, "Q.2"),
        ];
        let terminal = |pad: usize, at: [f64; 2], label: &str| Terminal { anchor: at, layers: 1, pad, contact: None, label: label.into() };
        let mut pour = plane(0, 0, Some((0.5, 0.5)));
        pour.class = 1;
        let mut board = board(
            1,
            obstacles,
            vec![
                Net { name: "P".into(), class: 0, terminals: vec![terminal(0, [5.0, 5.0], "P.1"), terminal(1, [8.5, 8.5], "P.2")] },
                Net { name: "Q".into(), class: 0, terminals: vec![terminal(2, [5.0, 4.5], "Q.1"), terminal(3, [5.0, 5.5], "Q.2")] },
            ],
            vec![pour],
            vec!["P.2".into()],
        );
        board.classes.push(RuleClass { trace_width: 0.25, clearance: 0.3, via_diameter: 0.6, via_drill: 0.3 });
        let joined = |spoke_clearance: bool| {
            let router = Router::new(&board, &Config { pitches: vec![0.1], spoke_clearance, ..Config::default() });
            let (pours, mut parent, _, _) = router.analyze_pours_full(0, &|_| false);
            let base = pours.pieces + 1;
            find(&mut parent, base) == find(&mut parent, base + 1)
        };
        // Without the check the probes beyond the row count as spokes.
        assert!(joined(false));
        assert!(!joined(true));
    }


    #[test]
    fn a_pad_whose_stub_is_its_only_copper_in_the_ring_rejoins_the_plane() {
        // P.1 (the last terminal), a through-hole pad cut off before, has a stub inside its own
        // thermal ring down to the second layer: with the stub the pour has
        // two more pieces (the stub's nodes on either layer) than without
        // it. The pad's spokes reach fill that P.3 (solid) holds, so it
        // rejoins the plane and loses the stub; the terminals of the
        // analysis without the stub are looked up by its own pieces
        // (ASH-ART Qfwfq panicked: index 143 of 143).
        let mut obstacles = vec![
            copper(Shape::Circle { center: [3.0, 5.0], radius: 0.3 }, 0, "P.1"),
            copper(Shape::Circle { center: [7.0, 5.0], radius: 0.3 }, 0, "P.2"),
            copper(Shape::Circle { center: [5.0, 8.0], radius: 0.3 }, 0, "P.3"),
        ];
        for obstacle in &mut obstacles {
            obstacle.layers = 0b11;
        }
        let terminal = |pad: usize, at: [f64; 2], label: &str| Terminal { anchor: at, layers: 0b11, pad, contact: None, label: label.into() };
        let board = board(
            2,
            obstacles,
            vec![Net {
                name: "P".into(),
                class: 0,
                terminals: vec![terminal(2, [5.0, 8.0], "P.3"), terminal(1, [7.0, 5.0], "P.2"), terminal(0, [3.0, 5.0], "P.1")],
            }],
            vec![plane(0, 0, Some((0.5, 0.5))), plane(0, 1, Some((0.5, 0.5)))],
            vec!["P.3".into()],
        );
        let mut router = Router::new(&board, &Config { pitches: vec![0.1], ..Config::default() });
        let grid = router.grid().clone();
        let node = |layer: u8, x: f64| Node { layer, cell: grid.nearest_node([x, 5.0]).unwrap() as u32 };
        {
            let state = &mut router.nets[0];
            assert!(state.on_plane[2]);
            state.on_plane[2] = false;
            state.branches = vec![Branch {
                nodes: vec![node(0, 3.0), node(0, 3.1), node(0, 3.2), node(1, 3.2)],
                start_terminal: 2,
                end_terminal: PLANE_TERMINAL,
            }];
        }
        router.stamp(0);
        let with_stub = router.analyze_pours(0).0.pieces;
        let without_stub = router.analyze_pours_skipping(0, &|_| true).0.pieces;
        assert_eq!(with_stub, without_stub + 2);
        router.refresh_pour(0);
        let state = &router.nets[0];
        assert!(state.on_plane[2]);
        assert!(state.branches.is_empty());
    }

    /// The spokes of `net`'s pads as the model sees them, with the
    /// union-find and the terminal base of the analysis.
    fn spoke_report(router: &Router, net: NetId) -> (Vec<SpokeRecord>, Vec<usize>, usize) {
        let mut report = Vec::new();
        let (pours, parent, _, _) = router.analyze_pours_reporting(net, &|_| false, Some(&mut report));
        (report, parent, pours.pieces + 1)
    }

    fn terminal(pad: usize, at: [f64; 2], label: &str) -> Terminal {
        Terminal { anchor: at, layers: 1, pad, contact: None, label: label.into() }
    }

    #[test]
    fn a_spoke_ends_at_the_gap_and_half_the_minimum_width() {
        // Right of P.1 a wall of Q leaves a channel of brush centres 0.15 mm
        // wide, from 0.425 to 0.575 mm off the pad: KiCad's spoke ends at
        // the gap and half the minimum width (0.43 mm) and lands there; the
        // old probe, at the gap and the spoke's width (0.7 mm), found no
        // fill. P.2 is solid: the fill has an item.
        let obstacles = vec![
            copper(rect([5.0, 5.0], [0.5, 0.5]), 0, "P.1"),
            copper(Shape::Circle { center: [2.0, 8.0], radius: 0.4 }, 0, "P.2"),
            copper(rect([8.15, 5.0], [1.75, 4.0]), 1, "Q.1"),
        ];
        let board = board(
            1,
            obstacles,
            vec![
                Net { name: "P".into(), class: 0, terminals: vec![terminal(0, [5.0, 5.0], "P.1"), terminal(1, [2.0, 8.0], "P.2")] },
                Net { name: "Q".into(), class: 0, terminals: vec![terminal(2, [8.15, 5.0], "Q.1")] },
            ],
            vec![plane(0, 0, Some((0.3, 0.4)))],
            vec!["P.2".into()],
        );
        let outcome = |kicad_spokes: bool| {
            let router = Router::new(&board, &Config { pitches: vec![0.1], kicad_spokes, ..Config::default() });
            let (report, _, _) = spoke_report(&router, 0);
            report.iter().find(|record| record.terminal == 0).unwrap().outcomes[0]
        };
        assert!(matches!(outcome(true), SpokeOutcome::Good(piece) if piece != 0));
        assert_eq!(outcome(false), SpokeOutcome::NoFill);
    }

    #[test]
    fn a_piece_that_two_pads_spoke_into_is_no_island() {
        // P.1 and P.2 (thermal) in a pour without any other copper of P:
        // one fill piece touches both, so KiCad's connectivity joins them
        // through it and its DRC counts their spokes. P.3, alone in a
        // piece walled off by Q, has spokes into a piece that only it
        // touches: an isolated island to the DRC.
        let obstacles = vec![
            copper(rect([2.0, 2.0], [0.4, 0.4]), 0, "P.1"),
            copper(rect([4.0, 2.0], [0.4, 0.4]), 0, "P.2"),
            copper(rect([8.0, 8.0], [0.4, 0.4]), 0, "P.3"),
            copper(rect([6.5, 8.0], [0.1, 3.0]), 1, "Q.1"),
            copper(rect([8.5, 5.5], [2.0, 0.1]), 1, "Q.2"),
        ];
        let board = board(
            1,
            obstacles,
            vec![
                Net { name: "P".into(), class: 0, terminals: vec![terminal(0, [2.0, 2.0], "P.1"), terminal(1, [4.0, 2.0], "P.2"), terminal(2, [8.0, 8.0], "P.3")] },
                Net { name: "Q".into(), class: 0, terminals: vec![terminal(3, [6.5, 8.0], "Q.1"), terminal(4, [8.5, 5.5], "Q.2")] },
            ],
            vec![plane(0, 0, Some((0.3, 0.4)))],
            Vec::new(),
        );
        let router = Router::new(&board, &Config { pitches: vec![0.1], ..Config::default() });
        let (report, mut parent, base) = spoke_report(&router, 0);
        let outcomes = |terminal: usize| report.iter().find(|record| record.terminal == terminal).unwrap().outcomes;
        assert!(outcomes(0).iter().all(|outcome| matches!(outcome, SpokeOutcome::Good(_))));
        assert!(outcomes(2).iter().all(|outcome| matches!(outcome, SpokeOutcome::Island(_))));
        assert_eq!(find(&mut parent, base), find(&mut parent, base + 1));
        assert_ne!(find(&mut parent, base), find(&mut parent, base + 2));
        // The old model: a piece without a track, a via or a solid pad is
        // an island to every pad.
        let old = Router::new(&board, &Config { pitches: vec![0.1], kicad_spokes: false, ..Config::default() });
        let (report, _, _) = spoke_report(&old, 0);
        assert!(report.iter().find(|record| record.terminal == 0).unwrap().outcomes.iter().all(|outcome| matches!(outcome, SpokeOutcome::Island(_))));
    }

    /// P.1 at the middle of a pour, walled in by Q on three sides (+x, +y
    /// and -y, right next to its spokes) so that only its -x spoke lands;
    /// P.2, solid, gives the fill an item.
    fn one_spoke_board() -> Board {
        let obstacles = vec![
            copper(rect([5.0, 5.0], [0.3, 0.3]), 0, "P.1"),
            copper(Shape::Circle { center: [2.0, 8.0], radius: 0.4 }, 0, "P.2"),
            copper(rect([6.0, 5.0], [0.25, 0.25]), 1, "Q.1"),
            copper(rect([5.0, 6.0], [0.25, 0.25]), 1, "Q.2"),
            copper(rect([5.0, 4.0], [0.25, 0.25]), 1, "Q.3"),
        ];
        board(
            1,
            obstacles,
            vec![
                Net { name: "P".into(), class: 0, terminals: vec![terminal(0, [5.0, 5.0], "P.1"), terminal(1, [2.0, 8.0], "P.2")] },
                Net { name: "Q".into(), class: 0, terminals: vec![terminal(2, [6.0, 5.0], "Q.1"), terminal(3, [5.0, 6.0], "Q.2"), terminal(4, [5.0, 4.0], "Q.3")] },
            ],
            vec![plane(0, 0, Some((0.3, 0.4)))],
            vec!["P.2".into()],
        )
    }

    #[test]
    fn a_track_from_the_pad_into_the_fill_counts_as_a_spoke() {
        // KiCad's DRC counts a track that starts within half a gap of the
        // pad and ends in fill as a spoke; a track that bends in the gap
        // does not count (its first segment ends outside the fill).
        let board = one_spoke_board();
        let mut router = Router::new(&board, &Config { pitches: vec![0.1], ..Config::default() });
        let grid = router.grid().clone();
        let node = |at: [f64; 2]| Node { layer: 0, cell: grid.nearest_node(at).unwrap() as u32 };
        let record = |router: &Router| spoke_report(router, 0).0.into_iter().find(|record| record.terminal == 0).unwrap();
        let before = record(&router);
        assert_eq!(before.outcomes.iter().filter(|outcome| matches!(outcome, SpokeOutcome::Good(_))).count(), 1);
        assert_eq!(Router::spokes_short(&before), 1);
        // Diagonally out of the pad, past the corners of Q, into the fill.
        let straight: Vec<Node> = (0..=12).map(|step| node([5.0 + 0.1 * step as f64, 5.0 + 0.1 * step as f64])).collect();
        router.nets[0].branches = vec![Branch { nodes: straight, start_terminal: 0, end_terminal: PLANE_TERMINAL }];
        let after = record(&router);
        assert_eq!(after.tracks, 1);
        assert_eq!(Router::spokes_short(&after), 0);
        // Bent inside the gap: the first run ends at (5.3, 5.3), in the
        // pad's relief.
        let mut bent: Vec<Node> = (0..=3).map(|step| node([5.0 + 0.1 * step as f64, 5.0 + 0.1 * step as f64])).collect();
        bent.extend((1..=9).map(|step| node([5.3 + 0.1 * step as f64, 5.3 + 0.1 * (step / 2) as f64])));
        router.nets[0].branches = vec![Branch { nodes: bent, start_terminal: 0, end_terminal: PLANE_TERMINAL }];
        assert_eq!(record(&router).tracks, 0);
    }

    #[test]
    fn manual_spokes_add_a_track_from_a_starved_pad_into_the_main_piece() {
        let board = one_spoke_board();
        let mut router = Router::new(&board, &Config { pitches: vec![0.1], ..Config::default() });
        assert_eq!(router.add_manual_spokes(), 1);
        let state = &router.nets[0];
        assert_eq!(state.branches.len(), 1);
        let branch = &state.branches[0];
        assert_eq!((branch.start_terminal, branch.end_terminal), (0, PLANE_TERMINAL));
        assert!(router.branch_is_legal(0, branch));
        let record = spoke_report(&router, 0).0.into_iter().find(|record| record.terminal == 0).unwrap();
        assert_eq!(Router::spokes_short(&record), 0);
        // Nothing more to add.
        assert_eq!(router.add_manual_spokes(), 0);
    }

    #[test]
    fn manual_spokes_reach_pours_of_nets_routed_as_tracks() {
        // The tracks rung: P's pour does not connect pads (the net is routed
        // as tracks), but KiCad fills it and counts P.1's spokes all the
        // same; the router lends the net its pour masks for the analysis.
        let mut board = one_spoke_board();
        board.planes[0].connect = false;
        let mut router = Router::new(&board, &Config { pitches: vec![0.1], ..Config::default() });
        assert!(router.nets[0].plane.is_empty());
        assert_eq!(router.add_manual_spokes(), 1);
        assert!(router.nets[0].plane.is_empty());
        let branch = &router.nets[0].branches[0];
        assert_eq!((branch.start_terminal, branch.end_terminal), (0, PLANE_TERMINAL));
        assert!(router.branch_is_legal(0, branch));
    }

    /// P.1, a through-hole pad (0.8 mm, drill 0.4 mm), in a box of Q walls
    /// on layer 0: its spokes there land in a piece that nothing else of P
    /// touches (KiCad's isolated island). P.2, solid, outside the box. With
    /// `both_layers`, P's pour covers layer 1 too, where nothing cuts it.
    fn island_board(both_layers: bool) -> Board {
        let mut obstacles = vec![
            copper(Shape::Circle { center: [5.0, 5.0], radius: 0.4 }, 0, "P.1"),
            copper(Shape::Circle { center: [8.5, 8.5], radius: 0.4 }, 0, "P.2"),
            copper(rect([3.5, 5.0], [0.1, 1.6]), 1, "Q.1"),
            copper(rect([6.5, 5.0], [0.1, 1.6]), 1, "Q.2"),
            copper(rect([5.0, 3.5], [1.6, 0.1]), 1, "Q.3"),
            copper(rect([5.0, 6.5], [1.6, 0.1]), 1, "Q.4"),
        ];
        obstacles[0].layers = 0b11;
        obstacles[1].layers = 0b11;
        obstacles.push(Obstacle {
            shape: Shape::Circle { center: [5.0, 5.0], radius: 0.2 },
            layers: 0b11,
            kind: ObstacleKind::Hole,
            net: Some(0),
            clearance: 0.0,
            clearance_override: None,
            blocks_tracks: true,
            blocks_vias: true,
            label: "P.1 plated hole".into(),
        });
        let through = |pad: usize, at: [f64; 2], label: &str| Terminal { anchor: at, layers: 0b11, pad, contact: None, label: label.into() };
        let mut planes = vec![plane(0, 0, Some((0.3, 0.4)))];
        if both_layers {
            planes.push(plane(0, 1, Some((0.3, 0.4))));
        }
        board(
            2,
            obstacles,
            vec![
                Net { name: "P".into(), class: 0, terminals: vec![through(0, [5.0, 5.0], "P.1"), through(1, [8.5, 8.5], "P.2")] },
                Net {
                    name: "Q".into(),
                    class: 0,
                    terminals: vec![terminal(2, [3.5, 5.0], "Q.1"), terminal(3, [6.5, 5.0], "Q.2"), terminal(4, [5.0, 3.5], "Q.3"), terminal(5, [5.0, 6.5], "Q.4")],
                },
            ],
            planes,
            vec!["P.2".into()],
        )
    }

    /// P.1's track on layer 1: out of the pad along +x to (5.9, 5.0), over
    /// the island's fill on layer 0 (KiCad's DRC counts it as a spoke of
    /// layer 0: it starts in the pad and ends in that layer's fill), then
    /// up and out of the box.
    fn layer_one_track(router: &Router) -> Branch {
        let grid = router.grid();
        let mut nodes: Vec<Node> = (0..=9).map(|step| Node { layer: 1, cell: grid.nearest_node([5.0 + 0.1 * step as f64, 5.0]).unwrap() as u32 }).collect();
        nodes.extend((1..=30).map(|step| Node { layer: 1, cell: grid.nearest_node([5.9, 5.0 + 0.1 * step as f64]).unwrap() as u32 }));
        Branch { nodes, start_terminal: 0, end_terminal: PLANE_TERMINAL }
    }

    fn island_record(router: &Router) -> SpokeRecord {
        spoke_report(router, 0).0.into_iter().find(|record| record.terminal == 0 && record.layer == 0).unwrap()
    }

    #[test]
    fn a_track_leaving_a_through_hole_pad_between_its_spokes_does_not_join_their_island() {
        // P.1 leaves on layer 0 by a via at its centre: its track there is
        // no longer than the pad's hole. KiCad's fill reaches a plated pad
        // only up to its drill, so the track touches no spoke and the
        // island stays one (mackerel-30's C50.2 on In1). The old rule
        // joined a routed track to every piece its spokes reach.
        let board = island_board(false);
        let outcomes = |track_meets_spokes: bool| {
            let mut router = Router::new(&board, &Config { pitches: vec![0.1], track_meets_spokes, ..Config::default() });
            let centre = router.grid().nearest_node([5.0, 5.0]).unwrap() as u32;
            router.nets[0].branches = vec![Branch { nodes: vec![Node { layer: 0, cell: centre }, Node { layer: 1, cell: centre }], start_terminal: 0, end_terminal: PLANE_TERMINAL }];
            island_record(&router).outcomes
        };
        assert!(outcomes(true).iter().all(|outcome| matches!(outcome, SpokeOutcome::Island(_))), "{:?}", outcomes(true));
        assert!(outcomes(false).iter().all(|outcome| matches!(outcome, SpokeOutcome::Good(_))), "{:?}", outcomes(false));
    }

    #[test]
    fn a_pad_in_an_island_gets_a_track_into_it() {
        // Layer 0's island holds only P.1, and its track on layer 1 counts
        // as a spoke, so the island's spokes do not: a starved thermal. No
        // straight track reaches kept fill and no other layer has a pour to
        // stitch to: P.1 gets a track on layer 0 into its island, the
        // island holds two items and its spokes count.
        let board = island_board(false);
        let mut router = Router::new(&board, &Config { pitches: vec![0.1], ..Config::default() });
        router.nets[0].branches = vec![layer_one_track(&router)];
        router.stamp(0);
        let before = island_record(&router);
        assert_eq!(before.tracks, 1);
        assert!(Router::spokes_short(&before) > 0, "{before:?}");
        assert_eq!(router.add_manual_spokes(), 1);
        let added = router.nets[0].branches.last().unwrap().clone();
        assert_eq!((added.start_terminal, added.end_terminal), (0, PLANE_TERMINAL));
        assert!(added.nodes.iter().all(|node| node.layer == 0));
        assert!(router.branch_is_legal(0, &added));
        assert_eq!(Router::spokes_short(&island_record(&router)), 0);
        // Without island stubs nothing helps.
        let mut router = Router::new(&board, &Config { pitches: vec![0.1], island_stubs: false, ..Config::default() });
        router.nets[0].branches = vec![layer_one_track(&router)];
        router.stamp(0);
        assert_eq!(router.add_manual_spokes(), 0);
    }

    #[test]
    fn a_pad_in_an_island_gets_a_via_to_the_main_piece_on_another_layer() {
        // As above, with P's pour on layer 1 too: a via from the island to
        // layer 1's fill (the main piece, which P.2 holds) joins the island
        // to the plane, and its spokes count.
        let board = island_board(true);
        let mut router = Router::new(&board, &Config { pitches: vec![0.1], ..Config::default() });
        router.nets[0].branches = vec![layer_one_track(&router)];
        router.stamp(0);
        assert!(Router::spokes_short(&island_record(&router)) > 0);
        assert!(router.add_manual_spokes() >= 1);
        let via = router.nets[0].branches[1].clone();
        assert_eq!(via.nodes.len(), 2);
        assert_eq!(via.nodes[0].cell, via.nodes[1].cell);
        assert_ne!(via.nodes[0].layer, via.nodes[1].layer);
        let at = router.grid().center_of(via.nodes[0].cell as usize);
        assert!((3.6..6.4).contains(&at[0]) && (3.6..6.4).contains(&at[1]), "{at:?}");
        assert_eq!(Router::spokes_short(&island_record(&router)), 0);
    }

    #[test]
    fn spokes_of_two_pads_side_by_side_meet() {
        // P.1 and P.2 0.5 mm apart: no brush fits between them (it keeps
        // 0.425 mm from each), but each spoke's end lies in the other's
        // spoke, and KiCad keeps both: one outline holds the two pads.
        let obstacles = vec![
            copper(rect([4.0, 5.0], [0.4, 0.4]), 0, "P.1"),
            copper(rect([4.9 + 0.4, 5.0], [0.4, 0.4]), 0, "P.2"),
        ];
        let board = board(
            1,
            obstacles,
            vec![Net { name: "P".into(), class: 0, terminals: vec![terminal(0, [4.0, 5.0], "P.1"), terminal(1, [5.3, 5.0], "P.2")] }],
            vec![plane(0, 0, Some((0.3, 0.4)))],
            Vec::new(),
        );
        let router = Router::new(&board, &Config { pitches: vec![0.1], ..Config::default() });
        let (report, mut parent, base) = spoke_report(&router, 0);
        assert_eq!(report.iter().find(|record| record.terminal == 0).unwrap().outcomes[0], SpokeOutcome::Pad(1));
        assert_eq!(report.iter().find(|record| record.terminal == 1).unwrap().outcomes[1], SpokeOutcome::Pad(0));
        assert_eq!(find(&mut parent, base), find(&mut parent, base + 1));
    }

    #[test]
    fn no_spoke_is_narrower_than_the_minimum_width() {
        // A pad 0.2 mm wide: its spokes would be 0.2 mm wide (no wider than
        // the pad), less than the zone's 0.25 mm minimum: KiCad draws none.
        let obstacles = vec![
            copper(rect([5.0, 5.0], [0.5, 0.1]), 0, "P.1"),
            copper(Shape::Circle { center: [2.0, 8.0], radius: 0.4 }, 0, "P.2"),
        ];
        let board = board(
            1,
            obstacles,
            vec![Net { name: "P".into(), class: 0, terminals: vec![terminal(0, [5.0, 5.0], "P.1"), terminal(1, [2.0, 8.0], "P.2")] }],
            vec![plane(0, 0, Some((0.3, 0.4)))],
            vec!["P.2".into()],
        );
        let router = Router::new(&board, &Config { pitches: vec![0.1], ..Config::default() });
        let (report, _, _) = spoke_report(&router, 0);
        assert_eq!(report.iter().find(|record| record.terminal == 0).unwrap().outcomes, [SpokeOutcome::NoFill; 4]);
    }

    #[test]
    fn the_fill_probe_finds_brush_centres_between_the_nodes() {
        // Two walls of Q 0.7 mm apart leave brush centres only for x in
        // [5.025, 5.075] (0.325 mm from each): no node of the 0.1 mm
        // lattice lies there, the probe finds the fill all the same.
        let obstacles = vec![
            copper(rect([3.7, 5.0], [1.0, 4.0]), 1, "Q.1"),
            copper(rect([6.4, 5.0], [1.0, 4.0]), 1, "Q.2"),
            copper(Shape::Circle { center: [5.0, 9.5], radius: 0.2 }, 0, "P.1"),
        ];
        let board = board(
            1,
            obstacles,
            vec![
                Net { name: "P".into(), class: 0, terminals: vec![terminal(2, [5.0, 9.5], "P.1")] },
                Net { name: "Q".into(), class: 0, terminals: vec![terminal(0, [3.7, 5.0], "Q.1"), terminal(1, [6.4, 5.0], "Q.2")] },
            ],
            vec![plane(0, 0, Some((0.3, 0.4)))],
            Vec::new(),
        );
        let router = Router::new(&board, &Config { pitches: vec![0.1], ..Config::default() });
        let grid = router.grid().clone();
        let state = &router.nets[0];
        assert!(!state.plane[0][grid.nearest_node([5.0, 5.0]).unwrap()]);
        assert!(!state.plane[0][grid.nearest_node([5.1, 5.0]).unwrap()]);
        let rings = vec![Vec::new()];
        let probe = fill::FillProbe::new(&router, 0, &rings);
        assert!(probe.fits(0, [5.05, 5.0], &[]));
        assert!(!probe.fits(0, [5.0, 5.0], &[]));
        assert!(probe.fill_at(0, [5.0, 5.0]));
        assert!(!probe.fill_at(0, [4.85, 5.0]));
    }

    #[test]
    fn a_pads_own_clearance_lets_the_fill_closer() {
        // Q.1 sits 0.7 mm right of P.1's edge. With the zone's 0.2 mm
        // clearance no brush centre fits between P.1's relief (0.425 mm
        // out) and Q.1 (0.325 mm off): P.1's +x spoke finds no fill. Q.1's
        // own 0.05 mm override replaces the zone's clearance in KiCad's
        // fill (Hub's USB-C pins): the spoke lands, on the lattice too.
        let outcome = |clearance_override: Option<f64>| {
            let mut obstacles = vec![
                copper(rect([5.0, 5.0], [0.3, 0.3]), 0, "P.1"),
                copper(Shape::Circle { center: [2.0, 8.0], radius: 0.4 }, 0, "P.2"),
                copper(rect([6.15, 5.0], [0.15, 0.15]), 1, "Q.1"),
            ];
            obstacles[2].clearance_override = clearance_override;
            let board = board(
                1,
                obstacles,
                vec![
                    Net { name: "P".into(), class: 0, terminals: vec![terminal(0, [5.0, 5.0], "P.1"), terminal(1, [2.0, 8.0], "P.2")] },
                    Net { name: "Q".into(), class: 0, terminals: vec![terminal(2, [6.15, 5.0], "Q.1")] },
                ],
                vec![plane(0, 0, Some((0.3, 0.4)))],
                vec!["P.2".into()],
            );
            let router = Router::new(&board, &Config { pitches: vec![0.1], ..Config::default() });
            let (report, _, _) = spoke_report(&router, 0);
            report.iter().find(|record| record.terminal == 0).unwrap().outcomes[0]
        };
        assert!(!matches!(outcome(None), SpokeOutcome::Good(_)));
        assert!(matches!(outcome(Some(0.05)), SpokeOutcome::Good(piece) if piece != 0));
    }

    #[test]
    fn spokes_meet_whatever_their_directions() {
        // Two round pads 1.35 mm apart (a USB-C receptacle's A1 and B12):
        // their diagonal spokes cross at right angles, each end inside the
        // other's spoke, in the other pad's relief and short of room beside
        // Q.1. KiCad keeps both.
        let obstacles = vec![
            copper(Shape::Circle { center: [5.0, 5.0], radius: 0.35 }, 0, "P.1"),
            copper(Shape::Circle { center: [5.0, 3.65], radius: 0.35 }, 0, "P.2"),
            copper(Shape::Circle { center: [6.2, 4.325], radius: 0.15 }, 1, "Q.1"),
        ];
        let board = board(
            1,
            obstacles,
            vec![
                Net { name: "P".into(), class: 0, terminals: vec![terminal(0, [5.0, 5.0], "P.1"), terminal(1, [5.0, 3.65], "P.2")] },
                Net { name: "Q".into(), class: 0, terminals: vec![terminal(2, [6.2, 4.325], "Q.1")] },
            ],
            vec![plane(0, 0, Some((0.5, 0.5)))],
            Vec::new(),
        );
        let router = Router::new(&board, &Config { pitches: vec![0.1], ..Config::default() });
        let (report, mut parent, base) = spoke_report(&router, 0);
        // P.1's +x-y spoke (index 1) and P.2's +x+y (index 0).
        assert_eq!(report.iter().find(|record| record.terminal == 0).unwrap().outcomes[1], SpokeOutcome::Pad(1));
        assert_eq!(report.iter().find(|record| record.terminal == 1).unwrap().outcomes[0], SpokeOutcome::Pad(0));
        assert_eq!(find(&mut parent, base), find(&mut parent, base + 1));
    }

    #[test]
    fn a_turned_pads_spokes_turn_with_it() {
        let close = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).abs() < 1.0e-9 && (a[1] - b[1]).abs() < 1.0e-9;
        let d = std::f64::consts::FRAC_1_SQRT_2;
        // A 1.0 x 0.4 pad at 45 degrees: spokes along its own axes, its
        // local x turned as KiCad's RotatePoint turns it.
        let frame = crate::board::PadSpokes { center: [0.0, 0.0], half: [0.5, 0.2], orientation: 45.0, spoke_angle: 90.0, circular: false };
        let spokes = frame.spokes(0.6);
        let along_x = spokes.iter().find(|spoke| close(spoke.0, [d, -d])).unwrap();
        assert!((along_x.1 - 0.5).abs() < 1.0e-9 && (along_x.2 - 1.1).abs() < 1.0e-9);
        let along_y = spokes.iter().find(|spoke| close(spoke.0, [d, d])).unwrap();
        assert!((along_y.1 - 0.2).abs() < 1.0e-9 && (along_y.2 - 0.8).abs() < 1.0e-9);
        // The same pad unturned with a 45 degree spoke angle: diagonal
        // spokes to its box grown by the reach.
        let frame = crate::board::PadSpokes { orientation: 0.0, spoke_angle: 45.0, ..frame };
        let spokes = frame.spokes(0.6);
        assert!(close(spokes[0].0, [d, d]));
        assert!((spokes[0].1 - 0.2 / d).abs() < 1.0e-9 && (spokes[0].2 - 0.8 / d).abs() < 1.0e-9);
    }

    #[test]
    fn a_pad_merged_into_its_terminal_is_checked_on_its_own_layers() {
        // P.1 twice: a pad on the back (the terminal) and a through-hole
        // pad at its spot, merged into it. On the front the through-hole
        // pad is copper of its own with a thermal relief: it has spokes;
        // on the back it lies inside the larger pad.
        let mut obstacles = vec![
            copper(rect([5.0, 5.0], [0.8, 0.4]), 0, "P.1"),
            copper(Shape::Circle { center: [5.0, 5.0], radius: 0.4 }, 0, "P.1"),
            copper(Shape::Circle { center: [2.0, 8.0], radius: 0.4 }, 0, "P.2"),
        ];
        obstacles[0].layers = 0b10;
        obstacles[1].layers = 0b11;
        obstacles[2].layers = 0b11;
        let mut board = board(
            2,
            obstacles,
            vec![Net {
                name: "P".into(),
                class: 0,
                terminals: vec![
                    Terminal { anchor: [5.0, 5.0], layers: 0b10, pad: 0, contact: None, label: "P.1".into() },
                    Terminal { anchor: [2.0, 8.0], layers: 0b11, pad: 2, contact: None, label: "P.2".into() },
                ],
            }],
            vec![plane(0, 0, Some((0.3, 0.4))), plane(0, 1, Some((0.3, 0.4)))],
            vec!["P.2".into()],
        );
        board.merged_pads.push(crate::board::MergedPad { net: 0, host: 0, pad: 1, anchor: [5.0, 5.0], layers: 0b11 });
        // Inside the larger pad too with `inner_pads`.
        for (inner_pads, layers) in [(false, vec![0]), (true, vec![0, 1])] {
            let router = Router::new(&board, &Config { pitches: vec![0.1], inner_pads, ..Config::default() });
            let (report, _, _) = spoke_report(&router, 0);
            let merged: Vec<usize> = report.iter().filter(|record| record.pad == 1).map(|record| record.layer).collect();
            assert_eq!(merged, layers);
            assert!(report.iter().find(|record| record.pad == 1).unwrap().outcomes.iter().any(|outcome| matches!(outcome, SpokeOutcome::Good(_))));
        }
    }

    #[test]
    fn thermal_vias_inside_an_exposed_pad_have_their_own_rings_in_a_hatched_zone() {
        // P.1, an exposed pad 2 mm square, with two thermal vias of its
        // number merged into it: one at its centre, one in its corner by
        // the walls of Q. In a hatched zone KiCad gives every thermal pad a
        // ring a spoke wide, inside the exposed pad too, and keeps a via's
        // spokes where they reach it (0xCB's U3.45): the centre via has
        // four; the corner via's ring and spokes outside the pad fall to
        // Q's clearance, its inward spoke alone stays, and the DRC finds
        // it starved. The outline holds both pads: no island.
        let obstacles = vec![
            copper(rect([5.0, 5.0], [1.0, 1.0]), 0, "P.1"),
            copper(Shape::Circle { center: [5.0, 5.0], radius: 0.25 }, 0, "P.1"),
            copper(Shape::Circle { center: [5.75, 5.75], radius: 0.25 }, 0, "P.1"),
            copper(rect([6.4, 5.5], [0.2, 1.5]), 1, "Q.1"),
            copper(rect([5.5, 6.4], [1.5, 0.2]), 1, "Q.2"),
        ];
        let mut planes = vec![plane(0, 0, Some((0.15, 0.3)))];
        planes[0].hatch = Some(crate::board::Hatch { thickness: 0.25, gap: 1.0, orientation: 0.0, smoothing_level: 0, smoothing_value: 0.0, min_hole_area: 0.3 });
        let mut board = board(
            1,
            obstacles,
            vec![
                Net { name: "P".into(), class: 0, terminals: vec![terminal(0, [5.0, 5.0], "P.1")] },
                Net { name: "Q".into(), class: 0, terminals: vec![terminal(3, [6.4, 5.5], "Q.1"), terminal(4, [5.5, 6.4], "Q.2")] },
            ],
            planes,
            Vec::new(),
        );
        for (pad, anchor) in [(1, [5.0, 5.0]), (2, [5.75, 5.75])] {
            board.merged_pads.push(crate::board::MergedPad { net: 0, host: 0, pad, anchor, layers: 1 });
        }
        let good = |record: &SpokeRecord| record.outcomes.iter().filter(|outcome| matches!(outcome, SpokeOutcome::Good(_) | SpokeOutcome::Pad(_))).count() + record.crossings;
        let router = Router::new(&board, &Config { pitches: vec![0.1], ..Config::default() });
        let (report, _, _) = spoke_report(&router, 0);
        let centre = report.iter().find(|record| record.pad == 1).unwrap();
        assert_eq!(good(centre), 4, "{:?}", centre.outcomes);
        let corner = report.iter().find(|record| record.pad == 2).unwrap();
        assert_eq!(good(corner), 1, "{:?}", corner.outcomes);
        assert!(corner.outcomes.iter().all(|outcome| !matches!(outcome, SpokeOutcome::Island(_))));
        // Without `inner_pads` neither via is checked on the pad's layer.
        let router = Router::new(&board, &Config { pitches: vec![0.1], inner_pads: false, ..Config::default() });
        let (report, _, _) = spoke_report(&router, 0);
        assert!(report.iter().all(|record| record.pad == 0));
    }

    #[test]
    fn the_exposed_pads_spoke_through_a_thermal_vias_ring_counts_twice() {
        // In a solid zone the exposed pad's relief takes its thermal vias'
        // spokes; the DRC counts what crosses a via's ring: the pad's own
        // spoke along +x runs from its centre right through via A's ring,
        // crossing it on both sides, two spokes (mackerel's mounting holes
        // on In1). Via B, off the spokes, crosses no fill: not connected,
        // no finding either.
        let obstacles = vec![
            copper(rect([5.0, 5.0], [1.0, 1.0]), 0, "P.1"),
            copper(Shape::Circle { center: [5.6, 5.0], radius: 0.2 }, 0, "P.1"),
            copper(Shape::Circle { center: [4.4, 4.4], radius: 0.2 }, 0, "P.1"),
            copper(Shape::Circle { center: [8.0, 5.0], radius: 0.4 }, 0, "P.2"),
        ];
        // P.2, solid, is the fill's second item.
        let mut board = board(
            1,
            obstacles,
            vec![Net { name: "P".into(), class: 0, terminals: vec![terminal(0, [5.0, 5.0], "P.1"), terminal(3, [8.0, 5.0], "P.2")] }],
            vec![plane(0, 0, Some((0.3, 0.4)))],
            vec!["P.2".into()],
        );
        for (pad, anchor) in [(1, [5.6, 5.0]), (2, [4.4, 4.4])] {
            board.merged_pads.push(crate::board::MergedPad { net: 0, host: 0, pad, anchor, layers: 1 });
        }
        let router = Router::new(&board, &Config { pitches: vec![0.1], ..Config::default() });
        let (report, _, _) = spoke_report(&router, 0);
        let on_spoke = report.iter().find(|record| record.pad == 1).unwrap();
        assert!(on_spoke.outcomes.iter().all(|outcome| matches!(outcome, SpokeOutcome::NoFill)), "{:?}", on_spoke.outcomes);
        assert_eq!(on_spoke.crossings, 2);
        let off_spoke = report.iter().find(|record| record.pad == 2).unwrap();
        assert!(off_spoke.outcomes.iter().all(|outcome| matches!(outcome, SpokeOutcome::NoFill)), "{:?}", off_spoke.outcomes);
        assert_eq!(off_spoke.crossings, 0);
    }

    #[test]
    fn a_spoke_cut_off_at_the_pad_brings_no_item() {
        // P.1, 0.3 x 0.8 mm, with Q.1 and Q.2 beside the end of its -y
        // spoke, whose clearances cut the spoke where it leaves the pad: no
        // brush centre within half the minimum width of the pad keeps its
        // distance from them; past them the fill is free. KiCad's spoke still
        // crosses the ring there, but its piece does not hold P.1: with
        // only P.2 in it, the crossing is an island's. Without Q the spoke
        // joins P.1 to the piece: two items, a counted spoke.
        let outcome = |walls: bool| {
            let mut obstacles = vec![
                copper(rect([5.0, 5.0], [0.15, 0.4]), 0, "P.1"),
                copper(Shape::Circle { center: [5.0, 1.5], radius: 0.4 }, 0, "P.2"),
                // The other spokes end in R's copper.
                copper(rect([5.8, 5.0], [0.15, 0.3]), 2, "R.1"),
                copper(rect([4.2, 5.0], [0.15, 0.3]), 2, "R.2"),
                copper(rect([5.0, 6.1], [0.3, 0.15]), 2, "R.3"),
            ];
            let mut terminals_q = Vec::new();
            if walls {
                obstacles.push(copper(Shape::Circle { center: [4.75, 4.55], radius: 0.05 }, 1, "Q.1"));
                obstacles.push(copper(Shape::Circle { center: [5.25, 4.55], radius: 0.05 }, 1, "Q.2"));
                terminals_q = vec![terminal(5, [4.75, 4.55], "Q.1"), terminal(6, [5.25, 4.55], "Q.2")];
            }
            let board = board(
                1,
                obstacles,
                vec![
                    Net { name: "P".into(), class: 0, terminals: vec![terminal(0, [5.0, 5.0], "P.1"), terminal(1, [5.0, 1.5], "P.2")] },
                    Net { name: "Q".into(), class: 0, terminals: terminals_q },
                    Net { name: "R".into(), class: 0, terminals: vec![terminal(2, [5.8, 5.0], "R.1"), terminal(3, [4.2, 5.0], "R.2"), terminal(4, [5.0, 6.1], "R.3")] },
                ],
                vec![plane(0, 0, Some((0.5, 0.4)))],
                vec!["P.2".into()],
            );
            let router = Router::new(&board, &Config { pitches: vec![0.1], ..Config::default() });
            let (report, _, _) = spoke_report(&router, 0);
            report.iter().find(|record| record.terminal == 0).unwrap().outcomes[3]
        };
        assert!(matches!(outcome(true), SpokeOutcome::Island(_)), "{:?}", outcome(true));
        assert!(matches!(outcome(false), SpokeOutcome::Good(_)));
    }

    #[test]
    fn spokes_of_other_pads_across_the_ring_count() {
        // P.1 and P.2 meet with diagonal spokes beside P.3, whose ring
        // half a gap out the two spokes cross: KiCad's DRC counts both
        // crossings as P.3's spokes.
        let obstacles = vec![
            copper(Shape::Circle { center: [5.0, 5.0], radius: 0.35 }, 0, "P.1"),
            copper(Shape::Circle { center: [5.0, 6.35], radius: 0.35 }, 0, "P.2"),
            copper(rect([6.0, 5.675], [0.2, 0.2]), 0, "P.3"),
        ];
        let board = board(
            1,
            obstacles,
            vec![Net {
                name: "P".into(),
                class: 0,
                terminals: vec![terminal(0, [5.0, 5.0], "P.1"), terminal(1, [5.0, 6.35], "P.2"), terminal(2, [6.0, 5.675], "P.3")],
            }],
            vec![plane(0, 0, Some((0.5, 0.5)))],
            Vec::new(),
        );
        let crossings = |foreign_spokes: bool| {
            let router = Router::new(&board, &Config { pitches: vec![0.1], foreign_spokes, ..Config::default() });
            let (report, _, _) = spoke_report(&router, 0);
            report.iter().find(|record| record.terminal == 2).unwrap().crossings
        };
        assert_eq!(crossings(true), 2);
        assert_eq!(crossings(false), 0);
    }

    #[test]
    fn a_hatched_zones_holes_lie_on_its_grid() {
        // A hatched pour (web 0.5, holes 1.0 mm, at 0 degrees): the holes
        // sit on a grid of 1.5 mm from the origin, a hole (grown back by
        // half the minimum width) from 0.125 to 1.125 mm; a web's width
        // along the fill's edge stays filled.
        let mut planes = vec![plane(0, 0, Some((0.3, 0.4)))];
        planes[0].hatch = Some(crate::board::Hatch { thickness: 0.5, gap: 1.0, orientation: 0.0, smoothing_level: 0, smoothing_value: 0.0, min_hole_area: 0.3 });
        let board = board(
            1,
            vec![copper(Shape::Circle { center: [8.0, 8.0], radius: 0.4 }, 0, "P.1")],
            vec![Net { name: "P".into(), class: 0, terminals: vec![terminal(0, [8.0, 8.0], "P.1")] }],
            planes,
            vec!["P.1".into()],
        );
        let router = Router::new(&board, &Config { pitches: vec![0.1], ..Config::default() });
        let rings = vec![Vec::new()];
        let probe = fill::FillProbe::new(&router, 0, &rings);
        assert!(probe.in_hatch_hole(0, [3.6, 3.6]));
        assert!(!probe.in_hatch_hole(0, [4.4, 3.6]));
        assert!(!probe.final_fill_at(0, [3.6, 3.6]));
        assert!(probe.final_fill_at(0, [4.4, 3.6]));
        // By the board's edge: the web along it.
        assert!(!probe.in_hatch_hole(0, [0.4, 3.6]));
        assert!((probe.spoke_half_width(0) - 0.25).abs() < 1.0e-9);
    }

    #[test]
    fn a_track_to_a_pad_at_the_same_spot_counts_for_both() {
        // P.1 and P.2 at one spot (a USB-C receptacle's A1 and B12): the
        // DRC counts the track that leaves them, ending in fill, for both,
        // though the router's branch names one.
        let obstacles = vec![
            copper(rect([5.0, 5.0], [0.3, 0.3]), 0, "P.1"),
            copper(rect([5.0, 5.0], [0.3, 0.3]), 0, "P.2"),
            copper(Shape::Circle { center: [2.0, 8.0], radius: 0.4 }, 0, "P.3"),
            copper(rect([6.0, 5.0], [0.25, 0.25]), 1, "Q.1"),
            copper(rect([5.0, 6.0], [0.25, 0.25]), 1, "Q.2"),
            copper(rect([5.0, 4.0], [0.25, 0.25]), 1, "Q.3"),
        ];
        let board = board(
            1,
            obstacles,
            vec![
                Net { name: "P".into(), class: 0, terminals: vec![terminal(0, [5.0, 5.0], "P.1"), terminal(1, [5.0, 5.0], "P.2"), terminal(2, [2.0, 8.0], "P.3")] },
                Net { name: "Q".into(), class: 0, terminals: vec![terminal(3, [6.0, 5.0], "Q.1"), terminal(4, [5.0, 6.0], "Q.2"), terminal(5, [5.0, 4.0], "Q.3")] },
            ],
            vec![plane(0, 0, Some((0.3, 0.4)))],
            vec!["P.3".into()],
        );
        let mut router = Router::new(&board, &Config { pitches: vec![0.1], ..Config::default() });
        let grid = router.grid().clone();
        let node = |at: [f64; 2]| Node { layer: 0, cell: grid.nearest_node(at).unwrap() as u32 };
        let straight: Vec<Node> = (0..=12).map(|step| node([5.0 + 0.1 * step as f64, 5.0 + 0.1 * step as f64])).collect();
        router.nets[0].branches = vec![Branch { nodes: straight, start_terminal: 0, end_terminal: PLANE_TERMINAL }];
        let (report, _, _) = spoke_report(&router, 0);
        assert_eq!(report.iter().find(|record| record.terminal == 0).unwrap().tracks, 1);
        assert_eq!(report.iter().find(|record| record.terminal == 1).unwrap().tracks, 1);
    }

    #[test]
    fn a_spoke_ends_in_a_neck_exactly_the_minimum_width() {
        // P.1's +y spoke ends between its own relief and P.2's, 0.25 mm
        // apart in a zone of 0.25 mm minimum width (1.25 mm between the
        // pads, 0.5 mm gaps): brush centres fit on
        // one line only, which the probe's samples around the end miss and
        // its search along the spoke finds (Castor's J9.10).
        let obstacles = vec![
            copper(rect([5.0, 5.0], [0.3, 0.3]), 0, "P.1"),
            copper(rect([5.0, 6.85], [0.3, 0.3]), 0, "P.2"),
            copper(Shape::Circle { center: [2.0, 8.0], radius: 0.4 }, 0, "P.3"),
        ];
        let board = board(
            1,
            obstacles,
            vec![Net { name: "P".into(), class: 0, terminals: vec![terminal(0, [5.0, 5.0], "P.1"), terminal(1, [5.0, 6.85], "P.2"), terminal(2, [2.0, 8.0], "P.3")] }],
            vec![plane(0, 0, Some((0.5, 0.4)))],
            vec!["P.3".into()],
        );
        let router = Router::new(&board, &Config { pitches: vec![0.1], ..Config::default() });
        let rings = router.thermal_ring_pads(0);
        let probe = fill::FillProbe::new(&router, 0, &rings);
        let end = [5.0, 5.0 + 0.3 + 0.5 + fill::KICAD_MAX_ERROR + 0.125];
        assert!(probe.fill_along(0, end, [0.0, 1.0]));
    }

    #[test]
    fn a_track_ending_in_what_clearances_leave_of_a_spoke_counts() {
        // Q.1's clearance trims the left of P.1's +y spoke (0.4 mm wide)
        // to x >= 4.85; brush centres fit from x = 4.975. P.1's track ends
        // at x = 4.86, 0.01 mm inside what is left of the spoke (PixelWave's
        // C4.1): the DRC counts it as a spoke. The probe's old samples (the
        // spoke's middle line) found no brush there.
        let obstacles = vec![
            copper(rect([5.0, 5.0], [0.28, 0.31]), 0, "P.1"),
            copper(Shape::Circle { center: [2.0, 8.0], radius: 0.4 }, 0, "P.2"),
            copper(rect([4.4, 5.3], [0.25, 0.5]), 1, "Q.1"),
            copper(rect([5.85, 5.0], [0.15, 0.3]), 2, "R.1"),
            copper(rect([5.0, 4.15], [0.3, 0.15]), 2, "R.2"),
        ];
        let board = board(
            1,
            obstacles,
            vec![
                Net { name: "P".into(), class: 0, terminals: vec![terminal(0, [5.0, 5.0], "P.1"), terminal(1, [2.0, 8.0], "P.2")] },
                Net { name: "Q".into(), class: 0, terminals: vec![terminal(2, [4.4, 5.3], "Q.1")] },
                Net { name: "R".into(), class: 0, terminals: vec![terminal(3, [5.85, 5.0], "R.1"), terminal(4, [5.0, 4.15], "R.2")] },
            ],
            vec![plane(0, 0, Some((0.3, 0.4)))],
            vec!["P.2".into()],
        );
        let mut router = Router::new(&board, &Config { pitches: vec![0.1], ..Config::default() });
        let grid = router.grid().clone();
        let start = Node { layer: 0, cell: grid.nearest_node([5.0, 5.0]).unwrap() as u32 };
        router.nets[0].branches = vec![Branch { nodes: vec![start], start_terminal: 0, end_terminal: NO_TERMINAL }];
        router.nets[0].adopted.insert(0, ([5.0, 5.0], [4.86, 5.46], 0.05));
        let (report, _, _) = spoke_report(&router, 0);
        let record = report.iter().find(|record| record.terminal == 0).unwrap();
        assert_eq!(record.outcomes.iter().filter(|outcome| matches!(outcome, SpokeOutcome::Good(_))).count(), 1, "{:?}", record.outcomes);
        assert_eq!(record.tracks, 1);
    }

    #[test]
    fn a_pads_spokes_make_the_pieces_they_land_in_one_outline() {
        // Q's walls above and below P.1 split the pour into a left piece,
        // with P.2 in it, and a right piece with nothing. P.1 has no hole:
        // its spokes overlap at its centre, so the two pieces and the
        // spokes are one outline to KiCad, holding P.1 and P.2, and its
        // right spoke counts too.
        let obstacles = vec![
            copper(rect([5.0, 5.0], [0.3, 0.3]), 0, "P.1"),
            copper(Shape::Circle { center: [2.0, 5.0], radius: 0.4 }, 0, "P.2"),
            copper(rect([5.0, 2.15], [0.1, 2.15]), 1, "Q.1"),
            copper(rect([5.0, 7.85], [0.1, 2.15]), 1, "Q.2"),
        ];
        let board = board(
            1,
            obstacles,
            vec![
                Net { name: "P".into(), class: 0, terminals: vec![terminal(0, [5.0, 5.0], "P.1"), terminal(1, [2.0, 5.0], "P.2")] },
                Net { name: "Q".into(), class: 0, terminals: vec![terminal(2, [5.0, 2.15], "Q.1"), terminal(3, [5.0, 7.85], "Q.2")] },
            ],
            vec![plane(0, 0, Some((0.3, 0.4)))],
            vec!["P.2".into()],
        );
        let outcomes = |neck_joins: bool| {
            let router = Router::new(&board, &Config { pitches: vec![0.1], neck_joins, ..Config::default() });
            let (report, _, _) = spoke_report(&router, 0);
            report.iter().find(|record| record.terminal == 0).unwrap().outcomes
        };
        let joined = outcomes(true);
        assert!(matches!(joined[0], SpokeOutcome::Good(_)) && matches!(joined[1], SpokeOutcome::Good(_)), "{joined:?}");
        assert!(matches!(outcomes(false)[0], SpokeOutcome::Island(_)), "{:?}", outcomes(false));
    }

}
