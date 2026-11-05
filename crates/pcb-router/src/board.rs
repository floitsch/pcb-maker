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
    /// The layers the class's tracks may use (bits); 0: every layer. Its
    /// vias pass through all of them. A differential pair's coupled run
    /// (route mode's `diff_pairs`) keeps to the layers the pair's rules
    /// give its width and gap on.
    pub layers: LayerMask,
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
    /// KiCad's island removal (the zone's `island_removal_mode`): which
    /// fill pieces holding no copper of the net the filler deletes.
    /// `Some(0.0)` every one (always, KiCad's default), `Some(area)` those
    /// under `area` mm², None none (never).
    pub island_min_area: Option<f64>,
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
    /// Whether the net's vias get thermal reliefs in this zone: KiCad 10
    /// treats a via in a hatched zone like a pad (knockoutThermalReliefs:
    /// a relief of the thermal gap, a ring and spokes) where the zone's
    /// pad connection is thermal; with `connect_pads yes` or
    /// `thru_hole_only` a via connects solidly.
    pub via_reliefs: bool,
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
    /// The same per pad, as indices into `obstacles` (sorted): a pad's own
    /// zone connection, else its footprint's. A label can name pads of
    /// both kinds (74th's J3.S1, a USB-C shield: its SMD pads have no
    /// zone connection, its through-hole pads the zone's thermal one),
    /// which the label lists cannot tell apart.
    pub solid_pad_obstacles: Vec<usize>,
    pub isolated_pad_obstacles: Vec<usize>,
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
    /// Pads no via may overlap, as indices into `obstacles` (sorted): a
    /// via in a reflowed SMD pad takes the pad's solder down its hole
    /// (pcb-kicad lists every SMD pad but an exposed one unless the board
    /// fills and caps its vias, `via_in_pad`). Every via keeps its ring out
    /// of their copper: another net's vias their clearance beyond it as
    /// from any copper, the pad's own net's may touch it (a via beside the
    /// pad). Other surface pads of a net keep the rule of before: that
    /// net's vias may lie wholly inside them (`StaticMaps::via_owner`).
    pub no_via_pads: Vec<usize>,
    /// Keepouts a pour's fill passes, as indices into `obstacles` (sorted):
    /// rule areas that keep tracks out but let pours in (KiCad fills them:
    /// a plane layer's `no tracks` area, an `add_pours` plane). The static
    /// maps keep them out of the fill trace, as solder mask openings.
    pub fill_passes: Vec<usize>,
    /// The gaps between adjacent SMD pads of one footprint's pin rows
    /// (`PinRowGap`): another net's track there snakes between a part's
    /// pins, which the search charges `Config::pin_row_cost` a step.
    pub pin_row_gaps: Vec<PinRowGap>,
    /// Copper a terminal reaches beside its pad (`TerminalCopper`): the
    /// search ends on any of it. Empty but for route mode's differential
    /// pairs (`diff_pairs`): the pads a pair's links join (merged into one
    /// terminal), and in the fallback the coupled tracks, kept copper on
    /// no pad.
    pub terminal_copper: Vec<TerminalCopper>,
    /// What the adapter that lowered the board keeps with it to read back
    /// with the routes (route mode's differential pairs: how the coupled
    /// runs' nets map back to the pairs; the router ignores it), and the
    /// nets a route may give up on (`GiveUp`: the pairs' links).
    pub attachment: Attachment,
    /// The via spans the board allows beside the through via (blind,
    /// buried and micro vias; docs/router.md, "Blind, buried and micro
    /// vias (2026-11-05)"). Empty: every via is a through via, as before
    /// the spans; the router's maps and its routes are then those of a
    /// board without them.
    pub via_spans: Vec<ViaSpan>,
}

/// A via's layers short of the whole stack (`Board::via_spans`): copper
/// and a hole on the layers `top..=bottom` only (KiCad's blind, buried and
/// micro vias), so that the layers it does not pierce stay free for other
/// nets' tracks and vias.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViaSpan {
    /// The span's first and last copper layers (`top < bottom`).
    pub top: usize,
    pub bottom: usize,
    /// A micro via (laser drilled, one dielectric deep).
    pub micro: bool,
    /// The via's own diameter and drill where they are smaller than its
    /// class's (a micro via's): it is written at that size, while the
    /// maps keep the class's via clear (a smaller via inside a legal one
    /// is legal). None: the class's via.
    pub size: Option<[f64; 2]>,
}

