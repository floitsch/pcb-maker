// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Geometry for topological wires ("metrization"). A wire is cut at its
//! vias into pieces, one per layer; each piece comes with an embedding, a
//! polyline that fixes on which side of every obstacle it passes. On each
//! layer the pieces are pulled tight in their homotopy class ([`taut`]):
//!
//! - the layer's obstacles and the vias are triangulated; circles and
//!   round ends are single points with a radius;
//! - a piece's embedding traced through that triangulation gives its
//!   channel, and the order in which pieces cross each edge;
//! - every vertex of a channel is a disc of the room the piece needs from
//!   it: the obstacle's clearance plus half the width, grown by the pieces
//!   of other nets that pass it on the inside (their actual distance plus
//!   their width and clearance). Pieces are pulled tight again until those
//!   radii settle;
//! - exact distances to the obstacles' shapes add the corners of any
//!   obstacle a path still comes too close to.
//!
//! Arcs are emitted as polylines circumscribing them.

use crate::taut::{self, Disc, Path, Portal, cross, distance, sub};
use pcb_router::{Board, ClassId, NetId, NetRoute, ObstacleKind, Point, Segment, Shape, Via};
use spade::{ConstrainedDelaunayTriangulation, HasPosition, Intersection, LineIntersectionIterator, Point2, Triangulation};
use std::collections::HashMap;

/// Room kept beyond every clearance, for rounding in the output.
pub const MARGIN: f64 = 0.001;
/// How far an arc's polyline may stray outside the arc.
const BULGE: f64 = 0.0005;
const ROUNDS: usize = 40;

#[derive(Clone, Copy, Debug)]
struct Vertex {
    position: Point2<f64>,
}

impl HasPosition for Vertex {
    type Scalar = f64;
    fn position(&self) -> Point2<f64> {
        self.position
    }
}

/// A stretch of wire on one layer.
#[derive(Clone, Debug)]
pub struct Piece {
    pub wire: usize,
    pub net: NetId,
    pub class: ClassId,
    pub layer: usize,
    pub width: f64,
    /// A polyline in the piece's homotopy class, from its start to its end.
    pub points: Vec<Point>,
    /// The pad (obstacle) each end connects to, if it is a terminal: the
    /// end may then move inside the pad to where it is legal.
    pub pads: [Option<usize>; 2],
}

#[derive(Clone, Debug)]
pub struct PlacedVia {
    pub wire: usize,
    pub net: NetId,
    pub class: ClassId,
    pub at: Point,
}

/// What a vertex of a layer's triangulation stands for.
#[derive(Clone, Copy, Debug)]
enum Feature {
    /// A corner (radius 0) or centre of an obstacle's shape.
    Obstacle { obstacle: usize, radius: f64 },
    Via { via: usize },
    Outline,
}

struct LayerMesh {
    cdt: ConstrainedDelaunayTriangulation<Vertex>,
    positions: Vec<Point>,
    features: Vec<Vec<Feature>>,
    /// Vertex by position, for obstacle corners found by exact checks.
    at: HashMap<(i64, i64), usize>,
}

fn key(point: Point) -> (i64, i64) {
    ((point[0] * 1.0e6).round() as i64, (point[1] * 1.0e6).round() as i64)
}

