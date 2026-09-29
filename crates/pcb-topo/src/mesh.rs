// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The topological routing space: one constrained Delaunay triangulation
//! shared by all layers (as in TopoR). Its vertices are the corners of every
//! obstacle (pads, holes, rule areas) and of the board outline; obstacle and
//! outline boundaries are constrained edges. Each triangle knows which
//! obstacles cover it, so it can say on which layers a net may pass.

use pcb_router::geometry::point_in_polygon;
use pcb_router::{Board, LayerMask, NetId, ObstacleKind, Point, Shape};
use spade::{ConstrainedDelaunayTriangulation, HasPosition, Point2, Triangulation};

/// No face on this side of an edge (the convex hull's outside).
pub const NONE: usize = usize::MAX;

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

pub struct Mesh {
    pub points: Vec<Point>,
    /// Vertex ids; faces[e][0] lies to the left of `v0 -> v1`.
    pub edges: Vec<[usize; 2]>,
    pub edge_faces: Vec<[usize; 2]>,
    pub constraint: Vec<bool>,
    /// Vertices counter-clockwise; `face_edges[f][i]` joins vertex `i` to
    /// vertex `i + 1`.
    pub faces: Vec<[usize; 3]>,
    pub face_edges: Vec<[usize; 3]>,
    /// Obstacles covering the face's centroid.
    pub face_blockers: Vec<Vec<usize>>,
    /// Whether the face lies inside the board outline.
    pub inside: Vec<bool>,
    /// Obstacles whose boundary passes through the vertex.
    pub vertex_obstacles: Vec<Vec<usize>>,
    /// Whether the vertex lies on the board outline.
    pub vertex_on_outline: Vec<bool>,
}

/// The outline of a shape as polygons that contain it (circles and round
/// ends are circumscribed, so the polygon never cuts into the shape).
pub fn polygons_of(shape: &Shape) -> Vec<Vec<Point>> {
    const SIDES: usize = 8;
    let circumscribed = |radius: f64| radius / (std::f64::consts::PI / SIDES as f64).cos();
    match shape {
        Shape::Circle { center, radius } => {
            let r = circumscribed(*radius);
            vec![
                (0..SIDES)
                    .map(|index| {
                        let angle = (index as f64 + 0.5) * std::f64::consts::TAU / SIDES as f64;
                        [center[0] + r * angle.cos(), center[1] + r * angle.sin()]
                    })
                    .collect(),
            ]
        }
        Shape::Capsule { start, end, radius } => {
            let r = circumscribed(*radius);
            let direction = [end[0] - start[0], end[1] - start[1]];
            let length = (direction[0].powi(2) + direction[1].powi(2)).sqrt();
            let base = if length > 1.0e-9 { direction[1].atan2(direction[0]) } else { 0.0 };
            let mut points = Vec::new();
            // Half circle around `end` (from -90° to +90° of the direction),
            // then around `start` (from +90° to +270°).
            // Every vertex at the circumscribed radius: each edge is tangent
            // to the round end, and the straight sides lie just outside.
            for (center, from) in [(*end, -0.5), (*start, 0.5)] {
                for index in 0..=SIDES / 2 {
                    let angle = base + std::f64::consts::PI * (from + index as f64 / (SIDES / 2) as f64);
                    points.push([center[0] + r * angle.cos(), center[1] + r * angle.sin()]);
                }
            }
            vec![points]
        }
        Shape::Polygon { points } => vec![points.clone()],
        Shape::Union { parts } => parts.iter().flat_map(polygons_of).collect(),
    }
}

/// Obstacles by bounding box on a coarse grid, for point queries.
pub struct Index {
    origin: Point,
    cell: f64,
    nx: usize,
    ny: usize,
    cells: Vec<Vec<usize>>,
}

