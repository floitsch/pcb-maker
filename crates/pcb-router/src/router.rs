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
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::board::{Board, NetId, NetRoute, RuleClass, Segment, Via};
use crate::grid::{DIRECTIONS, Grid, SAFETY, StaticMaps};
use crate::rules::ClassRules;
mod batching;
mod build;
mod cleanup;
mod config;
/// Read-only tile views of the negotiation, for the congestion model.
pub mod congestion;
mod driver;
mod escapes;
mod fill;
mod finish;
mod fixed;
mod global;
mod guards;
pub(crate) mod hash;
mod heap;
mod manual_spokes;
mod negotiate;
mod occupancy;
mod output;
mod planner;
mod pour_analysis;
mod pour_dump;
mod pour_refresh;
mod pour_report;
mod prepare;
mod profile;
mod resolve;
mod result;
mod route_net;
mod search;
mod sever;
mod spokes;
mod stamping;
mod stitch;
mod straighten;
mod trouble;
mod update;
mod vias;
mod wall;

pub use build::{
    BuildCounts, SharedBuild, StaticMapsHold, held_maps_bytes, hold_static_maps, static_maps_key,
};
pub use config::Config;
pub use pour_report::PourLayerReport;
pub use profile::{ProcessAccounting, build_totals, process_accounting};
pub use result::{DeadPad, Diagnostics, HotSpot, NetStatus, RoutingResult};
pub use trouble::{TROUBLE_REPAIR_DEFAULT, trouble_override};
pub use wall::{WALL_CUTS, note_wall_cut, wall_cuts, wall_guard};

use hash::*;
use heap::*;
use occupancy::Occupancy;
use profile::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Node {
    layer: u8,
    cell: u32,
}

const NO_TERMINAL: u16 = u16::MAX;
/// How far an escape stub may come short of a clearance: floating point
/// noise, far below the verifier's tolerance and KiCad's DRC epsilon.
const STUB_FIT: f64 = 1.0e-6;

/// Search expansions per second of a router's phases (their wall clock
/// without the pour analyses, which count by their own model) on the
/// reference: the 5950X nearly idle, one search thread (`RAYON_NUM_THREADS`
/// 1), route mode. Calibrated 2026-10-15 on x1150 (docs/router.md,
/// "Determinism and the wall clock"): the median of Castor 3.34M, jetson
/// 3.23M, k30-SBC 3.27M, Sisu 3.07M, urchin 2.78M and spell_tome 2.27M a
/// second. It was 2.0e6 (an idle eight-core machine's negotiation,
/// ATAT1800: 52M in 29 s), stale by four speed rounds. The rate a process
/// turns its time budgets into work at is `expansions_per_second`: this
/// one unless the process set its own (pcb-kicad's `work_rate`: the
/// start-up calibration scales it to the machine, `PCB_WORK_RATE` pins it).
pub const REFERENCE_EXPANSIONS_PER_SECOND: f64 = 3.15e6;

/// The rate `set_expansions_per_second` set (its bits; 0 when unset).
static EXPANSION_RATE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Search expansions a second of work stands for in this process: it turns
/// every time budget into work and the work back into seconds. The
/// reference's rate (`REFERENCE_EXPANSIONS_PER_SECOND`) unless the process
/// set its own; set once before any work, so that a run's results depend
/// on it only as a fixed input (docs/router.md, "Determinism and the wall
/// clock").
pub fn expansions_per_second() -> f64 {
    match EXPANSION_RATE.load(std::sync::atomic::Ordering::Relaxed) {
        0 => REFERENCE_EXPANSIONS_PER_SECOND,
        bits => f64::from_bits(bits),
    }
}

/// A negotiation's limit of `seconds` of work as search expansions
/// (`Config::negotiation_expansions`), at least one: 0 means no limit
/// there, so a bound of nothing (the ladder's `bounded_negotiation` with
/// nothing left) stops the negotiation after its first iteration by this
/// limit, not by the work deadline that happens to be reached too.
pub fn negotiation_expansions(seconds: f64) -> u64 {
    ((seconds.max(0.0) * expansions_per_second()) as u64).max(1)
}

/// Sets the rate of `expansions_per_second` for the rest of the process
/// (pcb-kicad's `work_rate`, before a run starts). Panics on a rate that
/// is not a positive finite number.
pub fn set_expansions_per_second(rate: f64) {
    assert!(
        rate.is_finite() && rate > 0.0,
        "a work rate must be positive: {rate}"
    );
    EXPANSION_RATE.store(rate.to_bits(), std::sync::atomic::Ordering::Relaxed);
}