/// KiCad's via kinds, by a via's span on a stack of `layers` copper layers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ViaKind {
    Through,
    /// From an outer layer to an inner one.
    Blind,
    /// Between inner layers.
    Buried,
    Micro,
}

impl ViaSpan {
    pub fn mask(&self) -> LayerMask {
        (self.top..=self.bottom).fold(0, |mask, layer| mask | 1 << layer)
    }

    pub fn depth(&self) -> usize {
        self.bottom - self.top
    }

    pub fn contains(&self, layer: usize) -> bool {
        layer >= self.top && layer <= self.bottom
    }

    pub fn kind(&self, layers: usize) -> ViaKind {
        if self.micro {
            ViaKind::Micro
        } else if self.top == 0 && self.bottom + 1 == layers {
            ViaKind::Through
        } else if self.top == 0 || self.bottom + 1 == layers {
            ViaKind::Blind
        } else {
            ViaKind::Buried
        }
    }

    /// Whether the span keeps KiCad's and the fab's rules on `layers`
    /// copper layers: inside the stack, at least two layers, short of the
    /// whole stack, and a micro via one dielectric deep from an outer
    /// layer or between two inner ones.
    pub fn is_valid(&self, layers: usize) -> bool {
        self.top < self.bottom
            && self.bottom < layers
            && !(self.top == 0 && self.bottom + 1 == layers)
            && (!self.micro || self.depth() == 1)
    }
}

impl Board {
    /// The span a via joining `a` and `b` takes: the allowed span with the
    /// fewest layers that holds both (the first listed of as many), else
    /// the through via (None).
    pub fn via_span(&self, a: usize, b: usize) -> Option<ViaSpan> {
        let (low, high) = (a.min(b), a.max(b));
        self.via_spans
            .iter()
            .filter(|span| span.top <= low && span.bottom >= high)
            .min_by_key(|span| span.depth())
            .copied()
    }
}

/// Data an adapter attaches to a board (`Board::attachment`): shared. The
/// router neither reads `data` nor keys its maps on it; `give_up` names the
/// nets that stand in for others (a differential pair's coupled run, which
/// joins the pair's two nets between its ends) and what those others
/// become once the negotiation gives such a net up (`GiveUp`).
#[derive(Clone, Default)]
pub struct Attachment {
    pub data: Option<std::sync::Arc<dyn std::any::Any + Send + Sync>>,
    pub give_up: Option<std::sync::Arc<dyn GiveUp>>,
}

impl std::fmt::Debug for Attachment {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match (&self.data, &self.give_up) {
            (None, None) => "Attachment(None)",
            (_, None) => "Attachment(..)",
            _ => "Attachment(.., give_up)",
        })
    }
}

/// Nets a route may give up on (`Attachment::give_up`; docs/router.md,
/// "Giving up on a link"): each stands in for a connection of other nets
/// (a differential pair's link joins the stubs' ends of the pair's two
/// nets), so that given up, those nets route the connection themselves.
pub trait GiveUp: Send + Sync {
    /// Whether `net` may be given up.
    fn optional(&self, net: NetId) -> bool;
    /// The nets that change once every net of `released` is given up (all
    /// of them so far, in the order they went), as they become: their
    /// terminals, and every merged pad and terminal copper of theirs (which
    /// replace the board's of those nets). The obstacles stay as they are.
    fn released(&self, board: &Board, released: &[NetId]) -> Vec<NetRewrite>;
    /// The nets `net` stands in for, as they are with it standing again
    /// and every net of `released` (not `net`) given up.
    fn standing(&self, board: &Board, net: NetId, released: &[NetId]) -> Vec<NetRewrite>;
}

