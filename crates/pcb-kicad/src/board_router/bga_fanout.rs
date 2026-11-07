// Copyright (C) 2026 Toit contributors.

//! Prepared fan-outs for BGAs (docs/router.md, "Prepared fan-outs for BGAs
//! (2026-11-04)"; router settings `bga_fanout`, `PCB_ROUTER_BGA_FANOUT`;
//! on by default).
//!
//! A ball of a grid array's inner rings leaves the part only by a via:
//! once the channels between the outer balls are taken (one track a
//! channel at Glasgow's 0.8 mm pitch), no track passes. The search takes
//! its vias at the class's size, which at 0.8 mm with 0.35 mm pads and
//! 0.15 mm clearance does not fit in the diagonal gap between four balls
//! (0.6 mm against 0.43 mm of room), so the inner balls stayed open; the
//! designer puts a 0.4/0.2 mm via there, one of the project's predefined
//! sizes.
//!
//! The plan (`plan`), a function of the lowered board and the settings
//! alone, so every lowering of one board places the same copper: per grid
//! array (SMD pads of one size, round or square, at least four rows and
//! columns of three, the pitch at most 1.27 mm), the rings no surface
//! track serves (ring 0 and as many more as tracks of the narrowest class
//! fit between two adjacent balls) take a dog-bone: a stub from the ball's
//! centre along the diagonal to the gap's centre and a through via there,
//! in the gap pointing away from the part's centre (each quadrant's vias
//! on one grid, so the channels between their rows stay straight; a ball
//! on a centre line points to its net's other pads), else in the next gap
//! that fits. The via is the largest of the class's and the project's via
//! sizes within the board's minimums whose copper keeps every clearance
//! from the lowered board's copper, holes and rule areas of other nets on
//! every layer (a part on the back under the array), the hole rules from
//! every hole, and the clearances from the vias and stubs planned before
//! it; the stub at the class's width, else the neck. A ball with a via in
//! its pad (`via_in_pad`) or a board's own via keeps that.
//!
//! The lowering makes each planned stub and via fixed copper of the
//! ball's net (`<ball> fan-out stub`, `... fan-out via ring` on the ball's
//! layer, `... fan-out via` on the others, `... fan-out hole`); on the
//! other layers the via is the ball's terminal (a padstack's copper there,
//! solid to a pour), so a route may end on it on any layer. `emit_routes`
//! writes a planned stub and via only where the net uses them (a route of
//! the net touches the via off the ball's layer or the stub beyond the
//! ball, or the net's pour lies on another layer): an escape the routes
//! did not take is dropped, its room was only kept from other nets.

use super::*;
use pcb_router as core;

/// `PCB_ROUTER_BGA_FANOUT` when set: overrides `bga_fanout` (`0` or empty
/// off, anything else on).
pub fn bga_fanout_override() -> Option<bool> {
    let value = std::env::var("PCB_ROUTER_BGA_FANOUT").ok()?;
    Some(!(value.is_empty() || value == "0"))
}

/// Whether the lowering plans fan-outs: the override, else the setting,
/// else on (since 2026-11-04: Glasgow's U30, the one task board whose
/// inner balls the search cannot leave, gains on every measure; no other
/// board's array takes a plan, so nothing else moves).
pub(crate) fn bga_fanout_on(config: &KiCadBoardRouterConfig) -> bool {
    bga_fanout_override().unwrap_or(config.bga_fanout.unwrap_or(true))
}

/// Grid arrays have at least this many pads.
const MIN_PADS: usize = 16;
/// And at least this many rows and columns of three pads or more.
const MIN_ROWS: usize = 4;
/// And their pitch is at most this (mm).
const MAX_PITCH: f64 = 1.27;
/// What every planned clearance keeps beyond the rule (mm): the file's
/// nanometres and KiCad's checks never see an exact fit fail.
const SAFETY: f64 = 1.0e-5;

/// A footprint's SMD pads as the lowering made them (`lower_board`
/// collects them with the fan-out on): per pad its copper obstacle, and
/// whether it is the footprint's exposed pad.
pub(crate) struct FanoutPart {
    pub reference: String,
    pub pads: Vec<(usize, bool)>,
}

/// One planned dog-bone.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PlannedFanout {
    pub net: core::NetId,
    /// The ball's copper obstacle.
    pub pad: usize,
    pub label: String,
    /// The ball's layer.
    pub layer: usize,
    pub start: [f64; 2],
    pub via: [f64; 2],
    pub diameter: f64,
    pub drill: f64,
    pub width: f64,
    pub ring: usize,
    /// The via's span where it is not a through via (`via_spans`): the
    /// shortest the board allows from the ball's layer to a routable one.
    pub span: Option<core::ViaSpan>,
}

/// The span a dog-bone's via takes from a ball on `layer` of `net`: the
/// shortest span the board allows (`spans`) from that layer whose far
/// layer is routable for the net (one its class may use, not another
/// net's exclusive plane); None (a through via) where none is.
pub(crate) fn fanout_span(
    spans: &[core::ViaSpan],
    layer: usize,
    net: core::NetId,
    class: &core::RuleClass,
    planes: &[core::Plane],
) -> Option<core::ViaSpan> {
    let routable = |far: usize| {
        (class.layers == 0 || class.layers & (1 << far) != 0)
            && !planes
                .iter()
                .any(|plane| plane.layer == far && plane.net != net && plane.exclusive)
    };
    spans
        .iter()
        .filter(|span| span.top == layer || span.bottom == layer)
        .filter(|span| {
            routable(if span.top == layer {
                span.bottom
            } else {
                span.top
            })
        })
        .min_by_key(|span| span.depth())
        .copied()
}

/// A grid array's plan, for the report (`PCB_ROUTER_BGA_FANOUT_WHY`).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct PartPlan {
    pub reference: String,
    pub pitch: f64,
    pub rows: usize,
    pub columns: usize,
    /// Rings 0 to this less one leave on the surface.
    pub surface_rings: usize,
    /// Balls of a net in the rings that take a via.
    pub inner: usize,
    pub planned: usize,
    /// Inner balls that hold a via already (in the pad, or the board's).
    pub kept: usize,
    /// Inner balls whose net's own via fits a diagonal gap: the search's.
    pub searched: usize,
    /// Inner balls no gap and size fits, by label.
    pub refused: Vec<String>,
    /// Planned vias by size.
    pub sizes: BTreeMap<String, usize>,
}

/// The plan: the dog-bones, the parts' summaries, the work (distance
/// tests and pads visited, about a lowering's expression each).
#[derive(Default)]
pub(crate) struct FanoutPlan {
    pub fanouts: Vec<PlannedFanout>,
    pub parts: Vec<PartPlan>,
    pub work: u64,
}

/// A part's ball grid in its own frame.
struct Grid {
    /// The frame's axes: x along (cos, sin), y along (-sin, cos).
    axis: [f64; 2],
    origin: [f64; 2],
    pitch: f64,
    rows: usize,
    columns: usize,
    /// Per pad (the part's order), its centre in the frame.
    local: Vec<[f64; 2]>,
    /// The frame's box of the pad centres.
    bounds: [f64; 4],
    /// Per pad its ring (`rings_of`).
    rings: Vec<usize>,
    /// Per pad the side of the nearer way out along each axis (`rings_of`).
    toward: Vec<[f64; 2]>,
}

impl Grid {
    fn to_local(&self, point: [f64; 2]) -> [f64; 2] {
        let d = [point[0] - self.origin[0], point[1] - self.origin[1]];
        [
            d[0] * self.axis[0] + d[1] * self.axis[1],
            -d[0] * self.axis[1] + d[1] * self.axis[0],
        ]
    }

    fn to_board(&self, point: [f64; 2]) -> [f64; 2] {
        [
            self.origin[0] + point[0] * self.axis[0] - point[1] * self.axis[1],
            self.origin[1] + point[0] * self.axis[1] + point[1] * self.axis[0],
        ]
    }
}

/// Cluster centres of `values` (gaps over `tolerance` part them) and the
/// size of each cluster.
fn clusters(values: &mut [f64], tolerance: f64) -> Vec<usize> {
    values.sort_by(f64::total_cmp);
    let mut sizes: Vec<usize> = Vec::new();
    let mut last = f64::NEG_INFINITY;
    for &value in values.iter() {
        if value - last > tolerance || sizes.is_empty() {
            sizes.push(0);
        }
        *sizes.last_mut().unwrap() += 1;
        last = value;
    }
    sizes
}

