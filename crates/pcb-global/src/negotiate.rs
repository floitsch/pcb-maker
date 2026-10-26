// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Nets on the tile graph, negotiated PathFinder style.
//!
//! A state is a tile on a layer (`layer * tiles + tile`). A step to a
//! neighbouring tile crosses a boundary and costs the tile's side times
//! the layer's direction factor, times the pour factor of the tile entered
//! (another net's pour), times `(1 + history) * (1 + present * over)`,
//! where `over` is how far the boundary would be over its capacity with
//! this net. A via (a layer change inside a tile) costs `via_cost_mm`,
//! priced alike on the tile's via sites. A boundary without capacity is
//! closed, except next to the net's own terminals (pads crowd their own
//! tiles: a net must be able to leave them, overflow and all).

use crate::GlobalConfig;
use crate::capacity::{Capacity, Frame};
use pcb_router::Board;
use serde::Serialize;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// One net's global route.
#[derive(Clone, Debug, Default, Serialize)]
pub struct NetRoute {
    /// The net (index into `Board::nets`).
    pub net: usize,
    /// States (`layer * tiles + tile`) of the route and its terminals.
    pub states: Vec<u32>,
    /// Boundaries crossed (`direction * layers * tiles + layer * tiles +
    /// tile`).
    pub crossings: Vec<u32>,
    /// Tiles of its vias.
    pub vias: Vec<u32>,
    /// Every terminal reached.
    pub routed: bool,
    /// The terminals' rectilinear spanning tree in tile steps (0 for a
    /// pour net).
    pub lower_bound: u32,
    /// Connected through its pour.
    pub pour: bool,
}

/// The routability measures of one global route.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Score {
    /// Tracks over capacity, summed over every boundary and layer.
    pub total_overflow: f64,
    /// The most tracks over capacity on one boundary.
    pub max_overflow: f64,
    /// Boundaries (per layer) over capacity.
    pub overflowed_edges: usize,
    /// Vias over the tiles' via sites, summed.
    pub via_overflow: f64,
    /// `total_overflow` per layer.
    pub overflow_per_layer: Vec<f64>,
    /// Tracks over boundaries per layer (the usage summed), and the
    /// capacity summed.
    pub usage_per_layer: Vec<f64>,
    pub capacity_per_layer: Vec<f64>,
    /// Usage above `alpha` times capacity summed over the boundaries, per
    /// `SOFT_ALPHAS` (a continuous measure where nothing overflows).
    pub soft_overflow: Vec<f64>,
    /// Mean utilisation (usage over capacity) of the most used boundaries
    /// with capacity, per `ACE_SHARES` of them (ISPD's ACE).
    pub ace: Vec<f64>,
    /// Nets routed on the graph (two or more terminals, or a pour).
    pub nets: usize,
    /// Nets with a terminal the graph could not reach.
    pub unrouted: usize,
    /// Nets crossing an overflowing boundary or via site.
    pub overflowed_nets: usize,
    /// Signal nets (not pour nets) whose route detours.
    pub detoured: usize,
    /// `detoured` over the signal nets.
    pub detour_fraction: f64,
    /// Boundaries crossed times the tile side.
    pub wirelength_mm: f64,
    pub vias: usize,
    pub iterations: usize,
    /// The iteration whose state is kept (the least overflow).
    pub best_iteration: usize,
}

/// The utilisation levels of `Score::soft_overflow`.
pub const SOFT_ALPHAS: [f64; 2] = [0.5, 0.75];
/// The shares of boundaries `Score::ace` averages over.
pub const ACE_SHARES: [f64; 3] = [0.005, 0.02, 0.05];

/// The global route of a board: the grid, its capacities, the usage of
/// the kept iteration, every net's route and the score.
#[derive(Clone, Debug)]
pub struct GlobalRoute {
    pub frame: Frame,
    pub capacity: Capacity,
    /// Tracks (in reference-class tracks) over each boundary, as
    /// `Capacity::edges`.
    pub usage: [Vec<f32>; 2],
    pub via_usage: Vec<f32>,
    pub nets: Vec<NetRoute>,
    pub score: Score,
    pub capacity_seconds: f64,
    pub seconds: f64,
    /// States popped by all searches (the negotiation's work).
    pub pops: u64,
    /// The history on the boundaries and via sites and the present price
    /// where the negotiation ended (for `Incremental`).
    pub history: [Vec<f32>; 2],
    pub via_history: Vec<f32>,
    pub present: f32,
}