/// How many seconds of this machine a second of the reference's modelled
/// work takes (the reference's rate over this process's: 1 at the
/// reference's rate): the pour analyses' nanoseconds and KiCad's checks
/// are modelled on the reference and scale by it.
pub fn reference_time_scale() -> f64 {
    REFERENCE_EXPANSIONS_PER_SECOND / expansions_per_second()
}
/// How much longer than its work the clock may run before it stops a
/// route-mode negotiation anyway (a loaded machine), on a machine as fast
/// as its work rate says (`guard`).
pub const GUARD: f64 = 4.0;

static WALL_STRETCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// How many times longer than its work rate says this machine takes a
/// second of work (pcb-kicad's `KiCadWorkRate::wall_stretch`: a pinned
/// rate over the measured one, at least 1): the wall-clock guards stretch
/// by it, so that a pinned rate faster than the machine does not turn
/// them into budgets. 1 until set.
pub fn wall_stretch() -> f64 {
    match WALL_STRETCH.load(std::sync::atomic::Ordering::Relaxed) {
        0 => 1.0,
        bits => f64::from_bits(bits),
    }
}

/// Sets `wall_stretch` for the rest of the process (at least 1).
pub fn set_wall_stretch(stretch: f64) {
    let stretch = if stretch.is_finite() {
        stretch.max(1.0)
    } else {
        1.0
    };
    WALL_STRETCH.store(stretch.to_bits(), std::sync::atomic::Ordering::Relaxed);
}

/// `GUARD` stretched by `wall_stretch`: how much longer than its work the
/// wall clock may run before a guard stops it.
pub fn guard() -> f64 {
    GUARD * wall_stretch()
}
/// The wall clock a phase budget always has before its guard (`spent`)
/// may stop it: `GUARD` times a budget under about half a second is less
/// than one pour analysis takes on a big board (a second of wall), so the
/// guard cut such a phase before any work was done ("budget guard: 0 s of
/// wall clock for a budget of 0 s of work", Batch_1 in route mode).
pub const BUDGET_GUARD_FLOOR: f64 = 5.0;