/// A pad's half extent from its centre along `direction` (bisection on
/// its copper).
pub(crate) fn half_extent(
    shape: &core::Shape,
    center: [f64; 2],
    direction: [f64; 2],
    limit: f64,
) -> f64 {
    let (mut inside, mut outside) = (0.0, limit);
    for _ in 0..30 {
        let middle = (inside + outside) / 2.0;
        let point = [
            center[0] + direction[0] * middle,
            center[1] + direction[1] * middle,
        ];
        if shape.distance_to_point(point) <= 0.0 {
            inside = middle;
        } else {
            outside = middle;
        }
    }
    inside
}

/// Per ball of an axis-aligned grid (`local`: centres, `pitch`): its ring,
/// the steps along the axes to the nearest place without a ball (outside
/// the array or a depopulated middle) less one, so the outer ring and the
/// balls beside an empty middle are ring 0; and per axis the side where
/// that place is nearer (-1 or 1, 0 where both are as near). One pure
/// function of the centres, so the placer's frame and the router's agree.
pub(crate) fn rings_of(local: &[[f64; 2]], pitch: f64) -> (Vec<usize>, Vec<[f64; 2]>) {
    let Some(origin) = local
        .iter()
        .copied()
        .reduce(|a, b| [a[0].min(b[0]), a[1].min(b[1])])
    else {
        return (Vec::new(), Vec::new());
    };
    let cell = |p: [f64; 2]| {
        (
            ((p[0] - origin[0]) / pitch).round() as i64,
            ((p[1] - origin[1]) / pitch).round() as i64,
        )
    };
    let occupied: BTreeSet<(i64, i64)> = local.iter().map(|p| cell(*p)).collect();
    let steps = |(x, y): (i64, i64), (dx, dy): (i64, i64)| {
        let mut n = 1;
        while occupied.contains(&(x + n * dx, y + n * dy)) {
            n += 1;
        }
        n
    };
    let side = |minus: i64, plus: i64| match minus.cmp(&plus) {
        std::cmp::Ordering::Less => -1.0,
        std::cmp::Ordering::Greater => 1.0,
        std::cmp::Ordering::Equal => 0.0,
    };
    local
        .iter()
        .map(|p| {
            let at = cell(*p);
            let [left, right, up, down] =
                [(-1, 0), (1, 0), (0, -1), (0, 1)].map(|direction| steps(at, direction));
            let ring = (left.min(right).min(up).min(down) - 1) as usize;
            (ring, [side(left, right), side(up, down)])
        })
        .unzip()
}

/// The part's ball grid (`pads`: centre and copper of its SMD pads but the
/// exposed one), or `None` where it is no grid array.
fn grid_of(pads: &[([f64; 2], &core::Shape)], work: &mut u64) -> Option<Grid> {
    if pads.len() < MIN_PADS {
        return None;
    }
    // Round or square pads of one size: a QFN's or a connector's are long.
    let sizes: Vec<[f64; 2]> = pads
        .iter()
        .map(|(_, shape)| {
            let bounds = shape.aabb();
            [
                bounds.maximum[0] - bounds.minimum[0],
                bounds.maximum[1] - bounds.minimum[1],
            ]
        })
        .collect();
    let squat = sizes
        .iter()
        .filter(|size| size[0].max(size[1]) <= 1.5 * size[0].min(size[1]))
        .count();
    let mut areas: Vec<f64> = sizes.iter().map(|size| size[0] * size[1]).collect();
    areas.sort_by(f64::total_cmp);
    let middle = areas[areas.len() / 2];
    let alike = areas
        .iter()
        .filter(|area| **area >= middle / 1.5 && **area <= middle * 1.5)
        .count();
    if 4 * squat < 3 * pads.len() || 4 * alike < 3 * pads.len() {
        return None;
    }
    // The nearest neighbours: the pitch and the frame's angle.
    let mut distances = Vec::with_capacity(pads.len());
    let mut angles = Vec::with_capacity(pads.len());
    for (index, (a, _)) in pads.iter().enumerate() {
        let mut best: Option<(f64, [f64; 2])> = None;
        for (other, (b, _)) in pads.iter().enumerate() {
            if other == index {
                continue;
            }
            let d = core::geometry::distance(*a, *b);
            if d > 1.0e-3 && best.is_none_or(|(least, _)| d < least) {
                best = Some((d, [b[0] - a[0], b[1] - a[1]]));
            }
        }
        *work += pads.len() as u64;
        let (d, v) = best?;
        distances.push(d);
        // The angle folded into a quarter turn around 0.
        let quarter = std::f64::consts::FRAC_PI_2;
        let mut angle = v[1].atan2(v[0]).rem_euclid(quarter);
        if angle >= quarter / 2.0 {
            angle -= quarter;
        }
        angles.push(angle);
    }
    distances.sort_by(f64::total_cmp);
    angles.sort_by(f64::total_cmp);
    let pitch = distances[distances.len() / 2];
    if pitch > MAX_PITCH + 1.0e-6 {
        return None;
    }
    let angle = angles[angles.len() / 2];
    let origin = pads[0].0;
    let mut grid = Grid {
        axis: [angle.cos(), angle.sin()],
        origin,
        pitch,
        rows: 0,
        columns: 0,
        local: Vec::new(),
        bounds: [0.0; 4],
        rings: Vec::new(),
        toward: Vec::new(),
    };
    grid.local = pads
        .iter()
        .map(|(center, _)| grid.to_local(*center))
        .collect();
    let tolerance = 0.3 * pitch;
    let mut xs: Vec<f64> = grid.local.iter().map(|p| p[0]).collect();
    let mut ys: Vec<f64> = grid.local.iter().map(|p| p[1]).collect();
    let columns = clusters(&mut xs, tolerance);
    let rows = clusters(&mut ys, tolerance);
    if columns.iter().filter(|size| **size >= 3).count() < MIN_ROWS
        || rows.iter().filter(|size| **size >= 3).count() < MIN_ROWS
    {
        return None;
    }
    grid.columns = columns.len();
    grid.rows = rows.len();
    grid.bounds = [xs[0], ys[0], xs[xs.len() - 1], ys[ys.len() - 1]];
    let (rings, toward) = rings_of(&grid.local, pitch);
    grid.rings = rings;
    grid.toward = toward;
    if grid.rings.iter().all(|ring| *ring == 0) {
        return None;
    }
    Some(grid)
}

/// Whether a via of `diameter` and `drill` keeps `clearance` and the hole
/// clearance from the balls around a diagonal gap of an array at `pitch`
/// with balls `half` wide from their centres (the footprint alone).
fn search_via_fits(
    pitch: f64,
    half: f64,
    diameter: f64,
    drill: f64,
    clearance: f64,
    config: &KiCadBoardRouterConfig,
) -> bool {
    let room = pitch / std::f64::consts::SQRT_2 - half;
    diameter / 2.0 + clearance + SAFETY <= room
        && drill / 2.0 + config.hole_clearance_mm + SAFETY <= room
}

/// The via sizes to try, largest first: the class's and the project's
/// predefined sizes within the board's via minimums.
pub(crate) fn via_sizes(config: &KiCadBoardRouterConfig, class: &core::RuleClass) -> Vec<[f64; 2]> {
    let (predefined, minimums) = config
        .project_rules
        .as_ref()
        .map_or((Vec::new(), [0.0; 3]), |report| {
            (report.via_sizes.clone(), report.via_minimums)
        });
    let mut sizes = predefined;
    sizes.push([class.via_diameter, class.via_drill]);
    sizes.retain(|&[diameter, drill]| {
        drill > 0.0
            && diameter > drill
            && diameter + 1.0e-9 >= minimums[0]
            && drill + 1.0e-9 >= minimums[1]
            && (diameter - drill) / 2.0 + 1.0e-9 >= minimums[2]
    });
    sizes.sort_by(|a, b| b[0].total_cmp(&a[0]).then(b[1].total_cmp(&a[1])));
    sizes.dedup_by(|a, b| (a[0] - b[0]).abs() < 1.0e-9 && (a[1] - b[1]).abs() < 1.0e-9);
    sizes
}

/// The context the legality tests read.
struct Context<'a> {
    obstacles: &'a [core::Obstacle],
    nets: &'a [core::Net],
    classes: &'a [core::RuleClass],
    /// Sorted: SMD pads no via may overlap (`Board::no_via_pads`).
    no_via_pads: &'a [usize],
    hole_clearance: f64,
    hole_to_hole: f64,
}

