// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Geometry for topological wires ("metrization"). On each layer the pieces
//! of wire assigned to it are pulled tight in their homotopy class: every
//! obstacle corner (and via) is a disc of the clearance it needs from the
//! wire plus the room of the wires already passing closer to it, and the
//! wire becomes tangent segments between those discs and arcs around them.
//! Arcs are emitted as polylines circumscribing them, so the copper never
//! comes closer than the arc would.

use crate::layers::Assignment;
use crate::mesh::Mesh;
use crate::topo::Topology;
use pcb_router::{Board, NetId, NetRoute, ObstacleKind, Point, Segment, Shape, Via};
use spade::{ConstrainedDelaunayTriangulation, HasPosition, Intersection, LineIntersectionIterator, Point2, Triangulation};
use std::collections::HashMap;

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

/// What a per-layer vertex stands for.
#[derive(Clone, Debug)]
enum Feature {
    /// A corner or centre of an obstacle; `radius` of the obstacle around
    /// this point (circles and round ends are single points).
    Obstacle { obstacle: usize, radius: f64 },
    Via { net: NetId, radius: f64, clearance: f64 },
    Outline,
}

/// A piece of wire on one layer, as a polyline in its homotopy class.
#[derive(Clone, Debug)]
pub struct Piece {
    pub wire: usize,
    pub net: NetId,
    pub layer: usize,
    pub points: Vec<Point>,
    /// The piece starts or ends at a via of its own.
    pub via_at: [Option<Point>; 2],
}

/// A wire's pieces and vias.
pub struct Layout {
    pub pieces: Vec<Piece>,
    pub vias: Vec<(usize, Point)>,
}

fn cross(a: Point, b: Point) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1]]
}

fn length(a: Point) -> f64 {
    (a[0] * a[0] + a[1] * a[1]).sqrt()
}

