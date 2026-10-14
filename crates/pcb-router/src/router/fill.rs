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
use std::collections::HashMap;

use super::{FREE_END, NO_TERMINAL, PLANE_TERMINAL, Router};
use crate::board::{NetId, ObstacleKind};
use crate::geometry::{Point, distance, point_in_polygon, point_segment_distance, polygon_edges, segments_intersect};

/// Side of the index's buckets, in mm.
const BUCKET: f64 = 1.0;

/// KiCad's `m_MaxError` (the default 0.005 mm): its spokes reach that much
/// further than the thermal gap and half the minimum width.
pub(super) const KICAD_MAX_ERROR: f64 = 0.005;

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
    buckets: HashMap<(usize, i64, i64), Vec<Item>>,
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
        Self { router, net, rings, index: OnceCell::new() }
    }

    fn half(&self, layer: usize) -> f64 {
        self.router.board.classes[self.router.nets[self.net as usize].plane_class[layer]].trace_width / 2.0
    }

    fn index(&self) -> &Index {
        self.index.get_or_init(|| {
            let router = self.router;
            let board = &router.board;
            let layers = board.layer_count;
            let state = &router.nets[self.net as usize];
            let mut buckets: HashMap<(usize, i64, i64), Vec<Item>> = HashMap::new();
            let mut reach: f64 = board.edge_clearance.max(board.hole_clearance);
            let insert = |layer: usize, (minimum, maximum): (Point, Point), item: Item, buckets: &mut HashMap<(usize, i64, i64), Vec<Item>>| {
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
                for plane in board.planes.iter().filter(|plane| plane.net == self.net && plane.layer == layer && plane.connect) {
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
        board.planes.iter().filter(|plane| plane.net == self.net && plane.layer == layer && plane.connect).any(|plane| {
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
        let (x0, y0, x1, y1) = bucket_range([point[0] - span, point[1] - span], [point[0] + span, point[1] + span]);
        let mut edges: Vec<(Point, Point)> = Vec::new();
        for by in y0..=y1 {
            for bx in x0..=x1 {
                let Some(items) = index.buckets.get(&(layer, bx, by)) else {
                    continue;
                };
                for item in items {
                    let blocked = match item {
                        Item::Obstacle(position) => {
                            let obstacle = &board.obstacles[*position];
                            let clearance = match obstacle.kind {
                                ObstacleKind::Copper => board.copper_clearance(&brush, obstacle),
                                ObstacleKind::Keepout => obstacle.clearance,
                                ObstacleKind::Hole => board.hole_clearance.max(obstacle.clearance),
                            };
                            obstacle.shape.distance_to_point(point) < clearance + half
                        }
                        Item::Segment { a, b, half_width, clearance } => {
                            point_segment_distance(point, *a, *b) < half_width + clearance.max(brush.clearance) + half
                        }
                        Item::Via { at, radius, clearance } => distance(point, *at) < radius + clearance.max(brush.clearance) + half,
                        Item::Edge { a, b, margin } => {
                            edges.push((*a, *b));
                            point_segment_distance(point, *a, *b) < *margin
                        }
                        Item::Ring { pad, reach } => except.is_some_and(|except| !except.contains(pad)) && board.obstacles[*pad].shape.distance_to_point(point) < *reach,
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
