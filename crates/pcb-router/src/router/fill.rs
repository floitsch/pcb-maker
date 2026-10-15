// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! KiCad's zone fill near a point, off the lattice.
//!
//! The pour map samples the brush centres (where a disc of the zone's
//! minimum width keeps the clearance) at the lattice nodes. Near the rim of
//! the fill that is too coarse for KiCad's thermal spokes: a spoke is kept
//! when its end lies in the fill (within half the minimum width of a brush
//! centre), and the nearest node may miss a centre a fraction of a pitch
//! away (Castor's D7.3: KiCad's spoke ends 0.12 mm from a centre, the
//! nearest free node is 0.25 mm away). A `FillProbe` answers such
//! questions from the geometry itself: the board's obstacles and the
//! routed copper of the other nets, at their exact distances.

use std::cell::OnceCell;

use super::{FREE_END, FastMap, NO_TERMINAL, PLANE_TERMINAL, Router};
use crate::board::{Hatch, NetId, ObstacleKind, Plane};
use crate::geometry::{Point, distance, point_in_polygon, point_segment_distance, polygon_edges, segments_intersect};

/// Side of the index's buckets, in mm.
const BUCKET: f64 = 1.0;

/// KiCad's `m_MaxError` (the default 0.005 mm): its spokes reach that much
/// further than the thermal gap and half the minimum width.
pub(super) const KICAD_MAX_ERROR: f64 = 0.005;

/// How much closer than the exact clearance a brush centre may come: KiCad
/// prunes the fill by half the minimum width less 0.001 mm, so a neck of
/// exactly the minimum width survives, and its arcs are polygons within
/// `KICAD_MAX_ERROR` of the true curve, so a relief or a clearance around
/// a round pad is a few microns off either way. Census of 44 boards: 0.0005
/// mm (the pruning alone) 34 false alarms and 17 misses, 0.003 mm 30 and 18
/// (k30's J7.2: its spoke ends 2 microns short of room between its relief
/// and a track's clearance, and KiCad keeps it).
const EPSILON: f64 = 0.003;

/// `EPSILON` for the DRC's test of a track's end in the fill: a point,
/// where KiCad's fill keeps the clearance to a micron on straight edges
/// (its pruning) and up to 5 microns more on arcs (polygons outside the
/// curve), not a spoke's end with room around it. The value is KiCad's DRC
/// epsilon (`BOARD_DESIGN_SETTINGS::GetDRCEpsilon`, ADVANCED_CFG
/// `m_DRCEpsilon` = 0.0005 mm, 500 nm on 0xCB's board in pcbnew), which its
/// clearance tests subtract from every clearance (`sub_e` in
/// drc_test_provider_copper_clearance.cpp; edge, physical and hole
/// clearance alike). The starved-thermal check itself tests the point
/// against the filled polygons with no epsilon and the filler prunes with
/// its own micron, so this is KiCad's tolerance for "keeps the clearance",
/// not a constant of that check. Census of 44 boards: at `EPSILON` 17
/// misses and 27 false alarms, 0.001 mm 16 and 27, 0.0005 mm 12 and 27
/// (0xCB's U3.45 thermal vias: a track from one ends 1.5 microns outside a
/// sliver of fill between two QFN pins), exact 12 and 29. These ends lie
/// within a micron of the fill's rim.
const TRACK_END_TOLERANCE: f64 = 0.0005;

#[derive(Clone, Copy)]
enum Item {
    /// One of the board's obstacles.
    Obstacle(usize),
    /// A routed track of another net: its centreline, half its width and
    /// the clearance of its net class.
    Segment { a: Point, b: Point, half_width: f64, clearance: f64 },
    /// A routed via of another net.
    Via { at: Point, radius: f64, clearance: f64 },
    /// An edge of the pour's outline, of a region it leaves out, or of the
    /// board: a brush centre keeps `margin` from it.
    Edge { a: Point, b: Point, margin: f64 },
    /// A pad of the net with a thermal relief: a centre keeps `reach`.
    Ring { pad: usize, reach: f64 },
}

struct Index {
    buckets: FastMap<(usize, i64, i64), Vec<Item>>,
    /// How far from a point an item can still matter (beyond its box).
    span: f64,
}