impl LayerMesh {
    fn new(board: &Board, layer: usize, vias: &[PlacedVia]) -> Self {
        let mut cdt: ConstrainedDelaunayTriangulation<Vertex> = ConstrainedDelaunayTriangulation::new();
        let mut features: HashMap<usize, Vec<Feature>> = HashMap::new();
        let insert = |cdt: &mut ConstrainedDelaunayTriangulation<Vertex>, point: Point| {
            cdt.insert(Vertex { position: Point2::new(point[0], point[1]) }).ok()
        };
        let ring = |cdt: &mut ConstrainedDelaunayTriangulation<Vertex>, points: &[Point], features: &mut HashMap<usize, Vec<Feature>>, feature: Feature| {
            let handles: Vec<_> = points.iter().filter_map(|&point| cdt.insert(Vertex { position: Point2::new(point[0], point[1]) }).ok()).collect();
            for handle in &handles {
                features.entry(handle.index()).or_default().push(feature);
            }
            for index in 0..handles.len() {
                let (from, to) = (handles[index], handles[(index + 1) % handles.len()]);
                if from != to && handles.len() > 1 {
                    cdt.add_constraint_and_split(from, to, |position| Vertex { position });
                }
            }
        };
        ring(&mut cdt, &board.outline, &mut features, Feature::Outline);
        fn add_shape(
            cdt: &mut ConstrainedDelaunayTriangulation<Vertex>,
            features: &mut HashMap<usize, Vec<Feature>>,
            obstacle: usize,
            shape: &Shape,
            ring: &dyn Fn(&mut ConstrainedDelaunayTriangulation<Vertex>, &[Point], &mut HashMap<usize, Vec<Feature>>, Feature),
        ) {
            match shape {
                Shape::Circle { center, radius } => {
                    if let Ok(handle) = cdt.insert(Vertex { position: Point2::new(center[0], center[1]) }) {
                        features.entry(handle.index()).or_default().push(Feature::Obstacle { obstacle, radius: *radius });
                    }
                }
                Shape::Capsule { start, end, radius } => {
                    let a = cdt.insert(Vertex { position: Point2::new(start[0], start[1]) });
                    let b = cdt.insert(Vertex { position: Point2::new(end[0], end[1]) });
                    if let (Ok(a), Ok(b)) = (a, b) {
                        if a != b {
                            cdt.add_constraint_and_split(a, b, |position| Vertex { position });
                        }
                        for handle in [a, b] {
                            features.entry(handle.index()).or_default().push(Feature::Obstacle { obstacle, radius: *radius });
                        }
                    }
                }
                Shape::Polygon { points } => ring(cdt, points, features, Feature::Obstacle { obstacle, radius: 0.0 }),
                Shape::Union { parts } => {
                    for part in parts {
                        add_shape(cdt, features, obstacle, part, ring);
                    }
                }
            }
        }
        for (index, obstacle) in board.obstacles.iter().enumerate() {
            if obstacle.layers & (1 << layer) != 0 && obstacle.blocks_tracks {
                add_shape(&mut cdt, &mut features, index, &obstacle.shape, &ring);
            }
        }
        for (index, via) in vias.iter().enumerate() {
            if let Some(handle) = insert(&mut cdt, via.at) {
                features.entry(handle.index()).or_default().push(Feature::Via { via: index });
            }
        }
        let positions: Vec<Point> = cdt.vertices().map(|vertex| [vertex.position().x, vertex.position().y]).collect();
        let mut list = vec![Vec::new(); positions.len()];
        for (vertex, found) in features {
            list[vertex] = found;
        }
        // Vertices made by splitting constraints lie on an obstacle's
        // boundary (or the outline) without being one of its corners.
        for (vertex, &point) in positions.iter().enumerate() {
            if !list[vertex].is_empty() {
                continue;
            }
            for (index, obstacle) in board.obstacles.iter().enumerate() {
                if obstacle.layers & (1 << layer) == 0 || !obstacle.blocks_tracks {
                    continue;
                }
                let bounds = obstacle.shape.aabb().inflated(1.0e-6);
                if point[0] < bounds.minimum[0] || point[0] > bounds.maximum[0] || point[1] < bounds.minimum[1] || point[1] > bounds.maximum[1] {
                    continue;
                }
                if obstacle.shape.distance_to_point(point) < 1.0e-6 {
                    list[vertex].push(Feature::Obstacle { obstacle: index, radius: 0.0 });
                }
            }
            if pcb_router::geometry::polygon_edges(&board.outline).any(|(a, b)| pcb_router::geometry::point_segment_distance(point, a, b) < 1.0e-6) {
                list[vertex].push(Feature::Outline);
            }
        }
        let at = positions.iter().enumerate().map(|(index, &point)| (key(point), index)).collect();
        Self { cdt, positions, features: list, at }
    }
}

/// Where a piece crosses an edge of the layer's triangulation.
#[derive(Clone, Copy, Debug)]
struct Crossing {
    edge: usize,
    left: usize,
    right: usize,
    /// From `left` (0) to `right` (1).
    at: f64,
}

/// The edges a polyline crosses, with back-and-forth crossings (no vertex
/// between) cancelled: its channel.
fn trace(mesh: &LayerMesh, points: &[Point]) -> Option<Vec<Crossing>> {
    let mut crossings: Vec<Crossing> = Vec::new();
    for pair in points.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        if distance(from, to) < 1.0e-9 {
            continue;
        }
        let direction = sub(to, from);
        for intersection in LineIntersectionIterator::new(&mesh.cdt, Point2::new(from[0], from[1]), Point2::new(to[0], to[1])) {
            match intersection {
                Intersection::EdgeIntersection(edge) => {
                    let (a, b) = (edge.from(), edge.to());
                    let pa = [a.position().x, a.position().y];
                    let pb = [b.position().x, b.position().y];
                    let (left, right, pl, pr) = if cross(direction, sub(pa, from)) > 0.0 {
                        (a.fix().index(), b.fix().index(), pa, pb)
                    } else {
                        (b.fix().index(), a.fix().index(), pb, pa)
                    };
                    let span = sub(pr, pl);
                    let denominator = cross(direction, span);
                    let at = if denominator.abs() < 1.0e-15 { 0.5 } else { cross(sub(pl, from), direction) / denominator };
                    let undirected = edge.as_undirected().fix().index();
                    if crossings.last().is_some_and(|last| last.edge == undirected) {
                        crossings.pop();
                        continue;
                    }
                    crossings.push(Crossing { edge: undirected, left, right, at: at.clamp(0.0, 1.0) });
                }
                Intersection::VertexIntersection(vertex) => {
                    let p = [vertex.position().x, vertex.position().y];
                    // Starting or ending on a vertex (a via, a round pad's
                    // centre) is expected; passing through one is not.
                    if distance(p, from) > 1.0e-7 && distance(p, to) > 1.0e-7 {
                        return None;
                    }
                }
                // Along an edge: nothing crossed.
                Intersection::EdgeOverlap(_) => {}
            }
        }
    }
    Some(crossings)
}

