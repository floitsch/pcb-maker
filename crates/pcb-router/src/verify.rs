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
/// the layer (`None`: a via), a via's drill radius, the index of the
/// segment it is and its layers (a via's: every layer, or its span's).
type RoutedItem = (
    Point,
    Point,
    f64,
    Option<usize>,
    Option<f64>,
    Option<usize>,
    crate::board::LayerMask,
);

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
/// between routed copper and against copper obstacles, and the width floor
/// per layer of a class with widths per layer (a track narrower than its
/// layer's minimum: `other` "track width"). Without rules, the classes'
/// clearances only, as `verify`.
pub fn verify_with_rules(
    board: &Board,
    routes: &[NetRoute],
    rules: Option<&crate::rules::ClassRules>,
) -> Vec<Violation> {
    let widths = rules.filter(|rules| rules.has_layer_widths());
    let mut verifier = Verifier::new(board, routes, rules.filter(|rules| rules.clearances));
    for (net, route) in routes.iter().enumerate() {
        let net = net as NetId;
        let class_index = board.nets[net as usize].class;
        // A class with widths per layer: each track at least its layer's
        // floor (KiCad's track_width minimum there).
        if let Some(widths) = widths.filter(|widths| !widths.class_floors[class_index].is_empty()) {
            verifier.check_widths(net, route, widths);
        }
        for item in routed_items(board, route) {
            verifier.check_item(&Checked::new(board, net, item));
        }
    }
    verifier.violations
}

/// What `verify_with_rules` reads and the violations it found.
struct Verifier<'a> {
    board: &'a Board,
    routes: &'a [NetRoute],
    /// The rules beyond the classes, when they ask clearances.
    rules: Option<&'a crate::rules::ClassRules>,
    /// The obstacles and routed items by `BUCKET` square, each in every
    /// bucket within the largest clearance of its box.
    index: HashMap<(i64, i64), Vec<Item>>,
    violations: Vec<Violation>,
}

/// One routed item being checked, as a centreline and a copper radius.
struct Checked {
    net: NetId,
    class_index: usize,
    class: crate::board::RuleClass,
    start: Point,
    end: Point,
    radius: f64,
    /// The segment's layer, `None` for a via.
    layer: Option<usize>,
    /// A via's drill radius.
    drill: Option<f64>,
    /// The index of the segment it is.
    own_segment: Option<usize>,
    layer_mask: crate::board::LayerMask,
    /// Its kind for the rules (`TRACK` or `VIA`).
    own_kind: usize,
    /// Where a violation is reported, and on which layer.
    at: Point,
    report_layer: usize,
}

impl Checked {
    fn new(board: &Board, net: NetId, item: RoutedItem) -> Self {
        let (start, end, radius, layer, drill, own_segment, layer_mask) = item;
        let class_index = board.nets[net as usize].class;
        Self {
            net,
            class_index,
            class: board.classes[class_index],
            start,
            end,
            radius,
            layer,
            drill,
            own_segment,
            layer_mask,
            own_kind: if layer.is_some() {
                crate::rules::TRACK
            } else {
                crate::rules::VIA
            },
            at: [(start[0] + end[0]) / 2.0, (start[1] + end[1]) / 2.0],
            report_layer: layer.unwrap_or(layer_mask.trailing_zeros() as usize),
        }
    }
}

/// A net's routed items: its segments, then its vias.
fn routed_items(board: &Board, route: &NetRoute) -> Vec<RoutedItem> {
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
                1 << segment.layer,
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
            via.mask(board.layer_count),
        )
    }));
    items
}

impl<'a> Verifier<'a> {
    /// The verifier with its spatial index of the obstacles and the routed
    /// items.
    fn new(
        board: &'a Board,
        routes: &'a [NetRoute],
        rules: Option<&'a crate::rules::ClassRules>,
    ) -> Self {
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
        Self {
            board,
            routes,
            rules,
            index,
            violations: Vec::new(),
        }
    }

    /// The clearance the rules ask beyond the classes between an item of
    /// kind `kind_a` of `net_a` and one of kind `kind_b` of `net_b`, on
    /// `layer` (`None`: a via against a via, on every layer).
    fn extra(
        &self,
        net_a: NetId,
        kind_a: usize,
        net_b: NetId,
        kind_b: usize,
        layer: Option<usize>,
    ) -> f64 {
        self.rules.map_or(0.0, |rules| {
            let group = |net: NetId| rules.class_group[self.board.nets[net as usize].class];
            let (a, b) = (group(net_a), group(net_b));
            match layer {
                Some(layer) => rules.resolved.extra_on(a, kind_a, b, kind_b, layer),
                None => rules.resolved.extra_anywhere(a, kind_a, b, kind_b),
            }
        })
    }

