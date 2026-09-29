// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Global routing: the upper layer above the lattice. The board is cut into
//! tiles (per layer); each boundary between two neighbouring tiles has a
//! capacity, the number of tracks that fit across it (counted exactly from
//! where track centrelines may run). Nets negotiate for those capacities on
//! this small graph (PathFinder: present and history costs), which takes
//! milliseconds where the lattice takes minutes. Each net's global route,
//! widened by a margin, is the corridor its lattice search is confined to.
//!
//! When the lattice cannot realize a plan (nets keep conflicting in some
//! tiles), the plan learns: the boundaries there lose capacity and the nets
//! involved are planned again. The lattice never retries a plan the graph
//! already knows fails.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// A node of the graph: a tile on a layer.
pub(crate) type State = usize;

/// The graph and every net's plan.
#[derive(Clone)]
pub(crate) struct GlobalPlan {
    pub tiles_x: usize,
    pub tiles_y: usize,
    pub layers: usize,
    /// Per layer, per tile: capacity (in tracks of the reference class) of
    /// the boundary to the tile on the right, and to the tile below.
    pub capacity: [Vec<Vec<f32>>; 2],
    /// Learned reductions of those capacities.
    pub learned: [Vec<Vec<f32>>; 2],
    pub usage: [Vec<Vec<f32>>; 2],
    pub history: [Vec<Vec<f32>>; 2],
    /// Per layer and tile: whether any track may run there at all.
    pub open: Vec<Vec<bool>>,
    /// Per layer and tile: whether a via may stand somewhere in it.
    pub via: Vec<Vec<bool>>,
    /// Per net: tiles of each terminal (as states), its demand in tracks,
    /// and its current route (the states it passes, and the boundaries it
    /// crosses as (direction, layer, tile)).
    pub nets: Vec<GlobalNet>,
    pub tile_mm: f32,
    pub via_cost: f32,
}

#[derive(Clone, Default)]
pub(crate) struct GlobalNet {
    pub terminals: Vec<Vec<State>>,
    pub demand: f32,
    pub states: Vec<State>,
    pub crossings: Vec<(usize, usize, usize)>,
    pub routed: bool,
}

/// What the graph is built from.
pub(crate) struct Inputs<'a> {
    pub nx: usize,
    pub ny: usize,
    pub tile: usize,
    pub pitch: f64,
    pub layers: usize,
    /// Per layer and lattice node: whether the reference class's track
    /// centreline may be there (free of fixed objects).
    pub free: Vec<&'a [bool]>,
    /// Per lattice node: whether the reference class's via may stand there.
    pub via_free: &'a [bool],
    /// Track room (width plus clearance) of the reference class.
    pub room: f64,
    pub via_cost: f64,
}

impl GlobalPlan {
    pub fn new(inputs: &Inputs, nets: Vec<GlobalNet>) -> Self {
        let tiles_x = inputs.nx.div_ceil(inputs.tile);
        let tiles_y = inputs.ny.div_ceil(inputs.tile);
        let tiles = tiles_x * tiles_y;
        let spacing = (inputs.room / inputs.pitch).max(1.0);
        // Tracks that fit along a line of lattice nodes: each run of free
        // nodes holds one track plus one per `spacing` nodes.
        let fit = |free: &mut dyn Iterator<Item = bool>| -> f32 {
            let mut total = 0.0;
            let mut run = 0usize;
            for is_free in free.chain(std::iter::once(false)) {
                if is_free {
                    run += 1;
                } else if run > 0 {
                    total += ((run - 1) as f64 / spacing).floor() as f32 + 1.0;
                    run = 0;
                }
            }
            total
        };
        let mut capacity = [vec![vec![0.0; tiles]; inputs.layers], vec![vec![0.0; tiles]; inputs.layers]];
        let mut open = vec![vec![false; tiles]; inputs.layers];
        let mut via = vec![vec![false; tiles]; inputs.layers];
        for layer in 0..inputs.layers {
            let free = inputs.free[layer];
            for ty in 0..tiles_y {
                for tx in 0..tiles_x {
                    let tile = ty * tiles_x + tx;
                    let (x0, y0) = (tx * inputs.tile, ty * inputs.tile);
                    let (x1, y1) = ((x0 + inputs.tile).min(inputs.nx), (y0 + inputs.tile).min(inputs.ny));
                    open[layer][tile] = (y0..y1).any(|y| (x0..x1).any(|x| free[y * inputs.nx + x]));
                    via[layer][tile] = (y0..y1).any(|y| (x0..x1).any(|x| inputs.via_free[y * inputs.nx + x] && free[y * inputs.nx + x]));
                    // The boundary to the right: the last column of this
                    // tile (a track crossing it passes a node there that is
                    // free, and so is its neighbour to the right).
                    if tx + 1 < tiles_x {
                        let x = x1 - 1;
                        capacity[0][layer][tile] = fit(&mut (y0..y1).map(|y| free[y * inputs.nx + x] && free[y * inputs.nx + x + 1]));
                    }
                    if ty + 1 < tiles_y {
                        let y = y1 - 1;
                        capacity[1][layer][tile] = fit(&mut (x0..x1).map(|x| free[y * inputs.nx + x] && free[(y + 1) * inputs.nx + x]));
                    }
                }
            }
        }
        let zero = || vec![vec![0.0f32; tiles]; inputs.layers];
        Self {
            tiles_x,
            tiles_y,
            layers: inputs.layers,
            capacity,
            learned: [zero(), zero()],
            usage: [zero(), zero()],
            history: [zero(), zero()],
            open,
            via,
            nets,
            tile_mm: (inputs.tile as f64 * inputs.pitch) as f32,
            via_cost: inputs.via_cost as f32,
        }
    }