/// `trace`, retried with the inner points nudged off vertices and edges.
fn trace_nudged(mesh: &LayerMesh, points: &[Point]) -> Option<Vec<Crossing>> {
    if let Some(found) = trace(mesh, points) {
        return Some(found);
    }
    for attempt in 1..4 {
        let count = points.len();
        let nudged: Vec<Point> = points
            .iter()
            .enumerate()
            .map(|(index, &point)| {
                if index == 0 || index + 1 == count {
                    point
                } else {
                    let offset = 1.0e-6 * attempt as f64;
                    [point[0] + offset * 1.3, point[1] + offset * 0.7]
                }
            })
            .collect();
        if let Some(found) = trace(mesh, &nudged) {
            return Some(found);
        }
    }
    None
}

/// The room a piece needs from a feature (0 for its own copper).
fn required(board: &Board, vias: &[PlacedVia], feature: Feature, piece: &Piece) -> f64 {
    let (net, rule) = (piece.net, &board.classes[piece.class]);
    let half = piece.width / 2.0 + MARGIN;
    match feature {
        Feature::Obstacle { obstacle, radius } => {
            let obstacle = &board.obstacles[obstacle];
            if obstacle.kind == ObstacleKind::Copper && obstacle.net == Some(net) {
                return 0.0;
            }
            let gap = match obstacle.kind {
                ObstacleKind::Copper => board.copper_clearance(rule, obstacle),
                ObstacleKind::Hole => board.hole_clearance.max(obstacle.clearance),
                ObstacleKind::Keepout => 0.0,
            };
            radius + gap + half
        }
        Feature::Via { via } => {
            let via = &vias[via];
            if via.net == net {
                return 0.0;
            }
            let other = &board.classes[via.class];
            other.via_diameter / 2.0 + rule.clearance.max(other.clearance) + half
        }
        Feature::Outline => board.edge_clearance + half,
    }
}

/// Obstacles and vias of one layer in buckets, for exact checks.
struct Nearby {
    buckets: HashMap<(i64, i64), Vec<usize>>,
}

const BUCKET: f64 = 2.0;

impl Nearby {
    fn new(board: &Board, layer: usize) -> Self {
        let mut buckets: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
        for (index, obstacle) in board.obstacles.iter().enumerate() {
            if obstacle.layers & (1 << layer) == 0 || !obstacle.blocks_tracks {
                continue;
            }
            let bounds = obstacle.shape.aabb();
            for x in (bounds.minimum[0] / BUCKET).floor() as i64..=(bounds.maximum[0] / BUCKET).floor() as i64 {
                for y in (bounds.minimum[1] / BUCKET).floor() as i64..=(bounds.maximum[1] / BUCKET).floor() as i64 {
                    buckets.entry((x, y)).or_default().push(index);
                }
            }
        }
        Self { buckets }
    }

    fn around(&self, a: Point, b: Point, reach: f64) -> Vec<usize> {
        let mut found = Vec::new();
        let x0 = ((a[0].min(b[0]) - reach) / BUCKET).floor() as i64;
        let x1 = ((a[0].max(b[0]) + reach) / BUCKET).floor() as i64;
        let y0 = ((a[1].min(b[1]) - reach) / BUCKET).floor() as i64;
        let y1 = ((a[1].max(b[1]) + reach) / BUCKET).floor() as i64;
        for x in x0..=x1 {
            for y in y0..=y1 {
                for &index in self.buckets.get(&(x, y)).map(Vec::as_slice).unwrap_or(&[]) {
                    if !found.contains(&index) {
                        found.push(index);
                    }
                }
            }
        }
        found
    }
}

/// The discs of an obstacle's outline nearest to `near`: a polygon's
/// closest edge's two corners, a capsule's ends, a circle's centre.
fn corner_discs(shape: &Shape, near: Point) -> Vec<(Point, f64)> {
    match shape {
        Shape::Circle { center, radius } => vec![(*center, *radius)],
        Shape::Capsule { start, end, radius } => vec![(*start, *radius), (*end, *radius)],
        Shape::Polygon { points } => {
            let mut best = (f64::INFINITY, 0);
            for index in 0..points.len() {
                let d = taut::point_segment_distance(near, points[index], points[(index + 1) % points.len()]);
                if d < best.0 {
                    best = (d, index);
                }
            }
            vec![(points[best.1], 0.0), (points[(best.1 + 1) % points.len()], 0.0)]
        }
        Shape::Union { parts } => parts
            .iter()
            .min_by(|a, b| a.distance_to_point(near).total_cmp(&b.distance_to_point(near)))
            .map(|part| corner_discs(part, near))
            .unwrap_or_default(),
    }
}