fn lerp(a: Point, b: Point, t: f64) -> Point {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

/// Splits every routed wire into per-layer pieces at its vias, as polylines
/// through its crossing points.
pub fn pieces(board: &Board, mesh: &Mesh, topology: &Topology, assignment: &Assignment) -> Layout {
    let mut layout = Layout { pieces: Vec::new(), vias: Vec::new() };
    // Crossing positions per layer: which wires cross each edge on which
    // layer, so a piece's points spread only among its layer's wires.
    let mut edge_layer: HashMap<(usize, usize), usize> = HashMap::new();
    let mut via_points: Vec<Vec<(usize, Point)>> = vec![Vec::new(); topology.wires.len()];
    for (wire, path) in topology.wires.iter().enumerate() {
        if !path.routed || assignment.tokens[wire].is_empty() {
            continue;
        }
        let tokens = &assignment.tokens[wire];
        let layers = &assignment.layers[wire];
        // The layer each crossed edge is on, and where the vias go.
        for step in 1..path.faces.len() {
            // Last token of step - 1 and first of step.
            let before = tokens.iter().rposition(|token| token.step == step - 1).expect("token before");
            let after = before + 1;
            let edge = path.edges[step - 1];
            let layer = if layers[before] == layers[after] {
                layers[before]
            } else {
                // The via goes where both layers may meet; the crossing then
                // takes the other side's layer.
                let both = (1 << layers[before]) | (1 << layers[after]);
                if tokens[before].via_mask & both == both { layers[after] } else { layers[before] }
            };
            edge_layer.insert((edge, wire), layer);
        }
        for index in 0..tokens.len().saturating_sub(1) {
            if layers[index] == layers[index + 1] {
                continue;
            }
            let both = (1 << layers[index]) | (1 << layers[index + 1]);
            let (step, from, to) = if tokens[index].step == tokens[index + 1].step {
                (tokens[index].step, tokens[index].along, tokens[index + 1].along)
            } else if tokens[index].via_mask & both == both {
                (tokens[index].step, tokens[index].along, 1.0)
            } else {
                let next = tokens.get(index + 2).filter(|token| token.step == tokens[index + 1].step).map_or(1.0, |token| token.along);
                (tokens[index + 1].step, 0.0, next)
            };
            let (p, q) = crate::layers::chord(board, mesh, topology, wire, step);
            via_points[wire].push((index, lerp(p, q, (from + to) / 2.0)));
        }
    }
    let portal_on = |edge: usize, wire: usize, layer: usize| -> Point {
        let [a, b] = mesh.edges[edge].map(|vertex| mesh.points[vertex]);
        let same: Vec<usize> = topology.order[edge]
            .iter()
            .copied()
            .filter(|&other| edge_layer.get(&(edge, other)) == Some(&layer))
            .collect();
        let rooms: Vec<f64> = same.iter().map(|&other| Topology::room(board, topology.wires[other].class)).collect();
        let total: f64 = rooms.iter().sum();
        let index = same.iter().position(|&other| other == wire).unwrap_or(0);
        let before: f64 = rooms.iter().take(index).sum::<f64>() + rooms.get(index).copied().unwrap_or(0.0) / 2.0;
        let length = mesh.edge_length(edge);
        let t = if total <= length { (length - total) / 2.0 + before } else { before * length / total.max(1.0e-12) };
        lerp(a, b, (t / length.max(1.0e-12)).clamp(0.02, 0.98))
    };
    for (wire, path) in topology.wires.iter().enumerate() {
        if !path.routed || assignment.tokens[wire].is_empty() {
            continue;
        }
        let tokens = &assignment.tokens[wire];
        let layers = &assignment.layers[wire];
        // Walk the tokens; cut a piece at every via.
        let mut current = Piece { wire, net: path.net, layer: layers[0], points: vec![path.from], via_at: [None, None] };
        let mut vias = via_points[wire].iter().peekable();
        for step in 0..path.faces.len() {
            // Vias inside this step, in token order.
            while let Some(&&(index, point)) = vias.peek() {
                let at_step = if tokens[index].step == tokens[index + 1].step
                    || tokens[index].via_mask & ((1 << layers[index]) | (1 << layers[index + 1])) == ((1 << layers[index]) | (1 << layers[index + 1]))
                {
                    tokens[index].step
                } else {
                    tokens[index + 1].step
                };
                if at_step != step {
                    break;
                }
                vias.next();
                current.points.push(point);
                current.via_at[1] = Some(point);
                layout.pieces.push(current);
                layout.vias.push((wire, point));
                current = Piece { wire, net: path.net, layer: layers[index + 1], points: vec![point], via_at: [Some(point), None] };
            }
            if step + 1 < path.faces.len() {
                let edge = path.edges[step];
                let layer = edge_layer.get(&(edge, wire)).copied().unwrap_or(current.layer);
                current.points.push(portal_on(edge, wire, layer));
            }
        }
        current.points.push(path.to);
        layout.pieces.push(current);
    }
    layout
}

/// A layer's obstacles and vias, triangulated, with what each vertex is.
struct LayerMesh {
    cdt: ConstrainedDelaunayTriangulation<Vertex>,
    features: HashMap<usize, Vec<Feature>>,
    positions: Vec<Point>,
}

fn add_ring(cdt: &mut ConstrainedDelaunayTriangulation<Vertex>, ring: &[Point]) -> Vec<usize> {
    let handles: Vec<_> = ring.iter().filter_map(|point| cdt.insert(Vertex { position: Point2::new(point[0], point[1]) }).ok()).collect();
    for index in 0..handles.len() {
        let (from, to) = (handles[index], handles[(index + 1) % handles.len()]);
        if from != to && handles.len() > 1 {
            cdt.add_constraint_and_split(from, to, |position| Vertex { position });
        }
    }
    handles.iter().map(|handle| handle.index()).collect()
}

fn layer_mesh(board: &Board, layer: usize, vias: &[(NetId, Point, f64, f64)]) -> LayerMesh {
    let mut cdt: ConstrainedDelaunayTriangulation<Vertex> = ConstrainedDelaunayTriangulation::new();
    let mut features: HashMap<usize, Vec<Feature>> = HashMap::new();
    for vertex in add_ring(&mut cdt, &board.outline) {
        features.entry(vertex).or_default().push(Feature::Outline);
    }
    fn add_shape(cdt: &mut ConstrainedDelaunayTriangulation<Vertex>, features: &mut HashMap<usize, Vec<Feature>>, obstacle: usize, shape: &Shape) {
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
            Shape::Polygon { points } => {
                for vertex in add_ring(cdt, points) {
                    features.entry(vertex).or_default().push(Feature::Obstacle { obstacle, radius: 0.0 });
                }
            }
            Shape::Union { parts } => {
                for part in parts {
                    add_shape(cdt, features, obstacle, part);
                }
            }
        }
    }
    for (index, obstacle) in board.obstacles.iter().enumerate() {
        if obstacle.layers & (1 << layer) == 0 || !obstacle.blocks_tracks {
            continue;
        }
        add_shape(&mut cdt, &mut features, index, &obstacle.shape);
    }
    for &(net, at, radius, clearance) in vias {
        if let Ok(handle) = cdt.insert(Vertex { position: Point2::new(at[0], at[1]) }) {
            features.entry(handle.index()).or_default().push(Feature::Via { net, radius, clearance });
        }
    }
    let positions: Vec<Point> = cdt.vertices().map(|vertex| [vertex.position().x, vertex.position().y]).collect();
    // Vertices made by splitting constraints lie on an obstacle's boundary
    // (or the outline): they are corners of what they lie on.
    for (vertex, &point) in positions.iter().enumerate() {
        if features.contains_key(&vertex) {
            continue;
        }
        let mut found = Vec::new();
        for (index, obstacle) in board.obstacles.iter().enumerate() {
            if obstacle.layers & (1 << layer) == 0 || !obstacle.blocks_tracks {
                continue;
            }
            let bounds = obstacle.shape.aabb().inflated(1.0e-6);
            if point[0] < bounds.minimum[0] || point[0] > bounds.maximum[0] || point[1] < bounds.minimum[1] || point[1] > bounds.maximum[1] {
                continue;
            }
            if obstacle.shape.distance_to_point(point) < 1.0e-6 {
                found.push(Feature::Obstacle { obstacle: index, radius: 0.0 });
            }
        }
        let on_outline = pcb_router::geometry::polygon_edges(&board.outline)
            .any(|(a, b)| pcb_router::geometry::point_segment_distance(point, a, b) < 1.0e-6);
        if on_outline {
            found.push(Feature::Outline);
        }
        if !found.is_empty() {
            features.insert(vertex, found);
        }
    }
    LayerMesh { cdt, features, positions }
}