/// A net as it becomes when the nets standing in for some of its
/// connections are given up (`GiveUp::released`).
#[derive(Clone, Debug)]
pub struct NetRewrite {
    pub net: NetId,
    pub terminals: Vec<Terminal>,
    pub merged_pads: Vec<MergedPad>,
    pub terminal_copper: Vec<TerminalCopper>,
}

impl Board {
    /// The board with `rewrites` applied: each net's terminals, merged pads
    /// and terminal copper replaced (`GiveUp::released`).
    pub fn rewrite_nets(&mut self, rewrites: &[NetRewrite]) {
        for rewrite in rewrites {
            self.nets[rewrite.net as usize].terminals = rewrite.terminals.clone();
            self.merged_pads.retain(|merged| merged.net != rewrite.net);
            self.merged_pads.extend(rewrite.merged_pads.iter().cloned());
            self.terminal_copper
                .retain(|extra| extra.net != rewrite.net);
            self.terminal_copper
                .extend(rewrite.terminal_copper.iter().copied());
        }
    }
}

/// A copper obstacle of `net` that its terminal `terminal` reaches
/// (`Board::terminal_copper`): kept copper of one piece with the
/// terminal's own, so that a search may end anywhere on the piece (a
/// differential pair's coupled run, whose two ends the pads' stubs join;
/// a pad merged into the terminal), on the obstacle's layers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalCopper {
    pub net: NetId,
    pub terminal: usize,
    pub obstacle: usize,
}

/// The gap between two adjacent SMD pads of one footprint on one layer
/// (`Board::pin_row_gaps`): the line between their centres, which a track
/// of another net than theirs crosses when it passes between them.
#[derive(Clone, Debug, PartialEq)]
pub struct PinRowGap {
    pub layer: usize,
    pub a: Point,
    pub b: Point,
    /// The two pads' nets: theirs pass freely.
    pub nets: [Option<NetId>; 2],
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
    /// The host's own pad on the layers its padstack names (a KiCad 9
    /// padstack's inner or back shape): the terminal's copper there, not
    /// another pad of its number (its zone connection is the terminal's,
    /// its tracks end on the terminal, its nodes are the terminal's).
    pub padstack: bool,
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
    /// A custom pad's frame (its copper's box at orientation 0, from the
    /// box's centre): the router takes it only for the measurement
    /// (`Config::measured_thermals`); routing gives a custom pad spokes
    /// along the axes of its copper's box, as before.
    pub custom: bool,
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

    /// How far the fill of `plane` keeps from its excluded region
    /// `polygon` (one of `plane.excluded`): another net's zone of a higher
    /// priority on the layer is filled first, up to its outline, and this
    /// zone keeps the larger of the two zones' clearances from that fill
    /// (KiCad's zone filler knocks other nets' fills out with their
    /// clearance; USSTRocketry's /Vin under GND's zone of priority 3, whose
    /// fill ends half a millimetre short of GND's outline). A keepout's
    /// region is cut exactly: 0.
    pub fn excluded_clearance(&self, plane: &Plane, polygon: &[Point]) -> f64 {
        self.planes
            .iter()
            .filter(|other| {
                other.net != plane.net
                    && other.layer == plane.layer
                    && other.polygon.as_slice() == polygon
            })
            .map(|other| {
                self.classes[plane.class]
                    .clearance
                    .max(self.classes[other.class].clearance)
            })
            .fold(0.0, f64::max)
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
    /// Its layers where it is not a through via (`Board::via_spans`).
    pub span: Option<ViaSpan>,
}

impl Via {
    /// Whether the via is copper (and a hole) on `layer`.
    pub fn on_layer(&self, layer: usize) -> bool {
        self.span.is_none_or(|span| span.contains(layer))
    }

    /// Its layers as a mask, on a board of `layers` copper layers.
    pub fn mask(&self, layers: usize) -> LayerMask {
        self.span.map_or(
            if layers >= 32 {
                LayerMask::MAX
            } else {
                (1 << layers) - 1
            },
            |span| span.mask(),
        )
    }
}

#[derive(Clone, Debug, Default)]
pub struct NetRoute {
    pub segments: Vec<Segment>,
    pub vias: Vec<Via>,
}