/// Whether a phase budget of `budget` work seconds is over by the wall
/// clock after `elapsed` seconds: `GUARD` times the budget, but never
/// before `BUDGET_GUARD_FLOOR`.
pub(crate) fn budget_guard_fires(elapsed: f64, budget: f64) -> bool {
    elapsed > (budget * guard()).max(BUDGET_GUARD_FLOOR)
}
/// The pour analysis's work (`analyze_pours`, one thread), in nanoseconds
/// of an idle machine: per lattice node inside the net's pour (the fill
/// mask and its labelling), per stamp of the net (its own copper, masked
/// out), and for its fill probe (`fill::FillProbe`, the spoke tests) per
/// item it indexes or looks at, per query (a point tested against its
/// index: the bucket lookups and the polygon test of `inside`, whatever
/// the items) and per polygon vertex its shape tests go through (a board
/// cutout of 134 vertices in every query near it: Batch_1's panel). The
/// budgets of the phases after negotiation count it beside the search
/// (`budget_work`). Calibrated at one thread in route mode on twelve
/// boards (docs/router.md, "Determinism and the wall clock"; 2026-10-15,
/// x1013: Batch_1, WLED_Matrix, USB_Keypad, Qfwfq, 1337-v4.0 and the A13
/// module hatched, Castor, Sisu, reCamera, k30-SBC, USB_Cable_Tester and
/// spell_tome solid): per board 0.89-1.12 of the analyses' wall time but
/// spell_tome's 1.1 s (1.41). Items alone (42 ns, x816) gave hatched
/// boards 0.34-1.82: their probes ask many more points (hatch holes,
/// rings), each costing far more than its items.
pub const POUR_NODE_NANOS: u64 = 23;
pub const POUR_STAMP_NANOS: u64 = 6;
pub const PROBE_ITEM_NANOS: u64 = 9;
pub const PROBE_QUERY_NANOS: u64 = 850;
pub const PROBE_VERTEX_NANOS: u64 = 9;
/// A router's build (`Router::new`: the static maps of every rule class,
/// the nets' terminals and pour masks, the search records), in
/// nanoseconds of an idle machine (`BuildCounts::work_nanos`): per lattice
/// node of every map (a node of a layer, for every class: the maps'
/// allocation, the outline fill, the tile counts, the search records),
/// per node tested (against the outline and the obstacles in the static
/// maps, inside the nets' pads and pour boxes), per shape edge those
/// tests measure and per edge a node near an obstacle measures its eight
/// steps against (a segment against a segment). Route mode's ladder
/// charges it with each attempt (docs/router.md, "All the work
/// charged"). Fitted 2026-10-21 on 83 distinct builds of the expansion
/// tier's boards and the named boards (route mode at 4 s, one thread,
/// the fastest of each build's repeats, scaled by the machine's measured
/// share at load 20 to 30): the sum 1.0 of the measured, per board 0.6
/// to 1.6 (10 % to 90 %); repeats of one build in one process differ by
/// 1.34 times (median) and up to 2 under that load.
pub const BUILD_MAP_NANOS: f64 = 22.0;
pub const BUILD_TEST_NANOS: f64 = 11.0;
pub const BUILD_EDGE_NANOS: f64 = 2.75;
pub const BUILD_STEP_NANOS: f64 = 22.0;
/// A build whose static maps another router built (`BuildCounts::
/// shared_map_nodes`): per node of its maps, its own maps alone (the
/// occupancy, the tile counts, the search records; and the static maps'
/// key). Fitted 2026-10-21 on the shared builds of eleven probed boards
/// of the expansion tier (route mode at 4 s, one thread, the fastest of
/// each board's repeats scaled by the machine's measured share): 11.7 ns
/// a node over their sum, 7.0 to 20.6 per board (docs/router.md, "The
/// probes' overhead").
pub const BUILD_SHARED_MAP_NANOS: f64 = 12.0;
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
    /// which KiCad's DRC counts as this pad's.
    pub crossings: usize,
    pub layer: usize,
    pub round: bool,
    pub outcomes: [SpokeOutcome; 4],
    /// The pad's own tracks that KiCad's DRC counts as spokes (a run
    /// leaving the pad and ending in fill), counted when the spokes alone
    /// are short of two (`kicad_spokes`).
    pub tracks: usize,
    /// Where several zones of the net fill the layer:
    /// how many spokes the worst of them is short (KiCad's DRC checks the
    /// pad against each zone apart, with what lies in that zone's part),
    /// and that zone (an index into the board's planes).
    pub seam_short: usize,
    pub seam_zone: Option<usize>,
    /// Whether KiCad draws the pad's spokes at all (they are no narrower
    /// than the zone's minimum width).
    pub drawn: bool,
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
    stamped: Stamped,
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
    /// Short tracks from a pad merged into its terminal's larger pad (an
    /// exposed pad's thermal via) into fill over that pad, which KiCad's
    /// DRC counts as the via's spokes (`add_inner_spokes`): layer, start
    /// (the via's centre), end and width. Inside the larger pad's copper
    /// they keep every clearance it keeps.
    inner_spokes: Vec<(usize, crate::geometry::Point, crate::geometry::Point, f64)>,
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
    /// Per terminal: no path reaches it from the net's tree on the whole
    /// board in this negotiation (a board-wide search failed with it open).
    /// Its searches leave it out, so that one pad walled in by pads does
    /// not stop the rest of the net. Empty: none.
    unreachable: Vec<bool>,
    /// Per layer: the rule class describing the pour there.
    plane_class: Vec<usize>,
    /// When set, only these pour nodes (the main piece) are valid targets.
    plane_target: Vec<Vec<bool>>,
    /// Leading branches that are the plane skeleton; never ripped up.
    fixed: usize,
    /// Node ranges (x0, y0, x1, y1) around the net's narrow pads where it
    /// routes at the neck width.
    neck_zones: Vec<(usize, usize, usize, usize)>,
    /// What preparing the net took, counted (`BuildCounts`): nodes tested
    /// and the shape edges measured (its terminals' nodes).
    prepare_counts: [u64; 2],
    /// The same of its connected pours' masks, whether computed here or
    /// found (`POUR_MASKS`: the canonical price, `SharedBuild::masks`).
    mask_counts: [u64; 2],
    /// Whether its masks were found rather than computed here (the work
    /// this process did, `Router::physical_build_work`).
    masks_found: bool,
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
    /// What the node footprints add to the same footprint stamped at the
    /// node before (see `Fresh`).
    fresh: Fresh,
}

/// The offsets of a footprint that the same footprint, stamped one step
/// earlier, does not already cover: `o` with `o + step` not in it, in the
/// footprint's order. A routed path is mostly straight runs on one layer,
/// so a node of a run marks only its footprint's leading edge; the rest
/// of its cells were marked by the node before (`Router::stamp`).
#[derive(Clone, Default)]
struct Fresh {
    /// Per step from the node before, at `(dx + 1) * 3 + (dy + 1)`.
    trace_to_trace: [Vec<(i32, i32)>; 9],
    trace_to_via: [Vec<(i32, i32)>; 9],
    diagonal_block: [[Vec<(i32, i32)>; 9]; 2],
    /// The largest coordinate of any offset of the footprints: a node at
    /// least this far from the lattice's edges needs no bounds test.
    reach: i64,
}

