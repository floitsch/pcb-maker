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
    /// Index into the net's segments, or `None` for one of its vias.
    pub segment: Option<usize>,
    /// The other routed segment involved, if any.
    pub other_segment: Option<(NetId, usize)>,
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

/// Well under KiCad's DRC epsilon (0.5 µm): positions are written in
/// nanometres, and two tracks at exactly their clearance are legal.
const TOLERANCE: f64 = 1.0e-5;
const BUCKET: f64 = 2.0;

/// A routed item as a centreline and a copper radius: start, end, radius,
/// the layer (`None`: a via, on every layer), a via's drill radius and the
/// index of the segment it is.
type RoutedItem = (Point, Point, f64, Option<usize>, Option<f64>, Option<usize>);

fn buckets(bounds: Aabb) -> impl Iterator<Item = (i64, i64)> {
    let x0 = (bounds.minimum[0] / BUCKET).floor() as i64;
    let y0 = (bounds.minimum[1] / BUCKET).floor() as i64;
    let x1 = (bounds.maximum[0] / BUCKET).floor() as i64;
    let y1 = (bounds.maximum[1] / BUCKET).floor() as i64;
    (y0..=y1).flat_map(move |y| (x0..=x1).map(move |x| (x, y)))
}

pub fn verify(board: &Board, routes: &[NetRoute]) -> Vec<Violation> {
    verify_with_rules(board, routes, None)
}

