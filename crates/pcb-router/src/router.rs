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

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashSet};

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
    pub history_increment: f64,
    pub max_iterations: usize,
    /// Minimum search window margin around a net's terminals.
    pub window_margin: f64,
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
            history_increment: 0.3,
            max_iterations: 80,
            window_margin: 10.0,
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
const PARENT_SOURCE: u8 = 255;

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
}

struct Stamps {
    trace_to_trace: Vec<(i32, i32)>,
    trace_to_via: Vec<(i32, i32)>,
    via_to_trace: Vec<(i32, i32)>,
    via_to_via: Vec<(i32, i32)>,
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

pub struct Router<'a> {
    board: &'a Board,
    config: &'a Config,
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

    cost: Vec<f32>,
    seen: Vec<u32>,
    closed: Vec<u32>,
    parent: Vec<u8>,
    target_mark: Vec<u32>,
    target_terminal: Vec<u16>,
    tree_mark: Vec<u32>,
    tree_terminal: Vec<u16>,
    generation: u32,
    expansions: u64,
    searches: u64,
}

impl<'a> Router<'a> {
    pub fn new(board: &'a Board, config: &'a Config) -> Self {
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
                        Stamps {
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
        let mut router = Self {
            board,
            config,
            statics,
            occupancy: vec![vec![0; cells]; maps],
            stamp_mark: vec![vec![0; cells]; maps],
            stamp_generation: 0,
            history: vec![vec![0.0; cells]; layers + 1],
            stamps,
            nets: Vec::new(),
            direction_cost,
            cost: vec![0.0; states],
            seen: vec![0; states],
            closed: vec![0; states],
            parent: vec![0; states],
            target_mark: vec![0; states],
            target_terminal: vec![NO_TERMINAL; states],
            tree_mark: vec![0; states],
            tree_terminal: vec![NO_TERMINAL; states],
            generation: 0,
            expansions: 0,
            searches: 0,
            grid,
        };
        router.nets = (0..board.nets.len())
            .map(|net| router.prepare_net(net as NetId))
            .collect();
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

    /// The legal lattice nodes inside each terminal's pad copper.
    fn prepare_net(&self, net: NetId) -> NetState {
        let description = &self.board.nets[net as usize];
        let statics = &self.statics[description.class];
        let mut terminal_nodes = Vec::new();
        let mut terminal_reach = Vec::new();
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
                                && pad.shape.contains(self.grid.center(x, y))
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
        let routable = description.terminals.len() >= 2
            && terminal_nodes.iter().all(|nodes| !nodes.is_empty());
        NetState {
            connected: vec![false; description.terminals.len()],
            terminal_nodes,
            terminal_reach,
            routable,
            ..Default::default()
        }
    }

    fn rip_up(&mut self, net: NetId) {
        let state = &mut self.nets[net as usize];
        for (map, cell) in state.stamped.drain(..) {
            self.occupancy[map as usize][cell as usize] -= 1;
        }
        state.branches.clear();
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
                        stamped.push((map as u32, target as u32));
                    }
                }
            };
            let base = querying * (layers + 1);
            for branch in &branches {
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
        state.stamped = stamped;
    }