    fn tiles(&self) -> usize {
        self.tiles_x * self.tiles_y
    }

    /// Capacity left of a boundary for everyone.
    fn room(&self, direction: usize, layer: usize, tile: usize) -> f32 {
        (self.capacity[direction][layer][tile] - self.learned[direction][layer][tile]).max(0.0)
    }

    /// Takes a net's route out of the usage.
    pub fn rip_up(&mut self, net: usize) {
        let demand = self.nets[net].demand;
        for &(direction, layer, tile) in &self.nets[net].crossings {
            self.usage[direction][layer][tile] -= demand;
        }
        let entry = &mut self.nets[net];
        entry.crossings.clear();
        entry.states.clear();
        entry.routed = false;
    }

    /// Routes `net` on the graph as a tree (from its first terminal, the
    /// cheapest way to each remaining one in turn) at price `present`.
    pub fn route(&mut self, net: usize, present: f32) {
        let tiles = self.tiles();
        let states = tiles * self.layers;
        let demand = self.nets[net].demand;
        let terminals = self.nets[net].terminals.clone();
        let Some(first) = terminals.iter().position(|terminal| !terminal.is_empty()) else {
            return;
        };
        let endpoint: Vec<bool> = {
            let mut marks = vec![false; tiles];
            for terminal in &terminals {
                for &state in terminal {
                    marks[state % tiles] = true;
                }
            }
            marks
        };
        let mut in_tree = vec![false; states];
        let mut tree: Vec<State> = Vec::new();
        for &state in &terminals[first] {
            if !in_tree[state] {
                in_tree[state] = true;
                tree.push(state);
            }
        }
        let mut connected = vec![false; terminals.len()];
        connected[first] = true;
        let mut crossings = Vec::new();
        let mut cost = vec![f32::INFINITY; states];
        let mut from: Vec<(u32, u8)> = vec![(u32::MAX, 0); states];
        loop {
            let mut target_of = vec![usize::MAX; states];
            for (index, terminal) in terminals.iter().enumerate() {
                if connected[index] {
                    continue;
                }
                for &state in terminal {
                    target_of[state] = index;
                }
            }
            if !target_of.iter().any(|&index| index != usize::MAX) {
                break;
            }
            cost.iter_mut().for_each(|value| *value = f32::INFINITY);
            from.iter_mut().for_each(|value| *value = (u32::MAX, 0));
            let mut heap: BinaryHeap<Reverse<(u32, u32)>> = BinaryHeap::new();
            for &state in &tree {
                cost[state] = 0.0;
                heap.push(Reverse((0f32.to_bits(), state as u32)));
            }
            let mut reached = None;
            while let Some(Reverse((bits, state))) = heap.pop() {
                let state = state as usize;
                let here = f32::from_bits(bits);
                if here > cost[state] {
                    continue;
                }
                if target_of[state] != usize::MAX {
                    reached = Some(state);
                    break;
                }
                let (layer, tile) = (state / tiles, state % tiles);
                let (x, y) = (tile % self.tiles_x, tile / self.tiles_x);
                // Moves: right, left, down, up (direction 0 or 1 boundary of
                // this tile or the neighbour), and vias.
                let mut moves: [(usize, usize, usize); 4] = [(usize::MAX, 0, 0); 4];
                if x + 1 < self.tiles_x {
                    moves[0] = (tile + 1, 0, tile);
                }
                if x > 0 {
                    moves[1] = (tile - 1, 0, tile - 1);
                }
                if y + 1 < self.tiles_y {
                    moves[2] = (tile + self.tiles_x, 1, tile);
                }
                if y > 0 {
                    moves[3] = (tile - self.tiles_x, 1, tile - self.tiles_x);
                }
                for (next, direction, boundary) in moves {
                    if next == usize::MAX || !(self.open[layer][next] || endpoint[next]) {
                        continue;
                    }
                    let room = self.room(direction, layer, boundary);
                    if room <= 0.0 && !(endpoint[next] && endpoint[tile]) {
                        continue;
                    }
                    let over = (self.usage[direction][layer][boundary] + demand - room).max(0.0);
                    let step = self.tile_mm * (1.0 + self.history[direction][layer][boundary]) * (1.0 + present * over);
                    let next_state = layer * tiles + next;
                    let total = here + step;
                    if total < cost[next_state] {
                        cost[next_state] = total;
                        from[next_state] = (state as u32, 1 + direction as u8);
                        heap.push(Reverse((total.to_bits(), next_state as u32)));
                    }
                }
                if self.via[layer][tile] || endpoint[tile] {
                    for other in 0..self.layers {
                        if other == layer || !(self.via[other][tile] || endpoint[tile]) {
                            continue;
                        }
                        let next_state = other * tiles + tile;
                        let total = here + self.via_cost;
                        if total < cost[next_state] {
                            cost[next_state] = total;
                            from[next_state] = (state as u32, 0);
                            heap.push(Reverse((total.to_bits(), next_state as u32)));
                        }
                    }
                }
            }
            let Some(mut state) = reached else {
                break;
            };
            connected[target_of[state]] = true;
            for &other in &terminals[target_of[state]] {
                if !in_tree[other] {
                    in_tree[other] = true;
                    tree.push(other);
                }
            }
            while !in_tree[state] || from[state].0 != u32::MAX {
                if !in_tree[state] {
                    in_tree[state] = true;
                    tree.push(state);
                }
                let (previous, kind) = from[state];
                if previous == u32::MAX {
                    break;
                }
                let previous = previous as usize;
                if kind > 0 {
                    let direction = (kind - 1) as usize;
                    let layer = state / tiles;
                    let (a, b) = (state % tiles, previous % tiles);
                    crossings.push((direction, layer, a.min(b)));
                }
                if in_tree[previous] && cost[previous] == 0.0 {
                    break;
                }
                state = previous;
            }
        }
        for &(direction, layer, tile) in &crossings {
            self.usage[direction][layer][tile] += demand;
        }
        let entry = &mut self.nets[net];
        entry.crossings = crossings;
        entry.states = tree;
        entry.routed = connected.iter().all(|&done| done);
    }