impl Context<'_> {
    /// `core::Board::copper_clearance` before the board exists.
    fn clearance(&self, net: core::NetId, obstacle: &core::Obstacle) -> f64 {
        let own = self.classes[self.nets[net as usize].class].clearance;
        let other = obstacle.clearance_override.unwrap_or_else(|| {
            let class = obstacle.net.map_or(0.0, |other| {
                self.classes[self.nets[other as usize].class].clearance
            });
            obstacle.clearance.max(class)
        });
        own.max(other)
    }

    /// Whether a through via of `net` at `at` (radius, drill radius) keeps
    /// every rule against the obstacles `near` (indices).
    #[allow(clippy::too_many_arguments)]
    fn via_clear(
        &self,
        net: core::NetId,
        at: [f64; 2],
        radius: f64,
        hole: f64,
        layers: core::LayerMask,
        near: &[usize],
        work: &mut u64,
    ) -> bool {
        *work += near.len() as u64;
        near.iter().all(|&index| {
            let obstacle = &self.obstacles[index];
            // A via of a span meets only what is on its layers.
            if !obstacle.blocks_vias
                || (layers != core::LayerMask::MAX && obstacle.layers & layers == 0)
            {
                return true;
            }
            let gap = obstacle.shape.distance_to_point(at);
            match obstacle.kind {
                core::ObstacleKind::Hole => {
                    gap >= hole + self.hole_to_hole + SAFETY
                        && (obstacle.net == Some(net)
                            || gap >= radius + self.hole_clearance + SAFETY)
                }
                core::ObstacleKind::Keepout => gap >= radius + obstacle.clearance + SAFETY,
                core::ObstacleKind::Copper if obstacle.net == Some(net) => {
                    // The net's own SMD pads keep the via's ring out.
                    self.no_via_pads.binary_search(&index).is_err() || gap >= radius + SAFETY
                }
                core::ObstacleKind::Copper => {
                    gap >= radius + self.clearance(net, obstacle) + SAFETY
                        && gap >= hole + self.hole_clearance + SAFETY
                }
            }
        })
    }

    /// Whether a track of `net` from `a` to `b`, `width` wide, on `layer`
    /// keeps every rule against the obstacles `near`.
    #[allow(clippy::too_many_arguments)]
    fn stub_clear(
        &self,
        net: core::NetId,
        layer: usize,
        a: [f64; 2],
        b: [f64; 2],
        width: f64,
        near: &[usize],
        work: &mut u64,
    ) -> bool {
        *work += near.len() as u64;
        near.iter().all(|&index| {
            let obstacle = &self.obstacles[index];
            if !obstacle.blocks_tracks || obstacle.layers & (1 << layer) == 0 {
                return true;
            }
            let gap = obstacle.shape.distance_to_segment(a, b);
            match obstacle.kind {
                core::ObstacleKind::Hole => {
                    obstacle.net == Some(net) || gap >= width / 2.0 + self.hole_clearance + SAFETY
                }
                core::ObstacleKind::Keepout => gap >= width / 2.0 + obstacle.clearance + SAFETY,
                core::ObstacleKind::Copper if obstacle.net == Some(net) => true,
                core::ObstacleKind::Copper => {
                    gap >= width / 2.0 + self.clearance(net, obstacle) + SAFETY
                }
            }
        })
    }
}

/// Whether a planned dog-bone keeps its rules against those planned before
/// (`planned`) of other nets (and the hole rule against all).
fn clear_of_planned(
    context: &Context,
    candidate: &PlannedFanout,
    planned: &[PlannedFanout],
    work: &mut u64,
) -> bool {
    *work += planned.len() as u64;
    let own = context.classes[context.nets[candidate.net as usize].class].clearance;
    let mask =
        |fanout: &PlannedFanout| fanout.span.map_or(core::LayerMask::MAX, |span| span.mask());
    planned.iter().all(|other| {
        let apart = core::geometry::distance(candidate.via, other.via);
        // Vias of disjoint spans meet nowhere (no hole-to-hole either).
        let vias_meet = mask(candidate) & mask(other) != 0;
        if vias_meet
            && apart < (candidate.drill + other.drill) / 2.0 + context.hole_to_hole + SAFETY
        {
            return false;
        }
        if other.net == candidate.net {
            return true;
        }
        let clearance = own.max(context.classes[context.nets[other.net as usize].class].clearance);
        let vias =
            !vias_meet || apart >= (candidate.diameter + other.diameter) / 2.0 + clearance + SAFETY;
        let same_layer = candidate.layer == other.layer;
        let stub_via = |stub: &PlannedFanout, via: &PlannedFanout| {
            core::geometry::point_segment_distance(via.via, stub.start, stub.via)
                >= stub.width / 2.0 + via.diameter / 2.0 + clearance + SAFETY
        };
        let stubs = !same_layer
            || core::geometry::segment_segment_distance(
                candidate.start,
                candidate.via,
                other.start,
                other.via,
            ) >= (candidate.width + other.width) / 2.0 + clearance + SAFETY;
        vias && stubs && stub_via(candidate, other) && stub_via(other, candidate)
    })
}