/// A disc the wire must pass on one side.
#[derive(Clone, Copy, Debug)]
struct Disc {
    center: Point,
    radius: f64,
    /// +1: the disc lies to the left of the wire, -1: to the right.
    side: f64,
    vertex: usize,
}

/// One portal of a piece's channel: the vertices on its left and right.
#[derive(Clone, Copy, Debug)]
struct Portal {
    left: usize,
    right: usize,
    edge: usize,
    /// Where the piece's polyline crosses, 0 at `left` to 1 at `right`.
    at: f64,
}

/// The channel of a polyline through the layer mesh: the edges it crosses,
/// with the homotopic back-and-forth crossings cancelled.
fn channel(mesh: &LayerMesh, points: &[Point]) -> Option<Vec<Portal>> {
    let mut portals: Vec<Portal> = Vec::new();
    for pair in points.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        if length(sub(to, from)) < 1.0e-9 {
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
                    // Where the segment meets the portal.
                    let d2 = sub(pr, pl);
                    let denominator = cross(direction, d2);
                    let at = if denominator.abs() < 1.0e-15 { 0.5 } else { cross(sub(pl, from), direction) / denominator };
                    let undirected = edge.as_undirected().fix().index();
                    if let Some(last) = portals.last()
                        && last.edge == undirected
                    {
                        portals.pop();
                        continue;
                    }
                    portals.push(Portal { left, right, edge: undirected, at: at.clamp(0.0, 1.0) });
                }
                Intersection::VertexIntersection(vertex) => {
                    let p = vertex.position();
                    let at = [p.x, p.y];
                    // Starting or ending on a vertex (a via) is expected;
                    // passing through one is degenerate.
                    if length(sub(at, from)) > 1.0e-7 && length(sub(at, to)) > 1.0e-7 {
                        return None;
                    }
                }
                Intersection::EdgeOverlap(_) => return None,
            }
        }
    }
    Some(portals)
}

