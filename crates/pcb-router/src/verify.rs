// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Exact clearance verification of emitted copper. It is independent of the
//! lattice and of the occupancy maps: it only sees segments, vias, obstacles
//! and the outline, and measures true distances between them.

use std::collections::HashMap;

use crate::board::{Board, NetId, NetRoute, ObstacleKind};
use crate::geometry::{
    Aabb, Point, Shape, point_in_polygon, point_segment_distance, polygon_edges,
    segment_segment_distance,
};

#[derive(Clone, Debug)]
pub struct Violation {
    pub net: NetId,
    pub other: String,
    pub layer: usize,
    pub at: Point,
    pub required: f64,
    pub actual: f64,
}

#[derive(Clone, Copy)]
enum Item {
    Segment { net: NetId, index: usize },
    Via { net: NetId, index: usize },
    Obstacle { index: usize },
}

const TOLERANCE: f64 = 1.0e-6;
const BUCKET: f64 = 2.0;

fn buckets(bounds: Aabb) -> impl Iterator<Item = (i64, i64)> {
    let x0 = (bounds.minimum[0] / BUCKET).floor() as i64;
    let y0 = (bounds.minimum[1] / BUCKET).floor() as i64;
    let x1 = (bounds.maximum[0] / BUCKET).floor() as i64;
    let y1 = (bounds.maximum[1] / BUCKET).floor() as i64;
    (y0..=y1).flat_map(move |y| (x0..=x1).map(move |x| (x, y)))
}