/// The fan-outs of the lowered board: per grid array of `parts`, the
/// dog-bones of its inner rings' balls (see the module's comment).
/// `holding`: the balls (obstacles) that hold a via already.
#[allow(clippy::too_many_arguments)]
pub(crate) fn plan(
    parts: &[FanoutPart],
    obstacles: &[core::Obstacle],
    nets: &[core::Net],
    classes: &[core::RuleClass],
    no_via_pads: &[usize],
    holding: &BTreeSet<usize>,
    spans: (&[core::ViaSpan], &[core::Plane]),
    config: &KiCadBoardRouterConfig,
) -> FanoutPlan {
    let (spans, planes) = spans;
    let mut sorted_no_via = no_via_pads.to_vec();
    sorted_no_via.sort_unstable();
    let context = Context {
        obstacles,
        nets,
        classes,
        no_via_pads: &sorted_no_via,
        hole_clearance: config.hole_clearance_mm,
        hole_to_hole: config.hole_to_hole_clearance_mm,
    };
    let neck = config
        .neck_width_mm
        .map(|neck| neck.max(config.width_floor()));
    let mut plan = FanoutPlan::default();
    for part in parts {
        let pads: Vec<usize> = part
            .pads
            .iter()
            .filter(|(index, exposed)| !exposed && obstacles[*index].layers.count_ones() == 1)
            .map(|(index, _)| *index)
            .collect();
        let shapes: Vec<([f64; 2], &core::Shape)> = pads
            .iter()
            .map(|&index| {
                let bounds = obstacles[index].shape.aabb();
                (
                    [
                        (bounds.minimum[0] + bounds.maximum[0]) / 2.0,
                        (bounds.minimum[1] + bounds.maximum[1]) / 2.0,
                    ],
                    &obstacles[index].shape,
                )
            })
            .collect();
        // The part's work (`work`) is charged only where it plans a
        // dog-bone: a board the plan leaves alone lowers as with it off,
        // its work too (the few microseconds of tests uncharged).
        let mut work = 0;
        let Some(grid) = grid_of(&shapes, &mut work) else {
            continue;
        };
        // The narrowest class of the balls' nets: how many tracks pass
        // between two adjacent balls.
        let ball_nets: Vec<core::NetId> = pads
            .iter()
            .filter_map(|&index| obstacles[index].net)
            .collect();
        let Some(narrow) = ball_nets
            .iter()
            .map(|net| classes[nets[*net as usize].class])
            .min_by(|a, b| (a.trace_width + a.clearance).total_cmp(&(b.trace_width + b.clearance)))
        else {
            continue;
        };
        let (cos, sin) = (grid.axis[0], grid.axis[1]);
        let mut halves: Vec<f64> = shapes
            .iter()
            .map(|(center, shape)| {
                half_extent(shape, *center, [cos, sin], grid.pitch).max(half_extent(
                    shape,
                    *center,
                    [-sin, cos],
                    grid.pitch,
                ))
            })
            .collect();
        halves.sort_by(f64::total_cmp);
        let half = halves[halves.len() / 2];
        let gap = grid.pitch - 2.0 * half;
        let tracks = ((gap - narrow.clearance) / (narrow.trace_width + narrow.clearance) + 1.0e-6)
            .floor()
            .max(0.0) as usize;
        let surface_rings = 1 + tracks;
        let mut summary = PartPlan {
            reference: part.reference.clone(),
            pitch: grid.pitch,
            rows: grid.rows,
            columns: grid.columns,
            surface_rings,
            ..PartPlan::default()
        };
        // The obstacles near the part.
        let field = shapes
            .iter()
            .fold(shapes[0].1.aabb(), |bounds, (_, shape)| {
                bounds.union(shape.aabb())
            })
            .inflated(grid.pitch + 1.0);
        let near: Vec<usize> = obstacles
            .iter()
            .enumerate()
            .filter(|(_, obstacle)| obstacle.shape.aabb().intersects(field))
            .map(|(index, _)| index)
            .collect();
        work += obstacles.len() as u64 / 8;
        // The outer rings first: their balls take their own gaps.
        let mut order: Vec<usize> = (0..pads.len())
            .filter(|&position| grid.rings[position] >= surface_rings)
            .collect();
        order.sort_by_key(|&position| (grid.rings[position], position));
        let first = plan.fanouts.len();
        for position in order {
            let index = pads[position];
            let obstacle = &obstacles[index];
            let Some(net) = obstacle.net else {
                continue;
            };
            summary.inner += 1;
            if holding.contains(&index) {
                summary.kept += 1;
                continue;
            }
            // Where the net's own via fits a diagonal gap, the search puts
            // its dog-bone there itself (ZSWatch's NORA-B106 at 1.1 mm,
            // vd-rd's BGA-96): the plan prepares only what it cannot.
            let own = classes[nets[net as usize].class];
            if search_via_fits(
                grid.pitch,
                half,
                own.via_diameter,
                own.via_drill,
                own.clearance,
                config,
            ) {
                summary.searched += 1;
                continue;
            }
            let layer = obstacle.layers.trailing_zeros() as usize;
            let local = grid.local[position];
            // Where the net's other pads lie, in the frame (a ball on a
            // centre line points there).
            let target = {
                let others: Vec<[f64; 2]> = nets[net as usize]
                    .terminals
                    .iter()
                    .filter(|terminal| terminal.pad != index)
                    .map(|terminal| grid.to_local(terminal.anchor))
                    .collect();
                if others.is_empty() {
                    [1.0, 1.0]
                } else {
                    let n = others.len() as f64;
                    [
                        others.iter().map(|p| p[0]).sum::<f64>() / n - local[0],
                        others.iter().map(|p| p[1]).sum::<f64>() / n - local[1],
                    ]
                }
            };
            // Toward the nearer way out along each axis (away from the
            // array's middle on a full grid), else toward the net's pads.
            let sign = |nearer: f64, toward: f64| {
                if nearer != 0.0 {
                    nearer
                } else if toward < 0.0 {
                    -1.0
                } else {
                    1.0
                }
            };
            let su = sign(grid.toward[position][0], target[0]);
            let sv = sign(grid.toward[position][1], target[1]);
            let class = classes[nets[net as usize].class];
            let mut widths = vec![class.trace_width];
            if let Some(neck) = neck.filter(|neck| *neck < class.trace_width) {
                widths.push(neck);
            }
            let start = shapes[position].0;
            let label = obstacle.label.clone();
            let mut found = None;
            // The via: the shortest span the board allows to a routable
            // layer (a micro via at its own size), else a through via.
            let span = fanout_span(spans, layer, net, &class, planes);
            let through_sizes = via_sizes(config, &class);
            let choices: Vec<(Option<core::ViaSpan>, Vec<[f64; 2]>)> = match span {
                Some(span) => vec![
                    (
                        Some(span),
                        span.size
                            .map_or_else(|| through_sizes.clone(), |size| vec![size]),
                    ),
                    (None, through_sizes),
                ],
                None => vec![(None, through_sizes)],
            };
            'sites: for (span, sizes) in &choices {
                let layers = span.map_or(core::LayerMask::MAX, |span| span.mask());
                for (du, dv) in [(su, sv), (su, -sv), (-su, sv), (-su, -sv)] {
                    let site = grid.to_board([
                        local[0] + du * grid.pitch / 2.0,
                        local[1] + dv * grid.pitch / 2.0,
                    ]);
                    for &[diameter, drill] in sizes {
                        if !context.via_clear(
                            net,
                            site,
                            diameter / 2.0,
                            drill / 2.0,
                            layers,
                            &near,
                            &mut work,
                        ) {
                            continue;
                        }
                        for &width in &widths {
                            let candidate = PlannedFanout {
                                net,
                                pad: index,
                                label: label.clone(),
                                layer,
                                start,
                                via: site,
                                diameter,
                                drill,
                                width,
                                ring: grid.rings[position],
                                span: *span,
                            };
                            if context.stub_clear(net, layer, start, site, width, &near, &mut work)
                                && clear_of_planned(
                                    &context,
                                    &candidate,
                                    &plan.fanouts[first..],
                                    &mut work,
                                )
                            {
                                found = Some(candidate);
                                break 'sites;
                            }
                        }
                    }
                }
            }
            match found {
                Some(fanout) => {
                    *summary
                        .sizes
                        .entry(format!("{}/{}", fanout.diameter, fanout.drill))
                        .or_default() += 1;
                    summary.planned += 1;
                    plan.fanouts.push(fanout);
                }
                None => summary.refused.push(label),
            }
        }
        if summary.planned > 0 {
            plan.work += work;
        }
        plan.parts.push(summary);
    }
    plan
}

/// A grid array whose inner balls take dog-bones, as the placer keeps the
/// far side free under them (`far_side_balls`).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FarSideArray {
    /// Index of the footprint in the board's order.
    pub footprint: usize,
    pub pitch: f64,
    pub diameter: f64,
    pub drill: f64,
    /// The array's balls (pad numbers): the placer finds their rings and
    /// sides in its own frame (`rings_of`).
    pub balls: Vec<String>,
    /// Those that take a dog-bone.
    pub inner: Vec<String>,
}

/// A grid array as its footprint alone shows it (`board_arrays`): its
/// balls (the SMD pads of one copper layer but the exposed pad), the grid
/// in the router's frame, the narrowest class of its balls' nets, the
/// balls' half width from their centres and the tracks of that class that
/// pass between two adjacent balls.
struct BoardArray<'a> {
    footprint: usize,
    own: Vec<&'a via_in_pad::BoardPad>,
    grid: Grid,
    narrow: &'a KiCadConnectionRoutingRules,
    half: f64,
    tracks: usize,
}

/// The board's grid arrays as the plan finds them (`grid_of` on each
/// footprint's balls), judged by the footprint alone: one detection for
/// the router's plan's placer side (`far_side_balls`) and the placer's
/// room around the array (`array_rooms`).
fn board_arrays<'a>(
    footprints: usize,
    pads: &'a [via_in_pad::BoardPad],
    config: &'a KiCadBoardRouterConfig,
) -> Vec<BoardArray<'a>> {
    let mut arrays = Vec::new();
    for footprint in 0..footprints {
        let own: Vec<&via_in_pad::BoardPad> = pads
            .iter()
            .filter(|pad| pad.footprint == footprint && pad.smd && pad.layers.count_ones() == 1)
            .collect();
        if own.len() < MIN_PADS {
            continue;
        }
        let boxes: Vec<PadBox> = own
            .iter()
            .map(|pad| {
                let bounds = pad.shape.aabb();
                PadBox {
                    number: pad.number.clone(),
                    center: pad.center,
                    smd: true,
                    bounds: [bounds.minimum, bounds.maximum],
                }
            })
            .collect();
        let exposed = exposed_pad(&boxes);
        let own: Vec<&via_in_pad::BoardPad> = own
            .into_iter()
            .enumerate()
            .filter(|(index, _)| !exposed.contains(index))
            .map(|(_, pad)| pad)
            .collect();
        let shapes: Vec<([f64; 2], &core::Shape)> =
            own.iter().map(|pad| (pad.center, &pad.shape)).collect();
        let Some(grid) = grid_of(&shapes, &mut 0) else {
            continue;
        };
        let rules: Vec<&KiCadConnectionRoutingRules> = own
            .iter()
            .filter_map(|pad| via_in_pad::rules_of(config, pad.net.as_deref()))
            .collect();
        let Some(narrow) = rules.into_iter().min_by(|a, b| {
            (a.trace_width_mm + a.clearance_mm).total_cmp(&(b.trace_width_mm + b.clearance_mm))
        }) else {
            continue;
        };
        let (cos, sin) = (grid.axis[0], grid.axis[1]);
        let mut halves: Vec<f64> = shapes
            .iter()
            .map(|(center, shape)| {
                half_extent(shape, *center, [cos, sin], grid.pitch).max(half_extent(
                    shape,
                    *center,
                    [-sin, cos],
                    grid.pitch,
                ))
            })
            .collect();
        halves.sort_by(f64::total_cmp);
        let half = halves[halves.len() / 2];
        let gap = grid.pitch - 2.0 * half;
        let tracks = ((gap - narrow.clearance_mm) / (narrow.trace_width_mm + narrow.clearance_mm)
            + 1.0e-6)
            .floor()
            .max(0.0) as usize;
        arrays.push(BoardArray {
            footprint,
            own,
            grid,
            narrow,
            half,
            tracks,
        });
    }
    arrays
}