pub(super) struct FillProbe<'a> {
    router: &'a Router,
    net: NetId,
    /// Per layer, the pads whose thermal relief the fill keeps away from,
    /// with that reach (the gap and half the minimum width).
    rings: &'a [Vec<(usize, f64)>],
    index: OnceCell<Index>,
    /// Per hatch hole (layer and its place in the pattern), whether KiCad
    /// keeps it (`hole_kept`).
    holes: std::cell::RefCell<FastMap<(usize, i64, i64), bool>>,
    /// How much closer than the clearance to other nets' copper a brush
    /// centre may come (`EPSILON`, `TRACK_END_TOLERANCE` inside
    /// `for_track_end`; the board's edges and the reliefs keep `EPSILON`).
    tolerance: std::cell::Cell<f64>,
    /// The probe's work: items put in the index and items its queries
    /// looked at, added to the router's pour work by `charge`.
    ops: std::cell::Cell<u64>,
    /// Its queries (`blocker`, `slack`: a point against the index) and
    /// the polygon vertices its shape tests went through, charged beside
    /// the items.
    queries: std::cell::Cell<u64>,
    vertices: std::cell::Cell<u64>,
}

/// The polygon vertices a distance test of `shape` goes through (none for
/// a circle or a capsule, whose test an item's cost covers).
fn polygon_vertices(shape: &crate::geometry::Shape) -> u64 {
    match shape {
        crate::geometry::Shape::Polygon { points } => points.len() as u64,
        crate::geometry::Shape::Union { parts } => parts.iter().map(polygon_vertices).sum(),
        _ => 0,
    }
}


fn bucket_range(minimum: Point, maximum: Point) -> (i64, i64, i64, i64) {
    (
        (minimum[0] / BUCKET).floor() as i64,
        (minimum[1] / BUCKET).floor() as i64,
        (maximum[0] / BUCKET).floor() as i64,
        (maximum[1] / BUCKET).floor() as i64,
    )
}

fn segment_box(a: Point, b: Point) -> (Point, Point) {
    ([a[0].min(b[0]), a[1].min(b[1])], [a[0].max(b[0]), a[1].max(b[1])])
}

impl<'a> FillProbe<'a> {
    pub(super) fn new(router: &'a Router, net: NetId, rings: &'a [Vec<(usize, f64)>]) -> Self {
        Self { router, net, rings, index: OnceCell::new(), holes: std::cell::RefCell::new(FastMap::default()), tolerance: std::cell::Cell::new(EPSILON), ops: std::cell::Cell::new(0), queries: std::cell::Cell::new(0), vertices: std::cell::Cell::new(0) }
    }

    /// Adds the probe's work so far to its router's pour work.
    pub(super) fn charge(&self) {
        self.router.scratch.probe_ops.fetch_add(self.ops.replace(0), std::sync::atomic::Ordering::Relaxed);
        self.router.scratch.probe_queries.fetch_add(self.queries.replace(0), std::sync::atomic::Ordering::Relaxed);
        self.router.scratch.probe_vertices.fetch_add(self.vertices.replace(0), std::sync::atomic::Ordering::Relaxed);
    }

    /// `f` with brush centres allowed `TRACK_END_TOLERANCE` closer than
    /// the clearance to other nets' copper rather than `EPSILON`.
    pub(super) fn for_track_end<T>(&self, f: impl FnOnce() -> T) -> T {
        let before = self.tolerance.replace(TRACK_END_TOLERANCE);
        let result = f();
        self.tolerance.set(before);
        result
    }

    /// `shape`'s distance to `point`, counting its vertices.
    fn distance_to(&self, shape: &crate::geometry::Shape, point: Point) -> f64 {
        self.vertices.set(self.vertices.get() + polygon_vertices(shape));
        shape.distance_to_point(point)
    }

    fn half(&self, layer: usize) -> f64 {
        self.router.board.classes[self.router.nets[self.net as usize].plane_class[layer]].trace_width / 2.0
    }

    /// The net's zones on `layer` that KiCad fills: all of them, those of
    /// a net routed as tracks too (the tracks rung lowers its pours with
    /// `connect` false, and KiCad fills them all the same); with
    /// `tracks_rung_fill` off only those pads connect through (and, for
    /// the `hatch` and its rings, any while `Router::hatch_any_plane` is
    /// set).
    fn filled_planes(&self, layer: usize, hatch: bool) -> impl Iterator<Item = &'a Plane> + 'a {
        let router = self.router;
        let net = self.net;
        let any = router.config.tracks_rung_fill || (hatch && router.hatch_any_plane);
        router.board.planes.iter().filter(move |plane| plane.net == net && plane.layer == layer && (plane.connect || any))
    }

