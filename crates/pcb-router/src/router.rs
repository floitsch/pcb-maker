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

use crate::board::{Board, NetId, NetRoute, Segment, Via};
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
    /// After the board is complete, renegotiate the nets that have vias with
    /// the via cost multiplied by `via_reduction_factor`, this many times,
    /// keeping a round only if nothing opens and vias go down.
    pub via_reduction_rounds: usize,
    pub via_reduction_factor: f64,
    /// Heuristic weight while reducing vias (searches there are long
    /// same-layer detours; a little greed keeps them cheap).
    pub via_reduction_weight: f64,
    /// Route spatially disjoint nets of one iteration in parallel.
    pub parallel: bool,
    /// Batch size when overlap is ignored (0 keeps batches disjoint). Nets in
    /// a batch then route against the same occupancy and settle their
    /// conflicts in later iterations.
    pub jacobi_batch: usize,
    /// Values above 1 trade path optimality for search speed.
    pub heuristic_weight: f64,
    /// Print one progress line per iteration to stderr.
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
            plane_cut_cost: 3.0,
            heuristic_weight: 1.0,
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
    Partial { unconnected_terminals: usize },
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Node {
    layer: u8,
    cell: u32,
}

const NO_TERMINAL: u16 = u16::MAX;
/// A branch end that lands on one of the net's copper pours.
const PLANE_TERMINAL: u16 = u16::MAX - 1;
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
    /// Per layer: the rule class describing the pour there.
    plane_class: Vec<usize>,
    /// When set, only these pour nodes (the main piece) are valid targets.
    plane_target: Vec<Vec<bool>>,
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

fn find(parent: &mut [usize], mut element: usize) -> usize {
    while parent[element] != element {
        parent[element] = parent[parent[element]];
        element = parent[element];
    }
    element
}

fn disc(radius: f64, pitch: f64) -> Vec<(i32, i32)> {
    let reach = (radius / pitch).ceil() as i32;
    let mut offsets = Vec::new();
    for dy in -reach..=reach {
        for dx in -reach..=reach {
            if ((dx * dx + dy * dy) as f64).sqrt() * pitch < radius {
                offsets.push((dx, dy));
            }
        }
    }
    offsets
}

/// Per-thread search state: A* arrays, generation marks and counters.
#[derive(Clone, Default)]
pub struct Scratch {
    cost: Vec<f32>,
    seen: Vec<u32>,
    closed: Vec<u32>,
    parent: Vec<u8>,
    target_mark: Vec<u32>,
    target_terminal: Vec<u16>,
    tree_mark: Vec<u32>,
    tree_terminal: Vec<u16>,
    own_via_near: Vec<u32>,
    generation: u32,
    corridor: Option<Vec<bool>>,
    expansions: u64,
    searches: u64,
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
            generation: 0,
            corridor: None,
            expansions: 0,
            searches: 0,
        }
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
        if !slot.as_ref().is_some_and(|scratch| scratch.fits(states, cells)) {
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
    occupancy: Vec<Vec<u16>>,
    stamp_mark: Vec<Vec<u32>>,
    stamp_generation: u32,
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

    scratch: Scratch,
    search_seconds: f64,
    stamp_seconds: f64,
    iterations: usize,
}