/// The balls of the board's grid arrays that take a dog-bone, judged by
/// the footprint alone (its balls and their nets' rules; not the board's
/// other parts, which the placer moves), with the largest via size whose
/// copper and drill keep their clearances from the four balls around a
/// diagonal gap: each via goes through the board, so the far side under
/// it is no place for another part's pads (the placer's
/// `via_in_pad_far_side`). Empty with the fan-out off.
pub(crate) fn far_side_balls(pcb: &Expr, config: &KiCadBoardRouterConfig) -> Vec<FarSideArray> {
    if !bga_fanout_on(config) {
        return Vec::new();
    }
    let Ok((footprints, pads)) = via_in_pad::board_pads(pcb, config) else {
        return Vec::new();
    };
    let (predefined, minimums) = config
        .project_rules
        .as_ref()
        .map_or((Vec::new(), [0.0; 3]), |report| {
            (report.via_sizes.clone(), report.via_minimums)
        });
    let mut balls = Vec::new();
    for array in board_arrays(footprints.len(), &pads, config) {
        let BoardArray {
            footprint,
            own,
            grid,
            narrow,
            half,
            tracks,
        } = array;
        // The room from a diagonal gap's centre to the balls around it.
        let room = grid.pitch / std::f64::consts::SQRT_2 - half;
        let mut sizes = predefined.clone();
        sizes.push([narrow.via_size_mm, narrow.via_drill_mm]);
        sizes.retain(|&[diameter, drill]| {
            drill > 0.0
                && diameter > drill
                && diameter + 1.0e-9 >= minimums[0]
                && drill + 1.0e-9 >= minimums[1]
                && diameter / 2.0 + narrow.clearance_mm + SAFETY <= room
                && drill / 2.0 + config.hole_clearance_mm + SAFETY <= room
        });
        let Some(&[diameter, drill]) = sizes.iter().max_by(|a, b| a[0].total_cmp(&b[0])) else {
            continue;
        };
        let inner: Vec<String> = own
            .iter()
            .enumerate()
            .filter(|(position, pad)| {
                grid.rings[*position] > tracks
                    && pad.net.is_some()
                    && via_in_pad::rules_of(config, pad.net.as_deref()).is_some_and(|rules| {
                        !search_via_fits(
                            grid.pitch,
                            half,
                            rules.via_size_mm,
                            rules.via_drill_mm,
                            rules.clearance_mm,
                            config,
                        )
                    })
            })
            .map(|(_, pad)| pad.number.clone())
            .collect();
        if !inner.is_empty() {
            balls.push(FarSideArray {
                footprint,
                pitch: grid.pitch,
                diameter,
                drill,
                balls: own.iter().map(|pad| pad.number.clone()).collect(),
                inner,
            });
        }
    }
    balls
}

/// A grid array's escapes as the placer keeps room for them
/// (`array_rooms`; the placer's `bga_room`, docs/placer.md, "Room around
/// grid arrays (2026-11-04)").
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ArrayRoom {
    /// Index of the footprint in the board's order.
    pub footprint: usize,
    pub pitch: f64,
    /// The array's balls (pad numbers).
    pub balls: Vec<String>,
    /// The routable nets on its balls.
    pub nets: BTreeSet<String>,
    /// Rings 0 to this less one leave on the surface (the tracks of the
    /// narrowest class between two adjacent balls, plus one).
    pub surface_rings: usize,
    /// The deepest ring holding a ball of a routable net.
    pub deepest: usize,
    /// The narrowest class of the balls' nets: track, clearance, via
    /// diameter.
    pub track: f64,
    pub clearance: f64,
    pub via: f64,
    /// The array's own decoupling capacitors (footprint indices, sorted):
    /// a `C` part of two pads between a ground net and a net of a ball.
    pub decoupling: Vec<usize>,
}

/// The board's grid arrays (`board_arrays`, the plan's detection) with the
/// numbers the placer sizes their room from. Independent of the fan-out
/// switch: the outer rings leave on the surface either way.
pub(crate) fn array_rooms(pcb: &Expr, config: &KiCadBoardRouterConfig) -> Vec<ArrayRoom> {
    let Ok((footprints, pads)) = via_in_pad::board_pads(pcb, config) else {
        return Vec::new();
    };
    let capacitor = |reference: &str| {
        reference
            .strip_prefix('C')
            .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
    };
    board_arrays(footprints.len(), &pads, config)
        .into_iter()
        .filter_map(|array| {
            let nets: BTreeSet<String> =
                array.own.iter().filter_map(|pad| pad.net.clone()).collect();
            let decoupling = (0..footprints.len())
                .filter(|footprint| capacitor(&footprints[*footprint].0))
                .filter(|footprint| {
                    let own: Vec<Option<&str>> = pads
                        .iter()
                        .filter(|pad| pad.footprint == *footprint)
                        .map(|pad| pad.net.as_deref())
                        .collect();
                    let [Some(a), Some(b)] = own[..] else {
                        return false;
                    };
                    let supplies = |rail: &str, return_: &str| {
                        crate::quality::ground(return_)
                            && !crate::quality::ground(rail)
                            && nets.contains(rail)
                    };
                    supplies(a, b) || supplies(b, a)
                })
                .collect();
            let deepest = array
                .own
                .iter()
                .enumerate()
                .filter(|(_, pad)| pad.net.is_some())
                .map(|(position, _)| array.grid.rings[position])
                .max()?;
            Some(ArrayRoom {
                footprint: array.footprint,
                pitch: array.grid.pitch,
                balls: array.own.iter().map(|pad| pad.number.clone()).collect(),
                nets,
                surface_rings: array.tracks + 1,
                deepest,
                track: array.narrow.trace_width_mm,
                clearance: array.narrow.clearance_mm,
                via: array.narrow.via_size_mm,
                decoupling,
            })
        })
        .collect()
}

/// Prints each distinct plan once a process (`PCB_ROUTER_BGA_FANOUT_WHY`).
pub(crate) fn report(plan: &FanoutPlan) {
    if std::env::var_os("PCB_ROUTER_BGA_FANOUT_WHY").is_none() {
        return;
    }
    static SEEN: std::sync::Mutex<BTreeSet<String>> = std::sync::Mutex::new(BTreeSet::new());
    for part in &plan.parts {
        let line = format!(
            "bga fan-out {}: pitch {:.3} mm, {} x {}, surface rings {}, inner balls {}, planned {} {:?}, kept {}, the search's (its via fits a gap) {}, refused {} {:?}",
            part.reference,
            part.pitch,
            part.rows,
            part.columns,
            part.surface_rings,
            part.inner,
            part.planned,
            part.sizes,
            part.kept,
            part.searched,
            part.refused.len(),
            part.refused
        );
        if SEEN.lock().is_ok_and(|mut seen| seen.insert(line.clone())) {
            eprintln!("{line}");
        }
    }
}

/// A planned dog-bone as the lowered board holds it (`fanouts_of`).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FanoutCopper {
    pub net: core::NetId,
    /// The ball's copper obstacle.
    pub pad: Option<usize>,
    pub layer: usize,
    pub start: [f64; 2],
    pub end: [f64; 2],
    pub width: f64,
    pub diameter: f64,
    pub drill: f64,
    /// Its span where it is not a through via (the hole's layers).
    pub span: Option<core::ViaSpan>,
}