    /// The hatch of the net's pour on `layer`, if it is hatched.
    pub(super) fn hatch(&self, layer: usize) -> Option<Hatch> {
        self.filled_planes(layer, true).find_map(|plane| plane.hatch)
    }

    /// How far past the gap KiCad tests a spoke's end: half the minimum
    /// width, or half a hatched zone's web.
    pub(super) fn spoke_half_width(&self, layer: usize) -> f64 {
        self.hatch(layer).map_or(self.half(layer), |hatch| hatch.thickness / 2.0)
    }

    /// The fill KiCad tests a spoke's end against (the fill before its
    /// hatch, with a hatched zone's thermal rings).
    pub(super) fn test_area_at(&self, layer: usize, point: Point) -> bool {
        self.fill_at(layer, point) || self.in_ring(layer, point)
    }

    /// `test_area_at` for a spoke's end, searching along the spoke too.
    pub(super) fn spoke_end_lands(&self, layer: usize, end: Point, direction: Point) -> bool {
        self.fill_along(layer, end, direction) || self.in_ring(layer, end)
    }

    /// Whether KiCad's final fill covers `point` (a hatched zone's holes
    /// left out, its rings in).
    pub(super) fn final_fill_at(&self, layer: usize, point: Point) -> bool {
        if self.hatch(layer).is_none() {
            return self.fill_at(layer, point);
        }
        self.in_ring(layer, point) || (self.fill_at(layer, point) && !self.in_hatch_hole(layer, point))
    }

