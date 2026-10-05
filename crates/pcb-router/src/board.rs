// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The router's complete in-memory problem. It is format independent: an
//! adapter lowers a KiCad (or any other) board into this once per run.

use crate::geometry::{Point, Shape};

pub type NetId = u32;
pub type ClassId = usize;

/// Copper layers as a bit mask; bit 0 is the front layer.
pub type LayerMask = u32;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RuleClass {
    pub trace_width: f64,
    pub clearance: f64,
    pub via_diameter: f64,
    pub via_drill: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObstacleKind {
    /// Copper: routed copper keeps the larger of both clearances away.
    Copper,
    /// A rule area: copper may touch its boundary but not enter it.
    Keepout,
    /// A drilled hole without copper; uses the board's hole clearance.
    Hole,
}

#[derive(Clone, Debug)]
pub struct Obstacle {
    pub shape: Shape,
    pub layers: LayerMask,
    pub kind: ObstacleKind,
    /// Copper of this net may touch the obstacle (its own pads).
    pub net: Option<NetId>,
    /// A clearance floor local to this object.
    pub clearance: f64,
    /// A pad or footprint clearance override: it replaces the clearance of
    /// the object's net class (KiCad: a positive override wins, also when
    /// it is smaller).
    pub clearance_override: Option<f64>,
    pub blocks_tracks: bool,
    pub blocks_vias: bool,
    /// Human-readable owner, for reports only.
    pub label: String,
}

#[derive(Clone, Debug)]
pub struct Terminal {
    /// Where copper must end to be electrically connected.
    pub anchor: Point,
    pub layers: LayerMask,
    /// Index into `Board::obstacles` of the pad copper.
    pub pad: usize,
    pub label: String,
}

#[derive(Clone, Debug)]
pub struct Net {
    pub name: String,
    pub class: ClassId,
    pub terminals: Vec<Terminal>,
}

/// A copper pour: the net's copper fills `polygon` on `layer` wherever no
/// other copper is in the way. Terminals touching it need no tracks.
#[derive(Clone, Debug)]
pub struct Plane {
    pub net: NetId,
    /// The pour as a brush: `trace_width` is its minimum width and
    /// `clearance` what it keeps from other copper.
    pub class: ClassId,
    pub layer: usize,
    pub polygon: Vec<Point>,
    /// Regions filled by other pours with a higher priority.
    pub excluded: Vec<Vec<Point>>,
    /// Whether pads may connect through the pour. If not, the net is routed
    /// with tracks and the pour only fills (its pads are still guarded).
    pub connect: bool,
    /// Other nets are discouraged from coming this close to pads that
    /// connect to the pour, so their thermal spokes survive. 0 disables.
    pub thermal_reach: f64,
    /// The thermal relief's gap around the pads it connects: its fill
    /// keeps that far from them except for the spokes, so it does not flow
    /// past a pad.
    pub thermal_gap: f64,
    /// A plane layer: other nets' tracks may not run inside the pour (vias
    /// pass through), so it stays whole.
    pub exclusive: bool,
    /// Pads join the pour wherever its fill touches them (KiCad's solid
    /// connection), not only through thermal spokes.
    pub solid: bool,
}

#[derive(Clone, Debug)]
pub struct Board {
    pub layer_count: usize,
    /// Outer boundary; copper must stay inside.
    pub outline: Vec<Point>,
    pub edge_clearance: f64,
    pub hole_clearance: f64,
    pub hole_to_hole: f64,
    pub classes: Vec<RuleClass>,
    /// Narrowest track allowed on the board. A pad too narrow for its own
    /// class's track is left with a stub of this width (a neck-down).
    pub neck_width: f64,
    pub obstacles: Vec<Obstacle>,
    /// Indexed by `NetId`.
    pub nets: Vec<Net>,
    pub planes: Vec<Plane>,
    /// Terminal labels whose pad joins a pour wherever the fill touches it
    /// (KiCad's per-pad solid connection), and labels whose pad never
    /// joins a pour (no connection): both sorted.
    pub solid_pads: Vec<String>,
    pub isolated_pads: Vec<String>,
}

impl Board {
    pub fn all_layers(&self) -> LayerMask {
        (1u32 << self.layer_count) - 1
    }

    /// Clearance between copper of `class` and `obstacle`, as KiCad resolves
    /// it: the larger of the two objects' clearances, where the obstacle's is
    /// its override if it has one, else its net class's (and its local
    /// floor).
    pub fn copper_clearance(&self, class: &RuleClass, obstacle: &Obstacle) -> f64 {
        let obstacle_side = obstacle.clearance_override.unwrap_or_else(|| {
            let obstacle_class = obstacle
                .net
                .map_or(0.0, |net| self.classes[self.nets[net as usize].class].clearance);
            obstacle.clearance.max(obstacle_class)
        });
        class.clearance.max(obstacle_side)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    pub layer: usize,
    pub start: Point,
    pub end: Point,
    pub width: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Via {
    pub at: Point,
    pub diameter: f64,
    pub drill: f64,
}

#[derive(Clone, Debug, Default)]
pub struct NetRoute {
    pub segments: Vec<Segment>,
    pub vias: Vec<Via>,
}