/// Why a piece has no geometry, and where it got stuck (with how many
/// millimetres were missing there), if known.
#[derive(Clone, Debug)]
pub struct Failed {
    pub reason: String,
    pub at: Option<Point>,
    pub deficit: f64,
}

impl Failed {
    fn new(reason: String) -> Self {
        Self { reason, at: None, deficit: 0.0 }
    }
}

/// Copper per net, and which wire each segment and via belongs to.
pub struct Realized {
    pub routes: Vec<NetRoute>,
    pub segment_wire: Vec<Vec<usize>>,
    pub via_wire: Vec<Vec<usize>>,
    /// Pieces (by index) that could not be pulled tight legally, and why.
    pub failed: Vec<(usize, Failed)>,
    /// Per piece: its path, if one was found.
    pub paths: Vec<Option<Vec<Point>>>,
}

/// Pulls every piece tight on its layer.
pub fn realize(board: &Board, pieces: &[Piece], vias: &[PlacedVia]) -> Realized {
    let mut routes = vec![NetRoute::default(); board.nets.len()];
    let mut segment_wire: Vec<Vec<usize>> = vec![Vec::new(); board.nets.len()];
    let mut via_wire: Vec<Vec<usize>> = vec![Vec::new(); board.nets.len()];
    let mut failed = Vec::new();
    let mut paths = vec![None; pieces.len()];
    for via in vias {
        let class = &board.classes[via.class];
        routes[via.net as usize].vias.push(Via { at: via.at, diameter: class.via_diameter, drill: class.via_drill });
        via_wire[via.net as usize].push(via.wire);
    }
    let pieces: Vec<Piece> = pieces.iter().map(|piece| settle_ends(board, vias, piece)).collect();
    let pieces = pieces.as_slice();
    for layer in 0..board.layer_count {
        let on_layer: Vec<usize> = (0..pieces.len()).filter(|&index| pieces[index].layer == layer).collect();
        if on_layer.is_empty() {
            continue;
        }
        let results = realize_layer(board, pieces, &on_layer, vias, layer);
        for (index, result) in on_layer.into_iter().zip(results) {
            let piece = &pieces[index];
            match result {
                Ok(points) => {
                    let width = piece.width;
                    for pair in points.windows(2) {
                        if distance(pair[0], pair[1]) < 1.0e-6 {
                            continue;
                        }
                        routes[piece.net as usize].segments.push(Segment { layer, start: pair[0], end: pair[1], width });
                        segment_wire[piece.net as usize].push(piece.wire);
                    }
                    paths[index] = Some(points);
                }
                Err(reason) => failed.push((index, reason)),
            }
        }
    }
    Realized { routes, segment_wire, via_wire, failed, paths }
}

/// Whether a track end of `piece` at `point` keeps its clearances.
fn legal_end(board: &Board, vias: &[PlacedVia], piece: &Piece, point: Point) -> bool {
    let rule = &board.classes[piece.class];
    let half = piece.width / 2.0 + MARGIN;
    let outline = pcb_router::geometry::polygon_edges(&board.outline)
        .map(|(a, b)| pcb_router::geometry::point_segment_distance(point, a, b))
        .fold(f64::INFINITY, f64::min);
    if outline < board.edge_clearance + half {
        return false;
    }
    for obstacle in &board.obstacles {
        if obstacle.layers & (1 << piece.layer) == 0 || !obstacle.blocks_tracks {
            continue;
        }
        if obstacle.kind == ObstacleKind::Copper && obstacle.net == Some(piece.net) {
            continue;
        }
        let gap = match obstacle.kind {
            ObstacleKind::Copper => board.copper_clearance(rule, obstacle),
            ObstacleKind::Hole => board.hole_clearance.max(obstacle.clearance),
            ObstacleKind::Keepout => 0.0,
        };
        let bounds = obstacle.shape.aabb().inflated(gap + half);
        if point[0] < bounds.minimum[0] || point[0] > bounds.maximum[0] || point[1] < bounds.minimum[1] || point[1] > bounds.maximum[1] {
            continue;
        }
        if obstacle.shape.distance_to_point(point) < gap + half {
            return false;
        }
    }
    vias.iter().all(|via| {
        let other = &board.classes[via.class];
        via.net == piece.net || distance(via.at, point) >= other.via_diameter / 2.0 + rule.clearance.max(other.clearance) + half
    })
}