/// The dog-bones the lowering placed (the obstacles it labels `<ball>
/// fan-out stub`, `... fan-out via ring` and `... fan-out hole`), in the
/// board's order.
pub(crate) fn fanouts_of(board: &core::Board) -> Vec<FanoutCopper> {
    // Per ball: the stub's layer, ends and width.
    type Stub = (usize, [f64; 2], [f64; 2], f64);
    let mut stubs: BTreeMap<&str, Stub> = BTreeMap::new();
    let mut rings: BTreeMap<&str, f64> = BTreeMap::new();
    let mut pads: BTreeMap<&str, usize> = BTreeMap::new();
    for (index, obstacle) in board.obstacles.iter().enumerate() {
        if obstacle.kind == core::ObstacleKind::Copper {
            pads.entry(obstacle.label.as_str()).or_insert(index);
        }
        match (&obstacle.shape, obstacle.kind) {
            (core::Shape::Capsule { start, end, radius }, core::ObstacleKind::Copper) => {
                if let Some(ball) = obstacle.label.strip_suffix(" fan-out stub") {
                    stubs.insert(
                        ball,
                        (
                            obstacle.layers.trailing_zeros() as usize,
                            *start,
                            *end,
                            2.0 * radius,
                        ),
                    );
                }
            }
            (core::Shape::Circle { radius, .. }, core::ObstacleKind::Copper) => {
                if let Some(ball) = obstacle.label.strip_suffix(" fan-out via ring") {
                    rings.insert(ball, 2.0 * radius);
                }
            }
            _ => {}
        }
    }
    board
        .obstacles
        .iter()
        .filter(|obstacle| obstacle.kind == core::ObstacleKind::Hole)
        .filter_map(|obstacle| {
            let ball = obstacle.label.strip_suffix(" fan-out hole")?;
            let core::Shape::Circle { radius, .. } = &obstacle.shape else {
                return None;
            };
            let (layer, start, end, width) = *stubs.get(ball)?;
            Some(FanoutCopper {
                net: obstacle.net?,
                pad: pads.get(ball).copied(),
                layer,
                start,
                end,
                width,
                diameter: *rings.get(ball)?,
                drill: 2.0 * radius,
                span: (obstacle.layers != board.all_layers())
                    .then(|| {
                        board
                            .via_spans
                            .iter()
                            .find(|span| span.mask() == obstacle.layers)
                            .copied()
                    })
                    .flatten(),
            })
        })
        .collect()
}