    /// Whether `point` lies in a hatched zone's thermal ring around a pad
    /// of the net (buildHatchZoneThermalRings): a band as wide as a spoke
    /// from the gap out, which other nets' clearances cut and no relief
    /// does, kept where it holds the minimum width.
    pub(super) fn in_ring(&self, layer: usize, point: Point) -> bool {
        if self.hatch(layer).is_none() {
            return false;
        }
        let board = &self.router.board;
        let half = self.half(layer);
        let bridge = self.filled_planes(layer, true).map(|plane| (plane.thermal_reach - plane.thermal_gap).max(0.0)).fold(0.0f64, f64::max);
        for (pad, reach) in &self.rings[layer] {
            let gap = reach - half;
            let shape = &board.obstacles[*pad].shape;
            let minor = match board.pad_spokes.get(pad) {
                Some(frame) => 2.0 * frame.half[0].min(frame.half[1]),
                None => {
                    let bounds = shape.aabb();
                    (bounds.maximum[0] - bounds.minimum[0]).min(bounds.maximum[1] - bounds.minimum[1])
                }
            };
            let width = bridge.min(minor);
            if width + 1.0e-9 < 2.0 * half {
                continue;
            }
            let distance = self.distance_to(shape, point);
            if distance < gap - 1.0e-9 || distance > gap + width + 1.0e-9 {
                continue;
            }
            for radius in [0.0, half * 0.5, half - 1.0e-3] {
                for step in 0..(if radius == 0.0 { 1 } else { 16 }) {
                    let angle = std::f64::consts::TAU * step as f64 / 16.0;
                    let centre = [point[0] + radius * angle.cos(), point[1] + radius * angle.sin()];
                    let from_pad = self.distance_to(shape, centre);
                    if from_pad >= gap + half - 1.0e-9 && from_pad <= gap + width - half + 1.0e-9 && self.blocker(layer, centre, None).is_none() {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Whether `point` lies in one of a hatched zone's holes as KiCad cuts
    /// them (addHatchFillTypeOnZone): squares of the gap on a grid of the
    /// gap and the web, turned by the hatch's angle about the origin, their
    /// corners smoothed; a web's width along the fill's edges stays, as do
    /// holes the edge cuts to less than the minimum area and holes around
    /// a whole thermal ring.
    pub(super) fn in_hatch_hole(&self, layer: usize, point: Point) -> bool {
        let Some(hatch) = self.hatch(layer) else {
            return false;
        };
        let half = self.half(layer);
        let thickness = hatch.thickness.max(2.0 * half + 0.001);
        let grid = thickness + hatch.gap;
        let size = hatch.gap + 2.0 * half;
        let (sine, cosine) = hatch.orientation.to_radians().sin_cos();
        let q = [point[0] * cosine - point[1] * sine, point[0] * sine + point[1] * cosine];
        let (k, m) = ((q[0] / grid).floor(), (q[1] / grid).floor());
        let (u, v) = (q[0] - k * grid, q[1] - m * grid);
        if !self.in_hole_shape(&hatch, half, size, u, v, half) {
            return false;
        }
        // The web along the fill's edge.
        if self.near_edge(layer, point, thickness) {
            return false;
        }
        self.hole_kept(layer, &hatch, k as i64, m as i64)
    }

    /// Whether (u, v) in a hole's square (side `size`, before the fill is
    /// grown back by half the minimum width) lies in the hole shrunk by
    /// `inset`, with KiCad's corner smoothing.
    fn in_hole_shape(&self, hatch: &Hatch, half: f64, size: f64, u: f64, v: f64, inset: f64) -> bool {
        let (low, high) = (inset, size - inset);
        if u <= low || u >= high || v <= low || v >= high {
            return false;
        }
        let mut smooth = (hatch.gap * hatch.smoothing_value / 2.0 * 1.0e6).round() / 1.0e6;
        let mut level = hatch.smoothing_level;
        if level == 0 || smooth <= 0.02 {
            return true;
        }
        if smooth < 0.04 && level > 1 {
            level = 1;
        }
        smooth = (smooth + half).min(hatch.gap / 2.0);
        // From the nearest side and the nearest end of the shrunk square.
        let (a, b) = ((u - low).min(high - u), (v - low).min(high - v));
        if level == 1 {
            // Chamfers of `smooth` along the sides, moved in with them.
            return a + b >= (smooth - inset * (2.0 - std::f64::consts::SQRT_2)).max(0.0);
        }
        let radius = (smooth - inset).max(0.0);
        a >= radius || b >= radius || (radius - a).hypot(radius - b) <= radius
    }

    /// Whether the fill's edge comes within `reach` of `point`.
    fn near_edge(&self, layer: usize, point: Point, reach: f64) -> bool {
        [reach * 0.5, reach - 0.005].iter().any(|radius| {
            (0..16).any(|step| {
                let angle = std::f64::consts::TAU * step as f64 / 16.0;
                !self.fill_at(layer, [point[0] + radius * angle.cos(), point[1] + radius * angle.sin()])
            })
        })
    }

    /// Whether KiCad keeps hole (k, m) of the hatch on `layer`: not when
    /// it holds a whole thermal ring, nor when the fill's edge (less a web)
    /// leaves less than the minimum share of it.
    fn hole_kept(&self, layer: usize, hatch: &Hatch, k: i64, m: i64) -> bool {
        if let Some(kept) = self.holes.borrow().get(&(layer, k, m)) {
            return *kept;
        }
        let board = &self.router.board;
        let half = self.half(layer);
        let thickness = hatch.thickness.max(2.0 * half + 0.001);
        let grid = thickness + hatch.gap;
        let size = hatch.gap + 2.0 * half;
        let (sine, cosine) = hatch.orientation.to_radians().sin_cos();
        let to_board = |u: f64, v: f64| {
            let q = [k as f64 * grid + u, m as f64 * grid + v];
            [q[0] * cosine + q[1] * sine, -q[0] * sine + q[1] * cosine]
        };
        let to_hatch = |point: Point| {
            let q = [point[0] * cosine - point[1] * sine, point[0] * sine + point[1] * cosine];
            (q[0] - k as f64 * grid, q[1] - m as f64 * grid)
        };
        let bridge = self.filled_planes(layer, true).map(|plane| (plane.thermal_reach - plane.thermal_gap).max(0.0)).fold(0.0f64, f64::max);
        let encloses_ring = self.rings[layer].iter().any(|(pad, reach)| {
            let outer = reach - half + bridge;
            let bounds = board.obstacles[*pad].shape.aabb().inflated(outer);
            [bounds.minimum, bounds.maximum, [bounds.minimum[0], bounds.maximum[1]], [bounds.maximum[0], bounds.minimum[1]]]
                .iter()
                .all(|corner| {
                    let (u, v) = to_hatch(*corner);
                    self.in_hole_shape(hatch, half, size, u, v, 0.0)
                })
        });
        let kept = !encloses_ring && {
            // The share of the hole the fill (less a web along its edge)
            // leaves, sampled.
            let (mut total, mut inside) = (0usize, 0usize);
            let samples = 8;
            for i in 0..samples {
                for j in 0..samples {
                    let (u, v) = ((i as f64 + 0.5) * size / samples as f64, (j as f64 + 0.5) * size / samples as f64);
                    if !self.in_hole_shape(hatch, half, size, u, v, 0.0) {
                        continue;
                    }
                    total += 1;
                    let at = to_board(u, v);
                    if self.fill_at(layer, at) && !self.near_edge(layer, at, thickness - half) {
                        inside += 1;
                    }
                }
            }
            total > 0 && (inside as f64) >= hatch.min_hole_area * total as f64
        };
        self.holes.borrow_mut().insert((layer, k, m), kept);
        kept
    }

    fn index(&self) -> &Index {
        self.index.get_or_init(|| {
            let router = self.router;
            let board = &router.board;
            let layers = board.layer_count;
            let state = &router.nets[self.net as usize];
            let mut buckets: FastMap<(usize, i64, i64), Vec<Item>> = FastMap::default();
            let mut reach: f64 = board.edge_clearance.max(board.hole_clearance);
            let insert = |layer: usize, (minimum, maximum): (Point, Point), item: Item, buckets: &mut FastMap<(usize, i64, i64), Vec<Item>>| {
                let (x0, y0, x1, y1) = bucket_range(minimum, maximum);
                for by in y0..=y1 {
                    for bx in x0..=x1 {
                        buckets.entry((layer, bx, by)).or_default().push(item);
                    }
                }
            };
            for (position, obstacle) in board.obstacles.iter().enumerate() {
                if obstacle.net == Some(self.net) || !obstacle.blocks_tracks {
                    continue;
                }
                // Mask openings do not stop a fill (the static maps' fill
                // trace).
                if obstacle.kind == ObstacleKind::Keepout && obstacle.label.starts_with("solder mask opening") {
                    continue;
                }
                reach = reach.max(obstacle.clearance).max(obstacle.clearance_override.unwrap_or(0.0));
                let bounds = obstacle.shape.aabb();
                for layer in (0..layers).filter(|layer| obstacle.layers & (1 << layer) != 0) {
                    insert(layer, (bounds.minimum, bounds.maximum), Item::Obstacle(position), &mut buckets);
                }
            }
            for class in &board.classes {
                reach = reach.max(class.clearance);
            }
            let mut extent: f64 = 0.0;
            for (other, other_state) in router.nets.iter().enumerate() {
                if other as NetId == self.net {
                    continue;
                }
                let description = &board.nets[other];
                let rules = board.classes[description.class];
                let clearance = rules.clearance;
                for branch in &other_state.branches {
                    for pair in branch.nodes.windows(2) {
                        let (a, b) = (router.grid.center_of(pair[0].cell as usize), router.grid.center_of(pair[1].cell as usize));
                        if pair[0].layer == pair[1].layer {
                            let half_width = router.width_at(other as NetId, pair[0].cell).min(router.width_at(other as NetId, pair[1].cell)) / 2.0;
                            extent = extent.max(half_width);
                            insert(pair[0].layer as usize, segment_box(a, b), Item::Segment { a, b, half_width, clearance }, &mut buckets);
                        } else if pair[0].cell == pair[1].cell {
                            extent = extent.max(rules.via_diameter / 2.0);
                            for layer in 0..layers {
                                insert(layer, (a, a), Item::Via { at: a, radius: rules.via_diameter / 2.0, clearance }, &mut buckets);
                            }
                        }
                    }
                    // The stubs from the pads to the lattice.
                    for (terminal, node) in [(branch.start_terminal, branch.nodes[0]), (branch.end_terminal, *branch.nodes.last().unwrap())] {
                        if terminal == NO_TERMINAL || terminal == PLANE_TERMINAL || terminal == FREE_END {
                            continue;
                        }
                        let center = router.grid.center_of(node.cell as usize);
                        if let Some((_, width, polyline)) = other_state.escapes.get(&node) {
                            extent = extent.max(width / 2.0);
                            for pair in polyline.windows(2) {
                                insert(node.layer as usize, segment_box(pair[0], pair[1]), Item::Segment { a: pair[0], b: pair[1], half_width: width / 2.0, clearance }, &mut buckets);
                            }
                        } else if let Some(described) = description.terminals.get(terminal as usize) {
                            let half_width = router.width_at(other as NetId, node.cell) / 2.0;
                            extent = extent.max(half_width);
                            insert(node.layer as usize, segment_box(described.anchor, center), Item::Segment { a: described.anchor, b: center, half_width, clearance }, &mut buckets);
                        }
                    }
                }
            }
            // The pour's own outlines, the board's, and the reliefs.
            let mut half_most: f64 = 0.0;
            for layer in 0..layers {
                if state.plane.get(layer).is_none_or(|mask| mask.is_empty()) {
                    continue;
                }
                let half = self.half(layer);
                half_most = half_most.max(half);
                for plane in self.filled_planes(layer, false) {
                    for polygon in std::iter::once(&plane.polygon).chain(plane.excluded.iter()) {
                        for (a, b) in polygon_edges(polygon) {
                            insert(layer, segment_box(a, b), Item::Edge { a, b, margin: half }, &mut buckets);
                        }
                    }
                }
                for (a, b) in polygon_edges(&board.outline) {
                    insert(layer, segment_box(a, b), Item::Edge { a, b, margin: board.edge_clearance + half }, &mut buckets);
                }
                for (pad, pad_reach) in &self.rings[layer] {
                    let bounds = board.obstacles[*pad].shape.aabb();
                    reach = reach.max(*pad_reach);
                    insert(layer, (bounds.minimum, bounds.maximum), Item::Ring { pad: *pad, reach: *pad_reach }, &mut buckets);
                }
            }
            self.ops.set(self.ops.get() + buckets.values().map(Vec::len).sum::<usize>() as u64);
            Index { buckets, span: reach + extent + half_most + board.edge_clearance + 1.0e-3 }
        })
    }

    /// Whether `point` lies inside the pour on `layer` (its outline, not a
    /// region it leaves out) and the board: as its nearest node, which the
    /// pour map knows, where none of `edges` (those near it) comes between
    /// them; else by the polygons.
    fn inside(&self, layer: usize, point: Point, edges: &[(Point, Point)]) -> bool {
        let router = self.router;
        let board = &router.board;
        let state = &router.nets[self.net as usize];
        if let Some(node) = router.grid.nearest_node(point) {
            let center = router.grid.center_of(node);
            if state.plane[layer][node] && !edges.iter().any(|(a, b)| segments_intersect(point, center, *a, *b) || point_segment_distance(center, *a, *b) < 1.0e-9) {
                return true;
            }
        }
        self.filled_planes(layer, false).any(|plane| {
            point_in_polygon(point, &plane.polygon) && !plane.excluded.iter().any(|other| point_in_polygon(point, other))
        }) && point_in_polygon(point, &board.outline)
    }

    /// Whether a brush centre of the pour fits at `point` on `layer`: the
    /// disc of half the minimum width inside the zone and the board, and
    /// clear of other nets' copper by the clearance, of the board's edge,
    /// holes and keepouts, and of the thermal reliefs of the net's pads
    /// (but those of `except`, whose spoke this is).
    pub(super) fn fits(&self, layer: usize, point: Point, except: &[usize]) -> bool {
        self.blocker(layer, point, Some(except)).is_none()
    }

    /// What keeps a brush centre from `point` (`fits`; with `except` None,
    /// no thermal relief does: a spoke's), for debugging: None where it
    /// fits.
    pub(super) fn blocker(&self, layer: usize, point: Point, except: Option<&[usize]>) -> Option<String> {
        let router = self.router;
        let board = &router.board;
        let state = &router.nets[self.net as usize];
        if state.plane.get(layer).is_none_or(|mask| mask.is_empty()) {
            return Some("no pour".into());
        }
        let brush = board.classes[state.plane_class[layer]];
        let half = brush.trace_width / 2.0;
        let index = self.index();
        let span = index.span;
        let tolerance = self.tolerance.get();
        let (x0, y0, x1, y1) = bucket_range([point[0] - span, point[1] - span], [point[0] + span, point[1] + span]);
        self.queries.set(self.queries.get() + 1);
        let mut edges: Vec<(Point, Point)> = Vec::new();
        for by in y0..=y1 {
            for bx in x0..=x1 {
                let Some(items) = index.buckets.get(&(layer, bx, by)) else {
                    continue;
                };
                self.ops.set(self.ops.get() + items.len() as u64);
                for item in items {
                    let blocked = match item {
                        Item::Obstacle(position) => {
                            let obstacle = &board.obstacles[*position];
                            let clearance = match obstacle.kind {
                                ObstacleKind::Copper => board.fill_clearance(&brush, obstacle),
                                ObstacleKind::Keepout => obstacle.clearance,
                                ObstacleKind::Hole => board.hole_clearance.max(obstacle.clearance),
                            };
                            self.distance_to(&obstacle.shape, point) < clearance + half - tolerance
                        }
                        Item::Segment { a, b, half_width, clearance } => {
                            point_segment_distance(point, *a, *b) < half_width + clearance.max(brush.clearance) + half - tolerance
                        }
                        Item::Via { at, radius, clearance } => distance(point, *at) < radius + clearance.max(brush.clearance) + half - tolerance,
                        Item::Edge { a, b, margin } => {
                            edges.push((*a, *b));
                            point_segment_distance(point, *a, *b) < *margin - EPSILON
                        }
                        Item::Ring { pad, reach } => except.is_some_and(|except| !except.contains(pad)) && self.distance_to(&board.obstacles[*pad].shape, point) < *reach - EPSILON,
                    };
                    if blocked {
                        return Some(match item {
                            Item::Obstacle(position) => format!("obstacle {}", board.obstacles[*position].label),
                            Item::Segment { a, b, .. } => format!("track {a:?}-{b:?}"),
                            Item::Via { at, .. } => format!("via {at:?}"),
                            Item::Edge { a, b, .. } => format!("edge {a:?}-{b:?}"),
                            Item::Ring { pad, .. } => format!("relief of {}", board.obstacles[*pad].label),
                        });
                    }
                }
            }
        }
        if self.inside(layer, point, &edges) { None } else { Some("outside".into()) }
    }

    /// How much room a brush centre at `point` has to spare: the least of
    /// its distances to what keeps it away less what each needs (negative
    /// where it does not fit; `blocker`'s rules, no relief but those not in
    /// `except`).
    fn slack(&self, layer: usize, point: Point, except: &[usize]) -> f64 {
        let router = self.router;
        let board = &router.board;
        let state = &router.nets[self.net as usize];
        if state.plane.get(layer).is_none_or(|mask| mask.is_empty()) {
            return -1.0;
        }
        let brush = board.classes[state.plane_class[layer]];
        let half = brush.trace_width / 2.0;
        let index = self.index();
        let span = index.span;
        let (x0, y0, x1, y1) = bucket_range([point[0] - span, point[1] - span], [point[0] + span, point[1] + span]);
        self.queries.set(self.queries.get() + 1);
        let mut edges: Vec<(Point, Point)> = Vec::new();
        let mut least = f64::INFINITY;
        for by in y0..=y1 {
            for bx in x0..=x1 {
                let Some(items) = index.buckets.get(&(layer, bx, by)) else {
                    continue;
                };
                self.ops.set(self.ops.get() + items.len() as u64);
                for item in items {
                    let room = match item {
                        Item::Obstacle(position) => {
                            let obstacle = &board.obstacles[*position];
                            let clearance = match obstacle.kind {
                                ObstacleKind::Copper => board.fill_clearance(&brush, obstacle),
                                ObstacleKind::Keepout => obstacle.clearance,
                                ObstacleKind::Hole => board.hole_clearance.max(obstacle.clearance),
                            };
                            self.distance_to(&obstacle.shape, point) - clearance - half
                        }
                        Item::Segment { a, b, half_width, clearance } => point_segment_distance(point, *a, *b) - half_width - clearance.max(brush.clearance) - half,
                        Item::Via { at, radius, clearance } => distance(point, *at) - radius - clearance.max(brush.clearance) - half,
                        Item::Edge { a, b, margin } => {
                            edges.push((*a, *b));
                            point_segment_distance(point, *a, *b) - margin
                        }
                        Item::Ring { pad, reach } => {
                            if except.contains(pad) {
                                f64::INFINITY
                            } else {
                                self.distance_to(&board.obstacles[*pad].shape, point) - reach
                            }
                        }
                    };
                    least = least.min(room);
                }
            }
        }
        if self.inside(layer, point, &edges) { least } else { -1.0 }
    }

    /// `fill_at`, and where its samples miss, the best brush centre along
    /// `axis` through `point` (a golden-section search of `slack`): a neck
    /// exactly the minimum width between two reliefs holds brush centres on
    /// one line only (Castor's J9.10, whose spoke ends between its own
    /// relief and J9.9's, 0.254 mm apart in a 0.254 mm zone).
    pub(super) fn fill_along(&self, layer: usize, point: Point, axis: Point) -> bool {
        if self.fill_at(layer, point) {
            return true;
        }
        let half = self.half(layer) - 1.0e-3;
        let at = |t: f64| self.slack(layer, [point[0] + axis[0] * t, point[1] + axis[1] * t], &[]);
        // The slack changes no faster than the point moves: from five
        // samples half a half-width apart, a centre that fits must lie
        // within a quarter of one of the best.
        let samples: Vec<(f64, f64)> = (0..5).map(|index| {
            let t = -half + half * index as f64 / 2.0;
            (t, at(t))
        }).collect();
        let (best_t, best) = samples.iter().copied().fold((0.0, f64::NEG_INFINITY), |a, b| if b.1 > a.1 { b } else { a });
        if best >= -EPSILON {
            return true;
        }
        if best + half / 4.0 < -EPSILON {
            return false;
        }
        let ratio = (5.0f64.sqrt() - 1.0) / 2.0;
        let (mut low, mut high) = ((best_t - half / 2.0).max(-half), (best_t + half / 2.0).min(half));
        let (mut a, mut b) = (high - ratio * (high - low), low + ratio * (high - low));
        let (mut fa, mut fb) = (at(a), at(b));
        for _ in 0..24 {
            if fa >= -EPSILON || fb >= -EPSILON {
                return true;
            }
            if fa < fb {
                low = a;
                a = b;
                fa = fb;
                b = low + ratio * (high - low);
                fb = at(b);
            } else {
                high = b;
                b = a;
                fb = fa;
                a = high - ratio * (high - low);
                fa = at(a);
            }
        }
        fa >= -EPSILON || fb >= -EPSILON
    }

    /// Whether KiCad's fill covers `point`: a brush centre fits within half
    /// the minimum width of it (sampled at the point and on two circles).
    pub(super) fn fill_at(&self, layer: usize, point: Point) -> bool {
        let half = self.half(layer);
        if self.fits(layer, point, &[]) {
            return true;
        }
        for radius in [half * 0.5, half - 1.0e-3] {
            for step in 0..16 {
                let angle = std::f64::consts::TAU * step as f64 / 16.0;
                if self.fits(layer, [point[0] + radius * angle.cos(), point[1] + radius * angle.sin()], &[]) {
                    return true;
                }
            }
        }
        false
    }

    /// Whether a spoke keeps a path for the brush from `from` along
    /// `direction` from `anchor` to its end (`to`), within `side` of its
    /// axis (half the spoke's width less half the minimum width): what is
    /// left of it after KiCad cuts other nets' clearances out and removes
    /// what is narrower than the minimum width. KiCad adds the spokes after
    /// cutting the thermal reliefs out, so no relief cuts a spoke (a USB-C
    /// receptacle's A1 beside B12: keyboar_'s J1).
    pub(super) fn spoke_clear(&self, layer: usize, anchor: Point, direction: Point, from: f64, to: f64, side: f64) -> bool {
        let half = self.half(layer);
        let step = (half / 2.0).max(0.01);
        let steps = (((to - from) / step).ceil() as usize).max(1);
        let across: Vec<f64> = if side < step / 2.0 {
            vec![0.0]
        } else {
            let count = (side / step).ceil() as usize;
            (0..=2 * count).map(|index| -side + side * index as f64 / count as f64).collect()
        };
        (0..=steps).all(|index| {
            let along = from + (to - from) * index as f64 / steps as f64;
            across.iter().any(|offset| {
                let at = [anchor[0] + direction[0] * along - direction[1] * offset, anchor[1] + direction[1] * along + direction[0] * offset];
                self.blocker(layer, at, None).is_none()
            })
        })
    }
}