/// Pulls a piece tight: the funnel over portals shrunk by each vertex's
/// radius picks the discs the wire wraps; the path is then exact tangents
/// and arcs.
fn tighten(start: Point, end: Point, portals: &[(Point, Point, Disc, Disc)]) -> Vec<Disc> {
    // Simple stupid funnel algorithm on shrunk points.
    let mut apexes: Vec<Disc> = Vec::new();
    let mut points: Vec<(Point, Point, Option<Disc>, Option<Disc>)> = vec![(start, start, None, None)];
    for &(left, right, disc_left, disc_right) in portals {
        points.push((left, right, Some(disc_left), Some(disc_right)));
    }
    points.push((end, end, None, None));
    let mut apex = start;
    let (mut left, mut right) = (start, start);
    let (mut left_index, mut right_index) = (0usize, 0usize);
    let mut index = 1;
    let mut guard = 0;
    while index < points.len() {
        guard += 1;
        if guard > 100_000 {
            break;
        }
        let (portal_left, portal_right, disc_left, disc_right) = points[index];
        // Tighten the right side.
        if cross(sub(right, apex), sub(portal_right, apex)) >= 0.0 {
            if length(sub(apex, right)) < 1.0e-12 || cross(sub(left, apex), sub(portal_right, apex)) < 0.0 {
                right = portal_right;
                right_index = index;
            } else {
                // The right side passes the left: the left disc is a corner.
                if let Some(disc) = points[left_index].2 {
                    apexes.push(disc);
                }
                apex = left;
                let restart = left_index;
                left = apex;
                right = apex;
                left_index = restart;
                right_index = restart;
                index = restart + 1;
                continue;
            }
        }
        // Tighten the left side.
        if cross(sub(left, apex), sub(portal_left, apex)) <= 0.0 {
            if length(sub(apex, left)) < 1.0e-12 || cross(sub(right, apex), sub(portal_left, apex)) > 0.0 {
                left = portal_left;
                left_index = index;
            } else {
                if let Some(disc) = points[right_index].3 {
                    apexes.push(disc);
                }
                apex = right;
                let restart = right_index;
                left = apex;
                right = apex;
                left_index = restart;
                right_index = restart;
                index = restart + 1;
                continue;
            }
        }
        let _ = (disc_left, disc_right);
        index += 1;
    }
    apexes
}

/// The directed tangent from disc `a` to disc `b`: its two touch points.
/// A disc's `side` says which side of the tangent its centre lies on.
fn tangent(a: Disc, b: Disc) -> Option<(Point, Point)> {
    let d = sub(b.center, a.center);
    let distance = length(d);
    if distance < 1.0e-12 {
        return None;
    }
    // With `n` the left normal of the tangent's direction, the centres lie
    // at signed distances side * radius: n . (b - a) = k.
    let k = b.side * b.radius - a.side * a.radius;
    if k.abs() > distance {
        return None;
    }
    let phi = d[1].atan2(d[0]);
    let delta = (k / distance).acos();
    for theta in [phi + delta, phi - delta] {
        let n = [theta.cos(), theta.sin()];
        let direction = [n[1], -n[0]];
        let p = [a.center[0] - a.side * a.radius * n[0], a.center[1] - a.side * a.radius * n[1]];
        let q = [b.center[0] - b.side * b.radius * n[0], b.center[1] - b.side * b.radius * n[1]];
        let run = sub(q, p);
        if run[0] * direction[0] + run[1] * direction[1] > -1.0e-12 {
            return Some((p, q));
        }
    }
    None
}

/// Points of an arc around `disc` from `from` to `to`, circumscribing it.
fn arc(disc: Disc, from: Point, to: Point, output: &mut Vec<Point>) {
    let a0 = (from[1] - disc.center[1]).atan2(from[0] - disc.center[0]);
    let a1 = (to[1] - disc.center[1]).atan2(to[0] - disc.center[0]);
    let tau = std::f64::consts::TAU;
    // Discs on the left are passed counter-clockwise (positive angles).
    let sweep = if disc.side > 0.0 { (a1 - a0).rem_euclid(tau) } else { -(a0 - a1).rem_euclid(tau) };
    if sweep.abs() < 1.0e-9 || disc.radius < 1.0e-9 {
        output.push(to);
        return;
    }
    let pieces = ((sweep.abs() / 10f64.to_radians()).ceil() as usize).max(1);
    let step = sweep / pieces as f64;
    let outer = disc.radius / (step.abs() / 2.0).cos();
    for index in 0..pieces {
        let angle = a0 + step * (index as f64 + 0.5);
        output.push([disc.center[0] + outer * angle.cos(), disc.center[1] + outer * angle.sin()]);
    }
    output.push(to);
}

/// Copper per net, and which wire each segment and via belongs to.
pub struct Realized {
    pub routes: Vec<NetRoute>,
    pub segment_wire: Vec<Vec<usize>>,
    pub via_wire: Vec<Vec<usize>>,
    /// Wires with a piece that could not be realized (a squeezed channel).
    pub failed: Vec<usize>,
}