impl GlobalRoute {
    /// Tracks over capacity per boundary: `[direction][layer * tiles +
    /// tile]`, 0 where within.
    pub fn overflow(&self) -> [Vec<f32>; 2] {
        [0, 1].map(|direction| {
            self.usage[direction]
                .iter()
                .zip(&self.capacity.edges[direction])
                .map(|(used, capacity)| (used - capacity).max(0.0))
                .collect()
        })
    }

    /// The tiles `net`'s route crossed (on any layer), sorted.
    pub fn corridor(&self, net: usize) -> Vec<u32> {
        let tiles = self.frame.tiles() as u32;
        let mut corridor: Vec<u32> = self
            .nets
            .iter()
            .filter(|route| route.net == net)
            .flat_map(|route| route.states.iter().map(|state| state % tiles))
            .collect();
        corridor.sort_unstable();
        corridor.dedup();
        corridor
    }
}

#[derive(Clone)]
pub(crate) struct Plan {
    pub(crate) net: usize,
    /// Per terminal its states.
    terminals: Vec<Vec<u32>>,
    pour: bool,
    pub(crate) demand: f32,
    lower_bound: u32,
    /// Terminals in Prim order of their spanning tree, each with its
    /// parent there (the first is its own).
    order: Vec<(usize, usize)>,
    /// Tiles the searches stay in first: the terminals' box grown by
    /// `window_tiles` or a quarter of its larger side.
    window: [usize; 4],
}

const SOURCE: u32 = u32::MAX;

pub(crate) struct Graph<'a> {
    frame: &'a Frame,
    capacity: &'a Capacity,
    config: &'a GlobalConfig,
    tiles: usize,
    layers: usize,
    pub(crate) usage: [Vec<f32>; 2],
    pub(crate) history: [Vec<f32>; 2],
    pub(crate) via_usage: Vec<f32>,
    pub(crate) via_history: Vec<f32>,
    /// Per layer: (along x, along y) cost factors.
    direction: Vec<[f32; 2]>,
    /// Per layer: the pour factor at full cover.
    layer_cut: Vec<f32>,
    // Search scratch, stamped per search by `generation`.
    generation: u32,
    seen: Vec<u32>,
    cost: Vec<f32>,
    from: Vec<u32>,
    // Per net, stamped by `net_generation`: states in the net's tree, its
    // terminals' states (and which terminal), its terminals' tiles.
    net_generation: u32,
    tree_mark: Vec<u32>,
    terminal_mark: Vec<u32>,
    terminal_of: Vec<u32>,
    endpoint: Vec<u32>,
    heap: BinaryHeap<Reverse<(u32, u32)>>,
    /// States popped by all searches (the work).
    pub(crate) pops: u64,
    /// Terminals (net, index) a search over the whole board did not
    /// connect: closed boundaries never open, so they are not searched
    /// again.
    dead: std::collections::HashSet<(usize, usize)>,
}

impl<'a> Graph<'a> {
    pub(crate) fn new(frame: &'a Frame, capacity: &'a Capacity, config: &'a GlobalConfig) -> Self {
        let tiles = frame.tiles();
        let layers = frame.layers;
        let states = tiles * layers;
        let against = config.against_direction as f32;
        let direction = (0..layers)
            .map(|layer| {
                if layer % 2 == 0 {
                    [1.0, against]
                } else {
                    [against, 1.0]
                }
            })
            .collect();
        // The router's plane cut cost: layers by their pour share relative
        // to the least poured layer.
        let inside = capacity
            .inside
            .iter()
            .filter(|inside| **inside)
            .count()
            .max(1) as f32;
        let share: Vec<f32> = (0..layers)
            .map(|layer| {
                (0..tiles)
                    .filter(|tile| capacity.inside[*tile])
                    .map(|tile| capacity.pour_share[layer * tiles + tile])
                    .sum::<f32>()
                    / inside
            })
            .collect();
        let least = share.iter().copied().fold(1.0f32, f32::min);
        let layer_cut = share
            .iter()
            .map(|value| {
                let relative = (value - least).max(0.0) / (1.0 - least).max(1.0e-3);
                1.0 + (config.plane_cut_cost as f32 - 1.0) * relative
            })
            .collect();
        Self {
            frame,
            capacity,
            config,
            tiles,
            layers,
            usage: [vec![0.0; states], vec![0.0; states]],
            history: [vec![0.0; states], vec![0.0; states]],
            via_usage: vec![0.0; tiles],
            via_history: vec![0.0; tiles],
            direction,
            layer_cut,
            generation: 0,
            seen: vec![0; states],
            cost: vec![0.0; states],
            from: vec![SOURCE; states],
            net_generation: 0,
            tree_mark: vec![0; states],
            terminal_mark: vec![0; states],
            terminal_of: vec![0; states],
            endpoint: vec![0; tiles],
            heap: BinaryHeap::new(),
            pops: 0,
            dead: Default::default(),
        }
    }