/// The piece with each terminal end moved to where its embedding enters
/// the pad (half a width inside): a track connects at the pad's edge, it
/// need not reach its centre. If that point is too close to something
/// else, the first legal point on the way to the anchor.
fn settle_ends(board: &Board, vias: &[PlacedVia], piece: &Piece) -> Piece {
    let mut points = piece.points.clone();
    if points.len() < 2 {
        return piece.clone();
    }
    for end in 0..2 {
        let Some(pad) = piece.pads[end] else { continue };
        let shape = &board.obstacles[pad].shape;
        if end == 1 {
            points.reverse();
        }
        // points[0] is the anchor; find where the embedding leaves the pad.
        let exit = (0..points.len() - 1).find(|&index| shape.contains(points[index]) && !shape.contains(points[index + 1]));
        if let Some(index) = exit {
            let (inside, outside) = (points[index], points[index + 1]);
            let (mut low, mut high) = (0.0, 1.0);
            for _ in 0..30 {
                let middle = (low + high) / 2.0;
                if shape.contains(taut::lerp(inside, outside, middle)) {
                    low = middle;
                } else {
                    high = middle;
                }
            }
            let edge = taut::lerp(inside, outside, low);
            let back = distance(edge, inside);
            let entry = if back > 1.0e-9 { taut::lerp(edge, inside, (piece.width / 2.0).min(back) / back) } else { edge };
            if legal_end(board, vias, piece, entry) {
                points.splice(0..=index, [entry]);
            } else if !legal_end(board, vias, piece, points[0]) {
                // A legal point inside the pad: towards the edge, towards
                // where the embedding goes next, then all around.
                let anchor = points[0];
                let mut targets = vec![entry, points[1]];
                let reach = shape.aabb();
                let size = (reach.maximum[0] - reach.minimum[0]).max(reach.maximum[1] - reach.minimum[1]);
                for turn in 0..16 {
                    let angle = turn as f64 * std::f64::consts::TAU / 16.0;
                    targets.push([anchor[0] + size * angle.cos(), anchor[1] + size * angle.sin()]);
                }
                'search: for target in targets {
                    for step in 1..=40 {
                        let candidate = taut::lerp(anchor, target, step as f64 / 40.0);
                        if !shape.contains(candidate) {
                            break;
                        }
                        if legal_end(board, vias, piece, candidate) {
                            points[0] = candidate;
                            break 'search;
                        }
                    }
                }
            }
        }
        if end == 1 {
            points.reverse();
        }
    }
    let mut settled = piece.clone();
    settled.points = points;
    settled
}

