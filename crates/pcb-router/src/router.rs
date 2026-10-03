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
    /// Stop negotiating when a quarter of the nets have no path at all
    /// after the first iteration (see `Router::hopeless`).
    pub abandon_hopeless: bool,
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
    /// Seconds the clean-up may take at most: no batch starts later than
    /// this, nor later than the negotiation took (at least 60 s). A
    /// 130 x 90 mm six-layer board spent 470 s in clean-up.
    pub cleanup_seconds: f64,
    /// Cost factor for another net's step on the ring around a pad that
    /// joins its pour by thermal spokes (copper there starves the spokes).
    pub thermal_guard_cost: f64,
    /// A negotiation that has not converged after this many seconds is
    /// handed to the resolution step as it is.
    pub negotiation_seconds: f64,
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
            stall_at_cap: 25,
            abandon_hopeless: true,
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
            cleanup_seconds: 180.0,
            thermal_guard_cost: 4.0,
            negotiation_seconds: 900.0,
            plane_cut_cost: 3.0,
            plane_skeleton: false,
            skeleton_bias: 6.0,
            heuristic_weight: 1.0,
            seed: None,
            verbose: false,
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

#[derive(Clone, Debug, Default)]
struct NetState {
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
    /// Terminals that touch a pour and therefore need no tracks.
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
fn junction_hosts(branches: &[Branch], keep: &[bool]) -> HashMap<Node, usize> {
    let mut hosts = HashMap::new();
    for (index, branch) in branches.iter().enumerate() {
        if !keep[index] {
            continue;
        }
        let first = (branch.start_terminal == NO_TERMINAL) as usize;
        let last = branch.nodes.len() - (branch.end_terminal == NO_TERMINAL) as usize;
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

impl Scratch {
    /// The next search generation; when the 16-bit counter wraps, the
    /// marks it guards are cleared.
    fn next_generation(&mut self) -> u16 {
        if self.generation == u16::MAX {
            self.seen.fill(0);
            self.closed.fill(0);
            self.target_mark.fill(0);
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

/// Per-thread search state: A* arrays, generation marks and counters.
#[derive(Clone, Default)]
pub struct Scratch {
    cost: Vec<f32>,
    /// Search generation marks: 16 bits (half the memory of a word per
    /// state; a thread's scratch is the biggest part of a large board's
    /// footprint), cleared when the counter wraps.
    seen: Vec<u16>,
    closed: Vec<u16>,
    parent: Vec<u8>,
    target_mark: Vec<u16>,
    target_terminal: Vec<u16>,
    /// Tree marks live across the searches of one net: their own counter.
    tree_mark: Vec<u32>,
    tree_terminal: Vec<u16>,
    own_via_near: Vec<u16>,
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
    tree_generation: u32,
    corridor: Option<Vec<bool>>,
    expansions: u64,
    searches: u64,
    /// Searches (and their expansions) that ran without a corridor.
    open_searches: u64,
    open_expansions: u64,
    /// Searches that found no path at all.
    failed_searches: u64,
}

impl Scratch {
    fn new(states: usize, cells: usize) -> Self {
        Self {
            cost: vec![0.0; states],
            seen: vec![0; states],
            closed: vec![0; states],
            parent: vec![0; states],
            target_mark: vec![0; states],
            target_terminal: vec![NO_TERMINAL; states],
            tree_mark: vec![0; states],
            tree_terminal: vec![NO_TERMINAL; states],
            own_via_near: vec![0; cells],
            own_mark: Vec::new(),
            own_claimed: Vec::new(),
            own_set: Vec::new(),
            own_active: false,
            generation: 0,
            tree_generation: 0,
            corridor: None,
            expansions: 0,
            searches: 0,
            open_searches: 0,
            open_expansions: 0,
            failed_searches: 0,
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

    fn fits(&self, states: usize, cells: usize) -> bool {
        self.cost.len() == states && self.own_via_near.len() == cells
    }
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

#[derive(Clone)]
pub struct Router {
    board: Board,
    config: Config,
    grid: Grid,
    statics: Vec<StaticMaps>,
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
    /// Per layer and node: the pour (as `owner`) covering the node, if any.
    covered: Vec<Vec<u32>>,
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
    /// Per layer: step cost multiplier while a plane skeleton is routed
    /// (empty otherwise).
    layer_bias: Vec<f32>,

    scratch: Scratch,
    search_seconds: f64,
    /// Wall time of the last full negotiation.
    negotiation_seconds: f64,
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
        let statics: Vec<_> = (0..board.classes.len())
            .map(|class| StaticMaps::build(board, &grid, class))
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
                            diagonal_block_via: diagonal_block(via_radius, grid.pitch),
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
            statics,
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
            covered: vec![Vec::new(); layers],
            tile_covered: vec![vec![0.0; tiles_x * tiles_y]; layers],
            layer_cut: vec![1.0; layers],
            neck_of,
            exclusive: (0..layers).map(|layer| config.exclusive_planes && board.planes.iter().any(|plane| plane.layer == layer && plane.exclusive)).collect(),
            layer_bias: Vec::new(),
            scratch: Scratch::new(states, cells),
            frame_hook: None,
            search_seconds: 0.0,
            negotiation_seconds: 0.0,
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
        router
    }

    pub fn grid(&self) -> &Grid {
        &self.grid
    }

    fn state(&self, node: Node) -> usize {
        node.layer as usize * self.grid.cells() + node.cell as usize
    }

    fn map_index(&self, class: usize, layer: usize) -> usize {
        class * (self.board.layer_count + 1) + layer
    }

    /// The map counting the nets whose copper a diagonal step of `class`
    /// on `layer` starting at a cell would pass too closely (`orientation`
    /// 0: the step towards +x +y, 1: towards +x -y; a step towards -x is
    /// the same step from its other end). A diagonal step passes the nodes
    /// on its bisector closer than its ends do; those nodes stamp here.
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
                                        crate::board::ObstacleKind::Keepout => 0.0,
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

    /// Nodes around pads that connect to a pour of their net on that layer.
    fn thermal_guards(&self) -> Vec<Vec<u32>> {
        let mut guard = vec![Vec::new(); self.board.layer_count];
        for plane in &self.board.planes {
            if plane.thermal_reach <= 0.0 {
                continue;
            }
            if guard[plane.layer].is_empty() {
                guard[plane.layer] = vec![0; self.grid.cells()];
            }
            for terminal in &self.board.nets[plane.net as usize].terminals {
                if terminal.layers & (1 << plane.layer) == 0
                    || !crate::geometry::point_in_polygon(terminal.anchor, &plane.polygon)
                {
                    continue;
                }
                let shape = &self.board.obstacles[terminal.pad].shape;
                let Some((x0, y0, x1, y1)) = self
                    .grid
                    .node_range(shape.aabb().inflated(plane.thermal_reach))
                else {
                    continue;
                };
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        if shape.distance_to_point(self.grid.center(x, y)) < plane.thermal_reach {
                            guard[plane.layer][self.grid.index(x, y)] =
                                crate::grid::owner(plane.net);
                        }
                    }
                }
            }
        }
        guard
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
            let mut nodes = Vec::new();
            if let Some((x0, y0, x1, y1)) = self.grid.node_range(pad.shape.aabb()) {
                for layer in 0..self.board.layer_count {
                    if terminal.layers & (1 << layer) == 0 {
                        continue;
                    }
                    for y in y0..=y1 {
                        for x in x0..=x1 {
                            let cell = self.grid.index(x, y);
                            if statics.trace_allowed(layer, cell, net)
                                && well_inside(&pad.shape, self.grid.center(x, y))
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
        let mut plane: Vec<Vec<bool>> = Vec::new();
        let mut plane_class = vec![description.class; self.board.layer_count];
        for pour in self
            .board
            .planes
            .iter()
            .filter(|pour| pour.net == net && pour.connect)
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
                    if statics.trace_allowed(pour.layer, cell, net)
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
        let on_plane: Vec<bool> = terminal_nodes
            .iter()
            .map(|nodes| {
                nodes.iter().any(|node| {
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
                    plane_element
                } else if terminal == FREE_END {
                    continue;
                } else if terminal != NO_TERMINAL {
                    terminal as usize
                } else {
                    terminal_count + hosts[&node]
                };
                let (a, b) = (find(&mut parent, element), find(&mut parent, other));
                parent[a] = b;
            }
        }
        let component: Vec<usize> = (0..parent.len())
            .map(|element| find(&mut parent, element))
            .collect();

        scratch.tree_generation += 1;
        let tree_generation = scratch.tree_generation;
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
                        let state = state_of(node);
                        scratch.tree_mark[state] = tree_generation;
                        scratch.tree_terminal[state] = element as u16;
                        tree.push(node);
                    }
                    net_state.connected[element] = true;
                } else {
                    for node in &retained[element - terminal_count].nodes {
                        let state = state_of(*node);
                        if scratch.tree_mark[state] != tree_generation {
                            scratch.tree_mark[state] = tree_generation;
                            scratch.tree_terminal[state] = NO_TERMINAL;
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
                let margin = (1 + reroutes / 3).min(10);
                for margin in [margin, margin + 3] {
                    let Some(corridor) =
                        self.plan_corridor(
                            net,
                            net_state,
                            &tree,
                            &targets,
                            hard,
                            margin,
                            scratch.own_active.then_some(scratch.own_claimed.as_slice()),
                        )
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
            let path = planned.or_else(|| {
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
                (windowed != full)
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
                break;
            };
            let first = self.state(path[0]);
            let last = self.state(*path.last().unwrap());
            let start_terminal = scratch.tree_terminal[first];
            let reached = scratch.target_terminal[last] as usize;
            for node in &path[1..] {
                let state = self.state(*node);
                if scratch.tree_mark[state] != tree_generation {
                    scratch.tree_mark[state] = tree_generation;
                    scratch.tree_terminal[state] = NO_TERMINAL;
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
            scratch.tree_generation += 1;
            let tree_generation = scratch.tree_generation;
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
                        let state = self.state(node);
                        if scratch.tree_mark[state] != tree_generation {
                            scratch.tree_mark[state] = tree_generation;
                            scratch.tree_terminal[state] = if element < terminal_count {
                                element as u16
                            } else {
                                NO_TERMINAL
                            };
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
            let start_terminal = scratch.tree_terminal[first];
            let (reached, end_terminal) = if scratch.target_mark[last] == scratch.generation {
                let element = scratch.target_terminal[last] as usize;
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
        for (node, element) in targets {
            if hard && !self.escape_is_free(scratch, net_state, net, *node) {
                continue;
            }
            let state = self.state(*node);
            scratch.target_mark[state] = generation;
            scratch.target_terminal[state] = *element;
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

        let mut heap: BinaryHeap<Reverse<(u32, u32)>> = BinaryHeap::new();
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
            scratch.seen[state] = generation;
            scratch.cost[state] = 0.0;
            scratch.parent[state] = PARENT_SOURCE;
            heap.push(Reverse((
                heuristic(node.layer as usize, x as i32, y as i32).to_bits(),
                state as u32,
            )));
        }

        let statics = &self.statics[class];
        let own = crate::grid::owner(net);
        let trace_base = class * (layers + 1);
        let via_map = trace_base + layers;
        let necks = !net_state.neck_zones.is_empty();
        let mut found = None;
        while let Some(Reverse((_, state))) = heap.pop() {
            let state = state as usize;
            if scratch.closed[state] == generation {
                continue;
            }
            scratch.closed[state] = generation;
            scratch.expansions += 1;
            if scratch.target_mark[state] == generation {
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
            let here = scratch.cost[state];
            let arrived = scratch.parent[state];
            let here_statics = if necks { &self.statics[self.class_at(net, net_state, cell as u32)] } else { statics };
            let blocked_edges = if here_statics.edge_owner[layer][cell] == own {
                0
            } else {
                here_statics.edge_block[layer][cell]
            };

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
                let target_class = if necks { self.class_at(net, net_state, target_cell as u32) } else { class };
                let allowed = self.statics[target_class].trace[layer][target_cell];
                if allowed != crate::grid::FREE && allowed != own {
                    continue;
                }
                let target_state = layer * cells + target_cell;
                if scratch.closed[target_state] == generation {
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
                let history = if self.cleanup {
                    0.0
                } else {
                    self.history[layer][target_cell]
                };
                let mut step =
                    direction_cost[layer][direction] * (1.0 + history) * (1.0 + present * occupied);
                if !self.guard[layer].is_empty() {
                    let guarded = self.guard[layer][target_cell];
                    if guarded != 0 && guarded != own {
                        step *= self.config.thermal_guard_cost as f32;
                    }
                }
                if !self.covered[layer].is_empty() {
                    let pour = self.covered[layer][target_cell];
                    if pour != 0 && pour != own {
                        if self.exclusive[layer] {
                            continue;
                        }
                        step *= self.layer_cut[layer];
                    }
                }
                if !self.layer_bias.is_empty() {
                    step *= self.layer_bias[layer];
                }
                if (arrived as usize) < 8 {
                    let turn = (direction as i32 - arrived as i32).rem_euclid(8);
                    step += bend_cost * turn.min(8 - turn) as f32;
                }
                let total = here + step;
                if scratch.seen[target_state] != generation || total < scratch.cost[target_state] {
                    scratch.seen[target_state] = generation;
                    scratch.cost[target_state] = total;
                    scratch.parent[target_state] = direction as u8;
                    let estimate = total + heuristic(layer, tx as i32, ty as i32);
                    heap.push(Reverse((estimate.to_bits(), target_state as u32)));
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
                for (covered, factor) in self.covered.iter().zip(&self.layer_cut) {
                    if !covered.is_empty() && covered[cell] != 0 && covered[cell] != own {
                        cut *= factor;
                    }
                }
                // A via is copper on every layer: inside another net's
                // thermal ring on any of them it starves the spokes there.
                if self.guard.iter().any(|guard| !guard.is_empty() && guard[cell] != 0 && guard[cell] != own) {
                    cut *= self.config.thermal_guard_cost as f32;
                }
                let step = via_cost * cut * (1.0 + history) * (1.0 + present * occupied);
                let cell_class = if necks { self.class_at(net, net_state, cell as u32) } else { class };
                for target_layer in 0..layers {
                    if target_layer == layer {
                        continue;
                    }
                    let allowed = self.statics[cell_class].trace[target_layer][cell];
                    if allowed != crate::grid::FREE && allowed != own {
                        continue;
                    }
                    if hard && self.occupancy_seen(scratch, class, cell_class * (layers + 1) + target_layer, cell) > 0 {
                        continue;
                    }
                    let target_state = target_layer * cells + cell;
                    if scratch.closed[target_state] == generation {
                        continue;
                    }
                    let total = here + step;
                    if scratch.seen[target_state] != generation
                        || total < scratch.cost[target_state]
                    {
                        scratch.seen[target_state] = generation;
                        scratch.cost[target_state] = total;
                        scratch.parent[target_state] = 8 + layer as u8;
                        let estimate = total + heuristic(target_layer, x as i32, y as i32);
                        heap.push(Reverse((estimate.to_bits(), target_state as u32)));
                    }
                }
            }
        }

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
            let parent = scratch.parent[state];
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
        self.statics = (0..self.board.classes.len())
            .map(|class| StaticMaps::build(&self.board, &self.grid, class))
            .collect();
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
        self.negotiate(&order, pending);
        self.finish_polished(&order, polish)
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
        self.fix_escapes(&order);
        self.route_skeletons(&order);
        self.fix_plane_stubs(&order);
        if self.config.global_routing {
            self.plan_globally(&order);
        }
        self.negotiate(&order, order.clone());
        self.finish_polished(&order, polish)
    }

    /// The first part of `run_in_place`: prepares the nets and negotiates
    /// for at most `seconds`, keeping every bit of state, so that `resume`
    /// can go on from here. Returns how many nets are still conflicted or
    /// incomplete (the measure a caller ranks probes by).
    pub fn probe(&mut self, seconds: f64) -> usize {
        let order = self.routing_order();
        self.fix_escapes(&order);
        self.route_skeletons(&order);
        self.fix_plane_stubs(&order);
        if self.config.global_routing {
            self.plan_globally(&order);
        }
        let limit = self.config.negotiation_seconds;
        self.config.negotiation_seconds = seconds;
        self.negotiate(&order, order.clone());
        self.config.negotiation_seconds = limit;
        self.unfinished()
    }

    /// After `probe`: negotiates on for at most `seconds` (the history of
    /// the probe is kept; the price of sharing starts low again), then
    /// finishes and polishes as `run` does.
    pub fn resume(&mut self, seconds: f64) -> RoutingResult {
        let limit = self.config.negotiation_seconds;
        self.config.negotiation_seconds = seconds;
        self.hopeless = false;
        let result = self.reroute(true);
        self.config.negotiation_seconds = limit;
        result
    }

    /// Routable nets in conflict or not complete.
    pub fn unfinished(&self) -> usize {
        (0..self.nets.len() as NetId)
            .filter(|net| {
                let state = &self.nets[*net as usize];
                state.routable && (!state.complete || !self.conflicts(*net).is_empty())
            })
            .count()
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
        let seconds = self.config.negotiation_seconds;
        self.config.negotiation_seconds = seconds.min(60.0);
        self.negotiate(&pour_nets, pour_nets.clone());
        self.config.negotiation_seconds = seconds;
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
    fn negotiate_from(&mut self, order: &[NetId], mut pending: Vec<NetId>, present: f32) {
        let started = std::time::Instant::now();
        let mut present = present;
        let mut best_conflicted = usize::MAX;
        let mut stalled = 0;
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
            for (batch, jacobi) in batches {
                let search_started = std::time::Instant::now();
                if jacobi && self.config.parallel {
                    self.route_jacobi(&batch, present, growth);
                    self.search_seconds += search_started.elapsed().as_secs_f64();
                    continue;
                }
                for net in &batch {
                    self.rip_up_conflicted(*net);
                }
                if batch.len() == 1 || !self.config.parallel {
                    for net in &batch {
                        self.route_net_seq(*net, present, false, growth);
                    }
                } else {
                    // The nets of a batch do not overlap: routing them against
                    // the same occupancy is the same as routing them in turn.
                    let states = self.grid.cells() * self.board.layer_count;
                    let cells = self.grid.cells();
                    let this: &Self = self;
                    let routed: Vec<(NetId, NetState, [u64; 5])> = batch
                        .par_iter()
                        .map(|net| {
                            with_thread_scratch(states, cells, |scratch| {
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
                                (*net, net_state, delta)
                            })
                        })
                        .collect();
                    for (net, net_state, counters) in routed {
                        self.nets[net as usize] = net_state;
                        self.scratch.searches += counters[0];
                        self.scratch.expansions += counters[1];
                        self.scratch.open_searches += counters[2];
                        self.scratch.open_expansions += counters[3];
                        self.scratch.failed_searches += counters[4];
                    }
                }
                for net in &batch {
                    self.nets[*net as usize].reroutes += 1;
                }
                self.search_seconds += search_started.elapsed().as_secs_f64();
                let stamp_started = std::time::Instant::now();
                for net in &batch {
                    self.stamp(*net);
                }
                self.stamp_seconds += stamp_started.elapsed().as_secs_f64();
            }
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
                for (layer, cell) in conflicts {
                    self.history[layer][cell as usize] += self.config.history_increment as f32;
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
            self.last_conflicted = conflicted.clone();
            if conflicted.is_empty() {
                break;
            }
            // The plan did not survive the lattice where nets conflict:
            // those tiles lose room on the graph and the nets are planned
            // again around them.
            if self.global.is_some() && iteration % 2 == 1 {
                self.replan(&conflicted);
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
            if conflicted.len() < best_conflicted {
                best_conflicted = conflicted.len();
                stalled = 0;
            } else {
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
            let patience = if present >= self.config.present_cap as f32 { self.config.stall_at_cap } else { 25 };
            if stalled > patience || started.elapsed().as_secs_f64() > self.config.negotiation_seconds {
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
        self.negotiation_seconds = started.elapsed().as_secs_f64();
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
        let budget = (self.config.via_reduction_budget * self.negotiation_seconds)
            .max(60.0)
            .min(self.config.via_reduction_seconds);
        let started = std::time::Instant::now();
        for round in 0..self.config.via_reduction_rounds {
            if started.elapsed().as_secs_f64() > budget {
                if self.config.verbose {
                    eprintln!(
                        "via reduction: budget of {budget:.0}s used, stopping before round {round}"
                    );
                }
                break;
            }
            let before = self.quality();
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
            let snapshot = self.clone();
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
            self.config.via_cost = via_cost;
            let after = self.quality();
            let keep = after.0 <= before.0 && after.1 < before.1;
            if self.config.verbose {
                eprintln!(
                    "via reduction round {round}: {:?} -> {:?}, {}",
                    before,
                    after,
                    if keep { "kept" } else { "reverted" }
                );
            }
            if !keep {
                // Restore, but still try the next (more expensive) round.
                let scratch = std::mem::take(&mut self.scratch);
                *self = snapshot;
                self.scratch = scratch;
            }
        }
    }

    fn finish(&mut self, order: &[NetId]) -> RoutingResult {
        self.resolve_remaining(order);
        let cleanup_started = std::time::Instant::now();
        let improved = if self.hopeless { 0 } else { self.clean_up(order) };
        let cleaned = cleanup_started.elapsed().as_secs_f64();
        self.reduce_vias(order);
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
            let trimmed = self.trim_skeleton(net);
            let stitches = self.stitch_pours(net, order);
            if self.config.verbose {
                eprintln!(
                    "pour {}: {stitches} stitching vias, {trimmed} skeleton nodes trimmed, complete {}",
                    self.board.nets[net as usize].name, self.nets[net as usize].complete
                );
            }
        }
        if self.config.verbose {
            eprintln!(
                "cleanup: {improved} improvements, {:.2}s",
                cleanup_started.elapsed().as_secs_f64()
            );
        }

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
        let layers = self.board.layer_count;
        let state = &self.nets[net as usize];
        let own: HashSet<(u32, u32)> = state.stamped.iter().copied().collect();
        let free: Vec<Vec<bool>> = (0..layers)
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
                                == own.contains(&(map as u32, cell as u32)) as usize
                    })
                    .collect()
            })
            .collect();
        let pours = crate::pour::PourMap::build(&self.grid, &free);
        let terminal_count = state.terminal_nodes.len();
        let terminal_base = pours.pieces + 1;
        let branch_base = terminal_base + terminal_count;
        let mut parent: Vec<usize> = (0..branch_base + state.branches.len()).collect();
        let reach = (0.8 / self.grid.pitch).ceil() as usize;
        let union = |parent: &mut Vec<usize>, a: usize, b: usize| {
            let (a, b) = (find(parent, a), find(parent, b));
            parent[a] = b;
        };
        for (terminal, nodes) in state.terminal_nodes.iter().enumerate() {
            for node in nodes {
                if let Some(piece) =
                    pours.piece_near(&self.grid, node.layer as usize, node.cell as usize, reach)
                {
                    union(&mut parent, terminal_base + terminal, piece as usize);
                }
            }
        }
        let keep = vec![true; state.branches.len()];
        let hosts = junction_hosts(&state.branches, &keep);
        for (index, branch) in state.branches.iter().enumerate() {
            for node in &branch.nodes {
                let piece = pours.label[node.layer as usize]
                    .get(node.cell as usize)
                    .copied()
                    .unwrap_or(0);
                if piece != 0 {
                    union(&mut parent, branch_base + index, piece as usize);
                }
            }
            for (terminal, node) in [
                (branch.start_terminal, branch.nodes[0]),
                (branch.end_terminal, *branch.nodes.last().unwrap()),
            ] {
                if terminal == NO_TERMINAL {
                    if let Some(host) = hosts.get(&node) {
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
        let mut votes: HashMap<usize, usize> = HashMap::new();
        for terminal in 0..terminal_count {
            *votes
                .entry(find(&mut parent, terminal_base + terminal))
                .or_default() += 1;
        }
        let main = votes
            .into_iter()
            .max_by_key(|(root, count)| (*count, usize::MAX - *root))
            .map_or(0, |(root, _)| root);
        (pours, parent, main)
    }

    /// Connects pour islands that hold terminals to the main piece with
    /// vias; terminals that cannot be reached that way are routed to the
    /// main piece like off-pour pads. Returns the number of stitching vias.
    fn stitch_pours(&mut self, net: NetId, order: &[NetId]) -> usize {
        let class = self.board.classes[self.board.nets[net as usize].class];
        let class_index = self.board.nets[net as usize].class;
        let layers = self.board.layer_count;
        let terminal_count = self.nets[net as usize].terminal_nodes.len();
        // A labelled node has pour copper; the via's own clearance against
        // foreign copper is checked through the via maps, so no extra ring.
        let via_reach = 0;
        let _ = class;
        let mut stitches = 0;
        let mut rerouted = false;
        let mut previous_islands = usize::MAX;
        let mut stalled = 0;
        for _ in 0..1500 {
            let (pours, mut parent, main) = self.analyze_pours(net);
            let terminal_base = pours.pieces + 1;
            let islands: Vec<usize> = (0..terminal_count)
                .filter(|terminal| find(&mut parent, terminal_base + terminal) != main)
                .collect();
            if islands.is_empty() {
                break;
            }
            // Vias that do not reduce the stranded count are not progress.
            if islands.len() >= previous_islands {
                stalled += 1;
                if stalled > 3 && rerouted {
                    break;
                }
            } else {
                stalled = 0;
            }
            previous_islands = islands.len();
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
                    "pour {}: {} pieces, {} stranded terminals ({} touch a piece)",
                    self.board.nets[net as usize].name,
                    pours.pieces,
                    islands.len(),
                    attached
                );
            }
            let own: HashSet<(u32, u32)> =
                self.nets[net as usize].stamped.iter().copied().collect();
            let via_map = self.map_index(class_index, layers);
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
            let mut best: HashMap<usize, (f64, usize, usize, usize)> = HashMap::new();
            if layers >= 2 {
                let roots: Vec<Vec<usize>> = (0..layers)
                    .map(|layer| {
                        pours.label[layer]
                            .iter()
                            .map(|piece| if *piece == 0 { 0 } else { find(&mut parent, *piece as usize) })
                            .collect()
                    })
                    .collect();
                for layer in 0..layers {
                    for (cell, island) in roots[layer].iter().enumerate() {
                        let Some(anchor) = anchors.get(island) else {
                            continue;
                        };
                        if !self.statics[class_index].via_allowed(cell, net)
                            || self.occupancy[via_map][cell] as usize
                                != own.contains(&(via_map as u32, cell as u32)) as usize
                            || !pours.solid_around(&self.grid, layer, cell, via_reach)
                        {
                            continue;
                        }
                        for other in (0..layers).filter(|other| *other != layer) {
                            let target = roots[other].get(cell).copied().unwrap_or(0);
                            if target == 0 || target == *island || !pours.solid_around(&self.grid, other, cell, via_reach) {
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
            let spacing = class.via_drill + self.board.hole_to_hole;
            let mut chosen: Vec<(usize, usize, usize)> = Vec::new();
            let mut candidates: Vec<(f64, usize, usize, usize)> = best.into_values().filter(|entry| entry.0.is_finite()).collect();
            candidates.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
            for (_, layer, other, cell) in candidates {
                let at = self.grid.center_of(cell);
                if chosen.iter().any(|(_, _, placed)| {
                    *placed == cell || crate::geometry::distance(at, self.grid.center_of(*placed)) < spacing.max(class.via_diameter)
                }) {
                    continue;
                }
                chosen.push((layer, other, cell));
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
            if rerouted {
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
        let nx = self.grid.nx as i64;
        let mut cost = 0.0;
        for branch in &self.nets[net as usize].branches {
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

    /// Reroutes each complete net alone against all other copper and keeps
    /// the result when it is strictly cheaper. Returns the improvements.
    fn clean_up(&mut self, order: &[NetId]) -> usize {
        self.cleanup = true;
        let mut improvements = 0;
        let started = std::time::Instant::now();
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
            for batch in batches {
                if started.elapsed().as_secs_f64() > budget {
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
                    let routed: Vec<(NetState, bool)> = batch
                        .par_iter()
                        .map(|net| {
                            with_thread_scratch(states, cells, |scratch| {
                                let mut net_state = this.nets[*net as usize].clone();
                                if !net_state.stamped.is_empty() {
                                    this.mask_own(scratch, *net, &net_state);
                                }
                                this.route_net(scratch, *net, &mut net_state, 0.0, true, 1.0);
                                scratch.unmask_own();
                                // Batch windows are kept two tiles apart:
                                // routes inside their own window stay clear
                                // of each other.
                                let window = this.window(*net, 1.0);
                                let within = net_state.branches.iter().flat_map(|branch| &branch.nodes).all(|node| {
                                    let (x, y) = this.grid.xy(node.cell as usize);
                                    x >= window.0 && x <= window.2 && y >= window.1 && y <= window.3
                                });
                                (net_state, within)
                            })
                        })
                        .collect();
                    for (index, (net_state, within)) in routed.into_iter().enumerate() {
                        self.nets[batch[index] as usize] = net_state;
                        inside[index] = within;
                    }
                }
                for (index, net) in batch.iter().enumerate() {
                    let state = &self.nets[*net as usize];
                    if inside[index] && state.complete && self.geometric_cost(*net) < before[index] - 1.0e-6 {
                        improved = true;
                        improvements += 1;
                    } else {
                        let state = &mut self.nets[*net as usize];
                        state.branches = saved[index].clone();
                        state.connected.fill(true);
                        state.complete = true;
                    }
                    self.stamp(*net);
                }
            }
            if !improved {
                break;
            }
        }
        self.cleanup = false;
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
        let mut skipped = 0;
        for net in &removed {
            if started.elapsed().as_secs_f64() > budget {
                skipped += 1;
                continue;
            }
            self.route_net_seq(*net, 0.0, true, 4.0);
            self.stamp(*net);
        }
        let rerouted = started.elapsed().as_secs_f64();
        let mut forced = 0;
        for net in removed.iter().copied() {
            if !self.nets[net as usize].complete && started.elapsed().as_secs_f64() <= 2.0 * budget {
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
            let required = half_width
                + SAFETY
                + match obstacle.kind {
                    crate::board::ObstacleKind::Copper => {
                        self.board.copper_clearance(&class, obstacle)
                    }
                    crate::board::ObstacleKind::Keepout => 0.0,
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