impl Fresh {
    fn of(stamps: &Stamps) -> Self {
        fn ahead(offsets: &[(i32, i32)], (sx, sy): (i32, i32)) -> Vec<(i32, i32)> {
            let all: HashSet<(i32, i32)> = offsets.iter().copied().collect();
            offsets
                .iter()
                .copied()
                .filter(|(dx, dy)| !all.contains(&(dx + sx, dy + sy)))
                .collect()
        }
        fn steps(offsets: &[(i32, i32)]) -> [Vec<(i32, i32)>; 9] {
            std::array::from_fn(|index| {
                ahead(offsets, (index as i32 / 3 - 1, index as i32 % 3 - 1))
            })
        }
        let lists = [
            &stamps.stub_to_trace,
            &stamps.stub_to_via,
            &stamps.trace_to_trace,
            &stamps.trace_to_via,
            &stamps.via_to_trace,
            &stamps.via_to_via,
            &stamps.diagonal_to_trace,
            &stamps.diagonal_to_via,
            &stamps.diagonal_block[0],
            &stamps.diagonal_block[1],
            &stamps.diagonal_block_via[0],
            &stamps.diagonal_block_via[1],
            &stamps.stub_diagonal_block[0],
            &stamps.stub_diagonal_block[1],
        ];
        let reach = lists
            .iter()
            .flat_map(|list| list.iter())
            .map(|(dx, dy)| dx.abs().max(dy.abs()) as i64)
            .max()
            .unwrap_or(0);
        Self {
            trace_to_trace: steps(&stamps.trace_to_trace),
            trace_to_via: steps(&stamps.trace_to_via),
            diagonal_block: [
                steps(&stamps.diagonal_block[0]),
                steps(&stamps.diagonal_block[1]),
            ],
            reach,
        }
    }
}

/// Asks the cache for the line holding `value` (a hint; nothing on other
/// architectures).
#[inline(always)]
fn prefetch<T>(value: &T) {
    // SAFETY: a prefetch reads nothing and faults on no address; SSE is
    // part of every x86_64 target.
    #[cfg(target_arch = "x86_64")]
    unsafe {
        std::arch::x86_64::_mm_prefetch::<{ std::arch::x86_64::_MM_HINT_T0 }>(
            value as *const T as *const i8,
        )
    };
    #[cfg(not(target_arch = "x86_64"))]
    let _ = value;
}

/// A net's stamps (`NetState::stamped`): (map index, cell) pairs in
/// order, packed in one 32-bit word each where the lattice's maps and
/// cells fit (the map above the cell's bits; every board measured), else
/// two. Four bytes a stamp instead of eight: the lists of a big board's
/// nets were its largest part beside the lattice maps (katia: 250 MB).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Stamped {
    words: Vec<u32>,
    /// The bits of a packed word below the map; `WIDE_STAMPS` for two
    /// words a stamp.
    shift: u32,
}

/// `Stamped::shift` where a stamp takes two words.
const WIDE_STAMPS: u32 = u32::MAX;

impl Stamped {
    /// An empty list for a lattice of `maps` maps of `cells` cells.
    fn with_capacity(capacity: usize, maps: usize, cells: usize) -> Self {
        let shift = usize::BITS - cells.saturating_sub(1).leading_zeros();
        let packs = shift < 32 && (maps.saturating_sub(1) as u64) < (1u64 << (32 - shift));
        let shift = if packs { shift } else { WIDE_STAMPS };
        Self {
            words: Vec::with_capacity(if packs { capacity } else { capacity * 2 }),
            shift,
        }
    }

    #[inline(always)]
    fn push(&mut self, map: u32, cell: u32) {
        if self.shift == WIDE_STAMPS {
            self.words.push(map);
            self.words.push(cell);
        } else {
            self.words.push(map << self.shift | cell);
        }
    }

    fn len(&self) -> usize {
        if self.shift == WIDE_STAMPS {
            self.words.len() / 2
        } else {
            self.words.len()
        }
    }

    fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    #[inline(always)]
    fn get(&self, index: usize) -> Option<(u32, u32)> {
        if self.shift == WIDE_STAMPS {
            Some((*self.words.get(2 * index)?, self.words[2 * index + 1]))
        } else {
            let word = *self.words.get(index)?;
            Some((word >> self.shift, word & ((1u32 << self.shift) - 1)))
        }
    }

    /// The (map, cell) pairs in order.
    fn iter(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        (0..self.len()).map(|index| self.get(index).unwrap())
    }

    fn contains(&self, pair: (u32, u32)) -> bool {
        self.iter().any(|stamp| stamp == pair)
    }
}