    fn edge_id(&self, direction: usize, layer: usize, tile: usize) -> u32 {
        (direction * self.layers * self.tiles + layer * self.tiles + tile) as u32
    }

    pub(crate) fn take(&mut self, route: &NetRoute, demand: f32, sign: f32) {
        let per = self.layers * self.tiles;
        for &edge in &route.crossings {
            let edge = edge as usize;
            self.usage[edge / per][edge % per] += sign * demand;
        }
        for &tile in &route.vias {
            self.via_usage[tile as usize] += sign;
        }
    }

    /// A* from `sources` (cost 0) to the first state `target` accepts,
    /// guided towards tile `goal` (Manhattan distance times the tile side:
    /// a lower bound of any path there), within `window` (tiles, inclusive:
    /// x0, y0, x1, y1). Returns the state reached; `from` holds the path.
    fn search(
        &mut self,
        sources: &[u32],
        plan: &Plan,
        present: f32,
        goal: Option<usize>,
        window: [usize; 4],
        target: &dyn Fn(&Self, usize) -> bool,
    ) -> Option<usize> {
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        let net_generation = self.net_generation;
        let tiles = self.tiles;
        let tiles_x = self.frame.tiles_x;
        let tile_mm = self.frame.tile_mm as f32;
        let estimate = |tile: usize| -> f32 {
            match goal {
                Some(goal) => {
                    let (x, y) = ((tile % tiles_x) as i64, (tile / tiles_x) as i64);
                    let (gx, gy) = ((goal % tiles_x) as i64, (goal / tiles_x) as i64);
                    ((x - gx).abs() + (y - gy).abs()) as f32 * tile_mm
                }
                None => 0.0,
            }
        };
        self.heap.clear();
        for &state in sources {
            let state = state as usize;
            if self.seen[state] == generation {
                continue;
            }
            self.seen[state] = generation;
            self.cost[state] = 0.0;
            self.from[state] = SOURCE;
            self.heap
                .push(Reverse((estimate(state % tiles).to_bits(), state as u32)));
        }
        let via_cost = self.config.via_cost_mm as f32;
        let net_owner = plan.net as u32 + 1;
        while let Some(Reverse((bits, state))) = self.heap.pop() {
            let state = state as usize;
            let here = self.cost[state];
            if f32::from_bits(bits) > here + estimate(state % tiles) {
                continue;
            }
            self.pops += 1;
            if target(&*self, state) {
                return Some(state);
            }
            let (layer, tile) = (state / tiles, state % tiles);
            let (x, y) = (tile % tiles_x, tile / tiles_x);
            let mut moves = [(usize::MAX, 0usize, 0usize); 4];
            if x < window[2] {
                moves[0] = (tile + 1, 0, tile);
            }
            if x > window[0] {
                moves[1] = (tile - 1, 0, tile - 1);
            }
            if y < window[3] {
                moves[2] = (tile + tiles_x, 1, tile);
            }
            if y > window[1] {
                moves[3] = (tile - tiles_x, 1, tile - tiles_x);
            }
            let near = |graph: &Self, tile: usize| graph.endpoint[tile] == net_generation;
            for (next, direction, boundary) in moves {
                if next == usize::MAX {
                    continue;
                }
                let index = layer * tiles + boundary;
                let capacity = self.capacity.edges[direction][index];
                if capacity <= 0.0 && !(near(self, tile) || near(self, next)) {
                    continue;
                }
                let over = (self.usage[direction][index] + plan.demand - capacity).max(0.0);
                let mut step = tile_mm * self.direction[layer][direction];
                let entered = layer * tiles + next;
                if !plan.pour && self.capacity.pour_owner[entered] != net_owner {
                    let share = self.capacity.pour_share[entered];
                    if share > 0.0 {
                        step *= 1.0 + (self.layer_cut[layer] - 1.0) * share;
                    }
                }
                step *= (1.0 + self.history[direction][index]) * (1.0 + present * over);
                let total = here + step;
                if self.seen[entered] != generation || total < self.cost[entered] {
                    self.seen[entered] = generation;
                    self.cost[entered] = total;
                    self.from[entered] = state as u32;
                    self.heap.push(Reverse((
                        (total + estimate(next)).to_bits(),
                        entered as u32,
                    )));
                }
            }
            let sites = self.capacity.vias[tile];
            if self.layers > 1 && (sites > 0.0 || near(self, tile)) {
                let over = (self.via_usage[tile] + 1.0 - sites).max(0.0);
                let step = via_cost * (1.0 + self.via_history[tile]) * (1.0 + present * over);
                let total = here + step;
                let key = (total + estimate(tile)).to_bits();
                for other in 0..self.layers {
                    if other == layer {
                        continue;
                    }
                    let next = other * tiles + tile;
                    if self.seen[next] != generation || total < self.cost[next] {
                        self.seen[next] = generation;
                        self.cost[next] = total;
                        self.from[next] = state as u32;
                        self.heap.push(Reverse((key, next as u32)));
                    }
                }
            }
        }
        None
    }