/// `verify` that also holds the rules beyond the classes (`ClassRules` of
/// the router's split `board`): pair clearances per item kind and layer
/// between routed copper and against copper obstacles. Without rules, the
/// classes' clearances only, as `verify`.
pub fn verify_with_rules(
    board: &Board,
    routes: &[NetRoute],
    rules: Option<&crate::rules::ClassRules>,
) -> Vec<Violation> {
    let rules = rules.filter(|rules| rules.clearances);
    // The clearance the rules ask beyond the classes between an item of
    // kind `kind_a` of `net_a` and one of kind `kind_b` of `net_b`, on
    // `layer` (`None`: a via against a via, on every layer).
    let extra = |net_a: NetId, kind_a: usize, net_b: NetId, kind_b: usize, layer: Option<usize>| {
        rules.map_or(0.0, |rules| {
            let group = |net: NetId| rules.class_group[board.nets[net as usize].class];
            let (a, b) = (group(net_a), group(net_b));
            match layer {
                Some(layer) => rules.resolved.extra_on(a, kind_a, b, kind_b, layer),
                None => rules.resolved.extra_anywhere(a, kind_a, b, kind_b),
            }
        })
    };
    let max_clearance = board
        .classes
        .iter()
        .map(|class| class.clearance)
        .chain(board.obstacles.iter().map(|obstacle| {
            obstacle
                .clearance
                .max(obstacle.clearance_override.unwrap_or(0.0))
        }))
        .fold(board.hole_clearance, f64::max)
        .max(rules.map_or(0.0, |rules| rules.resolved.largest()));

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
    let mut report = |net: NetId,
                      segment: Option<usize>,
                      other_segment: Option<(NetId, usize)>,
                      other: String,
                      layer: usize,
                      at: Point,
                      required: f64,
                      actual: f64| {
        if actual + TOLERANCE < required {
            violations.push(Violation {
                net,
                segment,
                other_segment,
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
        let class_index = board.nets[net as usize].class;
        let class = board.classes[class_index];
        // A routed item is described as a centreline plus a copper radius.
        let mut items: Vec<RoutedItem> = route
            .segments
            .iter()
            .enumerate()
            .map(|(index, segment)| {
                (
                    segment.start,
                    segment.end,
                    segment.width / 2.0,
                    Some(segment.layer),
                    None,
                    Some(index),
                )
            })
            .collect();
        items.extend(route.vias.iter().map(|via| {
            (
                via.at,
                via.at,
                via.diameter / 2.0,
                None,
                Some(via.drill / 2.0),
                None,
            )
        }));

        for (start, end, radius, layer, drill, own_segment) in items {
            let own_kind = if layer.is_some() {
                crate::rules::TRACK
            } else {
                crate::rules::VIA
            };
            let at = [(start[0] + end[0]) / 2.0, (start[1] + end[1]) / 2.0];
            let layer_mask = layer.map_or(board.all_layers(), |layer| 1 << layer);
            let report_layer = layer.unwrap_or(0);

            let inside =
                point_in_polygon(start, &board.outline) && point_in_polygon(end, &board.outline);
            let edge_distance = polygon_edges(&board.outline)
                .map(|(a, b)| segment_segment_distance(start, end, a, b))
                .fold(f64::INFINITY, f64::min);
            report(
                net,
                own_segment,
                None,
                "board edge".into(),
                report_layer,
                at,
                board.edge_clearance,
                if inside {
                    edge_distance - radius
                } else {
                    -radius
                },
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
                            // A pad inside a keepout leaves it by its stub
                            // (the router's `stub_is_clear`).
                            if !is_via
                                && obstacle.kind == ObstacleKind::Keepout
                                && obstacle.label.starts_with("solder mask opening")
                                && (obstacle.shape.contains(start) || obstacle.shape.contains(end))
                                && board.nets[net as usize]
                                    .terminals
                                    .iter()
                                    .any(|terminal| obstacle.shape.contains(terminal.anchor))
                            {
                                continue;
                            }
                            let distance = obstacle.shape.distance_to_segment(start, end);
                            let required = match obstacle.kind {
                                ObstacleKind::Copper => {
                                    let clearance = board.copper_clearance(&class, obstacle);
                                    match (rules, layer) {
                                        (None, _) => clearance,
                                        (Some(rules), Some(layer)) => {
                                            clearance.max(rules.obstacle_extra(
                                                class_index,
                                                crate::rules::TRACK,
                                                obstacle,
                                                layer,
                                            ))
                                        }
                                        (Some(rules), None) => {
                                            clearance.max(rules.obstacle_extra_via(
                                                class_index,
                                                obstacle,
                                                board.layer_count,
                                            ))
                                        }
                                    }
                                }
                                ObstacleKind::Keepout => obstacle.clearance,
                                ObstacleKind::Hole => board.hole_clearance.max(obstacle.clearance),
                            };
                            report(
                                net,
                                own_segment,
                                None,
                                obstacle.label.clone(),
                                report_layer,
                                at,
                                required,
                                distance - radius,
                            );
                            // A via's drill keeps the hole clearance from
                            // copper.
                            if let (Some(drill), ObstacleKind::Copper) = (drill, obstacle.kind) {
                                report(
                                    net,
                                    own_segment,
                                    None,
                                    format!("{} (via hole)", obstacle.label),
                                    report_layer,
                                    at,
                                    board.hole_clearance,
                                    distance - drill,
                                );
                            }
                            if let (Some(drill), ObstacleKind::Hole) = (drill, obstacle.kind) {
                                report(
                                    net,
                                    own_segment,
                                    None,
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
                                own_segment,
                                Some((other, index)),
                                board.nets[other as usize].name.clone(),
                                segment.layer,
                                at,
                                class.clearance.max(other_class.clearance).max(extra(
                                    net,
                                    own_kind,
                                    other,
                                    crate::rules::TRACK,
                                    Some(segment.layer),
                                )),
                                distance - radius - segment.width / 2.0,
                            );
                            if let Some(drill) = drill {
                                report(
                                    net,
                                    own_segment,
                                    Some((other, index)),
                                    format!("{} (via hole)", board.nets[other as usize].name),
                                    segment.layer,
                                    at,
                                    board.hole_clearance,
                                    distance - drill - segment.width / 2.0,
                                );
                            }
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
                                own_segment,
                                None,
                                format!("via of {}", board.nets[other as usize].name),
                                report_layer,
                                at,
                                class.clearance.max(other_class.clearance).max(extra(
                                    net,
                                    own_kind,
                                    other,
                                    crate::rules::VIA,
                                    layer,
                                )),
                                distance - radius - via.diameter / 2.0,
                            );
                            if let Some(drill) = drill {
                                report(
                                    net,
                                    own_segment,
                                    None,
                                    format!("via hole of {}", board.nets[other as usize].name),
                                    report_layer,
                                    at,
                                    board.hole_to_hole,
                                    distance - drill - via.drill / 2.0,
                                );
                                report(
                                    net,
                                    own_segment,
                                    None,
                                    format!(
                                        "via of {} (hole clearance)",
                                        board.nets[other as usize].name
                                    ),
                                    report_layer,
                                    at,
                                    board.hole_clearance,
                                    distance - drill - via.diameter / 2.0,
                                );
                            }
                            // The other via's drill keeps the hole clearance
                            // from this copper.
                            report(
                                net,
                                own_segment,
                                None,
                                format!(
                                    "via hole of {} (hole clearance)",
                                    board.nets[other as usize].name
                                ),
                                report_layer,
                                at,
                                board.hole_clearance,
                                distance - radius - via.drill / 2.0,
                            );
                        }
                    }
                }
            }
        }
    }
    violations
}