    /// Records a violation where `actual` falls short of `required`.
    #[allow(clippy::too_many_arguments)]
    fn report(
        &mut self,
        net: NetId,
        segment: Option<usize>,
        other_segment: Option<(NetId, usize)>,
        other: String,
        layer: usize,
        at: Point,
        required: f64,
        actual: f64,
    ) {
        if actual + TOLERANCE < required {
            self.violations.push(Violation {
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
    }

    /// Each track of `net` at least its layer's width floor.
    fn check_widths(&mut self, net: NetId, route: &NetRoute, widths: &crate::rules::ClassRules) {
        let class_index = self.board.nets[net as usize].class;
        for (index, segment) in route.segments.iter().enumerate() {
            self.report(
                net,
                Some(index),
                None,
                "track width".into(),
                segment.layer,
                [
                    (segment.start[0] + segment.end[0]) / 2.0,
                    (segment.start[1] + segment.end[1]) / 2.0,
                ],
                widths.floor_on(class_index, segment.layer),
                segment.width,
            );
        }
    }

    /// One routed item against the board edge and every obstacle and
    /// other net's item in its buckets (each once).
    fn check_item(&mut self, item: &Checked) {
        let board = self.board;
        let (start, end, radius) = (item.start, item.end, item.radius);
        let inside =
            point_in_polygon(start, &board.outline) && point_in_polygon(end, &board.outline);
        let edge_distance = polygon_edges(&board.outline)
            .map(|(a, b)| segment_segment_distance(start, end, a, b))
            .fold(f64::INFINITY, f64::min);
        self.report(
            item.net,
            item.own_segment,
            None,
            "board edge".into(),
            item.report_layer,
            item.at,
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
        // The index taken while the checks report (given back below).
        let spatial = std::mem::take(&mut self.index);
        for bucket in buckets(bounds) {
            for found in spatial.get(&bucket).into_iter().flatten() {
                match *found {
                    Item::Obstacle { index } => {
                        if seen.contains(&(0, index)) {
                            continue;
                        }
                        seen.push((0, index));
                        self.check_obstacle(item, index);
                    }
                    Item::Segment { net: other, index } => {
                        // Every unordered pair is visited from its lower net.
                        if other <= item.net || seen.contains(&(1 + other as usize, index)) {
                            continue;
                        }
                        seen.push((1 + other as usize, index));
                        self.check_segment(item, other, index);
                    }
                    Item::Via { net: other, index } => {
                        if other <= item.net || seen.contains(&(usize::MAX - other as usize, index))
                        {
                            continue;
                        }
                        seen.push((usize::MAX - other as usize, index));
                        self.check_via(item, other, index);
                    }
                }
            }
        }
        self.index = spatial;
    }

    /// The item against obstacle `index` (another net's, on its layers,
    /// keeping its kind out): the clearance, and a via's drill's hole
    /// clearance or hole-to-hole spacing.
    fn check_obstacle(&mut self, item: &Checked, index: usize) {
        let board = self.board;
        let (net, start, end, layer) = (item.net, item.start, item.end, item.layer);
        let obstacle = &board.obstacles[index];
        if obstacle.net == Some(net) || obstacle.layers & item.layer_mask == 0 {
            return;
        }
        let is_via = layer.is_none();
        if (is_via && !obstacle.blocks_vias) || (!is_via && !obstacle.blocks_tracks) {
            return;
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
            return;
        }
        let distance = obstacle.shape.distance_to_segment(start, end);
        let required = match obstacle.kind {
            ObstacleKind::Copper => {
                let clearance = board.copper_clearance(&item.class, obstacle);
                match (self.rules, layer) {
                    (None, _) => clearance,
                    (Some(rules), Some(layer)) => clearance.max(rules.obstacle_extra(
                        item.class_index,
                        crate::rules::TRACK,
                        obstacle,
                        layer,
                    )),
                    (Some(rules), None) => clearance.max(rules.obstacle_extra_via(
                        item.class_index,
                        obstacle,
                        board.layer_count,
                    )),
                }
            }
            ObstacleKind::Keepout => obstacle.clearance,
            ObstacleKind::Hole => board.hole_clearance.max(obstacle.clearance),
        };
        self.report(
            net,
            item.own_segment,
            None,
            obstacle.label.clone(),
            item.report_layer,
            item.at,
            required,
            distance - item.radius,
        );
        // A via's drill keeps the hole clearance from
        // copper.
        if let (Some(drill), ObstacleKind::Copper) = (item.drill, obstacle.kind) {
            self.report(
                net,
                item.own_segment,
                None,
                format!("{} (via hole)", obstacle.label),
                item.report_layer,
                item.at,
                board.hole_clearance,
                distance - drill,
            );
        }
        if let (Some(drill), ObstacleKind::Hole) = (item.drill, obstacle.kind) {
            self.report(
                net,
                item.own_segment,
                None,
                format!("{} (hole to hole)", obstacle.label),
                item.report_layer,
                item.at,
                board.hole_to_hole,
                distance - drill,
            );
        }
    }

    /// The item against segment `index` of net `other` (on a layer the
    /// item is on): the clearance, and a via's drill's hole clearance.
    fn check_segment(&mut self, item: &Checked, other: NetId, index: usize) {
        let board = self.board;
        let net = item.net;
        let segment = &self.routes[other as usize].segments[index];
        if item.layer_mask & (1 << segment.layer) == 0 {
            return;
        }
        let other_class = board.classes[board.nets[other as usize].class];
        let distance = segment_segment_distance(item.start, item.end, segment.start, segment.end);
        let required = item
            .class
            .clearance
            .max(other_class.clearance)
            .max(self.extra(
                net,
                item.own_kind,
                other,
                crate::rules::TRACK,
                Some(segment.layer),
            ));
        self.report(
            net,
            item.own_segment,
            Some((other, index)),
            board.nets[other as usize].name.clone(),
            segment.layer,
            item.at,
            required,
            distance - item.radius - segment.width / 2.0,
        );
        if let Some(drill) = item.drill {
            self.report(
                net,
                item.own_segment,
                Some((other, index)),
                format!("{} (via hole)", board.nets[other as usize].name),
                segment.layer,
                item.at,
                board.hole_clearance,
                distance - drill - segment.width / 2.0,
            );
        }
    }

    /// The item against via `index` of net `other` (on a layer both are
    /// on): the clearance, the hole-to-hole spacing and both drills' hole
    /// clearances.
    fn check_via(&mut self, item: &Checked, other: NetId, index: usize) {
        let board = self.board;
        let (net, start, end) = (item.net, item.start, item.end);
        let via = &self.routes[other as usize].vias[index];
        // Items meet only on a layer both are on: a
        // via of a span has neither copper nor a hole
        // elsewhere (KiCad checks no clearance and no
        // hole-to-hole between vias of disjoint spans).
        if via.mask(board.layer_count) & item.layer_mask == 0 {
            return;
        }
        let other_class = board.classes[board.nets[other as usize].class];
        let distance = if start == end {
            crate::geometry::distance(start, via.at)
        } else {
            point_segment_distance(via.at, start, end)
        };
        let required = item
            .class
            .clearance
            .max(other_class.clearance)
            .max(self.extra(net, item.own_kind, other, crate::rules::VIA, item.layer));
        self.report(
            net,
            item.own_segment,
            None,
            format!("via of {}", board.nets[other as usize].name),
            item.report_layer,
            item.at,
            required,
            distance - item.radius - via.diameter / 2.0,
        );
        if let Some(drill) = item.drill {
            self.report(
                net,
                item.own_segment,
                None,
                format!("via hole of {}", board.nets[other as usize].name),
                item.report_layer,
                item.at,
                board.hole_to_hole,
                distance - drill - via.drill / 2.0,
            );
            self.report(
                net,
                item.own_segment,
                None,
                format!(
                    "via of {} (hole clearance)",
                    board.nets[other as usize].name
                ),
                item.report_layer,
                item.at,
                board.hole_clearance,
                distance - drill - via.diameter / 2.0,
            );
        }
        // The other via's drill keeps the hole clearance
        // from this copper.
        self.report(
            net,
            item.own_segment,
            None,
            format!(
                "via hole of {} (hole clearance)",
                board.nets[other as usize].name
            ),
            item.report_layer,
            item.at,
            board.hole_clearance,
            distance - item.radius - via.drill / 2.0,
        );
    }
}

/// The distance between two filled shapes (zero where they overlap).
pub fn shape_distance(a: &Shape, b: &Shape) -> f64 {
    match (a, b) {
        (Shape::Circle { center, radius }, other) | (other, Shape::Circle { center, radius }) => {
            (other.distance_to_point(*center) - radius).max(0.0)
        }
        (Shape::Capsule { start, end, radius }, other)
        | (other, Shape::Capsule { start, end, radius }) => {
            (other.distance_to_segment(*start, *end) - radius).max(0.0)
        }
        (Shape::Union { parts }, other) | (other, Shape::Union { parts }) => parts
            .iter()
            .map(|part| shape_distance(part, other))
            .fold(f64::INFINITY, f64::min),
        (Shape::Polygon { points: first }, Shape::Polygon { points: second }) => {
            if first
                .first()
                .is_some_and(|point| point_in_polygon(*point, second))
                || second
                    .first()
                    .is_some_and(|point| point_in_polygon(*point, first))
            {
                return 0.0;
            }
            polygon_edges(first)
                .flat_map(|(a, b)| {
                    polygon_edges(second).map(move |(c, d)| segment_segment_distance(a, b, c, d))
                })
                .fold(f64::INFINITY, f64::min)
        }
    }
}

/// KiCad's connectivity of `net` without its pours: which of its terminals
/// the copper of the net joins, the emitted `route` (tracks and vias) and
/// the board's copper of the net (pads, the pads merged into a terminal,
/// existing tracks, graphics), items joined where their copper overlaps on
/// a layer they share. A terminal with a contact (a custom pad) is joined
/// through it only. Returns per terminal a component number (equal numbers
/// are joined). Without pours this is what KiCad's DRC counts; fill only
/// adds connections, so terminals it joins are joined in KiCad too.
pub fn copper_components(board: &Board, net: NetId, route: &NetRoute) -> Vec<usize> {
    let description = &board.nets[net as usize];
    // Elements: the terminals (their contact or pad), the net's other
    // copper obstacles, the segments, the vias.
    let mut shapes: Vec<(Shape, u64)> = Vec::new();
    let mut element_of_obstacle: HashMap<usize, usize> = HashMap::new();
    for terminal in &description.terminals {
        let pad = &board.obstacles[terminal.pad];
        let shape = terminal
            .contact
            .clone()
            .unwrap_or_else(|| pad.shape.clone());
        element_of_obstacle.insert(terminal.pad, shapes.len());
        shapes.push((shape, pad.layers as u64));
    }
    let terminal_count = shapes.len();
    let mut joins: Vec<(usize, usize)> = Vec::new();
    for merged in board.merged_pads.iter().filter(|merged| merged.net == net) {
        let element = shapes.len();
        let pad = &board.obstacles[merged.pad];
        shapes.push((pad.shape.clone(), pad.layers as u64));
        element_of_obstacle.insert(merged.pad, element);
        if merged.host < terminal_count {
            joins.push((element, merged.host));
        }
    }
    for (index, obstacle) in board.obstacles.iter().enumerate() {
        if obstacle.net == Some(net)
            && obstacle.kind == ObstacleKind::Copper
            && !element_of_obstacle.contains_key(&index)
            // Another terminal's custom pad joins through its contact only.
            && !description
                .terminals
                .iter()
                .any(|terminal| terminal.pad == index)
        {
            shapes.push((obstacle.shape.clone(), obstacle.layers as u64));
        }
    }
    for segment in &route.segments {
        shapes.push((
            Shape::Capsule {
                start: segment.start,
                end: segment.end,
                radius: segment.width / 2.0,
            },
            1u64 << segment.layer,
        ));
    }
    for via in &route.vias {
        shapes.push((
            Shape::Circle {
                center: via.at,
                radius: via.diameter / 2.0,
            },
            // A via of a span joins its layers only.
            via.span.map_or(u64::MAX, |span| span.mask() as u64),
        ));
    }
    let mut parent: Vec<usize> = (0..shapes.len()).collect();
    fn root(parent: &mut [usize], mut item: usize) -> usize {
        while parent[item] != item {
            parent[item] = parent[parent[item]];
            item = parent[item];
        }
        item
    }
    let join = |parent: &mut Vec<usize>, a: usize, b: usize| {
        let (a, b) = (root(parent, a), root(parent, b));
        if a != b {
            parent[a.max(b)] = a.min(b);
        }
    };
    for (a, b) in joins {
        join(&mut parent, a, b);
    }
    // Pairs by a sweep over the boxes' x ranges.
    let boxes: Vec<Aabb> = shapes.iter().map(|(shape, _)| shape.aabb()).collect();
    let mut order: Vec<usize> = (0..shapes.len()).collect();
    order.sort_by(|a, b| boxes[*a].minimum[0].total_cmp(&boxes[*b].minimum[0]));
    for (position, &a) in order.iter().enumerate() {
        for &b in &order[position + 1..] {
            if boxes[b].minimum[0] > boxes[a].maximum[0] + TOLERANCE {
                break;
            }
            if shapes[a].1 & shapes[b].1 == 0
                || !boxes[a].inflated(TOLERANCE).intersects(boxes[b])
                || root(&mut parent, a) == root(&mut parent, b)
            {
                continue;
            }
            if shape_distance(&shapes[a].0, &shapes[b].0) <= TOLERANCE {
                join(&mut parent, a, b);
            }
        }
    }
    (0..terminal_count)
        .map(|terminal| root(&mut parent, terminal))
        .collect()
}