/// Whether the net's route (or its pour) uses a dog-bone: a segment off
/// the ball's layer touches the via, a segment on it touches the via or
/// the stub beyond the ball, a via of the route touches the via, or a
/// pour of the net lies at the via on another layer.
pub(crate) fn fanout_used(
    board: &core::Board,
    fanout: &FanoutCopper,
    route: &core::NetRoute,
) -> bool {
    const TOUCH: f64 = 1.0e-6;
    let radius = fanout.diameter / 2.0;
    let pad = fanout.pad.map(|index| &board.obstacles[index].shape);
    // The stub beyond the ball: from where it leaves the ball's copper.
    let outside = pad.map_or(fanout.start, |shape| {
        let length = core::geometry::distance(fanout.start, fanout.end);
        let direction = if length > 0.0 {
            [
                (fanout.end[0] - fanout.start[0]) / length,
                (fanout.end[1] - fanout.start[1]) / length,
            ]
        } else {
            [0.0, 0.0]
        };
        let leave = half_extent(shape, fanout.start, direction, length);
        [
            fanout.start[0] + direction[0] * leave,
            fanout.start[1] + direction[1] * leave,
        ]
    });
    let on = |layer: usize| fanout.span.is_none_or(|span| span.contains(layer));
    let segment_uses = route.segments.iter().any(|segment| {
        let reach = segment.width / 2.0 + TOUCH;
        let to_via = core::geometry::point_segment_distance(fanout.end, segment.start, segment.end);
        if segment.layer != fanout.layer {
            return on(segment.layer) && to_via <= radius + reach;
        }
        to_via <= radius + reach
            || core::geometry::segment_segment_distance(
                segment.start,
                segment.end,
                outside,
                fanout.end,
            ) <= fanout.width / 2.0 + reach
    });
    let mask = fanout.span.map_or(core::LayerMask::MAX, |span| span.mask());
    let via_uses = route.vias.iter().any(|via| {
        via.mask(board.layer_count) & mask != 0
            && core::geometry::distance(via.at, fanout.end) <= radius + via.diameter / 2.0 + TOUCH
    });
    let pour_uses = board.planes.iter().any(|plane| {
        plane.net == fanout.net
            && plane.layer != fanout.layer
            && on(plane.layer)
            && core::geometry::point_in_polygon(fanout.end, &plane.polygon)
    });
    segment_uses || via_uses || pour_uses
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_rules::KiCadProjectRulesReport;

    fn class() -> core::RuleClass {
        core::RuleClass {
            trace_width: 0.15,
            clearance: 0.15,
            via_diameter: 0.6,
            via_drill: 0.3,
            layers: 0,
        }
    }

    /// A `size` x `size` array of round balls at `pitch` on F.Cu, each of
    /// its own net (a net of two terminals: the ball and a pad far away to
    /// the east), and the config with the project's via sizes.
    fn array(
        size: usize,
        pitch: f64,
        ball: f64,
    ) -> (
        Vec<FanoutPart>,
        Vec<core::Obstacle>,
        Vec<core::Net>,
        Vec<usize>,
    ) {
        let mut obstacles = Vec::new();
        let mut nets = Vec::new();
        let mut pads = Vec::new();
        for row in 0..size {
            for column in 0..size {
                let center = [column as f64 * pitch, row as f64 * pitch];
                let net = nets.len() as core::NetId;
                let index = obstacles.len();
                obstacles.push(core::Obstacle {
                    shape: core::Shape::Circle {
                        center,
                        radius: ball / 2.0,
                    },
                    layers: 1,
                    kind: core::ObstacleKind::Copper,
                    net: Some(net),
                    clearance: 0.0,
                    clearance_override: None,
                    blocks_tracks: true,
                    blocks_vias: true,
                    label: format!("U1.{}{}", (b'A' + row as u8) as char, column + 1),
                });
                pads.push((index, false));
                nets.push(core::Net {
                    name: format!("N{row}_{column}"),
                    class: 0,
                    terminals: vec![
                        core::Terminal {
                            anchor: center,
                            layers: 1,
                            pad: index,
                            contact: None,
                            label: String::new(),
                        },
                        core::Terminal {
                            anchor: [50.0, center[1]],
                            layers: 1,
                            pad: usize::MAX,
                            contact: None,
                            label: String::new(),
                        },
                    ],
                });
            }
        }
        let no_via: Vec<usize> = (0..obstacles.len()).collect();
        (
            vec![FanoutPart {
                reference: "U1".into(),
                pads,
            }],
            obstacles,
            nets,
            no_via,
        )
    }

    fn config() -> KiCadBoardRouterConfig {
        KiCadBoardRouterConfig {
            hole_clearance_mm: 0.25,
            hole_to_hole_clearance_mm: 0.25,
            project_rules: Some(KiCadProjectRulesReport {
                via_sizes: vec![[0.4, 0.2], [0.5, 0.3], [0.6, 0.3]],
                via_minimums: [0.4, 0.2, 0.05],
                ..KiCadProjectRulesReport::default()
            }),
            ..KiCadBoardRouterConfig::default()
        }
    }

    #[test]
    fn a_ten_by_ten_array_at_0_8_mm_gets_a_dog_bone_for_every_inner_ball() {
        let (parts, obstacles, nets, no_via) = array(10, 0.8, 0.35);
        let plan = super::plan(
            &parts,
            &obstacles,
            &nets,
            &[class()],
            &no_via,
            &BTreeSet::new(),
            (&[], &[]),
            &config(),
        );
        let part = &plan.parts[0];
        // One 0.15 mm track with 0.15 mm clearance between two 0.35 mm
        // balls at 0.8 mm: rings 0 and 1 leave on the surface.
        assert_eq!((part.rows, part.columns, part.surface_rings), (10, 10, 2));
        // The 6 x 6 middle: every ball a dog-bone, of the 0.4/0.2 via (the
        // class's 0.6/0.3 and the 0.5/0.3 do not fit the gap).
        assert_eq!(part.inner, 36);
        assert_eq!(part.planned, 36, "refused {:?}", part.refused);
        assert_eq!(part.sizes.get("0.4/0.2"), Some(&36));
        for fanout in &plan.fanouts {
            // In a diagonal gap: half a pitch off along both axes.
            let d = [
                (fanout.via[0] - fanout.start[0]).abs(),
                (fanout.via[1] - fanout.start[1]).abs(),
            ];
            assert!((d[0] - 0.4).abs() < 1.0e-9 && (d[1] - 0.4).abs() < 1.0e-9);
            // Pointing away from the middle (4.5 pitches is the centre of
            // 0..9): each quadrant's vias on one grid.
            let away = |at: f64, via: f64| (via - at).signum() == (at - 3.6).signum();
            assert!(away(fanout.start[0], fanout.via[0]) && away(fanout.start[1], fanout.via[1]));
            assert!(fanout.ring >= 2);
        }
        // No two vias share a gap, and the channels stay open: no via lies
        // between two balls of the outer two rings (the gaps the surface
        // tracks of ring 1 pass through), i.e. every via is at least 1.5
        // pitches inside the array's box.
        let mut sites: Vec<(i64, i64)> = plan
            .fanouts
            .iter()
            .map(|f| {
                (
                    (f.via[0] * 1000.0).round() as i64,
                    (f.via[1] * 1000.0).round() as i64,
                )
            })
            .collect();
        sites.sort_unstable();
        sites.dedup();
        assert_eq!(sites.len(), 36);
        for fanout in &plan.fanouts {
            for axis in 0..2 {
                assert!(fanout.via[axis] >= 1.5 * 0.8 - 1.0e-9);
                assert!(fanout.via[axis] <= 7.5 * 0.8 + 1.0e-9);
            }
        }
        // Every inner ring takes part: rings 2, 3 and 4.
        let rings: BTreeSet<usize> = plan.fanouts.iter().map(|f| f.ring).collect();
        assert_eq!(rings, BTreeSet::from([2, 3, 4]));
    }

    /// With blind and micro vias allowed (`via_spans`), a dog-bone's via is
    /// the shortest span from the ball's layer to a routable one: the micro
    /// via F.Cu-In1.Cu at its own size, or, with In1.Cu another net's
    /// exclusive plane, the blind via to In2.Cu at the class's sizes.
    #[test]
    fn a_dog_bone_takes_the_shortest_span_to_a_routable_layer() {
        let (parts, obstacles, nets, no_via) = array(10, 0.8, 0.35);
        let micro = core::ViaSpan {
            top: 0,
            bottom: 1,
            micro: true,
            size: Some([0.3, 0.1]),
        };
        let blind = core::ViaSpan {
            top: 0,
            bottom: 2,
            micro: false,
            size: None,
        };
        let deep = core::ViaSpan {
            top: 1,
            bottom: 3,
            micro: false,
            size: None,
        };
        let spans = [blind, micro, deep];
        let planned = |planes: &[core::Plane]| {
            super::plan(
                &parts,
                &obstacles,
                &nets,
                &[class()],
                &no_via,
                &BTreeSet::new(),
                (&spans, planes),
                &config(),
            )
        };
        let plan = planned(&[]);
        assert_eq!(plan.parts[0].planned, 36);
        for fanout in &plan.fanouts {
            assert_eq!(fanout.span, Some(micro));
            assert_eq!([fanout.diameter, fanout.drill], [0.3, 0.1]);
        }
        let plane = core::Plane {
            net: nets.len() as core::NetId,
            class: 0,
            layer: 1,
            polygon: vec![[-5.0, -5.0], [15.0, -5.0], [15.0, 15.0], [-5.0, 15.0]],
            excluded: Vec::new(),
            above: Vec::new(),
            connect: true,
            thermal_reach: 0.0,
            thermal_gap: 0.0,
            exclusive: true,
            solid: false,
            hatch: None,
            island_min_area: None,
        };
        let plan = planned(std::slice::from_ref(&plane));
        assert_eq!(plan.parts[0].planned, 36);
        for fanout in &plan.fanouts {
            assert_eq!(fanout.span, Some(blind));
            assert_eq!([fanout.diameter, fanout.drill], [0.4, 0.2]);
        }
        // No span from the ball's layer: a through via, as without spans.
        let plan = super::plan(
            &parts,
            &obstacles,
            &nets,
            &[class()],
            &no_via,
            &BTreeSet::new(),
            (&[deep], &[]),
            &config(),
        );
        assert!(plan.fanouts.iter().all(|fanout| fanout.span.is_none()));
        assert_eq!(plan.parts[0].sizes.get("0.4/0.2"), Some(&36));
    }

    #[test]
    fn an_array_whose_class_via_fits_a_gap_is_left_to_the_search() {
        // The class's own via 0.4/0.2 fits the diagonal gaps at 0.8 mm:
        // the search puts the dog-bones there itself, the plan none.
        let (parts, obstacles, nets, no_via) = array(10, 0.8, 0.35);
        let small = core::RuleClass {
            via_diameter: 0.4,
            via_drill: 0.2,
            ..class()
        };
        let plan = super::plan(
            &parts,
            &obstacles,
            &nets,
            &[small],
            &no_via,
            &BTreeSet::new(),
            (&[], &[]),
            &config(),
        );
        assert_eq!(plan.parts[0].inner, 36);
        assert_eq!(plan.parts[0].searched, 36);
        assert!(plan.fanouts.is_empty());
        assert_eq!(plan.work, 0);
    }

    #[test]
    fn a_ball_beside_a_depopulated_middle_is_on_the_outer_ring() {
        // Two blocks of three columns with three empty columns between
        // (a DDR3 BGA-96's): the third and fourth columns face the gap.
        let mut local = Vec::new();
        for row in 0..8 {
            for column in [0, 1, 2, 6, 7, 8] {
                local.push([column as f64 * 0.8, row as f64 * 0.8]);
            }
        }
        let (rings, toward) = rings_of(&local, 0.8);
        // Row 3, columns 0 to 2 and 6 to 8.
        let row: Vec<usize> = (18..24).map(|index| rings[index]).collect();
        assert_eq!(row, [0, 1, 0, 0, 1, 0]);
        // Column 2 leaves toward the gap, column 1 has both sides alike.
        assert_eq!(toward[20][0], 1.0);
        assert_eq!(toward[19][0], 0.0);
        assert_eq!(toward[18][0], -1.0);
    }

    #[test]
    fn a_ball_holding_a_via_in_its_pad_keeps_it_and_a_fine_pitch_array_gets_none() {
        let (parts, obstacles, nets, no_via) = array(10, 0.8, 0.35);
        // The ball at row 4, column 4 has a via in its pad (`via_in_pad`).
        let holding: BTreeSet<usize> = BTreeSet::from([4 * 10 + 4]);
        let plan = super::plan(
            &parts,
            &obstacles,
            &nets,
            &[class()],
            &no_via,
            &holding,
            (&[], &[]),
            &config(),
        );
        assert_eq!(plan.parts[0].kept, 1);
        assert_eq!(plan.parts[0].planned, 35);
        assert!(plan.fanouts.iter().all(|f| f.pad != 44));
        // At 0.5 mm with 0.3 mm balls no via fits a diagonal gap (0.35 mm
        // from a ball's centre against 0.2 + 0.15 + 0.15): the via in the
        // pad is the way out there, the plan places nothing.
        let (parts, obstacles, nets, no_via) = array(10, 0.5, 0.3);
        let fine = super::plan(
            &parts,
            &obstacles,
            &nets,
            &[class()],
            &no_via,
            &BTreeSet::new(),
            (&[], &[]),
            &config(),
        );
        assert_eq!(fine.parts[0].surface_rings, 1);
        assert_eq!(fine.parts[0].planned, 0);
        assert_eq!(fine.parts[0].refused.len(), 64);
    }

    #[test]
    fn a_part_on_the_back_under_the_array_takes_the_gaps_it_covers() {
        let (parts, mut obstacles, nets, no_via) = array(10, 0.8, 0.35);
        // A capacitor's pad of another net on B.Cu (layer 1 of 2) under
        // the gaps of the south-west quadrant's inner balls.
        obstacles.push(core::Obstacle {
            shape: core::Shape::rectangle([1.6, 1.6], [0.5, 0.5], 0.0),
            layers: 2,
            kind: core::ObstacleKind::Copper,
            net: None,
            clearance: 0.15,
            clearance_override: None,
            blocks_tracks: true,
            blocks_vias: true,
            label: "C1.1".into(),
        });
        let plan = super::plan(
            &parts,
            &obstacles,
            &nets,
            &[class()],
            &no_via,
            &BTreeSet::new(),
            (&[], &[]),
            &config(),
        );
        // The ball at (2, 2) points to (1.6, 1.6) and (2.0, 1.2) ... all
        // within the pad's reach: its gaps are taken by the pad's copper;
        // it is refused or takes a gap away from the pad, and no via comes
        // within the via's radius and clearance of the pad.
        for fanout in &plan.fanouts {
            let gap = obstacles
                .last()
                .unwrap()
                .shape
                .distance_to_point(fanout.via);
            assert!(gap >= fanout.diameter / 2.0 + 0.15, "{fanout:?}");
        }
        assert!(plan.parts[0].planned < 36);
    }

    /// A 6 x 6 array at 0.8 mm (0.35 mm balls) on F.Cu of a two-layer
    /// board, every ball its own net with its other end on a pad row to
    /// the east; tracks 0.15, clearance 0.15, the class's via 0.6/0.3 and
    /// the project's 0.4/0.2.
    fn bga_board() -> (Expr, KiCadBoardRouterConfig) {
        bga_board_with("")
    }

    /// `bga_board` with more footprints (`extra`).
    fn bga_board_with(extra: &str) -> (Expr, KiCadBoardRouterConfig) {
        let mut pads = String::new();
        let mut ends = String::new();
        for row in 0..6 {
            for column in 0..6 {
                let number = format!("{}{}", (b'A' + row as u8) as char, column + 1);
                pads.push_str(&format!(
                    r#"(pad "{number}" smd circle (at {} {}) (size 0.35 0.35) (layers "F.Cu" "F.Mask") (net "N{number}"))"#,
                    (column as f64 - 2.5) * 0.8,
                    (row as f64 - 2.5) * 0.8
                ));
                // One long row: no grid.
                ends.push_str(&format!(
                    r#"(pad "{number}" smd rect (at 0 {}) (size 0.4 0.4) (layers "F.Cu") (net "N{number}"))"#,
                    (row * 6 + column) as f64 * 0.5
                ));
            }
        }
        let pcb = parse(&format!(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
                 (gr_rect (start 0 0) (end 30 20) (layer "Edge.Cuts"))
                 (footprint "BGA-36" (at 8 10) (property "Reference" "U1") {pads})
                 (footprint "J" (at 20 1) (property "Reference" "J1") {ends})
                 {extra})"#
        ))
        .unwrap();
        let config = KiCadBoardRouterConfig {
            default_rules: Some(KiCadConnectionRoutingRules {
                trace_width_mm: 0.15,
                clearance_mm: 0.15,
                via_size_mm: 0.6,
                via_drill_mm: 0.3,
            }),
            hole_clearance_mm: 0.25,
            hole_to_hole_clearance_mm: 0.25,
            bga_fanout: Some(true),
            project_rules: Some(KiCadProjectRulesReport {
                via_sizes: vec![[0.4, 0.2]],
                via_minimums: [0.4, 0.2, 0.05],
                ..KiCadProjectRulesReport::default()
            }),
            ..KiCadBoardRouterConfig::default()
        };
        (pcb, config)
    }

    #[test]
    fn the_lowering_makes_a_planned_via_the_balls_terminal_and_drops_one_its_route_does_not_use() {
        let (mut pcb, config) = bga_board();
        let off = KiCadBoardRouterConfig {
            bga_fanout: Some(false),
            ..config.clone()
        };
        // Unset, the fan-out is on (`PCB_ROUTER_BGA_FANOUT` overrides it).
        let unset = KiCadBoardRouterConfig {
            bga_fanout: None,
            ..config.clone()
        };
        assert_eq!(bga_fanout_on(&unset), bga_fanout_override().unwrap_or(true));
        let plain = lower(&pcb, &off, false).unwrap();
        let lowered = lower(&pcb, &config, false).unwrap();
        // The placer keeps the far side free under the same four balls'
        // vias (judged by the footprint alone); nothing with the fan-out off.
        let far = far_side_balls(&pcb, &config);
        assert_eq!(far.len(), 1);
        assert_eq!(far[0].inner, ["C3", "C4", "D3", "D4"]);
        assert_eq!(far[0].balls.len(), 36);
        assert_eq!((far[0].diameter, far[0].drill), (0.4, 0.2));
        assert!(far_side_balls(&pcb, &off).is_empty());
        // Off, nothing planned and nothing charged.
        assert!(fanouts_of(&plain.board).is_empty());
        assert_eq!(plain.fanout_work, 0);
        let board = lowered.board;
        let fanouts = fanouts_of(&board);
        // Rings 0 and 1 leave on the surface; the middle 2 x 2 (ring 2)
        // take dog-bones of the project's 0.4/0.2 via.
        assert_eq!(fanouts.len(), 4);
        assert!(lowered.fanout_work > 0);
        assert!(
            fanouts
                .iter()
                .all(|f| (f.diameter, f.drill) == (0.4, 0.2) && f.layer == 0)
        );
        let terminal = |label: &str| {
            board
                .nets
                .iter()
                .flat_map(|net| &net.terminals)
                .find(|terminal| terminal.label == label)
                .unwrap()
        };
        // The inner balls reach B.Cu through their vias; an outer one not.
        assert_eq!(terminal("U1.C3").layers, 0b11);
        assert_eq!(terminal("U1.A1").layers, 0b01);
        let net = |name: &str| board.nets.iter().position(|net| net.name == name).unwrap();
        let c3 = fanouts
            .iter()
            .find(|f| f.net as usize == net("NC3"))
            .unwrap()
            .clone();
        let c4 = fanouts
            .iter()
            .find(|f| f.net as usize == net("NC4"))
            .unwrap()
            .clone();
        // C3's route leaves the via on B.Cu; C4's reaches the ball on F.Cu
        // from the west, away from its stub (its fan-out unused: dropped).
        let mut result = core::route(&board, &core_config(&off));
        result.routes[net("NC3")] = core::NetRoute {
            segments: vec![core::Segment {
                layer: 1,
                start: c3.end,
                end: [c3.end[0] + 5.0, c3.end[1]],
                width: 0.15,
            }],
            vias: Vec::new(),
        };
        result.routes[net("NC4")] = core::NetRoute {
            segments: vec![core::Segment {
                layer: 0,
                start: c4.start,
                end: [c4.start[0] - 0.4, c4.start[1]],
                width: 0.15,
            }],
            vias: Vec::new(),
        };
        assert!(fanout_used(&board, &c3, &result.routes[net("NC3")]));
        assert!(!fanout_used(&board, &c4, &result.routes[net("NC4")]));
        let names: Vec<String> = vec!["F.Cu".into(), "B.Cu".into()];
        let nets = emit_routes(&mut pcb, &board, &result, &names, false).unwrap();
        let at = |item: &Expr| form_at(item).ok().map(|at| [at[0], at[1]]);
        let vias: Vec<[f64; 2]> = pcb
            .children()
            .iter()
            .filter(|item| item.head() == Some("via"))
            .filter_map(at)
            .collect();
        let near =
            |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).abs() < 1.0e-6 && (a[1] - b[1]).abs() < 1.0e-6;
        assert!(vias.iter().any(|via| near(*via, c3.end)));
        assert!(!vias.iter().any(|via| near(*via, c4.end)));
        let count = |name: &str| nets.iter().find(|n| n.connection == name).unwrap().vias;
        assert_eq!(count("NC3"), 1);
        assert_eq!(count("NC4"), 0);
    }

    #[test]
    fn array_rooms_find_the_plans_grids_their_rings_class_and_own_decoupling() {
        // C1 decouples a ball's net (NC3) to ground; C2 joins two balls'
        // nets, R1 a ball's net to ground, C3 ground to a net off the
        // array: none of those three.
        let part = |kind: &str, reference: &str, a: &str, b: &str, x: f64| {
            format!(
                r#"(footprint "{kind}" (layer "B.Cu") (at {x} 15) (property "Reference" "{reference}")
                     (pad "1" smd rect (at -0.5 0) (size 0.5 0.5) (layers "B.Cu") (net "{a}"))
                     (pad "2" smd rect (at 0.5 0) (size 0.5 0.5) (layers "B.Cu") (net "{b}")))"#
            )
        };
        let extra = [
            part("C_0402", "C1", "NC3", "GND", 2.0),
            part("C_0402", "C2", "NA1", "NA2", 4.0),
            part("R_0402", "R1", "NC4", "GND", 6.0),
            part("C_0402", "C3", "VX", "GND", 8.0),
            part("R_0402", "R2", "VX", "Y", 10.0),
        ]
        .concat();
        let (pcb, config) = bga_board_with(&extra);
        let rooms = array_rooms(&pcb, &config);
        assert_eq!(rooms.len(), 1, "{rooms:?}");
        let room = &rooms[0];
        assert_eq!(room.footprint, 0);
        assert!((room.pitch - 0.8).abs() < 1.0e-9);
        assert_eq!(room.balls.len(), 36);
        // One track between two balls: rings 0 and 1 on the surface; the
        // middle 2 x 2 is ring 2.
        assert_eq!((room.surface_rings, room.deepest), (2, 2));
        assert_eq!((room.track, room.clearance, room.via), (0.15, 0.15, 0.6));
        assert_eq!(room.decoupling, [2]);
        assert!(room.nets.contains("NC3") && !room.nets.contains("GND"));
        // The detection is the plan's, the fan-out switch aside.
        let off = KiCadBoardRouterConfig {
            bga_fanout: Some(false),
            ..config.clone()
        };
        assert_eq!(array_rooms(&pcb, &off), rooms);
        assert_eq!(far_side_balls(&pcb, &config)[0].balls, room.balls);
    }
}