impl Router {
    pub fn new(board: &Board, config: &Config) -> Self {
        let grid = Grid::choose(board, &config.pitches);
        let cells = grid.cells();
        let layers = board.layer_count;
        let statics: Vec<_> = (0..board.classes.len())
            .map(|class| StaticMaps::build(board, &grid, class))
            .collect();
        let maps = board.classes.len() * (layers + 1);
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
                        let with_margin = |radius: f64| {
                            // Both polylines contribute one chord sagitta.
                            radius + SAFETY + step * step / (4.0 * radius)
                        };
                        let snap = grid.pitch * 0.75;
                        Stamps {
                            stub_to_trace: disc(
                                with_margin(
                                    own.trace_width / 2.0 + other.trace_width / 2.0 + clearance,
                                ) + snap,
                                grid.pitch,
                            ),
                            stub_to_via: disc(
                                with_margin(
                                    own.trace_width / 2.0 + other.via_diameter / 2.0 + clearance,
                                ) + snap,
                                grid.pitch,
                            ),
                            trace_to_trace: disc(
                                with_margin(
                                    own.trace_width / 2.0 + other.trace_width / 2.0 + clearance,
                                ),
                                grid.pitch,
                            ),
                            trace_to_via: disc(
                                with_margin(
                                    own.trace_width / 2.0 + other.via_diameter / 2.0 + clearance,
                                ),
                                grid.pitch,
                            ),
                            via_to_trace: disc(
                                with_margin(
                                    own.via_diameter / 2.0 + other.trace_width / 2.0 + clearance,
                                ),
                                grid.pitch,
                            ),
                            via_to_via: disc(
                                (own.via_diameter / 2.0 + other.via_diameter / 2.0 + clearance)
                                    .max(
                                        own.via_drill / 2.0
                                            + other.via_drill / 2.0
                                            + board.hole_to_hole,
                                    )
                                    + SAFETY,
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
            stamp_mark: vec![vec![0; cells]; maps],
            stamp_generation: 0,
            history: vec![vec![0.0; cells]; layers + 1],
            stamps,
            nets: Vec::new(),
            direction_cost,
            cleanup: false,
            tiles_x,
            tiles_y,
            tile_claimed: vec![vec![0; tiles_x * tiles_y]; maps],
            tile_routable,
            tile_history: vec![vec![0.0; tiles_x * tiles_y]; layers + 1],
            guard: vec![Vec::new(); layers],
            covered: vec![Vec::new(); layers],
            tile_covered: vec![vec![0.0; tiles_x * tiles_y]; layers],
            layer_cut: vec![1.0; layers],
            scratch: Scratch::new(states, cells),
            search_seconds: 0.0,
            stamp_seconds: 0.0,
            iterations: 0,
            grid,
        };
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
            let axis = if extent[0] >= extent[1] { [1.0, 0.0] } else { [0.0, 1.0] };
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
            for (_, cell) in candidates.into_iter().take(600) {
                let center = self.grid.center_of(cell);
                let mut found: Option<(f64, Vec<crate::geometry::Point>)> = None;
                'widths: for width in widths.iter().copied() {
                    if self.stub_is_clear(net, layer, terminal.anchor, center, width) {
                        found = Some((width, vec![terminal.anchor, center]));
                        break;
                    }
                    for exit in &exits {
                        if crate::geometry::distance(*exit, center) > 1.5 * self.grid.pitch + 0.15 {
                            continue;
                        }
                        if self.stub_is_clear(net, layer, terminal.anchor, *exit, width)
                            && self.stub_is_clear(net, layer, *exit, center, width)
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
        }
    }

    /// Whether the stub behind escape node `node` is free of other nets.
    fn escape_is_free(&self, net_state: &NetState, net: NetId, node: Node) -> bool {
        let Some((cells, _, _)) = net_state.escapes.get(&node) else {
            return true;
        };
        let map = self.map_index(self.board.nets[net as usize].class, node.layer as usize);
        let nx = self.grid.nx as i64;
        cells.iter().all(|cell| {
            (-1..=1).all(|dy| {
                (-1..=1).all(|dx| {
                    let target = *cell as i64 + dy * nx + dx;
                    target < 0
                        || target as usize >= self.grid.cells()
                        || self.occupancy[map][target as usize] == 0
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
                    per_tile[(cell / self.grid.nx / TILE) * self.tiles_x + (cell % self.grid.nx) / TILE] += 1;
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
                    self.covered[layer].iter().filter(|owner| **owner != 0).count() as f32
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
                let Some((x0, y0, x1, y1)) =
                    self.grid.node_range(shape.aabb().inflated(plane.thermal_reach))
                else {
                    continue;
                };
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        if shape.distance_to_point(self.grid.center(x, y)) < plane.thermal_reach {
                            guard[plane.layer][self.grid.index(x, y)] = crate::grid::owner(plane.net);
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
        let mut escapes: HashMap<Node, (Vec<u32>, f64, Vec<crate::geometry::Point>)> = HashMap::new();
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
        let mut plane: Vec<Vec<bool>> = Vec::new();
        let mut plane_class = vec![description.class; self.board.layer_count];
        for pour in self.board.planes.iter().filter(|pour| pour.net == net && pour.connect) {
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
                    eprintln!(
                        "dead pad {} of {} at {:?} layers {:#b}",
                        description.terminals[index].label,
                        description.name,
                        description.terminals[index].anchor,
                        description.terminals[index].layers
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
            ..Default::default()
        }
    }

    fn unstamp(&mut self, net: NetId) {
        let state = &mut self.nets[net as usize];
        for (map, cell) in state.stamped.drain(..) {
            self.occupancy[map as usize][cell as usize] -= 1;
            if self.occupancy[map as usize][cell as usize] == 0 {
                let cell = cell as usize;
                let tile = (cell / self.grid.nx / TILE) * self.tiles_x + (cell % self.grid.nx) / TILE;
                self.tile_claimed[map as usize][tile] -= 1;
            }
        }
    }

    fn rip_up(&mut self, net: NetId) {
        self.unstamp(net);
        let state = &mut self.nets[net as usize];
        state.branches.clear();
        state.connected.fill(false);
        state.complete = false;
    }

    /// Removes only the branches of `net` that are in conflict, plus any
    /// branch left dangling at a junction on a removed branch.
    fn rip_up_conflicted(&mut self, net: NetId) {
        let class = self.board.nets[net as usize].class;
        let layers = self.board.layer_count;
        let branches = std::mem::take(&mut self.nets[net as usize].branches);
        let mut keep: Vec<bool> = branches
            .iter()
            .map(|branch| {
                !branch.nodes.iter().enumerate().any(|(index, node)| {
                    let is_via = index > 0
                        && branch.nodes[index - 1].cell == node.cell
                        && branch.nodes[index - 1].layer != node.layer;
                    self.occupancy[self.map_index(class, node.layer as usize)]
                        [node.cell as usize]
                        > 1
                        || (is_via
                            && self.occupancy[self.map_index(class, layers)][node.cell as usize]
                                > 1)
                })
            })
            .collect();
        loop {
            let hosts = junction_hosts(&branches, &keep);
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
        self.unstamp(net);
        let mut keep = keep.into_iter();
        let mut branches = branches;
        branches.retain(|_| keep.next().unwrap());
        let state = &mut self.nets[net as usize];
        state.branches = branches;
        state.connected.fill(false);
        state.complete = false;
    }

    fn stamp(&mut self, net: NetId) {
        let class = self.board.nets[net as usize].class;
        let layers = self.board.layer_count;
        self.stamp_generation += 1;
        let generation = self.stamp_generation;
        let mut stamped = Vec::new();
        let nx = self.grid.nx as i64;
        let ny = self.grid.ny as i64;
        let branches = std::mem::take(&mut self.nets[net as usize].branches);
        let escapes = std::mem::take(&mut self.nets[net as usize].escapes);
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
                    if self.stamp_mark[map][target] != generation {
                        self.stamp_mark[map][target] = generation;
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
            for branch in &branches {
                for end in [branch.nodes[0], *branch.nodes.last().unwrap()] {
                    if let Some((cells, _, _)) = escapes.get(&end) {
                        for cell in cells {
                            apply(base + end.layer as usize, *cell, &stamps.stub_to_trace);
                            apply(base + layers, *cell, &stamps.stub_to_via);
                        }
                    }
                }
                for (index, node) in branch.nodes.iter().enumerate() {
                    apply(base + node.layer as usize, node.cell, &stamps.trace_to_trace);
                    apply(base + layers, node.cell, &stamps.trace_to_via);
                    let is_via = index > 0
                        && branch.nodes[index - 1].cell == node.cell
                        && branch.nodes[index - 1].layer != node.layer;
                    if is_via {
                        for layer in 0..layers {
                            apply(base + layer, node.cell, &stamps.via_to_trace);
                        }
                        apply(base + layers, node.cell, &stamps.via_to_via);
                    }
                }
            }
        }
        let state = &mut self.nets[net as usize];
        state.branches = branches;
        state.escapes = escapes;
        state.stamped = stamped;
    }

    /// Nodes of `net` that lie inside another net's clearance zone.
    fn conflicts(&self, net: NetId) -> Vec<(usize, u32)> {
        let class = self.board.nets[net as usize].class;
        let layers = self.board.layer_count;
        let mut result = Vec::new();
        for branch in &self.nets[net as usize].branches {
            for end in [branch.nodes[0], *branch.nodes.last().unwrap()] {
                if let Some((cells, _, _)) = self.nets[net as usize].escapes.get(&end) {
                    let map = self.map_index(class, end.layer as usize);
                    for cell in cells {
                        if self.occupancy[map][*cell as usize] > 1 {
                            result.push((end.layer as usize, *cell));
                        }
                    }
                }
            }
            for (index, node) in branch.nodes.iter().enumerate() {
                let map = self.map_index(class, node.layer as usize);
                if self.occupancy[map][node.cell as usize] > 1 {
                    result.push((node.layer as usize, node.cell));
                }
                let is_via = index > 0
                    && branch.nodes[index - 1].cell == node.cell
                    && branch.nodes[index - 1].layer != node.layer;
                if is_via && self.occupancy[self.map_index(class, layers)][node.cell as usize] > 1
                {
                    result.push((layers, node.cell));
                }
            }
        }
        result
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

        scratch.generation += 1;
        let tree_generation = scratch.generation;
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
            self.connect_to_plane(scratch, net, net_state, &retained, &component, plane_element, present, hard);
            return;
        }
        let first_live = (0..terminal_count)
            .find(|terminal| !net_state.terminal_nodes[*terminal].is_empty())
            .unwrap_or(0);
        absorb(scratch, net_state, &mut tree, &mut in_tree, component[first_live]);

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
            if self.config.corridors && !self.cleanup {
                let margin = (1 + reroutes / 3).min(10);
                for margin in [margin, margin + 3] {
                    let Some(corridor) = self.plan_corridor(net, net_state, &tree, &targets, hard, margin)
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
                self.search(scratch, net, net_state, &tree, &targets, points.as_deref(), false, present, hard, windowed)
            });
            let path = path
                .or_else(|| {
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
            absorb(scratch, net_state, &mut tree, &mut in_tree, component[reached]);
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
            scratch.generation += 1;
            let tree_generation = scratch.generation;
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
            let Some(path) = self.search(scratch, net, net_state, &sources, &targets, None, true, present, hard, full)
            else {
                if std::env::var_os("PCB_ROUTER_DEBUG").is_some() {
                    let restricted = !net_state.plane_target.is_empty();
                    let plane_nodes: usize = if restricted {
                        net_state.plane_target.iter().map(|m| m.iter().filter(|b| **b).count()).sum()
                    } else {
                        net_state.plane.iter().map(|m| m.iter().filter(|b| **b).count()).sum()
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
    ) -> Option<Vec<bool>> {
        let class = self.board.nets[net as usize].class;
        let layers = self.board.layer_count;
        let tiles = self.tiles_x * self.tiles_y;
        let tile_of = |cell: u32| {
            (cell as usize / self.grid.nx / TILE) * self.tiles_x + (cell as usize % self.grid.nx) / TILE
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
            let claimed = self.tile_claimed[self.map_index(class, layer)][tile];
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
                    relax(other * tiles + tile, self.config.via_cost as f32 * factor, &mut heap);
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
        scratch.generation += 1;
        let generation = scratch.generation;
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
            if hard && !self.escape_is_free(net_state, net, *node) {
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
                && (self.occupancy[self.map_index(class, node.layer as usize)]
                    [node.cell as usize]
                    > 0
                    || !self.escape_is_free(net_state, net, *node))
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
                    && !(hard && self.occupancy[pour_map][state % cells] > 0)
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
            let blocked_edges = if statics.edge_owner[layer][cell] == own {
                0
            } else {
                statics.edge_block[layer][cell]
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
                let allowed = statics.trace[layer][target_cell];
                if allowed != crate::grid::FREE && allowed != own {
                    continue;
                }
                let target_state = layer * cells + target_cell;
                if scratch.closed[target_state] == generation {
                    continue;
                }
                let occupied = self.occupancy[trace_base + layer][target_cell] as f32;
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
                        step *= 4.0;
                    }
                }
                if !self.covered[layer].is_empty() {
                    let pour = self.covered[layer][target_cell];
                    if pour != 0 && pour != own {
                        step *= self.layer_cut[layer];
                    }
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

            let via_occupied = self.occupancy[via_map][cell] as f32;
            if layers > 1
                && !statics.via_blocked[cell]
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
                let step = via_cost * cut * (1.0 + history) * (1.0 + present * occupied);
                for target_layer in 0..layers {
                    if target_layer == layer {
                        continue;
                    }
                    let allowed = statics.trace[target_layer][cell];
                    if allowed != crate::grid::FREE && allowed != own {
                        continue;
                    }
                    if hard && self.occupancy[trace_base + target_layer][cell] > 0 {
                        continue;
                    }
                    let target_state = target_layer * cells + cell;
                    if scratch.closed[target_state] == generation {
                        continue;
                    }
                    let total = here + step;
                    if scratch.seen[target_state] != generation || total < scratch.cost[target_state] {
                        scratch.seen[target_state] = generation;
                        scratch.cost[target_state] = total;
                        scratch.parent[target_state] = 8 + layer as u8;
                        let estimate = total + heuristic(target_layer, x as i32, y as i32);
                        heap.push(Reverse((estimate.to_bits(), target_state as u32)));
                    }
                }
            }
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
        let class = self.board.nets[net as usize].class;
        let statics = &self.statics[class];
        let nx = self.grid.nx as i64;
        for (index, node) in branch.nodes.iter().enumerate() {
            if !statics.trace_allowed(node.layer as usize, node.cell as usize, net) {
                return false;
            }
            if index == 0 {
                continue;
            }
            let previous = branch.nodes[index - 1];
            if previous.cell == node.cell {
                if statics.via_blocked[node.cell as usize] {
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
            if !statics.edge_allowed(previous.layer as usize, previous.cell as usize, direction, net)
            {
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
        if board.nets.len() != self.board.nets.len()
            || board.classes != self.board.classes
            || board.layer_count != self.board.layer_count
            || board
                .nets
                .iter()
                .zip(&self.board.nets)
                .any(|(new, old)| new.name != old.name || new.terminals.len() != old.terminals.len())
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
                        self.tile_routable[class * layers + layer]
                            [(cell / self.grid.nx / TILE) * self.tiles_x + (cell % self.grid.nx) / TILE] += 1;
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
                && old.branches.iter().all(|branch| self.branch_is_legal(id, branch));
            if legal && !old.branches.is_empty() {
                let state = &mut self.nets[net];
                state.branches = old.branches;
                state.connected = old.connected;
                state.complete = old.complete;
                state.reroutes = old.reroutes;
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
        let pending: Vec<NetId> = order
            .iter()
            .copied()
            .filter(|net| {
                let state = &self.nets[*net as usize];
                !state.complete || !self.conflicts(*net).is_empty()
            })
            .collect();
        self.negotiate(&order, pending);
        let cleanup = self.config.cleanup_passes;
        if !polish {
            self.config.cleanup_passes = 0;
        }
        let result = self.finish(&order);
        self.config.cleanup_passes = cleanup;
        result
    }

    /// Routes one net with the router's own scratch (sequential callers).
    fn route_net_seq(&mut self, net: NetId, present: f32, hard: bool, window_growth: f64) {
        let mut scratch = std::mem::take(&mut self.scratch);
        let mut net_state = std::mem::take(&mut self.nets[net as usize]);
        self.route_net(&mut scratch, net, &mut net_state, present, hard, window_growth);
        self.nets[net as usize] = net_state;
        self.scratch = scratch;
    }

    /// Groups `pending` into batches of nets whose search windows (with a
    /// tile of margin) do not overlap, keeping the given order inside each
    /// batch. Pour nets and nets without a bounded window get a batch each.
    fn batches(&self, pending: &[NetId], growth: f64) -> Vec<Vec<NetId>> {
        if self.config.jacobi_batch > 0 {
            return pending
                .chunks(self.config.jacobi_batch)
                .map(<[NetId]>::to_vec)
                .collect();
        }
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
                batches.iter().position(|(members, rects)| {
                    members.len() < 64 && !rects.iter().any(overlaps)
                })
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
        batches.into_iter().map(|(members, _)| members).collect()
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
        order.sort_by_key(|net| {
            (
                self.nets[*net as usize].plane.is_empty(),
                self.board.nets[*net as usize].terminals.len() > 8,
                span(self, *net),
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
        let order = self.routing_order();
        self.negotiate(&order, order.clone());
        self.finish(&order)
    }

    /// Negotiated congestion over `pending` (which grows to whatever those
    /// nets conflict with) until the board is conflict free or stalls.
    fn negotiate(&mut self, order: &[NetId], mut pending: Vec<NetId>) {
        let started = std::time::Instant::now();
        let mut present = self.config.present_factor as f32;
        let mut best_conflicted = usize::MAX;
        let mut stalled = 0;
        for iteration in 0..self.config.max_iterations {
            self.iterations += 1;
            let growth = 1.0 + iteration as f64 / 6.0;
            let batches = self.batches(&pending, growth);
            if std::env::var_os("PCB_ROUTER_DEBUG").is_some() {
                let sizes: Vec<usize> = batches.iter().map(Vec::len).collect();
                eprintln!("  {} nets in {} batches: {:?}", pending.len(), batches.len(), sizes);
            }
            for batch in batches {
                let search_started = std::time::Instant::now();
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
                    let routed: Vec<(NetId, NetState, u64, u64)> = batch
                        .par_iter()
                        .map(|net| {
                            with_thread_scratch(states, cells, |scratch| {
                                let (searches, expansions) = (scratch.searches, scratch.expansions);
                                let mut net_state = this.nets[*net as usize].clone();
                                this.route_net(scratch, *net, &mut net_state, present, false, growth);
                                (
                                    *net,
                                    net_state,
                                    scratch.searches - searches,
                                    scratch.expansions - expansions,
                                )
                            })
                        })
                        .collect();
                    for (net, net_state, searches, expansions) in routed {
                        self.nets[net as usize] = net_state;
                        self.scratch.searches += searches;
                        self.scratch.expansions += expansions;
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
            if self.config.verbose {
                eprintln!(
                    "iteration {iteration}: rerouted {}, conflicted {}, present {present:.2}, {:.2}s (search {:.2}s, stamp {:.2}s, {} searches, {}M expansions)",
                    pending.len(),
                    conflicted.len(),
                    started.elapsed().as_secs_f64(),
                    self.search_seconds,
                    self.stamp_seconds,
                    self.scratch.searches,
                    self.scratch.expansions / 1_000_000
                );
            }
            if conflicted.is_empty() {
                break;
            }
            if conflicted.len() < best_conflicted {
                best_conflicted = conflicted.len();
                stalled = 0;
            } else {
                stalled += 1;
            }
            pending = conflicted;
            present = (present * self.config.present_growth as f32).min(self.config.present_cap as f32);
            if stalled > 25 {
                break;
            }
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

    /// Renegotiates the nets that have vias with a higher via cost, from the
    /// converged state. A round is kept only if nothing opens and the via
    /// count drops.
    fn reduce_vias(&mut self, order: &[NetId]) {
        for round in 0..self.config.via_reduction_rounds {
            let before = self.quality();
            let pending: Vec<NetId> = order
                .iter()
                .copied()
                .filter(|net| {
                    self.nets[*net as usize]
                        .branches
                        .iter()
                        .any(|branch| branch.nodes.windows(2).any(|pair| pair[0].cell == pair[1].cell))
                })
                .collect();
            if pending.is_empty() {
                break;
            }
            let snapshot = self.clone();
            let via_cost = self.config.via_cost;
            let weight = self.config.heuristic_weight;
            self.config.via_cost = via_cost * self.config.via_reduction_factor.powi(round as i32 + 1);
            self.config.heuristic_weight = weight.max(self.config.via_reduction_weight);
            for net in &pending {
                self.rip_up(*net);
            }
            self.negotiate(order, pending.clone());
            self.resolve_remaining(order);
            self.config.heuristic_weight = weight;
            // Only the nets this round touched can have got worse.
            let touched: Vec<NetId> = order
                .iter()
                .copied()
                .filter(|net| self.nets[*net as usize].reroutes > snapshot.nets[*net as usize].reroutes || pending.contains(net))
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
        let improved = self.clean_up(order);
        self.reduce_vias(order);
        let plane_nets: Vec<NetId> = order
            .iter()
            .copied()
            .filter(|net| !self.nets[*net as usize].plane.is_empty())
            .collect();
        for net in plane_nets {
            let stitches = self.stitch_pours(net, order);
            if self.config.verbose {
                eprintln!(
                    "pour {}: {stitches} stitching vias, complete {}",
                    self.board.nets[net as usize].name,
                    self.nets[net as usize].complete
                );
            }
        }
        if self.config.verbose {
            eprintln!(
                "cleanup: {improved} improvements, {:.2}s",
                cleanup_started.elapsed().as_secs_f64()
            );
        }

        let status = (0..self.board.nets.len())
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
            .collect();
        let (mut routes, mut stubs): (Vec<NetRoute>, Vec<Vec<usize>>) =
            (0..self.board.nets.len() as NetId)
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
        RoutingResult {
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
            (net, state.branches.clone(), state.connected.clone(), state.complete)
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
                } else if terminal != PLANE_TERMINAL {
                    union(&mut parent, branch_base + index, terminal_base + terminal as usize);
                }
            }
        }
        let mut votes: HashMap<usize, usize> = HashMap::new();
        for terminal in 0..terminal_count {
            *votes.entry(find(&mut parent, terminal_base + terminal)).or_default() += 1;
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
                        .filter(|index| find(&mut parent, terminal_base + terminal_count + index) == root)
                        .count();
                    let pieces: Vec<usize> = (1..=pours.pieces).filter(|piece| find(&mut parent, *piece) == root).collect();
                    let sizes: Vec<usize> = pieces.iter().take(4).map(|piece| {
                        pours.label.iter().map(|layer| layer.iter().filter(|l| **l == *piece as u32).count()).sum()
                    }).collect();
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
            // it to the main piece. Islands without room stay for rerouting.
            let mut best: Option<(f64, usize, usize, usize)> = None;
            let mut tried: Vec<usize> = Vec::new();
            for terminal in &islands {
                let island = find(&mut parent, terminal_base + terminal);
                if tried.contains(&island) || layers < 2 {
                    continue;
                }
                tried.push(island);
                let anchor = self.board.nets[net as usize].terminals[*terminal].anchor;
                for layer in 0..layers {
                    for (cell, piece) in pours.label[layer].iter().enumerate() {
                        if *piece == 0 || find(&mut parent, *piece as usize) != island {
                            continue;
                        }
                        if self.statics[class_index].via_blocked[cell]
                            || self.occupancy[via_map][cell] as usize
                                != own.contains(&(via_map as u32, cell as u32)) as usize
                            || !pours.solid_around(&self.grid, layer, cell, via_reach)
                        {
                            continue;
                        }
                        for other in (0..layers).filter(|other| *other != layer) {
                            let target = pours.label[other].get(cell).copied().unwrap_or(0);
                            if target == 0
                                || find(&mut parent, target as usize) == island
                                || !pours.solid_around(&self.grid, other, cell, via_reach)
                            {
                                continue;
                            }
                            // Any other piece helps (islands merge step by
                            // step); the main piece is worth a detour.
                            let bonus = if find(&mut parent, target as usize) == main {
                                0.0
                            } else {
                                5.0
                            };
                            let distance = bonus
                                + crate::geometry::distance(anchor, self.grid.center_of(cell));
                            if best.is_none_or(|(best, ..)| distance < best) {
                                best = Some((distance, layer, other, cell));
                            }
                        }
                    }
                }
                if best.is_some() {
                    break;
                }
            }
            if let Some((_, layer, other, cell)) = best {
                self.unstamp(net);
                self.nets[net as usize].branches.push(Branch {
                    nodes: vec![
                        Node {
                            layer: layer as u8,
                            cell: cell as u32,
                        },
                        Node {
                            layer: other as u8,
                            cell: cell as u32,
                        },
                    ],
                    start_terminal: PLANE_TERMINAL,
                    end_terminal: PLANE_TERMINAL,
                });
                self.stamp(net);
                stitches += 1;
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
        for _ in 0..self.config.cleanup_passes {
            let mut improved = false;
            for net in order {
                if !self.nets[*net as usize].complete {
                    continue;
                }
                let before = self.geometric_cost(*net);
                let saved = self.nets[*net as usize].branches.clone();
                self.rip_up(*net);
                self.route_net_seq(*net, 0.0, true, 1.0);
                let state = &self.nets[*net as usize];
                if state.complete && self.geometric_cost(*net) < before - 1.0e-6 {
                    improved = true;
                    improvements += 1;
                } else {
                    let state = &mut self.nets[*net as usize];
                    state.branches = saved;
                    state.connected.fill(true);
                    state.complete = true;
                }
                self.stamp(*net);
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
        for net in &removed {
            self.route_net_seq(*net, 0.0, true, 4.0);
            self.stamp(*net);
        }
        for net in removed {
            if !self.nets[net as usize].complete && self.force_connect(net, order) {
                continue;
            }
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
                let end = (branch.end_terminal == NO_TERMINAL)
                    .then(|| *branch.nodes.last().unwrap());
                start.into_iter().chain(end)
            })
            .collect();
        for branch in &state.branches {
            let first = branch.nodes[0];
            let last = *branch.nodes.last().unwrap();
            let mut stub = |node: Node, terminal: u16| {
                if terminal == NO_TERMINAL || terminal == PLANE_TERMINAL {
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
                } else if crate::geometry::distance(anchor, center) > 1.0e-7
                    && self.stub_is_clear(net, node.layer as usize, anchor, center, rules.trace_width)
                {
                    stubs.push(route.segments.len());
                    route.segments.push(Segment {
                        layer: node.layer as usize,
                        start: anchor,
                        end: center,
                        width: rules.trace_width,
                    });
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
                self.emit_run(&branch.nodes[run_start..index], &junctions, &rules, &mut route);
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
        let mut start = 0;
        for index in 1..nodes.len() {
            let turning = index + 1 < nodes.len()
                && delta(nodes[index - 1], nodes[index]) != delta(nodes[index], nodes[index + 1]);
            let last = index + 1 == nodes.len();
            if turning || last || junctions.contains(&nodes[index]) {
                route.segments.push(Segment {
                    layer: nodes[start].layer as usize,
                    start: self.grid.center_of(nodes[start].cell as usize),
                    end: self.grid.center_of(nodes[index].cell as usize),
                    width: rules.trace_width,
                });
                start = index;
            }
        }
    }
}

pub fn route(board: &Board, config: &Config) -> RoutingResult {
    Router::new(board, config).run()
}