    /// Nodes of `net` that lie inside another net's clearance zone.
    fn conflicts(&self, net: NetId) -> Vec<(usize, u32)> {
        let class = self.board.nets[net as usize].class;
        let layers = self.board.layer_count;
        let mut result = Vec::new();
        for branch in &self.nets[net as usize].branches {
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

    /// Grows a tree over all terminals of `net`. With `hard`, nodes claimed
    /// by other nets are impassable and a partial tree may be kept.
    fn route_net(&mut self, net: NetId, present: f32, hard: bool, window_growth: f64) {
        let terminal_count = self.board.nets[net as usize].terminals.len();
        self.generation += 1;
        let tree_generation = self.generation;
        let mut tree: Vec<Node> = Vec::new();
        let add_terminal = |router: &mut Self, tree: &mut Vec<Node>, terminal: usize| {
            for node in router.nets[net as usize].terminal_nodes[terminal].clone() {
                let state = router.state(node);
                router.tree_mark[state] = tree_generation;
                router.tree_terminal[state] = terminal as u16;
                tree.push(node);
            }
            router.nets[net as usize].connected[terminal] = true;
        };
        add_terminal(self, &mut tree, 0);
        let mut remaining = terminal_count - 1;
        while remaining > 0 {
            let windowed = self.window(net, window_growth);
            let full = (0, 0, self.grid.nx - 1, self.grid.ny - 1);
            let path = self
                .search(net, &tree, present, hard, windowed)
                .or_else(|| {
                    (windowed != full)
                        .then(|| self.search(net, &tree, present, hard, full))
                        .flatten()
                });
            let Some(path) = path else {
                break;
            };
            let first = self.state(path[0]);
            let last = self.state(*path.last().unwrap());
            let start_terminal = self.tree_terminal[first];
            let end_terminal = self.target_terminal[last];
            for node in &path[1..] {
                let state = self.state(*node);
                if self.tree_mark[state] != tree_generation {
                    self.tree_mark[state] = tree_generation;
                    self.tree_terminal[state] = NO_TERMINAL;
                    tree.push(*node);
                }
            }
            add_terminal(self, &mut tree, end_terminal as usize);
            self.nets[net as usize].branches.push(Branch {
                nodes: path,
                start_terminal,
                end_terminal,
            });
            remaining -= 1;
        }
        self.nets[net as usize].complete = remaining == 0;
        if !hard && remaining > 0 {
            self.nets[net as usize].blocked = true;
        }
    }

    fn search(
        &mut self,
        net: NetId,
        sources: &[Node],
        present: f32,
        hard: bool,
        window: (usize, usize, usize, usize),
    ) -> Option<Vec<Node>> {
        self.searches += 1;
        self.generation += 1;
        let generation = self.generation;
        let class = self.board.nets[net as usize].class;
        let layers = self.board.layer_count;
        let cells = self.grid.cells();
        let nx = self.grid.nx;
        let pitch = self.grid.pitch as f32;
        let via_cost = self.config.via_cost as f32;
        let bend_cost = self.config.bend_cost as f32;

        let mut target_points: Vec<(i32, i32, f32)> = Vec::new();
        let mut any_target = false;
        for terminal in 0..self.nets[net as usize].connected.len() {
            if self.nets[net as usize].connected[terminal] {
                continue;
            }
            for index in 0..self.nets[net as usize].terminal_nodes[terminal].len() {
                let node = self.nets[net as usize].terminal_nodes[terminal][index];
                let state = self.state(node);
                self.target_mark[state] = generation;
                self.target_terminal[state] = terminal as u16;
                any_target = true;
            }
            let anchor = self.board.nets[net as usize].terminals[terminal].anchor;
            target_points.push((
                ((anchor[0] - self.grid.origin[0]) / self.grid.pitch).round() as i32,
                ((anchor[1] - self.grid.origin[1]) / self.grid.pitch).round() as i32,
                // Rounding the anchor to a node costs at most one pitch.
                self.nets[net as usize].terminal_reach[terminal] + pitch,
            ));
        }
        if !any_target {
            return None;
        }
        let use_heuristic = target_points.len() <= 12;
        let heuristic = |x: i32, y: i32| -> f32 {
            if !use_heuristic {
                return 0.0;
            }
            let mut best = f32::INFINITY;
            for (tx, ty, reach) in &target_points {
                let dx = (x - tx).abs() as f32;
                let dy = (y - ty).abs() as f32;
                let octile = dx.max(dy) + (std::f32::consts::SQRT_2 - 1.0) * dx.min(dy);
                best = best.min(octile * pitch - reach);
            }
            best.max(0.0) * 0.999
        };

        let mut heap: BinaryHeap<Reverse<(u32, u32)>> = BinaryHeap::new();
        for node in sources {
            let state = self.state(*node);
            let (x, y) = self.grid.xy(node.cell as usize);
            self.seen[state] = generation;
            self.cost[state] = 0.0;
            self.parent[state] = PARENT_SOURCE;
            heap.push(Reverse((heuristic(x as i32, y as i32).to_bits(), state as u32)));
        }

        let statics = &self.statics[class];
        let own = crate::grid::owner(net);
        let trace_base = class * (layers + 1);
        let via_map = trace_base + layers;
        let mut found = None;
        while let Some(Reverse((_, state))) = heap.pop() {
            let state = state as usize;
            if self.closed[state] == generation {
                continue;
            }
            self.closed[state] = generation;
            self.expansions += 1;
            if self.target_mark[state] == generation {
                found = Some(state);
                break;
            }
            let layer = state / cells;
            let cell = state % cells;
            let x = cell % nx;
            let y = cell / nx;
            let here = self.cost[state];
            let arrived = self.parent[state];
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
                let target_cell = ty as usize * nx + tx as usize;
                let allowed = statics.trace[layer][target_cell];
                if allowed != crate::grid::FREE && allowed != own {
                    continue;
                }
                let target_state = layer * cells + target_cell;
                if self.closed[target_state] == generation {
                    continue;
                }
                let occupied = self.occupancy[trace_base + layer][target_cell] as f32;
                if hard && occupied > 0.0 {
                    continue;
                }
                let mut step = self.direction_cost[layer][direction]
                    * (1.0 + self.history[layer][target_cell])
                    * (1.0 + present * occupied);
                if (arrived as usize) < 8 {
                    let turn = (direction as i32 - arrived as i32).rem_euclid(8);
                    step += bend_cost * turn.min(8 - turn) as f32;
                }
                let total = here + step;
                if self.seen[target_state] != generation || total < self.cost[target_state] {
                    self.seen[target_state] = generation;
                    self.cost[target_state] = total;
                    self.parent[target_state] = direction as u8;
                    let estimate = total + heuristic(tx as i32, ty as i32);
                    heap.push(Reverse((estimate.to_bits(), target_state as u32)));
                }
            }

            let via_occupied = self.occupancy[via_map][cell] as f32;
            if layers > 1 && !statics.via_blocked[cell] && !(hard && via_occupied > 0.0) {
                let occupied = via_occupied;
                let step = via_cost * (1.0 + self.history[layers][cell]) * (1.0 + present * occupied);
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
                    if self.closed[target_state] == generation {
                        continue;
                    }
                    let total = here + step;
                    if self.seen[target_state] != generation || total < self.cost[target_state] {
                        self.seen[target_state] = generation;
                        self.cost[target_state] = total;
                        self.parent[target_state] = 8 + layer as u8;
                        let estimate = total + heuristic(x as i32, y as i32);
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
            let parent = self.parent[state];
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

    pub fn run(mut self) -> RoutingResult {
        let started = std::time::Instant::now();
        let mut order: Vec<NetId> = (0..self.board.nets.len() as NetId)
            .filter(|net| self.nets[*net as usize].routable)
            .collect();
        let span = |router: &Self, net: NetId| {
            let (x0, y0, x1, y1) = router.window(net, 0.0);
            (x1 - x0) + (y1 - y0)
        };
        order.sort_by_key(|net| {
            (
                self.board.nets[*net as usize].terminals.len() > 8,
                span(&self, *net),
                *net,
            )
        });

        let mut present = self.config.present_factor as f32;
        let mut pending = order.clone();
        let mut iterations = 0;
        let mut best_conflicted = usize::MAX;
        let mut stalled = 0;
        for iteration in 0..self.config.max_iterations {
            iterations = iteration + 1;
            let growth = 1.0 + iteration as f64 / 6.0;
            for net in &pending {
                self.rip_up(*net);
                self.route_net(*net, present, false, growth);
                self.stamp(*net);
            }
            let mut conflicted = Vec::new();
            for net in &order {
                let conflicts = self.conflicts(*net);
                if self.nets[*net as usize].blocked
                    || (conflicts.is_empty() && self.nets[*net as usize].complete)
                {
                    continue;
                }
                for (layer, cell) in conflicts {
                    self.history[layer][cell as usize] += self.config.history_increment as f32;
                }
                conflicted.push(*net);
            }
            if self.config.verbose {
                eprintln!(
                    "iteration {iteration}: rerouted {}, conflicted {}, present {present:.2}, {:.2}s",
                    pending.len(),
                    conflicted.len(),
                    started.elapsed().as_secs_f64()
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
            present = (present * self.config.present_growth as f32).min(1.0e4);
            if stalled > 25 {
                break;
            }
        }

        self.resolve_remaining(&order);

        let status = (0..self.board.nets.len())
            .map(|net| {
                let state = &self.nets[net];
                if self.board.nets[net].terminals.len() < 2 {
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
        let routes = (0..self.board.nets.len() as NetId)
            .map(|net| self.materialize(net))
            .collect();
        RoutingResult {
            routes,
            status,
            iterations,
            expansions: self.expansions,
            searches: self.searches,
            grid: self.grid,
        }
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
        for net in removed {
            self.route_net(net, 0.0, true, 4.0);
            self.stamp(net);
        }
    }

    fn materialize(&self, net: NetId) -> NetRoute {
        let description = &self.board.nets[net as usize];
        let rules = self.board.classes[description.class];
        let state = &self.nets[net as usize];
        let mut route = NetRoute::default();
        let junctions: HashSet<Node> = state
            .branches
            .iter()
            .filter(|branch| branch.start_terminal == NO_TERMINAL)
            .map(|branch| branch.nodes[0])
            .collect();
        for branch in &state.branches {
            let first = branch.nodes[0];
            let last = *branch.nodes.last().unwrap();
            let mut stub = |node: Node, terminal: u16| {
                if terminal == NO_TERMINAL {
                    return;
                }
                let anchor = description.terminals[terminal as usize].anchor;
                let center = self.grid.center_of(node.cell as usize);
                if crate::geometry::distance(anchor, center) > 1.0e-7 {
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
                    route.vias.push(Via {
                        at: self.grid.center_of(branch.nodes[index].cell as usize),
                        diameter: rules.via_diameter,
                        drill: rules.via_drill,
                    });
                }
                run_start = index;
            }
        }
        route
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