    /// `search` within the net's window, then (when that fails) the board.
    fn search_wide(
        &mut self,
        sources: &[u32],
        plan: &Plan,
        present: f32,
        goal: Option<usize>,
        target: &dyn Fn(&Self, usize) -> bool,
    ) -> Option<usize> {
        let found = self.search(sources, plan, present, goal, plan.window, target);
        let board = [0, 0, self.frame.tiles_x - 1, self.frame.tiles_y - 1];
        if found.is_some() || plan.window == board {
            return found;
        }
        self.search(sources, plan, present, goal, board, target)
    }

    /// Walks `from` back from `state` to a source, recording the states,
    /// boundaries and vias into `route`; returns the states added.
    fn trace(&self, mut state: usize, route: &mut NetRoute) -> Vec<u32> {
        let tiles = self.tiles;
        let mut added = Vec::new();
        loop {
            added.push(state as u32);
            let previous = self.from[state];
            if previous == SOURCE {
                break;
            }
            let previous = previous as usize;
            let (layer, tile) = (state / tiles, state % tiles);
            let (previous_layer, previous_tile) = (previous / tiles, previous % tiles);
            if tile == previous_tile {
                route.vias.push(tile as u32);
            } else {
                debug_assert_eq!(layer, previous_layer);
                let direction = if tile.abs_diff(previous_tile) == 1 {
                    0
                } else {
                    1
                };
                route
                    .crossings
                    .push(self.edge_id(direction, layer, tile.min(previous_tile)));
            }
            state = previous;
        }
        added
    }

    pub(crate) fn route(&mut self, plan: &Plan, present: f32) -> NetRoute {
        let mut route = NetRoute {
            net: plan.net,
            lower_bound: plan.lower_bound,
            pour: plan.pour,
            ..NetRoute::default()
        };
        self.net_generation = self.net_generation.wrapping_add(1);
        let generation = self.net_generation;
        for (index, terminal) in plan.terminals.iter().enumerate() {
            for &state in terminal {
                self.endpoint[state as usize % self.tiles] = generation;
                self.terminal_mark[state as usize] = generation;
                self.terminal_of[state as usize] = index as u32;
            }
        }
        if plan.pour {
            let owner = plan.net as u32 + 1;
            let mut all = true;
            for (index, terminal) in plan.terminals.iter().enumerate() {
                route.states.extend_from_slice(terminal);
                if terminal
                    .iter()
                    .any(|state| self.capacity.pour_owner[*state as usize] == owner)
                {
                    continue;
                }
                if self.dead.contains(&(plan.net, index)) {
                    all = false;
                    continue;
                }
                let reached = self.search_wide(terminal, plan, present, None, &|graph, state| {
                    graph.capacity.pour_owner[state] == owner
                });
                match reached {
                    Some(state) => {
                        let added = self.trace(state, &mut route);
                        route.states.extend(added);
                    }
                    None => {
                        self.dead.insert((plan.net, index));
                        all = false;
                    }
                }
            }
            route.routed = all;
            route.states.sort_unstable();
            route.states.dedup();
            return route;
        }
        // A tree along the terminals' spanning tree (Prim order): each
        // terminal in turn searches back to the tree, guided towards its
        // spanning-tree parent; a terminal joins whole (all its layers).
        let count = plan.terminals.len();
        let mut connected = vec![false; count];
        let mut tree: Vec<u32> = Vec::new();
        let join = |graph: &mut Self, index: usize, tree: &mut Vec<u32>, connected: &mut [bool]| {
            connected[index] = true;
            for &state in &plan.terminals[index] {
                if graph.tree_mark[state as usize] != generation {
                    graph.tree_mark[state as usize] = generation;
                    tree.push(state);
                }
            }
        };
        let mut all = true;
        for &(index, parent) in &plan.order {
            if connected[index] {
                continue;
            }
            if tree.is_empty() {
                join(self, index, &mut tree, &mut connected);
                continue;
            }
            if plan.terminals[index]
                .iter()
                .any(|state| self.tree_mark[*state as usize] == generation)
            {
                join(self, index, &mut tree, &mut connected);
                continue;
            }
            if self.dead.contains(&(plan.net, index)) {
                all = false;
                continue;
            }
            let goal = plan.terminals[parent][0] as usize % self.tiles;
            let sources = plan.terminals[index].clone();
            let reached = self.search_wide(&sources, plan, present, Some(goal), &|graph, state| {
                graph.tree_mark[state] == generation
            });
            let Some(state) = reached else {
                self.dead.insert((plan.net, index));
                all = false;
                continue;
            };
            let added = self.trace(state, &mut route);
            join(self, index, &mut tree, &mut connected);
            for state in added {
                let index = state as usize;
                if self.terminal_mark[index] == generation {
                    let terminal = self.terminal_of[index] as usize;
                    if !connected[terminal] {
                        join(self, terminal, &mut tree, &mut connected);
                    }
                }
                if self.tree_mark[index] != generation {
                    self.tree_mark[index] = generation;
                    tree.push(state);
                }
            }
        }
        route.routed = all;
        tree.sort_unstable();
        route.states = tree;
        route
    }
}

