// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! A coarse global router (docs/global-route.md).
//!
//! The board is cut into square tiles (`tile_mm`, 1.5 mm by default: a few
//! routing pitches). Each boundary between two neighbouring tiles holds, per
//! copper layer, as many tracks of the reference class (the class most nets
//! use) as fit across it between the pads, keepouts, holes and fixed copper
//! (`capacity`); each tile holds as many vias as its free sites allow. Every
//! net is routed on this graph (a tree grown from its first terminal to the
//! nearest unconnected one in turn; a pour net's pads each to the nearest
//! tile its pour covers), with vias as layer changes at the router's via
//! cost and the router's preferred directions per layer, and the nets
//! negotiate for the capacities PathFinder style: a present price on
//! overflow that grows every iteration and a history on boundaries that
//! overflowed (`negotiate`).
//!
//! What it gives: the overflow per boundary and layer, a routability score
//! (total and largest overflow, overflowed vias, the nets crossing an
//! overflowing boundary, the nets that detour) and per net the tiles it
//! crossed (its corridor). It reads the lowered `pcb_router::Board` and
//! builds no lattice: seconds on the largest boards.

pub mod capacity;
mod negotiate;

pub use capacity::{Capacity, Frame, reference_class};
pub use negotiate::{ACE_SHARES, GlobalRoute, NetRoute, SOFT_ALPHAS, Score};

use pcb_router::Board;
use serde::{Deserialize, Serialize};

/// Settings of the global route.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct GlobalConfig {
    /// Tile side in millimetres.
    pub tile_mm: f64,
    /// Spacing of the samples along a tile boundary that measure its free
    /// width, in millimetres.
    pub sample_mm: f64,
    /// Lines a boundary's capacity is measured on: 1, the boundary; 3,
    /// also the lines a quarter tile into either tile (their mean).
    pub boundary_lines: usize,
    /// Spacing of the via sites tested in a tile, in millimetres.
    pub via_sample_mm: f64,
    /// Cost of a via in millimetres of track (the router's `via_cost`).
    pub via_cost_mm: f64,
    /// Cost factor for running across a layer's preferred axis (even
    /// layers horizontal, odd vertical; the router's `against_direction`).
    pub against_direction: f64,
    /// Cost factor for crossing another net's pour on the layer with the
    /// most pour (the router's `plane_cut_cost`; other layers by their
    /// share).
    pub plane_cut_cost: f64,
    /// A net searches within its terminals' box grown by this many tiles
    /// (or a quarter of the box's larger side) first, then the board.
    pub window_tiles: usize,
    /// Share of the measured capacities (boundaries and via sites) the
    /// nets negotiate for: what a detailed router realizes of the room.
    pub capacity_share: f64,
    /// Negotiation iterations at most (it stops earlier without overflow).
    pub iterations: usize,
    /// Price of a track over a boundary's capacity in the first iteration.
    pub present_start: f64,
    /// Factor on that price after every iteration.
    pub present_growth: f64,
    /// History added to an overflowing boundary or via site per iteration.
    pub history_increment: f64,
    /// A tile counts as a pour's when the pour covers this share of it.
    pub pour_share: f64,
    /// A net detours when its route crosses more boundaries than its
    /// terminals' rectilinear spanning tree (in tiles) times one plus
    /// this, plus two.
    pub detour_slack: f64,
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            tile_mm: 1.5,
            sample_mm: 0.05,
            boundary_lines: 1,
            via_sample_mm: 0.25,
            via_cost_mm: 8.0,
            against_direction: 1.6,
            plane_cut_cost: 3.0,
            window_tiles: 6,
            capacity_share: 1.0,
            iterations: 16,
            present_start: 0.5,
            present_growth: 1.8,
            history_increment: 1.0,
            pour_share: 0.5,
            detour_slack: 0.25,
        }
    }
}

/// Routes `board` globally.
pub fn route(board: &Board, config: &GlobalConfig) -> GlobalRoute {
    let started = std::time::Instant::now();
    let frame = Frame::of_board(board, config.tile_mm);
    let capacity = Capacity::build(board, &frame, config);
    let capacity_seconds = started.elapsed().as_secs_f64();
    let mut result = negotiate::negotiate(board, frame, capacity, config);
    result.capacity_seconds = capacity_seconds;
    result.seconds = started.elapsed().as_secs_f64();
    result
}