/// Realizes every piece on its layer.
pub fn realize(board: &Board, layout: &Layout, topology: &Topology) -> Realized {
    let mut routes = vec![NetRoute::default(); board.nets.len()];
    let mut segment_wire: Vec<Vec<usize>> = vec![Vec::new(); board.nets.len()];
    let mut via_wire: Vec<Vec<usize>> = vec![Vec::new(); board.nets.len()];
    let mut failed = Vec::new();
    let via_list: Vec<(NetId, Point, f64, f64)> = layout
        .vias
        .iter()
        .map(|&(wire, at)| {
            let class = &board.classes[topology.wires[wire].class];
            (topology.wires[wire].net, at, class.via_diameter / 2.0, class.clearance)
        })
        .collect();
    for &(wire, at) in &layout.vias {
        let class = &board.classes[topology.wires[wire].class];
        routes[topology.wires[wire].net as usize].vias.push(Via { at, diameter: class.via_diameter, drill: class.via_drill });
        via_wire[topology.wires[wire].net as usize].push(wire);
    }
    for layer in 0..board.layer_count {
        let pieces: Vec<&Piece> = layout.pieces.iter().filter(|piece| piece.layer == layer).collect();
        if pieces.is_empty() {
            continue;
        }
        let mesh = layer_mesh(board, layer, &via_list);
        // Channels, then which pieces cross each edge where.
        let mut channels: Vec<Option<Vec<Portal>>> = Vec::with_capacity(pieces.len());
        for piece in &pieces {
            let mut found = channel(&mesh, &piece.points);
            if found.is_none() {
                // Nudge the inner points off a vertex or edge and retry.
                let mut nudged = piece.points.clone();
                let count = nudged.len();
                for (index, point) in nudged.iter_mut().enumerate() {
                    if index > 0 && index + 1 < count {
                        point[0] += 1.3e-6;
                        point[1] += 0.7e-6;
                    }
                }
                found = channel(&mesh, &nudged);
            }
            channels.push(found);
        }
        let mut on_edge: HashMap<usize, Vec<(f64, usize)>> = HashMap::new();
        for (index, found) in channels.iter().enumerate() {
            let Some(portals) = found else { continue };
            for portal in portals {
                // Parameter from the edge's lower vertex id, comparable
                // between pieces crossing either way.
                let at = if portal.left < portal.right { portal.at } else { 1.0 - portal.at };
                on_edge.entry(portal.edge).or_default().push((at, index));
            }
        }
        for list in on_edge.values_mut() {
            list.sort_by(|a, b| a.0.total_cmp(&b.0));
        }
        let room = |index: usize| {
            let class = &board.classes[topology.wires[pieces[index].wire].class];
            (class.trace_width, class.clearance)
        };
        // Radius a piece keeps from a vertex: the vertex's own clearance
        // plus the pieces of other nets passing between.
        let base = |vertex: usize, index: usize| -> f64 {
            let piece = pieces[index];
            let (width, clearance) = room(index);
            let class = &board.classes[topology.wires[piece.wire].class];
            let mut radius: f64 = 0.0;
            for feature in mesh.features.get(&vertex).map(Vec::as_slice).unwrap_or(&[]) {
                let needed = match feature {
                    Feature::Obstacle { obstacle, radius } => {
                        let obstacle_ref = &board.obstacles[*obstacle];
                        if obstacle_ref.kind == ObstacleKind::Copper && obstacle_ref.net == Some(piece.net) {
                            0.0
                        } else {
                            let gap = match obstacle_ref.kind {
                                ObstacleKind::Copper => board.copper_clearance(class, obstacle_ref),
                                ObstacleKind::Hole => board.hole_clearance.max(clearance),
                                ObstacleKind::Keepout => 0.0,
                            };
                            radius + gap + width / 2.0
                        }
                    }
                    Feature::Via { net, radius, clearance: other } => {
                        if *net == piece.net { 0.0 } else { radius + clearance.max(*other) + width / 2.0 }
                    }
                    Feature::Outline => board.edge_clearance + width / 2.0,
                };
                radius = radius.max(needed);
            }
            radius
        };
        for (index, piece) in pieces.iter().enumerate() {
            let debug = std::env::var("PCB_TOPO_DEBUG").is_ok();
            let Some(portals) = &channels[index] else {
                if debug {
                    eprintln!("  piece of wire {} on layer {layer}: no channel", piece.wire);
                }
                failed.push(piece.wire);
                continue;
            };
            let (width, clearance) = room(index);
            let mut shrunk = Vec::with_capacity(portals.len());
            let mut squeezed = false;
            for portal in portals {
                let stack = |vertex: usize| -> f64 {
                    let list = &on_edge[&portal.edge];
                    let mine = list.iter().position(|&(_, other)| other == index).unwrap_or(0);
                    let lower_is_vertex = vertex == portal.left.min(portal.right);
                    let range: Vec<usize> = if lower_is_vertex { (0..mine).collect() } else { (mine + 1..list.len()).collect() };
                    range
                        .into_iter()
                        .map(|position| list[position].1)
                        .filter(|&other| pieces[other].net != piece.net)
                        .map(|other| {
                            let (w, c) = room(other);
                            w + c.max(clearance)
                        })
                        .sum()
                };
                let left_radius = base(portal.left, index) + stack(portal.left);
                let right_radius = base(portal.right, index) + stack(portal.right);
                let (pl, pr) = (mesh.positions[portal.left], mesh.positions[portal.right]);
                let span = length(sub(pr, pl));
                let (mut a, mut b) = (left_radius / span.max(1.0e-12), 1.0 - right_radius / span.max(1.0e-12));
                if a > b {
                    if debug {
                        eprintln!(
                            "  piece of wire {} on layer {layer}: squeezed between {:?} (r {:.3}) and {:?} (r {:.3}), span {:.3}",
                            piece.wire, pl, left_radius, pr, right_radius, span
                        );
                    }
                    squeezed = true;
                    let middle = (a + b) / 2.0;
                    a = middle;
                    b = middle;
                }
                shrunk.push((
                    lerp(pl, pr, a),
                    lerp(pl, pr, b),
                    Disc { center: pl, radius: left_radius, side: 1.0, vertex: portal.left },
                    Disc { center: pr, radius: right_radius, side: -1.0, vertex: portal.right },
                ));
            }
            let _ = width;
            let start = piece.points[0];
            let end = *piece.points.last().expect("piece end");
            // The funnel can name the piece's own end points, or one vertex
            // several times in a row: keep each wrapped disc once.
            let mut apexes: Vec<Disc> = Vec::new();
            for disc in tighten(start, end, &shrunk) {
                if length(sub(disc.center, start)) < 1.0e-9 || length(sub(disc.center, end)) < 1.0e-9 {
                    continue;
                }
                if let Some(last) = apexes.last_mut()
                    && last.vertex == disc.vertex
                {
                    last.radius = last.radius.max(disc.radius);
                    continue;
                }
                apexes.push(disc);
            }
            // Exact path: tangents between consecutive discs, arcs around.
            let mut discs = vec![Disc { center: start, radius: 0.0, side: 1.0, vertex: usize::MAX }];
            discs.extend(apexes);
            discs.push(Disc { center: end, radius: 0.0, side: 1.0, vertex: usize::MAX });
            let mut polyline = vec![start];
            let mut ok = !squeezed;
            let mut entry = start;
            for pair in 0..discs.len() - 1 {
                let (a, b) = (discs[pair], discs[pair + 1]);
                let Some((p, q)) = tangent(a, b) else {
                    if debug {
                        eprintln!("  piece of wire {} on layer {layer}: no tangent from {:?} to {:?}", piece.wire, a, b);
                    }
                    ok = false;
                    polyline.push(b.center);
                    entry = b.center;
                    continue;
                };
                if pair > 0 {
                    arc(a, entry, p, &mut polyline);
                } else if length(sub(p, start)) > 1.0e-9 {
                    polyline.push(p);
                }
                polyline.push(q);
                entry = q;
            }
            if length(sub(*polyline.last().unwrap(), end)) > 1.0e-9 {
                polyline.push(end);
            }
            if !ok {
                failed.push(piece.wire);
            }
            let route = &mut routes[piece.net as usize];
            for pair in polyline.windows(2) {
                if length(sub(pair[1], pair[0])) < 1.0e-6 {
                    continue;
                }
                route.segments.push(Segment { layer, start: pair[0], end: pair[1], width });
                segment_wire[piece.net as usize].push(piece.wire);
            }
        }
    }
    failed.sort_unstable();
    failed.dedup();
    Realized { routes, segment_wire, via_wire, failed }
}
