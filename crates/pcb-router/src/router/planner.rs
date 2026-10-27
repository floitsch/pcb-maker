// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The corridor planner on the tile graph.

use super::*;

impl Router {
    /// Plans a connection on the tile graph: Dijkstra over (layer, tile)
    /// from the source tiles to any target tile, where a tile costs more the
    /// fuller it is. Returns the tiles on the path, grown by `margin` tiles.
    /// With `within`, only those tiles (and the ends' own) are entered.
    #[allow(
        clippy::too_many_arguments,
        reason = "the search's state, its ends and the planner's settings, all read once"
    )]
    pub(super) fn plan_corridor(
        &self,
        net: NetId,
        net_state: &NetState,
        sources: &[Node],
        targets: &[(Node, u16)],
        hard: bool,
        margin: usize,
        own_claimed: Option<&[u32]>,
        within: Option<&[bool]>,
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
        // Keys (cost bits, state) as one word, popped in the order of
        // (cost, state) pairs (non-negative floats order as their bits).
        let mut heap = KeyHeap::default();
        for node in sources {
            let state = node.layer as usize * tiles + tile_of(node.cell);
            endpoint[tile_of(node.cell)] = true;
            if cost[state] > 0.0 {
                cost[state] = 0.0;
                heap.push(state as u64);
            }
        }
        // How expensive a tile is: free tiles cost their size; a full tile
        // is nearly a wall (a real wall when routing hard).
        let enter = |layer: usize, tile: usize| -> Option<f32> {
            let routable = self.tile_routable[class * layers + layer][tile];
            if routable == 0 && !endpoint[tile] {
                return None;
            }
            if within.is_some_and(|within| !within[tile]) && !endpoint[tile] {
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
        const UNKNOWN: u8 = 0;
        const WALL: u8 = 1;
        const KNOWN: u8 = 2;
        let mut entry_kind = vec![UNKNOWN; tiles * layers];
        let mut entry_factor = vec![0.0f32; tiles * layers];
        let mut enter_once = |layer: usize, tile: usize| -> Option<f32> {
            let index = layer * tiles + tile;
            match entry_kind[index] {
                WALL => None,
                KNOWN => Some(entry_factor[index]),
                _ => {
                    let factor = enter(layer, tile);
                    entry_kind[index] = if factor.is_some() { KNOWN } else { WALL };
                    entry_factor[index] = factor.unwrap_or(0.0);
                    factor
                }
            }
        };
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
            let (x, y) = (tile % self.tiles_x, tile / self.tiles_x);
            let mut relax = |next: usize, step: f32, heap: &mut KeyHeap| {
                let total = here + step;
                if total < cost[next] {
                    cost[next] = total;
                    from[next] = state as u32;
                    heap.push(((total.to_bits() as u64) << 32) | next as u64);
                }
            };
            for (direction, (dx, dy)) in DIRECTIONS.iter().enumerate() {
                let (tx, ty) = (x as i64 + *dx as i64, y as i64 + *dy as i64);
                if tx < 0 || ty < 0 || tx >= self.tiles_x as i64 || ty >= self.tiles_y as i64 {
                    continue;
                }
                let next_tile = ty as usize * self.tiles_x + tx as usize;
                let Some(factor) = enter_once(layer, next_tile) else {
                    continue;
                };
                let step = self.direction_cost[layer][direction] / self.grid.pitch as f32 * tile_mm;
                relax(layer * tiles + next_tile, step * factor, &mut heap);
            }
            for other in (0..layers).filter(|other| *other != layer) {
                if let Some(factor) = enter_once(other, tile) {
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
}