/// The rectilinear spanning tree of `points` (Prim from the first), in
/// tile steps, and the points in the order Prim adds them, each with its
/// parent (the first with itself).
fn spanning_tree(points: &[(i64, i64)]) -> (u32, Vec<(usize, usize)>) {
    let n = points.len();
    if n == 0 {
        return (0, Vec::new());
    }
    let distance = |a: (i64, i64), b: (i64, i64)| (a.0 - b.0).abs() + (a.1 - b.1).abs();
    let mut in_tree = vec![false; n];
    let mut best: Vec<(i64, usize)> = points
        .iter()
        .map(|point| (distance(*point, points[0]), 0))
        .collect();
    in_tree[0] = true;
    let mut order = vec![(0, 0)];
    let mut total = 0;
    for _ in 1..n {
        let next = (0..n)
            .filter(|index| !in_tree[*index])
            .min_by_key(|index| (best[*index].0, *index))
            .unwrap();
        in_tree[next] = true;
        total += best[next].0;
        order.push((next, best[next].1));
        for index in 0..n {
            if !in_tree[index] {
                let candidate = distance(points[index], points[next]);
                if candidate < best[index].0 {
                    best[index] = (candidate, next);
                }
            }
        }
    }
    (total as u32, order)
}

fn spanning_steps(points: &[(i64, i64)]) -> u32 {
    spanning_tree(points).0
}

/// The nets to route: per net its terminals as states, whether its pour
/// connects it, its demand in reference tracks; in the router's order
/// (pour nets first, then nets of up to eight terminals, each by span).
fn plans(board: &Board, frame: &Frame, capacity: &Capacity, config: &GlobalConfig) -> Vec<Plan> {
    let owned = pour_owned(board, capacity);
    let mut keyed: Vec<(PlanKey, Plan)> = (0..board.nets.len())
        .filter_map(|net| plan(board, frame, capacity, config, &owned, net))
        .collect();
    keyed.sort_by_key(|(key, _)| *key);
    keyed.into_iter().map(|(_, plan)| plan).collect()
}

/// The routing order's key of a plan (pour nets first, then nets of up to
/// eight terminals, each by span).
pub(crate) type PlanKey = (bool, bool, u64, usize);

/// Per net whether a connected pour owns tiles of it.
pub(crate) fn pour_owned(board: &Board, capacity: &Capacity) -> Vec<bool> {
    let mut owned = vec![false; board.nets.len()];
    for owner in &capacity.pour_owner {
        if *owner > 0 {
            owned[*owner as usize - 1] = true;
        }
    }
    owned
}