/// Marks the offsets around `cell`, mirrored by (`sx`, `sy`), in `map`'s
/// bitset of `marks` (`words` a map) and lists the newly marked cells of
/// `map` in `stamped`.
#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn mark_footprint(
    marks: &mut [u64],
    words: usize,
    stamped: &mut Stamped,
    map: usize,
    cell: u32,
    offsets: &[(i32, i32)],
    (sx, sy): (i64, i64),
    (nx, ny): (i64, i64),
    reach: i64,
) {
    let marks = &mut marks[map * words..(map + 1) * words];
    let x = cell as i64 % nx;
    let y = cell as i64 / nx;
    if x >= reach
        && y >= reach
        && x + reach < nx
        && y + reach < ny
        && sx.abs() == 1
        && sy.abs() == 1
    {
        let (row, column) = (sy * nx, sx);
        for (dx, dy) in offsets {
            let target = (cell as i64 + *dy as i64 * row + *dx as i64 * column) as usize;
            let bit = 1u64 << (target % 64);
            let word = &mut marks[target / 64];
            if *word & bit == 0 {
                *word |= bit;
                stamped.push(map as u32, target as u32);
            }
        }
        return;
    }
    for (dx, dy) in offsets {
        let tx = x + *dx as i64 * sx;
        let ty = y + *dy as i64 * sy;
        if tx < 0 || ty < 0 || tx >= nx || ty >= ny {
            continue;
        }
        let target = (ty * nx + tx) as usize;
        if marks[target / 64] & (1 << (target % 64)) == 0 {
            marks[target / 64] |= 1 << (target % 64);
            stamped.push(map as u32, target as u32);
        }
    }
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

/// The nodes where a branch ends on another (a junction): `materialize`
/// ends a segment there, so a straight run of the lattice is cut into
/// segments at them.
fn junction_nodes(branches: &[Branch]) -> HashSet<Node> {
    branches
        .iter()
        .filter(|branch| !branch.nodes.is_empty())
        .flat_map(|branch| {
            let start = (branch.start_terminal == NO_TERMINAL).then(|| branch.nodes[0]);
            let end = (branch.end_terminal == NO_TERMINAL).then(|| *branch.nodes.last().unwrap());
            start.into_iter().chain(end)
        })
        .collect()
}

/// Maps every node that can host a junction to the kept branch owning it.
/// A branch's own junction end belongs to its host, not to itself.
/// A net's own stamps on a few occupancy maps, as bitsets: what the pour
/// analysis asks of every node ("is the one stamp here this net's?"),
/// without hashing the net's whole stamp list each time (a ground net
/// stamps millions of cells).
struct OwnStamps {
    /// The bitset of each map given, by map index (`NO_SLOT`: not given):
    /// a table, not a hash map, as the pour analysis asks once per node.
    slot: Vec<usize>,
    bits: Vec<Vec<u64>>,
}

const NO_SLOT: usize = usize::MAX;

impl OwnStamps {
    fn new(stamped: &Stamped, maps: &[usize], cells: usize) -> Self {
        let mut slot = vec![NO_SLOT; maps.iter().max().map_or(0, |map| map + 1)];
        let mut count = 0;
        for map in maps {
            if slot[*map] == NO_SLOT {
                slot[*map] = count;
                count += 1;
            }
        }
        let mut bits = vec![vec![0u64; cells.div_ceil(64)]; count];
        for (map, cell) in stamped.iter() {
            if let Some(&index) = slot.get(map as usize)
                && index != NO_SLOT
            {
                bits[index][cell as usize / 64] |= 1 << (cell % 64);
            }
        }
        Self { slot, bits }
    }

    /// Whether the net stamped `cell` of `map` (one of the maps given).
    #[inline]
    fn contains(&self, map: usize, cell: usize) -> bool {
        let index = self.slot[map];
        assert!(
            index != NO_SLOT,
            "map {map} is not one of the net's own stamp maps"
        );
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
        let last = branch
            .nodes
            .len()
            .saturating_sub((branch.end_terminal == NO_TERMINAL) as usize);
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

/// `Router::class_at` for a node given by its coordinates: the neck class
/// inside one of the net's neck zones, else the net's class.
#[inline]
fn zone_class(
    class: usize,
    neck: usize,
    zones: &[(usize, usize, usize, usize)],
    x: usize,
    y: usize,
) -> usize {
    if zones
        .iter()
        .any(|&(x0, y0, x1, y1)| x >= x0 && x <= x1 && y >= y0 && y <= y1)
    {
        neck
    } else {
        class
    }
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
/// (see `Router::hot`). Eight bytes (twelve until 2026-10-22): the static
/// map names a net in 16 bits (`StaticMaps::trace`), and the guard and
/// the pour cover, which few nodes have and few combinations of nets
/// make, are an index into `Router::hot_pours`.
#[derive(Clone, Copy, Default)]
struct Hot {
    history: f32,
    /// `StaticMaps::trace`: `FREE16`, `BLOCKED16` or an `owner16`.
    trace: u16,
    /// 0 where neither `guard` nor `covered` holds a net; else the index
    /// of the pair (guard, cover) in `Router::hot_pours`, or `HOT_POURS_MAPS`
    /// where the table is full and the per-layer maps are to be read.
    pour: u16,
}

/// `Hot::pour` where the pair is read from `Router::guard` and
/// `Router::covered` (a table of more pairs than 16 bits index).
const HOT_POURS_MAPS: u16 = u16::MAX;

/// `Router::hot_of` of a class without records.
const NO_HOT: usize = usize::MAX;

/// `Router::via_cover` where the per-layer maps must be read.
const VIA_COVER_MIXED: u32 = u32::MAX;

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
    /// The pour analyses' work in nanoseconds of an idle machine
    /// (`pour_analysis_nanos`, `POUR_NODE_NANOS`), counted from `&self`
    /// (the analysis does not mutate the router).
    pour_units: std::sync::atomic::AtomicU64,
    /// Wall nanoseconds of those analyses (for the profile only).
    pour_nanos: std::sync::atomic::AtomicU64,
    /// Items the fill probes (`fill::FillProbe`: the pour analysis's
    /// spoke tests) put in their index and looked at.
    probe_ops: std::sync::atomic::AtomicU64,
    /// The fill probes' queries (a brush centre tested against the index)
    /// and the polygon vertices their shape tests went through.
    probe_queries: std::sync::atomic::AtomicU64,
    probe_vertices: std::sync::atomic::AtomicU64,
}

impl Scratch {
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
        self.nodes.len() == states && self.own_via_near.len() == cells
    }

    /// What the per-node arrays of a lattice of `states` take once a
    /// search allocated them (`ensure`): a router allocates its scratch on
    /// its first search, and one that never searches (the pour measure of
    /// a finished board) none.
    fn bytes_for(states: usize, cells: usize) -> usize {
        states * std::mem::size_of::<SearchNode>() + cells * 2
    }

    /// Moves the search state (the arrays and the generation their marks
    /// belong to) out, for a parallel batch to lend (`ScratchPool`); the
    /// counters stay.
    fn take_arrays(&mut self) -> Scratch {
        let counters = self.clone();
        std::mem::replace(self, counters)
    }

    /// Takes the search state of `arrays` (see `take_arrays`), keeping
    /// this scratch's counters.
    fn restore_arrays(&mut self, arrays: Scratch) {
        let counters = std::mem::replace(self, arrays);
        let Scratch {
            nodes,
            targets,
            tree,
            own_via_near,
            heap,
            own_mark,
            own_set,
            own_claimed,
            own_active,
            generation,
            corridor,
            ..
        } = std::mem::take(self);
        *self = Scratch {
            nodes,
            targets,
            tree,
            own_via_near,
            heap,
            own_mark,
            own_set,
            own_claimed,
            own_active,
            generation,
            corridor,
            ..counters.clone()
        };
    }

    /// Allocates the arrays for a lattice of `states` (a copy has none).
    /// The mask of the routed net's own stamps (`mask_own`, set before
    /// the search allocates: a router's scratch and a pool's are allocated
    /// on first use) stays.
    fn ensure(&mut self, states: usize, cells: usize) {
        if !self.fits(states, cells) {
            let own_mark = std::mem::take(&mut self.own_mark);
            let own_set = std::mem::take(&mut self.own_set);
            let own_claimed = std::mem::take(&mut self.own_claimed);
            *self = Self {
                nodes: vec![SearchNode::default(); states],
                own_via_near: vec![0; cells],
                own_mark,
                own_set,
                own_claimed,
                own_active: self.own_active,
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
            pour_units: std::sync::atomic::AtomicU64::new(
                self.pour_units.load(std::sync::atomic::Ordering::Relaxed),
            ),
            pour_nanos: std::sync::atomic::AtomicU64::new(
                self.pour_nanos.load(std::sync::atomic::Ordering::Relaxed),
            ),
            probe_ops: std::sync::atomic::AtomicU64::new(
                self.probe_ops.load(std::sync::atomic::Ordering::Relaxed),
            ),
            probe_queries: std::sync::atomic::AtomicU64::new(
                self.probe_queries
                    .load(std::sync::atomic::Ordering::Relaxed),
            ),
            probe_vertices: std::sync::atomic::AtomicU64::new(
                self.probe_vertices
                    .load(std::sync::atomic::Ordering::Relaxed),
            ),
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

/// The search scratches the jobs of a parallel batch take and give back
/// (`Router::lend_scratches`): the router's own and its spares, one a
/// thread at most. Until 2026-10-22 every pool thread kept a scratch of
/// its own in a thread local, beside the router's and after the router
/// was gone (one thread: two scratches of the board's size, katia 143 MB
/// each, jiran-ble-lite at 0.05 mm 343).
struct ScratchPool(std::sync::Mutex<Vec<Scratch>>);

impl ScratchPool {
    /// Runs `body` with a scratch of the pool (a new one when every
    /// scratch is in use; `route_net` allocates its arrays).
    fn with<T>(&self, body: impl FnOnce(&mut Scratch) -> T) -> T {
        let mut scratch = self
            .0
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .pop()
            .unwrap_or_default();
        let value = body(&mut scratch);
        self.0
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .push(scratch);
        value
    }
}

/// A router's spare search scratches, kept between its parallel batches
/// (`ScratchPool`). A copy of the router has none.
#[derive(Default)]
struct SpareScratches(Vec<Scratch>);

impl Clone for SpareScratches {
    fn clone(&self) -> Self {
        Self::default()
    }
}

/// Called with the iteration and a snapshot after each negotiation
/// iteration (`Router::set_frame_hook`).
pub type FrameHook = Arc<dyn Fn(usize, &RoutingResult) + Send + Sync>;

/// A pad's spoke corridors (`Router::spoke_corridors`): the layer, the net,
/// the cells of each of its four corridors (the spoke's own strip first)
/// and how many cells of each that strip is.
type SpokeCorridors = (usize, NetId, [Vec<usize>; 4], [usize; 4]);

/// A lattice window: the first and last column and row (inclusive).
type CellWindow = (usize, usize, usize, usize);

/// A pad the spoke model checks (`Router::kicad_spokes`): its terminal,
/// the pad, its anchor, the layers it is checked on and those of them on
/// which it lies inside its terminal's pad.
type SpokePad = (
    usize,
    usize,
    crate::geometry::Point,
    std::collections::BTreeSet<usize>,
    std::collections::BTreeSet<usize>,
);

#[derive(Clone)]
pub struct Router {
    board: Board,
    config: Config,
    grid: Grid,
    /// Shared by the copies of a router (they change only with the board,
    /// in `update`, which replaces them).
    statics: Arc<Vec<StaticMaps>>,
    /// Whether another router (or a hold) built `statics`
    /// (`shared_statics`): the work this process did for them is not this
    /// router's (`physical_build_work`); its charge is the same either way
    /// (`build_work`, `shared_build`).
    statics_shared: bool,
    /// The static maps' key when they are shared (`statics_key`): the
    /// pour masks held for them are this router's too.
    statics_key: Option<u128>,
    /// Dynamic occupancy, `[class * (layers + 1) + layer]`; index `layers`
    /// is the via map. A value counts the nets forbidding that class there
    /// (a byte a count, `Occupancy`). After the class maps follow the diagonal maps (see `diagonal_map`):
    /// the cells from which a diagonal step of that class would pass a
    /// node too closely.
    occupancy: Occupancy,
    /// Bits per (map, cell): set while one net is stamped, so that a cell
    /// two offsets reach counts once; cleared again from the stamped list.
    stamp_mark: Vec<u64>,
    history: Vec<Vec<f32>>,
    /// `stamps[net class][variant * classes + querying class]`, the
    /// variant the layer's (`Router::stamps_on`; one variant without
    /// layer-qualified rules).
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
    /// Per layer and node: the pour net (as `owner16`) whose pad needs the
    /// surrounding copper for its thermal spokes; other nets pay to pass
    /// (16 bits since 2026-10-23, as the search records hold it).
    guard: Vec<Vec<u16>>,
    /// Per node: bit 0 when any layer's `covered`, bit 1 when any layer's
    /// `guard` holds a net there (most nodes: neither, and a via step
    /// skips the per-layer maps).
    via_marks: Vec<u8>,
    /// Per node with bit 0 of `via_marks`: the one pour net covering it
    /// (`owner`, low 16 bits) and the layers it covers (bits 16 to 30),
    /// or `VIA_COVER_MIXED` where several nets cover it or the board has
    /// more than 15 layers (a via step then reads `covered` per layer).
    /// A via step reads one word for what were up to a cache line a layer.
    /// Empty when no layer has a `covered` map.
    via_cover: Vec<u32>,
    /// Per layer and node: the pour (as `owner16`) covering the node, if
    /// any.
    covered: Vec<Vec<u16>>,
    /// Per set of records (`hot_of`) and layer (index `set * layers +
    /// layer`), per node: what a search step reads of the static map, the
    /// thermal guard, the pour cover and the history, in one record. A big
    /// board's search is bound by memory (PolyKybd's 9.4M-state lattice
    /// ran at 0.65M expansions a second against 3M on a small one); one
    /// cache line per neighbour instead of four. Kept in step with
    /// `history`. Only the static map's value depends on the class: the
    /// classes of one static trace map share a set, and a neck class (no
    /// net of its own) has none: a search step into a neck zone reads its
    /// net class's record and the neck's static map (until 2026-10-23 a
    /// set per searched class: katia 730 MB, 365 now).
    hot: Vec<Vec<Hot>>,
    /// Per rule class: its set of `hot` records, `NO_HOT` for a class no
    /// net has.
    hot_of: Vec<usize>,
    /// The (guard, cover) pairs `Hot::pour` indexes, as `owner`s in 16
    /// bits; entry 0 is (0, 0).
    hot_pours: Vec<(u16, u16)>,
    /// Per layer and tile: share of the tile under some pour.
    tile_covered: Vec<Vec<f32>>,
    /// Per layer: cost factor for cutting a pour there. Relative to the
    /// least covered layer, so a board poured on every layer has no penalty.
    layer_cut: Vec<f32>,
    /// Per layer and node: what a cut there severs of the pour covering
    /// it, 0 to 255 (`refresh_sever`; empty for a layer without pours or
    /// with `pour_integrity` 0).
    sever: Vec<Vec<u8>>,
    /// `SEVER_COST * pour_integrity / 255`: the factor a level adds.
    sever_scale: f32,
    /// `copper_changes` when `sever` was last computed.
    sever_at: Option<u64>,
    /// The search expansions when `sever` was last computed.
    sever_expansions: u64,
    /// Counts every stamp and unstamp of a net: copper changed.
    copper_changes: u64,
    /// What computing the pour masks of the pours not connected (pour
    /// nets as tracks) for `covered` counted (`BuildCounts`).
    cover_mask_counts: [u64; 2],
    /// Per layer: an exclusive plane there (other nets' tracks may not run
    /// on its covered nodes).
    exclusive: Vec<bool>,
    /// Per rule class: the class with the neck width instead of the track
    /// width (an added class), if the track is wider.
    neck_of: Vec<Option<usize>>,
    /// The rules beyond the net classes, resolved against the board's
    /// split classes (`Config::net_rules`); `None` without them.
    class_rules: Option<Arc<ClassRules>>,
    /// Per rule class: whether a search runs with it (see
    /// `searched_classes`). The others (pour brushes) have no `hot`
    /// records and no via owners or free-layer words in their static maps.
    searched: Vec<bool>,
    /// Per layer: step cost multiplier while a plane skeleton is routed
    /// (empty otherwise).
    layer_bias: Vec<f32>,

    scratch: Scratch,
    /// The scratches beyond `scratch` that parallel batches used.
    spare_scratches: SpareScratches,
    profile: Profile,
    /// While the via reduction runs (its negotiations are its own phase).
    in_via_reduction: bool,
    /// While a layout's trial (`reroute` without the polish) finishes:
    /// the trouble-makers' repair waits for the board kept.
    in_trial: bool,
    /// Whether the negotiation running is a probe's (`probe`).
    in_probe: bool,
    /// The rest of the iteration a probe's work limit cut (`probe`), for
    /// `resume` to finish first.
    cut_iteration: Option<negotiate::CutIteration>,
    /// Island stubs `add_manual_spokes` added (`Config::island_stubs`),
    /// over every finish of this router: `RoutingResult::island_stubs`.
    island_stubs: usize,
    /// `RoutingResult::manual_spokes_work`.
    manual_spokes_work: f64,
    /// `RoutingResult::build_work`: this router's build, modelled
    /// (`BuildCounts::work_seconds`).
    build_work: f64,
    /// `RoutingResult::first_iteration_expansions` (0 before it).
    first_iteration_expansions: u64,
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
    frame_hook: Option<FrameHook>,
    /// Per obstacle with a polygon of many corners, its parts as the fill
    /// probe measures them (`probe_shape`): built on a probe's first
    /// question, shared by the router's copies, new with a new board.
    probe_shapes: Arc<std::sync::OnceLock<FastMap<usize, Vec<fill::ProbePart>>>>,
}

pub fn route(board: &Board, config: &Config) -> RoutingResult {
    Router::new(board, config).run()
}

#[cfg(test)]
mod rule_tests;
#[cfg(test)]
mod tests;