    /// Boundaries used beyond their room.
    pub fn overflowing(&self) -> Vec<(usize, usize, usize)> {
        let mut found = Vec::new();
        for direction in 0..2 {
            for layer in 0..self.layers {
                for tile in 0..self.tiles() {
                    if self.usage[direction][layer][tile] > self.room(direction, layer, tile) + 1.0e-3 {
                        found.push((direction, layer, tile));
                    }
                }
            }
        }
        found
    }

    /// Negotiates the plans of `nets` (all routed from scratch) until no
    /// boundary overflows or `iterations` run out. Returns the overflow
    /// left (in tracks).
    pub fn negotiate(&mut self, nets: &[usize], iterations: usize) -> f32 {
        for &net in nets {
            self.rip_up(net);
        }
        let mut pending: Vec<usize> = nets.to_vec();
        let mut present = 0.5f32;
        for _ in 0..iterations {
            for &net in &pending {
                self.rip_up(net);
                self.route(net, present);
            }
            let over = self.overflowing();
            if over.is_empty() {
                return 0.0;
            }
            for &(direction, layer, tile) in &over {
                self.history[direction][layer][tile] += 0.5;
            }
            pending = (0..self.nets.len())
                .filter(|&net| {
                    self.nets[net].crossings.iter().any(|crossing| over.contains(crossing))
                })
                .collect();
            present *= 1.8;
        }
        self.overflowing()
            .iter()
            .map(|&(direction, layer, tile)| self.usage[direction][layer][tile] - self.room(direction, layer, tile))
            .sum()
    }