/// `net`'s plan with its key in the routing order; `None` for a net
/// with nothing to route.
pub(crate) fn plan(
    board: &Board,
    frame: &Frame,
    capacity: &Capacity,
    config: &GlobalConfig,
    owned: &[bool],
    net: usize,
) -> Option<(PlanKey, Plan)> {
    let tiles = frame.tiles();
    let layers = frame.layers;
    let reference = capacity.reference.trace_width + capacity.reference.clearance;
    let record = &board.nets[net];
    {
        let pour = owned[net];
        if record.terminals.len() < 2 && !pour {
            return None;
        }
        let mut terminals: Vec<Vec<u32>> = Vec::new();
        let (mut low, mut high) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        for terminal in &record.terminals {
            let tile = frame.tile_of(terminal.anchor);
            let states: Vec<u32> = (0..layers)
                .filter(|layer| terminal.layers & (1 << layer) != 0)
                .map(|layer| (layer * tiles + tile) as u32)
                .collect();
            if states.is_empty() {
                continue;
            }
            for axis in 0..2 {
                low[axis] = low[axis].min(terminal.anchor[axis]);
                high[axis] = high[axis].max(terminal.anchor[axis]);
            }
            terminals.push(states);
        }
        terminals.sort();
        terminals.dedup();
        if terminals.is_empty() || (terminals.len() < 2 && !pour) {
            return None;
        }
        let point = |states: &Vec<u32>| {
            let tile = states[0] as usize % tiles;
            ((tile % frame.tiles_x) as i64, (tile / frame.tiles_x) as i64)
        };
        let points: Vec<(i64, i64)> = terminals.iter().map(point).collect();
        let (steps, order) = spanning_tree(&points);
        let mut unique = points.clone();
        unique.sort_unstable();
        unique.dedup();
        let (mut x0, mut y0, mut x1, mut y1) = (i64::MAX, i64::MAX, i64::MIN, i64::MIN);
        for (x, y) in &points {
            x0 = x0.min(*x);
            y0 = y0.min(*y);
            x1 = x1.max(*x);
            y1 = y1.max(*y);
        }
        let margin = (config.window_tiles as i64).max((x1 - x0).max(y1 - y0) / 4);
        let window = [
            (x0 - margin).max(0) as usize,
            (y0 - margin).max(0) as usize,
            ((x1 + margin) as usize).min(frame.tiles_x - 1),
            ((y1 + margin) as usize).min(frame.tiles_y - 1),
        ];
        let class = board.classes[record.class];
        let demand = ((class.trace_width + class.clearance) / reference) as f32;
        let span = (high[0] - low[0]) + (high[1] - low[1]);
        let lower_bound = if pour {
            0
        } else if unique.len() == points.len() {
            steps
        } else {
            spanning_steps(&unique)
        };
        Some((
            (
                !pour,
                record.terminals.len() > 8,
                (span * 1000.0) as u64,
                net,
            ),
            Plan {
                net,
                terminals,
                pour,
                demand,
                lower_bound,
                order,
                window,
            },
        ))
    }
}

struct Snapshot {
    usage: [Vec<f32>; 2],
    via_usage: Vec<f32>,
    routes: Vec<NetRoute>,
    overflow: f64,
    iteration: usize,
}

