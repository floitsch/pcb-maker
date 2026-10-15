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
    /// A clearance floor local to this object; for a keepout, how far
    /// copper stays from it (a solder mask opening: half the mask's minimum
    /// width, as KiCad's bridge test counts copper that near as exposed).
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
    /// Where copper connects to the pad, when that is less than its copper:
    /// KiCad joins a custom pad only through its anchor, not along its
    /// primitives (Sisu's dome switches: tracks ending on the ring stayed
    /// unconnected).
    pub contact: Option<crate::geometry::Shape>,
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
    /// The outlines of the net's own zones of a higher priority on the
    /// layer. KiCad fills each zone apart and cuts these from this one's
    /// fill (zone_filler.cpp, subtractHigherPriorityZones): the fills meet
    /// at a seam, and the DRC counts a pad's spokes per zone, each in its
    /// own part (`Router::kicad_spokes`).
    pub above: Vec<Vec<Point>>,
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
    /// KiCad's hatched fill: a web with square holes, a ring around each
    /// thermal pad (None: solid).
    pub hatch: Option<Hatch>,
}

/// A hatched zone's pattern (KiCad 10, zone_filler.cpp addHatchFillTypeOnZone
/// and buildHatchZoneThermalRings), in mm and degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hatch {
    /// The web's width.
    pub thickness: f64,
    /// The holes' side.
    pub gap: f64,
    pub orientation: f64,
    /// 0 square holes, 1 chamfered, 2 and more rounded corners.
    pub smoothing_level: u32,
    /// The corner's size as a share of half the gap.
    pub smoothing_value: f64,
    /// A hole the fill's edge cuts to less than this share of a whole one
    /// is filled.
    pub min_hole_area: f64,
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
    /// Per pad (index into `obstacles`), the frame KiCad builds its
    /// thermal spokes in, where the lowering knows it; pads without one get
    /// spokes along the axes of their bounding box (diagonal for a circle).
    pub pad_spokes: std::collections::HashMap<usize, PadSpokes>,
    /// Pads that share a number with a larger pad they overlap, which is
    /// their terminal (an exposed pad's thermal vias): KiCad's DRC checks
    /// each one's spokes where it is copper of its own (the vias on the
    /// other side of the board, U1.24's through-hole pad on F.Cu beside its
    /// B.Cu pad).
    pub merged_pads: Vec<MergedPad>,
}

/// A pad merged into its net's terminal `host` (`Board::merged_pads`).
#[derive(Clone, Debug, PartialEq)]
pub struct MergedPad {
    pub net: NetId,
    /// Index into the net's terminals.
    pub host: usize,
    /// Index into `Board::obstacles` of the pad's copper.
    pub pad: usize,
    pub anchor: Point,
    pub layers: LayerMask,
}

/// The frame of a pad's thermal spokes as KiCad builds them (zone_filler.cpp,
/// buildThermalSpokes): four spokes 90 degrees apart from the pad's shape
/// position, at the pad's spoke angle in its own frame (a round pad's are
/// built along the axes and then turned by it), each to the pad's box at
/// orientation 0 grown by the gap; then turned with the pad.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PadSpokes {
    /// The pad's shape position (its drill offset applied).
    pub center: Point,
    /// Half the pad's size at orientation 0.
    pub half: [f64; 2],
    /// The pad's orientation, degrees as KiCad's file gives it (absolute;
    /// Y down: a point turns as `RotatePoint` turns it).
    pub orientation: f64,
    /// The pad's thermal spoke angle, degrees (KiCad's default: 45 for a
    /// circle, else 90).
    pub spoke_angle: f64,
    /// A circle, or an oval of equal sides.
    pub circular: bool,
}

impl PadSpokes {
    /// The four spokes from `center`: per spoke its direction, the
    /// distance from the centre to the pad's box along it, and to the box
    /// grown by `reach` (where KiCad tests the spoke's end).
    pub fn spokes(&self, reach: f64) -> [(Point, f64, f64); 4] {
        // KiCad's RotatePoint: x' = x cos + y sin, y' = -x sin + y cos.
        let turn = |vector: Point, degrees: f64| {
            let (sine, cosine) = degrees.to_radians().sin_cos();
            [
                vector[0] * cosine + vector[1] * sine,
                -vector[0] * sine + vector[1] * cosine,
            ]
        };
        let mut result = [([0.0, 0.0], 0.0, 0.0); 4];
        for (index, slot) in result.iter_mut().enumerate() {
            if self.circular {
                let radius = self.half[0].max(self.half[1]);
                let axis = [[1.0, 0.0], [0.0, 1.0], [-1.0, 0.0], [0.0, -1.0]][index];
                *slot = (
                    turn(turn(axis, self.spoke_angle), self.orientation),
                    radius,
                    radius + reach,
                );
            } else {
                let (sine, cosine) = (self.spoke_angle + 90.0 * index as f64)
                    .to_radians()
                    .sin_cos();
                let along = |half: [f64; 2]| {
                    let x = if cosine.abs() > 1.0e-12 {
                        half[0] / cosine.abs()
                    } else {
                        f64::INFINITY
                    };
                    let y = if sine.abs() > 1.0e-12 {
                        half[1] / sine.abs()
                    } else {
                        f64::INFINITY
                    };
                    x.min(y)
                };
                let local = [
                    if cosine.abs() > 1.0e-12 { cosine } else { 0.0 },
                    if sine.abs() > 1.0e-12 { sine } else { 0.0 },
                ];
                *slot = (
                    turn(local, self.orientation),
                    along(self.half),
                    along([self.half[0] + reach, self.half[1] + reach]),
                );
            }
        }
        result
    }
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
            let obstacle_class = obstacle.net.map_or(0.0, |net| {
                self.classes[self.nets[net as usize].class].clearance
            });
            obstacle.clearance.max(obstacle_class)
        });
        class.clearance.max(obstacle_side)
    }

    /// The clearance KiCad's zone filler keeps between a pour of brush
    /// `class` and copper `obstacle`: as `copper_clearance`, but a pad's
    /// own override replaces the zone's clearance too, also when smaller
    /// (DRC_ENGINE: local overrides take precedence; Hub's USB-C J3, pads
    /// overridden to 0.15 mm in a 0.5 mm zone, keeps its diagonal spokes
    /// past the neighbouring pins).
    pub fn fill_clearance(&self, class: &RuleClass, obstacle: &Obstacle) -> f64 {
        obstacle
            .clearance_override
            .unwrap_or_else(|| self.copper_clearance(class, obstacle))
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
