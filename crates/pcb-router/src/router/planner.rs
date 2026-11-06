// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The corridor planner on the tile graph.

use super::*;

impl Router {
    /// Plans a connection on the tile graph: Dijkstra over (layer, tile)
    /// from the source tiles to any target tile, where a tile costs more the
    /// fuller it is. Returns the tiles on the path, grown by `margin` tiles.
    #[allow(
        clippy::too_many_arguments,
        reason = "the search's state, its ends and the planner's settings, all read once"
    )]
    pub(super) fn plan_corridor(
        &self,
        arrays: &mut PlannerArrays,
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
        arrays.reset(tiles, layers);
        let PlannerArrays {
            is_target,
            endpoint,
            cost,
            from,
            entry_kind,
            entry_factor,
            steps,
            heap,
        } = arrays;
        for (node, _) in targets {
            let tile = tile_of(node.cell);
            is_target[node.layer as usize * tiles + tile] = true;
            endpoint[tile] = true;
        }
        let tile_mm = (TILE as f64 * self.grid.pitch) as f32;
        // A step's cost per layer and direction, before the entered tile's
        // factor (the same expression as at each step until 2026-11-05).
        // Off the net's preferred layers (`Config::layer_preferences`) as
        // the search prices it.
        let preferred = self
            .preferred_layers
            .get(net as usize)
            .copied()
            .unwrap_or(0);
        steps.clear();
        steps.extend((0..layers).map(|layer| {
            let factor = if preferred != 0 && preferred & (1 << layer) == 0 {
                self.config.layer_preference_cost as f32
            } else {
                1.0
            };
            std::array::from_fn::<f32, 8, _>(|direction| {
                self.direction_cost[layer][direction] / self.grid.pitch as f32 * tile_mm * factor
            })
        }));
        let via_step = self.config.via_cost as f32;
        // Keys (cost bits, state) as one word, popped in the order of
        // (cost, state) pairs (non-negative floats order as their bits).
        for node in sources {
            let tile = tile_of(node.cell);
            let state = node.layer as usize * tiles + tile;
            endpoint[tile] = true;
            if cost[state] > 0.0 {
                cost[state] = 0.0;
                heap.push(state as u64);
            }
        }
        // How expensive a tile is: free tiles cost their size; a full tile
        // is nearly a wall (a real wall when routing hard).
        let enter = |layer: usize, tile: usize, endpoint: &[bool]| -> Option<f32> {
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
        // What `enter` gives each (layer, tile), computed once: a tile is
        // entered from up to eight neighbours and the other layers.
        macro_rules! enter_once {
            ($layer:expr, $tile:expr) => {{
                let (layer, tile): (usize, usize) = ($layer, $tile);
                let index = layer * tiles + tile;
                match entry_kind[index] {
                    ENTRY_WALL => None,
                    ENTRY_KNOWN => Some(entry_factor[index]),
                    _ => {
                        let factor = enter(layer, tile, endpoint);
                        entry_kind[index] = if factor.is_some() {
                            ENTRY_KNOWN
                        } else {
                            ENTRY_WALL
                        };
                        entry_factor[index] = factor.unwrap_or(0.0);
                        factor
                    }
                }
            }};
        }
        let (tiles_x, tiles_y) = (self.tiles_x as i64, self.tiles_y as i64);
        let mut reached = None;
        while let Some(key) = heap.pop() {
            let state = (key & u32::MAX as u64) as usize;
            let here = f32::from_bits((key >> 32) as u32);
            if here > cost[state] {
                continue;
            }
            if is_target[state] {
                reached = Some(state);
                break;
            }
            let (layer, tile) = (state / tiles, state % tiles);
            let (x, y) = ((tile % self.tiles_x) as i64, (tile / self.tiles_x) as i64);
            let layer_steps = &steps[layer];
            for (direction, (dx, dy)) in DIRECTIONS.iter().enumerate() {
                let (tx, ty) = (x + *dx as i64, y + *dy as i64);
                if tx < 0 || ty < 0 || tx >= tiles_x || ty >= tiles_y {
                    continue;
                }
                let next_tile = (ty * tiles_x + tx) as usize;
                let Some(factor) = enter_once!(layer, next_tile) else {
                    continue;
                };
                let next = layer * tiles + next_tile;
                let total = here + layer_steps[direction] * factor;
                if total < cost[next] {
                    cost[next] = total;
                    from[next] = state as u32;
                    heap.push(((total.to_bits() as u64) << 32) | next as u64);
                }
            }
            for other in (0..layers).filter(|other| *other != layer) {
                if let Some(factor) = enter_once!(other, tile) {
                    let next = other * tiles + tile;
                    let total = here + via_step * factor;
                    if total < cost[next] {
                        cost[next] = total;
                        from[next] = state as u32;
                        heap.push(((total.to_bits() as u64) << 32) | next as u64);
                    }
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
}

/// `PlannerArrays::entry_kind`: not asked yet, no entry, a factor known.
const ENTRY_UNKNOWN: u8 = 0;
const ENTRY_WALL: u8 = 1;
const ENTRY_KNOWN: u8 = 2;

/// The tile planner's arrays (`plan_corridor`), kept in the thread's
/// scratch from plan to plan: until 2026-11-05 each plan allocated six
/// arrays of the tile graph and its heap (a plan a search, 22 000 a
/// route of a dense board; the fifth router round, docs/profiling.md).
/// Each plan starts them afresh (`reset`), so a plan finds what it found
/// with new ones.
#[derive(Default)]
pub(super) struct PlannerArrays {
    /// Per (layer, tile): a target tile on that layer.
    is_target: Vec<bool>,
    /// Per tile: a source's or a target's (entered even when full).
    endpoint: Vec<bool>,
    /// Per (layer, tile): the cost so far and the state it came from.
    cost: Vec<f32>,
    from: Vec<u32>,
    /// Per (layer, tile): `enter`'s answer once asked (`ENTRY_*`, the
    /// factor).
    entry_kind: Vec<u8>,
    entry_factor: Vec<f32>,
    /// Per layer and direction: a step's cost before the tile's factor.
    steps: Vec<[f32; 8]>,
    heap: KeyHeap,
}

impl PlannerArrays {
    /// The arrays of a tile graph of `tiles` tiles and `layers` layers,
    /// as new ones.
    fn reset(&mut self, tiles: usize, layers: usize) {
        let states = tiles * layers;
        let fresh = |values: &mut Vec<_>, length: usize, value| {
            values.clear();
            values.resize(length, value);
        };
        fresh(&mut self.is_target, states, false);
        fresh(&mut self.endpoint, tiles, false);
        self.cost.clear();
        self.cost.resize(states, f32::INFINITY);
        self.from.clear();
        self.from.resize(states, u32::MAX);
        self.entry_kind.clear();
        self.entry_kind.resize(states, ENTRY_UNKNOWN);
        self.entry_factor.clear();
        self.entry_factor.resize(states, 0.0);
        self.heap.clear();
    }

    /// What they hold (the memory profile).
    pub(super) fn bytes(&self) -> usize {
        self.is_target.capacity()
            + self.endpoint.capacity()
            + 4 * (self.cost.capacity() + self.from.capacity() + self.entry_factor.capacity())
            + self.entry_kind.capacity()
            + 32 * self.steps.capacity()
            + self.heap.bytes()
    }
}