impl Index {
    pub fn new(board: &Board) -> Self {
        let mut minimum = [f64::INFINITY; 2];
        let mut maximum = [f64::NEG_INFINITY; 2];
        for point in &board.outline {
            for axis in 0..2 {
                minimum[axis] = minimum[axis].min(point[axis]);
                maximum[axis] = maximum[axis].max(point[axis]);
            }
        }
        for obstacle in &board.obstacles {
            let bounds = obstacle.shape.aabb();
            for axis in 0..2 {
                minimum[axis] = minimum[axis].min(bounds.minimum[axis]);
                maximum[axis] = maximum[axis].max(bounds.maximum[axis]);
            }
        }
        if !minimum[0].is_finite() {
            minimum = [0.0, 0.0];
            maximum = [1.0, 1.0];
        }
        let cell = 2.0;
        let nx = (((maximum[0] - minimum[0]) / cell).ceil() as usize).max(1);
        let ny = (((maximum[1] - minimum[1]) / cell).ceil() as usize).max(1);
        let mut cells = vec![Vec::new(); nx * ny];
        for (index, obstacle) in board.obstacles.iter().enumerate() {
            let bounds = obstacle.shape.aabb();
            let x0 = ((bounds.minimum[0] - minimum[0]) / cell).floor().max(0.0) as usize;
            let y0 = ((bounds.minimum[1] - minimum[1]) / cell).floor().max(0.0) as usize;
            let x1 = (((bounds.maximum[0] - minimum[0]) / cell).floor().max(0.0) as usize).min(nx - 1);
            let y1 = (((bounds.maximum[1] - minimum[1]) / cell).floor().max(0.0) as usize).min(ny - 1);
            for y in y0.min(ny - 1)..=y1 {
                for x in x0.min(nx - 1)..=x1 {
                    cells[y * nx + x].push(index);
                }
            }
        }
        Self { origin: minimum, cell, nx, ny, cells }
    }

    /// Obstacles whose bounding box may contain `point` (within `reach`).
    pub fn near(&self, point: Point, reach: f64) -> Vec<usize> {
        let x0 = ((point[0] - reach - self.origin[0]) / self.cell).floor().max(0.0) as usize;
        let y0 = ((point[1] - reach - self.origin[1]) / self.cell).floor().max(0.0) as usize;
        let x1 = (((point[0] + reach - self.origin[0]) / self.cell).floor().max(0.0) as usize).min(self.nx - 1);
        let y1 = (((point[1] + reach - self.origin[1]) / self.cell).floor().max(0.0) as usize).min(self.ny - 1);
        let mut found = Vec::new();
        for y in y0.min(self.ny - 1)..=y1 {
            for x in x0.min(self.nx - 1)..=x1 {
                for &index in &self.cells[y * self.nx + x] {
                    if !found.contains(&index) {
                        found.push(index);
                    }
                }
            }
        }
        found
    }
}

impl Mesh {
    pub fn build(board: &Board, index: &Index) -> Self {
        let mut cdt: ConstrainedDelaunayTriangulation<Vertex> = ConstrainedDelaunayTriangulation::new();
        let mut rings: Vec<Vec<Point>> = Vec::new();
        rings.push(board.outline.clone());
        for obstacle in &board.obstacles {
            rings.extend(polygons_of(&obstacle.shape));
        }
        for ring in &rings {
            let handles: Vec<_> = ring
                .iter()
                .filter_map(|point| cdt.insert(Vertex { position: Point2::new(point[0], point[1]) }).ok())
                .collect();
            for index in 0..handles.len() {
                let (from, to) = (handles[index], handles[(index + 1) % handles.len()]);
                if from != to {
                    cdt.add_constraint_and_split(from, to, |position| Vertex { position });
                }
            }
        }

        let points: Vec<Point> = cdt
            .vertices()
            .map(|vertex| {
                let position = vertex.position();
                [position.x, position.y]
            })
            .collect();
        let mut edges = Vec::new();
        let mut constraint = Vec::new();
        for edge in cdt.undirected_edges() {
            let [a, b] = edge.vertices();
            edges.push([a.fix().index(), b.fix().index()]);
            constraint.push(cdt.is_constraint_edge(edge.fix()));
        }
        let mut edge_faces = vec![[NONE, NONE]; edges.len()];
        let mut faces = Vec::new();
        let mut face_edges = Vec::new();
        for face in cdt.inner_faces() {
            let id = faces.len();
            let vertices = face.vertices().map(|vertex| vertex.fix().index());
            let mut ids = [0usize; 3];
            for directed in face.adjacent_edges() {
                let (from, to) = (directed.from().fix().index(), directed.to().fix().index());
                let edge = directed.as_undirected().fix().index();
                let slot = (0..3).find(|&i| vertices[i] == from && vertices[(i + 1) % 3] == to).expect("face edge");
                ids[slot] = edge;
                // The face lies to the left of its counter-clockwise edges.
                let side = if edges[edge] == [from, to] { 0 } else { 1 };
                edge_faces[edge][side] = id;
            }
            faces.push(vertices);
            face_edges.push(ids);
        }

        let mut face_blockers = Vec::with_capacity(faces.len());
        let mut inside = Vec::with_capacity(faces.len());
        for face in &faces {
            let [a, b, c] = face.map(|vertex| points[vertex]);
            let centroid = [(a[0] + b[0] + c[0]) / 3.0, (a[1] + b[1] + c[1]) / 3.0];
            inside.push(point_in_polygon(centroid, &board.outline));
            face_blockers.push(
                index
                    .near(centroid, 0.0)
                    .into_iter()
                    .filter(|&obstacle| board.obstacles[obstacle].shape.contains(centroid))
                    .collect(),
            );
        }
        let mut vertex_obstacles = Vec::with_capacity(points.len());
        let mut vertex_on_outline = Vec::with_capacity(points.len());
        for point in &points {
            vertex_obstacles.push(
                index
                    .near(*point, 0.01)
                    .into_iter()
                    .filter(|&obstacle| {
                        polygons_of(&board.obstacles[obstacle].shape)
                            .iter()
                            .any(|ring| ring_distance(*point, ring) < 1.0e-6)
                    })
                    .collect(),
            );
            vertex_on_outline.push(ring_distance(*point, &board.outline) < 1.0e-6);
        }
        Self {
            points,
            edges,
            edge_faces,
            constraint,
            faces,
            face_edges,
            face_blockers,
            inside,
            vertex_obstacles,
            vertex_on_outline,
        }
    }