fn realize_layer(board: &Board, pieces: &[Piece], on_layer: &[usize], vias: &[PlacedVia], layer: usize) -> Vec<Result<Vec<Point>, Failed>> {
    let mesh = LayerMesh::new(board, layer, vias);
    let nearby = Nearby::new(board, layer);
    let count = on_layer.len();
    let channels: Vec<Option<Vec<Crossing>>> = on_layer.iter().map(|&index| trace_nudged(&mesh, &pieces[index].points)).collect();
    // Per edge: (position from the edge's lower vertex, piece) in order.
    let mut on_edge: HashMap<usize, Vec<(f64, usize)>> = HashMap::new();
    for (local, channel) in channels.iter().enumerate() {
        for crossing in channel.iter().flatten() {
            let at = if crossing.left < crossing.right { crossing.at } else { 1.0 - crossing.at };
            on_edge.entry(crossing.edge).or_default().push((at, local));
        }
    }
    for list in on_edge.values_mut() {
        list.sort_by(|a, b| a.0.total_cmp(&b.0));
    }
    let piece = |local: usize| &pieces[on_layer[local]];
    let base = |local: usize, vertex: usize| -> f64 {
        let piece = piece(local);
        mesh.features[vertex].iter().map(|&feature| required(board, vias, feature, piece)).fold(0.0, f64::max)
    };
    // Radius per piece and vertex: the base, grown by nested pieces.
    let mut radii: Vec<HashMap<usize, f64>> = vec![HashMap::new(); count];
    for (local, channel) in channels.iter().enumerate() {
        for crossing in channel.iter().flatten() {
            for vertex in [crossing.left, crossing.right] {
                let radius = base(local, vertex);
                radii[local].insert(vertex, radius);
            }
        }
    }
    let mut results: Vec<Result<Path, Failed>> = vec![Err(Failed::new("not realized".into())); count];
    // Discs a piece keeps clear of besides its channel's: other pieces'
    // bends it runs along.
    let mut along: Vec<Vec<Disc>> = vec![Vec::new(); count];
    let mut dirty = vec![true; count];
    for _round in 0..ROUNDS {
        for local in 0..count {
            if !dirty[local] {
                continue;
            }
            dirty[local] = false;
            let Some(channel) = &channels[local] else {
                let points = &piece(local).points;
                results[local] = Err(Failed::new(format!("no channel (the embedding passes through a vertex): {} points {:?}", points.len(), &points[..points.len().min(6)])));
                continue;
            };
            results[local] = pull(board, &mesh, &nearby, vias, piece(local), channel, &radii[local], &along[local]);
        }
        // Grow radii by the pieces of other nets nested inside.
        let mut changed = false;
        for local in 0..count {
            let Some(channel) = &channels[local] else { continue };
            let own = piece(local);
            let own_class = &board.classes[own.class];
            for crossing in channel {
                let list = &on_edge[&crossing.edge];
                let mine = list.iter().position(|&(_, other)| other == local).expect("own crossing");
                for vertex in [crossing.left, crossing.right] {
                    // At its own copper a piece ends or may run over it:
                    // nothing nests there.
                    if own_copper_at(board, &mesh, vias, vertex, own.net) {
                        continue;
                    }
                    let lower = vertex == crossing.left.min(crossing.right);
                    let inside: &[(f64, usize)] = if lower { &list[..mine] } else { &list[mine + 1..] };
                    // Pieces of other nets passing the vertex inside this
                    // one: it keeps its room from them, as close as they
                    // come to the vertex where they cross this edge.
                    let mut need = radii[local][&vertex];
                    let (a, b) = (mesh.positions[crossing.left], mesh.positions[crossing.right]);
                    let wraps = |result: &Result<Path, Failed>| result.as_ref().is_ok_and(|path| path.discs.iter().any(|disc| disc.vertex == vertex));
                    let own_wraps = wraps(&results[local]);
                    for &(_, other) in inside {
                        let other_piece = piece(other);
                        if other_piece.net == own.net {
                            continue;
                        }
                        let Ok(path) = &results[other] else { continue };
                        // A piece wrapped around the vertex counts with its
                        // radius; one passing it straight only matters if
                        // this piece wraps the vertex (then at its distance
                        // there).
                        let gap = match path.discs.iter().find(|disc| disc.vertex == vertex) {
                            Some(disc) => disc.radius,
                            None if own_wraps => taut::closest_near(path, mesh.positions[vertex], a, b),
                            None => continue,
                        };
                        let other_class = &board.classes[other_piece.class];
                        let wanted = gap + other_piece.width / 2.0 + own_class.clearance.max(other_class.clearance) + own.width / 2.0 + MARGIN;
                        if wanted > need && trace_wire() == Some(own.wire) {
                            eprintln!("    wire {} at {:?}: stacked on wire {} (gap {gap:.3}) -> {wanted:.3}", own.wire, mesh.positions[vertex], other_piece.wire);
                        }
                        need = need.max(wanted);
                    }
                    if need > radii[local][&vertex] + 1.0e-6 {
                        radii[local].insert(vertex, need);
                        dirty[local] = true;
                        changed = true;
                    }
                }
            }
        }
        // Pieces of other nets still too close: the outer one runs along
        // the inner one, around its bends grown by the room between them.
        for (local, other, at, toward) in close_pairs(board, pieces, on_layer, &results) {
            let own = piece(local);
            let other_piece = piece(other);
            let Ok(path) = &results[other] else { continue };
            let Ok(own_path) = &results[local] else { continue };
            let room = other_piece.width / 2.0 + board.classes[own.class].clearance.max(board.classes[other_piece.class].clearance) + own.width / 2.0 + MARGIN;
            let (_, place) = taut::closest(path, at);
            let (direction, discs) = taut::element(path, place, toward);
            let (_, own_place) = taut::closest(own_path, toward);
            let (own_direction, _) = taut::element(own_path, own_place, at);
            // The side of the other piece this one is on.
            let side = if cross(direction, sub(toward, at)) >= 0.0 { 1.0 } else { -1.0 };
            let same_way = taut::dot(direction, own_direction) >= 0.0;
            for disc in discs {
                // This piece is the outer one if the disc's vertex lies on
                // the other side of the other piece; never around its own
                // copper.
                // Outer also means farther from the disc's vertex (so of two
                // pieces only one yields).
                if disc.side == side || own_copper_at(board, &mesh, vias, disc.vertex, own.net) || distance(toward, disc.center) <= distance(at, disc.center) {
                    continue;
                }
                let radius = disc.radius + room;
                let own_side = if same_way { disc.side } else { -disc.side };
                if trace_wire() == Some(own.wire) {
                    eprintln!("    wire {} along wire {} at {:?}: disc r {:.3} -> {radius:.3}", own.wire, other_piece.wire, disc.center, disc.radius);
                }
                if let Some(known) = radii[local].get_mut(&disc.vertex) {
                    if radius > *known + 1.0e-6 {
                        *known = radius;
                        dirty[local] = true;
                        changed = true;
                    }
                    continue;
                }
                match along[local].iter_mut().find(|known| known.vertex == disc.vertex) {
                    Some(known) if radius <= known.radius + 1.0e-6 => {}
                    Some(known) => {
                        known.radius = radius;
                        dirty[local] = true;
                        changed = true;
                    }
                    None => {
                        along[local].push(Disc { center: disc.center, radius, side: own_side, vertex: disc.vertex });
                        dirty[local] = true;
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    results
        .into_iter()
        .map(|result| result.map(|path| taut::polyline(&path, BULGE)))
        .collect()
}

/// The wire whose radii to trace (`PCB_TOPO_TRACE`), for debugging.
fn trace_wire() -> Option<usize> {
    std::env::var("PCB_TOPO_TRACE").ok().and_then(|value| value.parse().ok())
}

/// Whether `vertex` is (on) copper of `net`: a pad or a via of it.
fn own_copper_at(board: &Board, mesh: &LayerMesh, vias: &[PlacedVia], vertex: usize, net: NetId) -> bool {
    mesh.features.get(vertex).is_some_and(|features| {
        features.iter().any(|feature| match *feature {
            Feature::Obstacle { obstacle, .. } => board.obstacles[obstacle].kind == ObstacleKind::Copper && board.obstacles[obstacle].net == Some(net),
            Feature::Via { via } => vias[via].net == net,
            Feature::Outline => false,
        })
    })
}

/// Pairs of pieces of different nets on the layer that come closer than
/// their clearance: (piece, other piece, the point of the other nearest to
/// the piece, the point of the piece nearest to the other), both ways.
fn close_pairs(board: &Board, pieces: &[Piece], on_layer: &[usize], results: &[Result<Path, Failed>]) -> Vec<(usize, usize, Point, Point)> {
    const CELL: f64 = 1.0;
    let polylines: Vec<Option<Vec<Point>>> = results.iter().map(|result| result.as_ref().ok().map(|path| taut::polyline(path, BULGE))).collect();
    let mut cells: HashMap<(i64, i64), Vec<(usize, usize)>> = HashMap::new();
    for (local, polyline) in polylines.iter().enumerate() {
        let Some(points) = polyline else { continue };
        for (index, pair) in points.windows(2).enumerate() {
            let (x0, x1) = ((pair[0][0].min(pair[1][0]) / CELL).floor() as i64, (pair[0][0].max(pair[1][0]) / CELL).floor() as i64);
            let (y0, y1) = ((pair[0][1].min(pair[1][1]) / CELL).floor() as i64, (pair[0][1].max(pair[1][1]) / CELL).floor() as i64);
            for x in x0..=x1 {
                for y in y0..=y1 {
                    cells.entry((x, y)).or_default().push((local, index));
                }
            }
        }
    }
    let mut worst: HashMap<(usize, usize), (f64, Point, Point)> = HashMap::new();
    for list in cells.values() {
        for (i, &(a, sa)) in list.iter().enumerate() {
            for &(b, sb) in &list[i + 1..] {
                let (pa, pb) = (&pieces[on_layer[a]], &pieces[on_layer[b]]);
                if pa.net == pb.net {
                    continue;
                }
                let (Some(la), Some(lb)) = (&polylines[a], &polylines[b]) else { continue };
                let (p, q) = closest_points(la[sa], la[sa + 1], lb[sb], lb[sb + 1]);
                let required = pa.width / 2.0 + pb.width / 2.0 + board.classes[pa.class].clearance.max(board.classes[pb.class].clearance);
                let deficit = required - distance(p, q);
                if deficit > 1.0e-7 {
                    let key = (a.min(b), a.max(b));
                    let (on_a, on_b) = if a < b { (p, q) } else { (q, p) };
                    if worst.get(&key).is_none_or(|known| deficit > known.0) {
                        worst.insert(key, (deficit, on_a, on_b));
                    }
                }
            }
        }
    }
    let mut pairs = Vec::new();
    for ((a, b), (_, on_a, on_b)) in worst {
        pairs.push((a, b, on_b, on_a));
        pairs.push((b, a, on_a, on_b));
    }
    pairs.sort_by(|x, y| (x.0, x.1).cmp(&(y.0, y.1)));
    pairs
}

/// The closest points of segments `a`–`b` and `c`–`d`.
fn closest_points(a: Point, b: Point, c: Point, d: Point) -> (Point, Point) {
    let project = |point: Point, from: Point, to: Point| {
        let span = sub(to, from);
        let length = taut::dot(span, span);
        if length < 1.0e-24 {
            return from;
        }
        taut::lerp(from, to, (taut::dot(sub(point, from), span) / length).clamp(0.0, 1.0))
    };
    let candidates = [(a, project(a, c, d)), (b, project(b, c, d)), (project(c, a, b), c), (project(d, a, b), d)];
    let mut best = candidates[0];
    for candidate in candidates {
        if distance(candidate.0, candidate.1) < distance(best.0, best.1) {
            best = candidate;
        }
    }
    // Crossing segments meet.
    let (d1, d2) = (cross(sub(b, a), sub(c, a)), cross(sub(b, a), sub(d, a)));
    let (d3, d4) = (cross(sub(d, c), sub(a, c)), cross(sub(d, c), sub(b, c)));
    if d1 * d2 < 0.0 && d3 * d4 < 0.0 {
        let t = d1 / (d1 - d2);
        let meet = taut::lerp(c, d, t);
        return (meet, meet);
    }
    best
}

/// A piece pulled tight through its channel, then kept clear of the exact
/// shapes of the layer's obstacles.
fn pull(
    board: &Board,
    mesh: &LayerMesh,
    nearby: &Nearby,
    vias: &[PlacedVia],
    piece: &Piece,
    channel: &[Crossing],
    radii: &HashMap<usize, f64>,
    along: &[Disc],
) -> Result<Path, Failed> {
    let start = piece.points[0];
    let end = *piece.points.last().expect("piece end");
    let portals: Vec<Portal> = channel
        .iter()
        .map(|crossing| Portal {
            left: mesh.positions[crossing.left],
            right: mesh.positions[crossing.right],
            left_vertex: crossing.left,
            right_vertex: crossing.right,
            left_radius: radii[&crossing.left],
            right_radius: radii[&crossing.right],
        })
        .collect();
    let rule = &board.classes[piece.class];
    let mut extra: Vec<Disc> = along.to_vec();
    for _ in 0..20 {
        let path = taut::taut_with(start, end, &portals, &extra).map_err(|failure| {
            let describe = |vertex: usize| -> String {
                let Some(features) = mesh.features.get(vertex) else { return "end".into() };
                features
                    .iter()
                    .map(|feature| match *feature {
                        Feature::Obstacle { obstacle, radius } => format!("{} r{radius:.2} (r {:.3})", board.obstacles[obstacle].label, radii.get(&vertex).copied().unwrap_or(0.0)),
                        Feature::Via { via } => format!("via of wire {} (r {:.3})", vias[via].wire, radii.get(&vertex).copied().unwrap_or(0.0)),
                        Feature::Outline => "outline".into(),
                    })
                    .collect::<Vec<_>>()
                    .join("+")
            };
            match failure {
                taut::Failure::Squeezed { vertices, at, need, have } => Failed {
                    reason: format!("squeezed between {} and {} at [{:.3}, {:.3}]: {need:.3} > {have:.3}", describe(vertices.0), describe(vertices.1), at[0], at[1]),
                    at: Some(at),
                    deficit: need - have,
                },
                other => Failed::new(format!("{other:?}")),
            }
        })?;
        // The closest obstacle the path comes too close to.
        let points = taut::polyline(&path, BULGE);
        let mut worst: Option<(f64, usize, Point, Point)> = None;
        for pair in points.windows(2) {
            for index in nearby.around(pair[0], pair[1], 3.0) {
                let obstacle = &board.obstacles[index];
                if obstacle.kind == ObstacleKind::Copper && obstacle.net == Some(piece.net) {
                    continue;
                }
                let gap = match obstacle.kind {
                    ObstacleKind::Copper => board.copper_clearance(rule, obstacle),
                    ObstacleKind::Hole => board.hole_clearance.max(obstacle.clearance),
                    ObstacleKind::Keepout => 0.0,
                };
                let deficit = gap + piece.width / 2.0 - obstacle.shape.distance_to_segment(pair[0], pair[1]);
                if deficit > 1.0e-7 && worst.is_none_or(|w| deficit > w.0) {
                    worst = Some((deficit, index, pair[0], pair[1]));
                }
            }
        }
        let Some((deficit, index, a, b)) = worst else {
            return Ok(path);
        };
        let obstacle = &board.obstacles[index];
        let direction = sub(b, a);
        let middle = taut::lerp(a, b, 0.5);
        let mut added = false;
        for (center, radius) in corner_discs(&obstacle.shape, middle) {
            let vertex = mesh.at.get(&key(center)).copied().unwrap_or(usize::MAX / 2 + index * 64 + extra.len());
            if extra.iter().any(|disc| disc.vertex == vertex) {
                continue;
            }
            let side = if cross(direction, sub(center, a)) >= 0.0 { 1.0 } else { -1.0 };
            let feature = Feature::Obstacle { obstacle: index, radius };
            let needed = required(board, vias, feature, piece).max(radii.get(&vertex).copied().unwrap_or(0.0));
            extra.push(Disc { center, radius: needed, side, vertex });
            added = true;
        }
        if !added {
            return Err(Failed {
                reason: format!(
                    "too close to {} near [{:.3}, {:.3}] (from [{:.3}, {:.3}] to [{:.3}, {:.3}], {} discs)",
                    obstacle.label,
                    middle[0],
                    middle[1],
                    start[0],
                    start[1],
                    end[0],
                    end[1],
                    path.discs.len()
                ),
                at: Some(middle),
                deficit,
            });
        }
    }
    Err(Failed::new("unsettled against obstacles".into()))
}