    /// The tiles (any layer) of `net`'s plan, widened by `margin` tiles.
    pub fn corridor(&self, net: usize, margin: usize) -> Option<Vec<bool>> {
        let entry = &self.nets[net];
        if !entry.routed || entry.states.is_empty() {
            return None;
        }
        let tiles = self.tiles();
        let mut corridor = vec![false; tiles];
        for &state in &entry.states {
            let tile = state % tiles;
            let (x, y) = (tile % self.tiles_x, tile / self.tiles_x);
            for ty in y.saturating_sub(margin)..=(y + margin).min(self.tiles_y - 1) {
                for tx in x.saturating_sub(margin)..=(x + margin).min(self.tiles_x - 1) {
                    corridor[ty * self.tiles_x + tx] = true;
                }
            }
        }
        Some(corridor)
    }

    /// The lattice could not realize the plan around `tile` on `layer`
    /// (nets conflict there): the boundaries of that tile lose `amount`
    /// tracks of room, and grow in history.
    pub fn learn(&mut self, layer: usize, tile: usize, amount: f32) {
        let (x, y) = (tile % self.tiles_x, tile / self.tiles_x);
        let mut boundaries = vec![(0, tile), (1, tile)];
        if x > 0 {
            boundaries.push((0, tile - 1));
        }
        if y > 0 {
            boundaries.push((1, tile - self.tiles_x));
        }
        for (direction, boundary) in boundaries {
            if self.usage[direction][layer][boundary] > 0.0 {
                self.learned[direction][layer][boundary] =
                    (self.learned[direction][layer][boundary] + amount).min(self.capacity[direction][layer][boundary]);
                self.history[direction][layer][boundary] += 1.0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 64 x 32 lattice with a wall down the middle except for a gap.
    fn board(gap: usize) -> (Vec<bool>, Vec<bool>) {
        let (nx, ny) = (64, 32);
        let mut free = vec![true; nx * ny];
        for y in 0..ny {
            if y < 8 || y >= 8 + gap {
                free[y * nx + 31] = false;
                free[y * nx + 32] = false;
            }
        }
        (free, vec![false; nx * ny])
    }

    fn net(plan_tiles_x: usize, from: (usize, usize), to: (usize, usize)) -> GlobalNet {
        GlobalNet {
            terminals: vec![vec![from.1 * plan_tiles_x + from.0], vec![to.1 * plan_tiles_x + to.0]],
            demand: 1.0,
            ..GlobalNet::default()
        }
    }

    #[test]
    fn capacities_count_tracks_through_gaps() {
        let (free, via) = board(4);
        let inputs = Inputs { nx: 64, ny: 32, tile: 16, pitch: 0.1, layers: 1, free: vec![&free], via_free: &via, room: 0.2, via_cost: 1.0 };
        let plan = GlobalPlan::new(&inputs, Vec::new());
        // The wall is at columns 31 and 32: the boundary between tiles 1 and
        // 2 (column 47/48) is open, the one between 0 and 1 (15/16) too, and
        // the wall lies inside tile 1 and tile 2's shared boundary (31/32).
        // Tile 1's right boundary is column 31 -> 32: only the 4-node gap,
        // rows 8..12 in the upper tile: 2 tracks at a spacing of 2 nodes.
        assert_eq!(plan.capacity[0][0][1], 2.0);
        assert_eq!(plan.capacity[0][0][1 + 4], 0.0);
        assert_eq!(plan.capacity[0][0][0], 8.0);
    }

    #[test]
    fn negotiation_spreads_nets_over_the_capacity() {
        let (free, via) = board(4);
        let inputs = Inputs { nx: 64, ny: 32, tile: 16, pitch: 0.1, layers: 1, free: vec![&free], via_free: &via, room: 0.2, via_cost: 1.0 };
        let nets = (0..3).map(|_| net(4, (0, 0), (3, 0))).collect();
        let mut plan = GlobalPlan::new(&inputs, nets);
        let overflow = plan.negotiate(&[0, 1, 2], 20);
        // Only two tracks fit through the gap: the third cannot be placed.
        assert!(overflow > 0.0);
        let (free, via) = board(8);
        let inputs = Inputs { nx: 64, ny: 32, tile: 16, pitch: 0.1, layers: 1, free: vec![&free], via_free: &via, room: 0.2, via_cost: 1.0 };
        let nets = (0..3).map(|_| net(4, (0, 0), (3, 0))).collect();
        let mut plan = GlobalPlan::new(&inputs, nets);
        assert_eq!(plan.negotiate(&[0, 1, 2], 20), 0.0);
        assert!(plan.nets.iter().all(|net| net.routed));
        assert!(plan.corridor(0, 0).unwrap()[1]);
    }
}