    /// Layers on which `net` may run through `face`.
    pub fn face_layers(&self, board: &Board, face: usize, net: NetId) -> LayerMask {
        if face == NONE || !self.inside[face] {
            return 0;
        }
        let mut mask = board.all_layers();
        // Inside its own pad a net may go anywhere, the drill hole included
        // (the pad's centre is where its wires end).
        let own_pad = self.in_own_pad(board, face, net);
        for &blocker in &self.face_blockers[face] {
            let obstacle = &board.obstacles[blocker];
            let own = obstacle.kind == ObstacleKind::Copper && obstacle.net == Some(net);
            let own_hole = own_pad && obstacle.kind == ObstacleKind::Hole;
            if obstacle.blocks_tracks && !own && !own_hole {
                mask &= !obstacle.layers;
            }
        }
        mask
    }

    /// Whether the face is inside a pad of `net` (on any layer).
    pub fn in_own_pad(&self, board: &Board, face: usize, net: NetId) -> bool {
        self.face_blockers[face].iter().any(|&blocker| {
            let obstacle = &board.obstacles[blocker];
            obstacle.kind == ObstacleKind::Copper && obstacle.net == Some(net)
        })
    }

    pub fn face_centroid(&self, face: usize) -> Point {
        let [a, b, c] = self.faces[face].map(|vertex| self.points[vertex]);
        [(a[0] + b[0] + c[0]) / 3.0, (a[1] + b[1] + c[1]) / 3.0]
    }

    pub fn edge_length(&self, edge: usize) -> f64 {
        let [a, b] = self.edges[edge].map(|vertex| self.points[vertex]);
        ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
    }

    /// The face on the other side of `edge` from `face`.
    pub fn across(&self, edge: usize, face: usize) -> usize {
        let [left, right] = self.edge_faces[edge];
        if left == face { right } else { left }
    }

    /// The face containing `point` (a linear walk; boards have few queries).
    pub fn locate(&self, point: Point) -> Option<usize> {
        (0..self.faces.len()).find(|&face| {
            let [a, b, c] = self.faces[face].map(|vertex| self.points[vertex]);
            let side = |p: Point, q: Point| (q[0] - p[0]) * (point[1] - p[1]) - (q[1] - p[1]) * (point[0] - p[0]);
            side(a, b) >= -1.0e-12 && side(b, c) >= -1.0e-12 && side(c, a) >= -1.0e-12
        })
    }
}

fn ring_distance(point: Point, ring: &[Point]) -> f64 {
    pcb_router::geometry::polygon_edges(ring)
        .map(|(a, b)| pcb_router::geometry::point_segment_distance(point, a, b))
        .fold(f64::INFINITY, f64::min)
}