pub fn verify(board: &Board, routes: &[NetRoute]) -> Vec<Violation> {
    let max_clearance = board
        .classes
        .iter()
        .map(|class| class.clearance)
        .chain(board.obstacles.iter().map(|obstacle| obstacle.clearance))
        .fold(board.hole_clearance, f64::max);

    let mut index: HashMap<(i64, i64), Vec<Item>> = HashMap::new();
    for (obstacle_index, obstacle) in board.obstacles.iter().enumerate() {
        for bucket in buckets(obstacle.shape.aabb().inflated(max_clearance)) {
            index.entry(bucket).or_default().push(Item::Obstacle {
                index: obstacle_index,
            });
        }
    }
    for (net, route) in routes.iter().enumerate() {
        for (segment_index, segment) in route.segments.iter().enumerate() {
            let bounds = Shape::Capsule {
                start: segment.start,
                end: segment.end,
                radius: segment.width / 2.0,
            }
            .aabb();
            for bucket in buckets(bounds.inflated(max_clearance)) {
                index.entry(bucket).or_default().push(Item::Segment {
                    net: net as NetId,
                    index: segment_index,
                });
            }
        }
        for (via_index, via) in route.vias.iter().enumerate() {
            let bounds = Shape::Circle {
                center: via.at,
                radius: via.diameter / 2.0,
            }
            .aabb();
            for bucket in buckets(bounds.inflated(max_clearance)) {
                index.entry(bucket).or_default().push(Item::Via {
                    net: net as NetId,
                    index: via_index,
                });
            }
        }
    }

    let mut violations = Vec::new();
    let mut report = |net: NetId, other: String, layer: usize, at: Point, required: f64, actual: f64| {
        if actual + TOLERANCE < required {
            violations.push(Violation {
                net,
                other,
                layer,
                at,
                required,
                actual,
            });
        }
    };

    for (net, route) in routes.iter().enumerate() {
        let net = net as NetId;
        let class = board.classes[board.nets[net as usize].class];
        // A routed item is described as a centreline plus a copper radius.
        let mut items: Vec<(Point, Point, f64, Option<usize>, Option<f64>)> = route
            .segments
            .iter()
            .map(|segment| {
                (
                    segment.start,
                    segment.end,
                    segment.width / 2.0,
                    Some(segment.layer),
                    None,
                )
            })
            .collect();
        items.extend(
            route
                .vias
                .iter()
                .map(|via| (via.at, via.at, via.diameter / 2.0, None, Some(via.drill / 2.0))),
        );

        for (start, end, radius, layer, drill) in items {
            let at = [(start[0] + end[0]) / 2.0, (start[1] + end[1]) / 2.0];
            let layer_mask = layer.map_or(board.all_layers(), |layer| 1 << layer);
            let report_layer = layer.unwrap_or(0);

            let inside = point_in_polygon(start, &board.outline)
                && point_in_polygon(end, &board.outline);
            let edge_distance = polygon_edges(&board.outline)
                .map(|(a, b)| segment_segment_distance(start, end, a, b))
                .fold(f64::INFINITY, f64::min);
            report(
                net,
                "board edge".into(),
                report_layer,
                at,
                board.edge_clearance,
                if inside { edge_distance - radius } else { -radius },
            );

            let bounds = Aabb {
                minimum: [start[0].min(end[0]), start[1].min(end[1])],
                maximum: [start[0].max(end[0]), start[1].max(end[1])],
            }
            .inflated(radius);
            let mut seen = Vec::new();
            for bucket in buckets(bounds) {
                for item in index.get(&bucket).into_iter().flatten() {
                    match *item {
                        Item::Obstacle { index } => {
                            if seen.contains(&(0, index)) {
                                continue;
                            }
                            seen.push((0, index));
                            let obstacle = &board.obstacles[index];
                            if obstacle.net == Some(net) || obstacle.layers & layer_mask == 0 {
                                continue;
                            }
                            let is_via = layer.is_none();
                            if (is_via && !obstacle.blocks_vias)
                                || (!is_via && !obstacle.blocks_tracks)
                            {
                                continue;
                            }
                            let distance = obstacle.shape.distance_to_segment(start, end);
                            let required = match obstacle.kind {
                                ObstacleKind::Copper => class.clearance.max(obstacle.clearance),
                                ObstacleKind::Keepout => 0.0,
                                ObstacleKind::Hole => board.hole_clearance,
                            };
                            report(
                                net,
                                obstacle.label.clone(),
                                report_layer,
                                at,
                                required,
                                distance - radius,
                            );
                            if let (Some(drill), ObstacleKind::Hole) = (drill, obstacle.kind) {
                                report(
                                    net,
                                    format!("{} (hole to hole)", obstacle.label),
                                    report_layer,
                                    at,
                                    board.hole_to_hole,
                                    distance - drill,
                                );
                            }
                        }
                        Item::Segment { net: other, index } => {
                            // Every unordered pair is visited from its lower net.
                            if other <= net || seen.contains(&(1 + other as usize, index)) {
                                continue;
                            }
                            seen.push((1 + other as usize, index));
                            let segment = &routes[other as usize].segments[index];
                            if layer.is_some_and(|layer| layer != segment.layer) {
                                continue;
                            }
                            let other_class = board.classes[board.nets[other as usize].class];
                            let distance =
                                segment_segment_distance(start, end, segment.start, segment.end);
                            report(
                                net,
                                board.nets[other as usize].name.clone(),
                                segment.layer,
                                at,
                                class.clearance.max(other_class.clearance),
                                distance - radius - segment.width / 2.0,
                            );
                        }
                        Item::Via { net: other, index } => {
                            if other <= net || seen.contains(&(usize::MAX - other as usize, index))
                            {
                                continue;
                            }
                            seen.push((usize::MAX - other as usize, index));
                            let via = &routes[other as usize].vias[index];
                            let other_class = board.classes[board.nets[other as usize].class];
                            let distance = if start == end {
                                crate::geometry::distance(start, via.at)
                            } else {
                                point_segment_distance(via.at, start, end)
                            };
                            report(
                                net,
                                format!("via of {}", board.nets[other as usize].name),
                                report_layer,
                                at,
                                class.clearance.max(other_class.clearance),
                                distance - radius - via.diameter / 2.0,
                            );
                            if let Some(drill) = drill {
                                report(
                                    net,
                                    format!("via hole of {}", board.nets[other as usize].name),
                                    report_layer,
                                    at,
                                    board.hole_to_hole,
                                    distance - drill - via.drill / 2.0,
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    violations
}
