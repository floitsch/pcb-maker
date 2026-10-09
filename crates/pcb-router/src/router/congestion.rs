// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Read-only views of a router's negotiation state on its coarse tiles,
//! for the congestion model's training labels (see
//! `docs/congestion-model.md`). A child module of `router`, so it reads the
//! router's private maps without accessors there.

use super::{Router, TILE};
use crate::board::Board;
use crate::grid::Grid;

/// The router's tile grid: `TILE` x `TILE` lattice nodes a tile.
#[derive(Clone, Debug)]
pub struct TileFrame {
    /// Lattice origin (the centre of node 0, 0) and pitch, in millimetres.
    pub origin: [f64; 2],
    pub pitch: f64,
    /// Lattice nodes.
    pub nx: usize,
    pub ny: usize,
    /// Nodes per tile side.
    pub tile: usize,
    pub tiles_x: usize,
    pub tiles_y: usize,
}

impl TileFrame {
    pub fn of_grid(grid: &Grid) -> Self {
        Self {
            origin: grid.origin,
            pitch: grid.pitch,
            nx: grid.nx,
            ny: grid.ny,
            tile: TILE,
            tiles_x: grid.nx.div_ceil(TILE),
            tiles_y: grid.ny.div_ceil(TILE),
        }
    }

    /// Tile side in millimetres.
    pub fn tile_mm(&self) -> f64 {
        self.pitch * self.tile as f64
    }
}

/// The lattice `Router::new` would choose for `board` with `config`
/// (neck classes added first, as the router does), as tiles.
pub fn tile_frame(board: &Board, config: &super::Config) -> TileFrame {
    let (board, _) = Router::with_neck_classes(board, config);
    TileFrame::of_grid(&Grid::choose(&board, &config.pitches))
}

/// Per-tile maps of where negotiation fought, after a probe. Maps are
/// row-major `[plane][tile_y][tile_x]`; `planes` is the layer count plus
/// one for the via plane where noted.
#[derive(Clone, Debug)]
pub struct CongestionLabels {
    pub frame: TileFrame,
    pub layers: usize,
    /// Nodes of some net's branches (and escape stubs) inside another
    /// net's clearance zone now, per layer and the via plane: the overflow
    /// the negotiation has not resolved.
    pub conflict: Vec<f32>,
    /// Accumulated node history (conflict hits times the history
    /// increment), summed per tile, per layer and the via plane.
    pub history: Vec<f32>,
    /// The tile history the corridor planner learns from, per layer and
    /// the via plane (one increment per iteration a conflict hit the tile).
    pub tile_history: Vec<f32>,
    /// Route nodes per tile and layer (all nets' branches), and vias per
    /// tile in the last plane.
    pub usage: Vec<f32>,
    /// Share of the routable nodes of the tile that some net's clearance
    /// zone claims, per layer, for the rule class most nets use.
    pub claimed: Vec<f32>,
    /// Share of the tile's nodes routable at all for that class, per layer.
    pub routable: Vec<f32>,
    /// Terminals of nets left unfinished (incomplete or conflicted), per
    /// tile.
    pub open_terminals: Vec<f32>,
    /// Per net: routable, complete, conflicted.
    pub net_routable: Vec<bool>,
    pub net_complete: Vec<bool>,
    pub net_conflicted: Vec<bool>,
    /// The probe's measure (`Router::unfinished`).
    pub unfinished: usize,
    pub iterations: usize,
    pub expansions: u64,
}

impl Router {
    /// The tile grid of this router's lattice.
    pub fn tile_frame(&self) -> TileFrame {
        TileFrame::of_grid(&self.grid)
    }

    /// The negotiation state on the tiles (see `CongestionLabels`).
    pub fn congestion_labels(&self) -> CongestionLabels {
        let frame = self.tile_frame();
        let layers = self.board.layer_count;
        let tiles = frame.tiles_x * frame.tiles_y;
        let nx = self.grid.nx;
        let tile_of = |cell: usize| (cell / nx / TILE) * frame.tiles_x + (cell % nx) / TILE;
        let mut conflict = vec![0.0f32; (layers + 1) * tiles];
        let mut usage = vec![0.0f32; (layers + 1) * tiles];
        let mut open_terminals = vec![0.0f32; tiles];
        let mut net_routable = Vec::with_capacity(self.nets.len());
        let mut net_complete = Vec::with_capacity(self.nets.len());
        let mut net_conflicted = Vec::with_capacity(self.nets.len());
        for (net, state) in self.nets.iter().enumerate() {
            let conflicts = if state.routable { self.conflicts(net as u32) } else { Vec::new() };
            for (layer, cell) in &conflicts {
                conflict[layer * tiles + tile_of(*cell as usize)] += 1.0;
            }
            for branch in &state.branches {
                for (index, node) in branch.nodes.iter().enumerate() {
                    let tile = tile_of(node.cell as usize);
                    usage[node.layer as usize * tiles + tile] += 1.0;
                    if index > 0
                        && branch.nodes[index - 1].cell == node.cell
                        && branch.nodes[index - 1].layer != node.layer
                    {
                        usage[layers * tiles + tile] += 1.0;
                    }
                }
            }
            let unfinished = state.routable && (!state.complete || !conflicts.is_empty());
            if unfinished {
                for terminal in &self.board.nets[net].terminals {
                    if let Some(cell) = self.grid.nearest_node(terminal.anchor) {
                        open_terminals[tile_of(cell)] += 1.0;
                    }
                }
            }
            net_routable.push(state.routable);
            net_complete.push(state.complete);
            net_conflicted.push(!conflicts.is_empty());
        }
        let mut history = vec![0.0f32; (layers + 1) * tiles];
        for (layer, values) in self.history.iter().enumerate().take(layers + 1) {
            for (cell, value) in values.iter().enumerate() {
                if *value != 0.0 {
                    history[layer * tiles + tile_of(cell)] += *value;
                }
            }
        }
        let tile_history: Vec<f32> = self.tile_history.iter().take(layers + 1).flat_map(|values| values.iter().copied()).collect();
        // The class most nets route with.
        let mut counts = vec![0usize; self.board.classes.len()];
        for net in &self.board.nets {
            counts[net.class] += 1;
        }
        let class = (0..counts.len()).max_by_key(|class| (counts[*class], usize::MAX - class)).unwrap_or(0);
        let mut claimed = vec![0.0f32; layers * tiles];
        let mut routable = vec![0.0f32; layers * tiles];
        for layer in 0..layers {
            let free = &self.tile_routable[class * layers + layer];
            let taken = &self.tile_claimed[self.map_index(class, layer)];
            for tile in 0..tiles {
                let (x, y) = (tile % frame.tiles_x, tile / frame.tiles_x);
                let width = (nx - x * TILE).min(TILE);
                let height = (self.grid.ny - y * TILE).min(TILE);
                routable[layer * tiles + tile] = free[tile] as f32 / (width * height) as f32;
                claimed[layer * tiles + tile] = taken[tile].min(free[tile]) as f32 / free[tile].max(1) as f32;
            }
        }
        CongestionLabels {
            frame,
            layers,
            conflict,
            history,
            tile_history,
            usage,
            claimed,
            routable,
            open_terminals,
            net_routable,
            net_complete,
            net_conflicted,
            unfinished: self.unfinished(),
            iterations: self.iterations,
            expansions: self.scratch.expansions,
        }
    }
}