#[allow(
    clippy::needless_range_loop,
    reason = "usage, capacity, history and overflow marks are parallel arrays"
)]
pub(crate) fn negotiate(
    board: &Board,
    frame: Frame,
    capacity: Capacity,
    config: &GlobalConfig,
) -> GlobalRoute {
    let plans = plans(board, &frame, &capacity, config);
    let per = frame.layers * frame.tiles();
    let (best, iterations, pops, history, via_history, present) = {
        let mut graph = Graph::new(&frame, &capacity, config);
        let mut routes: Vec<NetRoute> = vec![NetRoute::default(); plans.len()];
        let mut pending: Vec<usize> = (0..plans.len()).collect();
        let mut present = config.present_start as f32;
        let mut best: Option<Snapshot> = None;
        let mut iterations = 0;
        for iteration in 0..config.iterations.max(1) {
            iterations = iteration + 1;
            for &index in &pending {
                let old = std::mem::take(&mut routes[index]);
                graph.take(&old, plans[index].demand, -1.0);
                let route = graph.route(&plans[index], present);
                graph.take(&route, plans[index].demand, 1.0);
                routes[index] = route;
            }
            // Overflowing boundaries and via sites.
            let mut over_edge = [vec![false; per], vec![false; per]];
            let mut over_via = vec![false; frame.tiles()];
            let mut total = 0.0f64;
            for direction in 0..2 {
                for index in 0..per {
                    let over = graph.usage[direction][index] - capacity.edges[direction][index];
                    if over > 1.0e-4 {
                        over_edge[direction][index] = true;
                        total += over as f64;
                    }
                }
            }
            for tile in 0..frame.tiles() {
                let over = graph.via_usage[tile] - capacity.vias[tile];
                if over > 1.0e-4 {
                    over_via[tile] = true;
                    total += over as f64;
                }
            }
            if best.as_ref().is_none_or(|kept| total < kept.overflow) {
                best = Some(Snapshot {
                    usage: graph.usage.clone(),
                    via_usage: graph.via_usage.clone(),
                    routes: routes.clone(),
                    overflow: total,
                    iteration: iterations,
                });
            }
            if total <= 0.0 {
                break;
            }
            let increment = config.history_increment as f32;
            for direction in 0..2 {
                for index in 0..per {
                    if over_edge[direction][index] {
                        graph.history[direction][index] += increment;
                    }
                }
            }
            for tile in 0..frame.tiles() {
                if over_via[tile] {
                    graph.via_history[tile] += increment;
                }
            }
            pending = (0..plans.len())
                .filter(|index| {
                    // A terminal the graph cannot reach stays so (closed
                    // boundaries do not open): only overflow reroutes.
                    let route = &routes[*index];
                    route.crossings.iter().any(|edge| {
                        let edge = *edge as usize;
                        over_edge[edge / per][edge % per]
                    }) || route.vias.iter().any(|tile| over_via[*tile as usize])
                })
                .collect();
            if std::env::var_os("PCB_GLOBAL_DEBUG").is_some() {
                eprintln!(
                    "global route: iteration {iterations}, {} nets rerouted next, overflow {total:.1}, pops {}",
                    pending.len(),
                    graph.pops
                );
            }
            present *= config.present_growth as f32;
        }
        (
            best.expect("one iteration at least"),
            iterations,
            graph.pops,
            std::mem::take(&mut graph.history),
            std::mem::take(&mut graph.via_history),
            present,
        )
    };
    // The score of the kept iteration.
    let mut score = Score {
        iterations,
        best_iteration: best.iteration,
        overflow_per_layer: vec![0.0; frame.layers],
        usage_per_layer: vec![0.0; frame.layers],
        capacity_per_layer: vec![0.0; frame.layers],
        nets: best.routes.len(),
        ..Score::default()
    };
    let mut over_edge = [vec![false; per], vec![false; per]];
    for direction in 0..2 {
        for index in 0..per {
            score.usage_per_layer[index / frame.tiles()] += best.usage[direction][index] as f64;
            score.capacity_per_layer[index / frame.tiles()] +=
                capacity.edges[direction][index] as f64;
            let over = (best.usage[direction][index] - capacity.edges[direction][index]) as f64;
            if over > 1.0e-4 {
                over_edge[direction][index] = true;
                score.total_overflow += over;
                score.max_overflow = score.max_overflow.max(over);
                score.overflowed_edges += 1;
                score.overflow_per_layer[index / frame.tiles()] += over;
            }
        }
    }
    let mut utilisation = Vec::new();
    score.soft_overflow = vec![0.0; SOFT_ALPHAS.len()];
    for direction in 0..2 {
        for index in 0..per {
            let (used, room) = (
                best.usage[direction][index] as f64,
                capacity.edges[direction][index] as f64,
            );
            if room > 0.0 {
                utilisation.push(used / room);
            }
            for (slot, alpha) in score.soft_overflow.iter_mut().zip(SOFT_ALPHAS) {
                *slot += (used - alpha * room).max(0.0);
            }
        }
    }
    utilisation.sort_by(|a, b| b.total_cmp(a));
    score.ace = ACE_SHARES
        .iter()
        .map(|share| {
            let count = ((utilisation.len() as f64 * share).ceil() as usize)
                .clamp(1, utilisation.len().max(1));
            utilisation.iter().take(count).sum::<f64>() / count as f64
        })
        .collect();
    let mut over_via = vec![false; frame.tiles()];
    for tile in 0..frame.tiles() {
        let over = (best.via_usage[tile] - capacity.vias[tile]) as f64;
        if over > 1.0e-4 {
            over_via[tile] = true;
            score.via_overflow += over;
        }
    }
    let mut signal = 0usize;
    for route in &best.routes {
        if !route.routed {
            score.unrouted += 1;
        }
        if route.crossings.iter().any(|edge| {
            let edge = *edge as usize;
            over_edge[edge / per][edge % per]
        }) || route.vias.iter().any(|tile| over_via[*tile as usize])
        {
            score.overflowed_nets += 1;
        }
        score.wirelength_mm += route.crossings.len() as f64 * frame.tile_mm;
        score.vias += route.vias.len();
        if !route.pour {
            signal += 1;
            let bound = route.lower_bound as f64;
            if route.crossings.len() as f64 > bound * (1.0 + config.detour_slack) + 2.0 {
                score.detoured += 1;
            }
        }
    }
    score.detour_fraction = if signal > 0 {
        score.detoured as f64 / signal as f64
    } else {
        0.0
    };
    GlobalRoute {
        frame,
        capacity,
        usage: best.usage,
        via_usage: best.via_usage,
        nets: best.routes,
        score,
        capacity_seconds: 0.0,
        seconds: 0.0,
        pops,
        history,
        via_history,
        present,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capacity::tests::{empty_board, wall};
    use pcb_router::{Net, Plane, Terminal};

    fn terminal(anchor: [f64; 2], layers: u32) -> Terminal {
        Terminal {
            anchor,
            layers,
            pad: 0,
            contact: None,
            label: String::new(),
        }
    }

    fn config() -> GlobalConfig {
        GlobalConfig {
            tile_mm: 1.0,
            ..GlobalConfig::default()
        }
    }

    #[test]
    fn a_free_two_pin_net_routes_straight() {
        let board = empty_board(6.0, 2.0, 2);
        let route = crate::route(&board, &config());
        assert_eq!(route.score.nets, 1);
        assert_eq!(route.score.total_overflow, 0.0);
        assert_eq!(route.score.unrouted, 0);
        // From tile (0, 0) to (5, 0): five boundaries on the front (its
        // preferred direction is horizontal), no via.
        assert_eq!(route.nets[0].crossings.len(), 5);
        assert!(route.nets[0].vias.is_empty());
        assert_eq!(route.score.detoured, 0);
        assert_eq!(route.corridor(0), vec![0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn nets_spread_over_a_gap_and_overflow_when_it_is_too_narrow() {
        // A one-layer board 6 x 4 mm, a wall across x = 3 but for a gap.
        let build = |gap_from: f64, gap_to: f64, nets: usize| {
            let mut board = empty_board(6.0, 4.0, 1);
            board
                .obstacles
                .push(wall([3.0, gap_from / 2.0], [0.2, gap_from / 2.0], 1));
            board.obstacles.push(wall(
                [3.0, (gap_to + 4.0) / 2.0],
                [0.2, (4.0 - gap_to) / 2.0],
                1,
            ));
            board.nets = (0..nets)
                .map(|index| Net {
                    name: format!("N{index}"),
                    class: 0,
                    terminals: vec![
                        terminal([0.5, 0.5 + index as f64], 1),
                        terminal([5.5, 0.5 + index as f64], 1),
                    ],
                })
                .collect();
            board
        };
        // A 1.6 mm gap from 1.2 to 2.8: centrelines keep 0.3 mm from the
        // walls, a 1.0 mm run split by the boundary at y = 2 into two
        // runs. Three nets fit through.
        let route = crate::route(&build(1.2, 2.8, 3), &config());
        assert_eq!(route.score.total_overflow, 0.0, "{:?}", route.score);
        // Six nets do not.
        let route = crate::route(&build(1.2, 2.8, 6), &config());
        assert!(route.score.total_overflow > 0.0, "{:?}", route.score);
        assert!(route.score.overflowed_nets > 0);
    }

    #[test]
    fn a_pour_net_reaches_its_pour_with_a_via() {
        let mut board = empty_board(4.0, 4.0, 2);
        // The net's pour covers the back; its two pads are on the front.
        board.nets[0].terminals = vec![terminal([0.5, 0.5], 1), terminal([3.5, 3.5], 1)];
        board.planes.push(Plane {
            net: 0,
            class: 0,
            layer: 1,
            polygon: board.outline.clone(),
            excluded: Vec::new(),
            above: Vec::new(),
            connect: true,
            thermal_reach: 0.0,
            thermal_gap: 0.5,
            exclusive: false,
            solid: false,
            hatch: None,
            island_min_area: Some(0.0),
        });
        let route = crate::route(&board, &config());
        assert_eq!(route.score.unrouted, 0);
        assert!(route.nets[0].pour);
        // One via under each pad, no boundary crossed.
        assert_eq!(route.nets[0].vias.len(), 2);
        assert!(route.nets[0].crossings.is_empty());
    }

    #[test]
    fn spanning_steps_are_rectilinear() {
        assert_eq!(spanning_steps(&[(0, 0), (3, 0), (3, 4)]), 7);
        assert_eq!(spanning_steps(&[(0, 0)]), 0);
    }
}
