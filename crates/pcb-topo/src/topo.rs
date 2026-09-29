// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Topological wires on the shared triangulation. A wire is only the
//! sequence of triangulation edges it crosses and its place in each edge's
//! order of crossings (TopoR's model); wires of different nets may cross,
//! and layers come later. A connection is found by A* over the gaps between
//! the wires already crossing each edge: its cost is length, crossings with
//! other nets and capacity overflow.

use crate::mesh::{Mesh, NONE};
use pcb_router::{Board, ClassId, NetId, Point};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

#[derive(Clone, Debug)]
pub struct Wire {
    pub net: NetId,
    pub class: ClassId,
    pub from: Point,
    pub to: Point,
    /// Terminal indices within the net.
    pub terminals: [usize; 2],
    /// Faces traversed, `edges.len() + 1` of them.
    pub faces: Vec<usize>,
    pub edges: Vec<usize>,
    pub routed: bool,
}

/// Cost weights of the topological search (TopoR tunes them per wire and
/// oscillates them; the optimizer does the same).
#[derive(Clone, Copy, Debug)]
pub struct Weights {
    /// Millimetres of length one crossing with another net costs.
    pub crossing: f64,
    /// Millimetres per millimetre of capacity overflow on an edge.
    pub overflow: f64,
    /// Millimetres per unit of an edge's history (how often it overflowed).
    pub history: f64,
}

pub struct Topology {
    pub wires: Vec<Wire>,
    /// Per edge: the wires crossing it, ordered from `edges[e][0]`.
    pub order: Vec<Vec<usize>>,
    /// Per face: wires passing through (entering or leaving by an edge).
    pub face_wires: Vec<Vec<usize>>,
    /// Per edge: congestion history.
    pub history: Vec<f64>,
    /// Per edge: room for tracks on all layers together, in millimetres.
    pub capacity: Vec<f64>,
}

#[derive(Clone, Copy, PartialEq)]
struct Queued {
    cost: f64,
    node: usize,
}

impl Eq for Queued {}

impl Ord for Queued {
    fn cmp(&self, other: &Self) -> Ordering {
        other.cost.total_cmp(&self.cost).then(other.node.cmp(&self.node))
    }
}

impl PartialOrd for Queued {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

struct Search {
    nodes: Vec<Node>,
    best: HashMap<(usize, usize), f64>,
    queue: BinaryHeap<Queued>,
}

fn distance(a: Point, b: Point) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// A search node: a gap on an edge, reached from a face.
#[derive(Clone, Copy)]
struct Node {
    edge: usize,
    gap: usize,
    /// The face the wire enters by crossing `edge`.
    face: usize,
    point: Point,
    cost: f64,
    parent: usize,
}

impl Topology {
    pub fn new(board: &Board, mesh: &Mesh) -> Self {
        let clearance = board.classes.iter().map(|class| class.clearance).fold(0.0, f64::max);
        let capacity = (0..mesh.edges.len())
            .map(|edge| {
                let [left, right] = mesh.edge_faces[edge];
                let layers = (0..board.layer_count)
                    .filter(|&layer| {
                        [left, right].iter().all(|&face| face != NONE && free_on(board, mesh, face, layer))
                    })
                    .count();
                let ends: f64 = mesh.edges[edge]
                    .iter()
                    .map(|&vertex| if mesh.vertex_obstacles[vertex].is_empty() && !mesh.vertex_on_outline[vertex] { 0.0 } else { clearance })
                    .sum();
                (mesh.edge_length(edge) - ends).max(0.0) * layers as f64
            })
            .collect();
        Self {
            wires: Vec::new(),
            order: vec![Vec::new(); mesh.edges.len()],
            face_wires: vec![Vec::new(); mesh.faces.len()],
            history: vec![0.0; mesh.edges.len()],
            capacity,
        }
    }

    /// Room a wire of `class` takes on an edge.
    pub fn room(board: &Board, class: ClassId) -> f64 {
        board.classes[class].trace_width + board.classes[class].clearance
    }

    pub fn used(&self, board: &Board, edge: usize) -> f64 {
        self.order[edge].iter().map(|&wire| Self::room(board, self.wires[wire].class)).sum()
    }

    /// Where on `edge` the wire at `position` of its order runs, spreading
    /// the crossing wires by their room from the edge's first vertex.
    /// `position` counts in halves: gap `g` is `2g`, wire `j` is `2j + 1`.
    pub fn portal(&self, board: &Board, mesh: &Mesh, edge: usize, position: usize, extra: f64) -> Point {
        let [a, b] = mesh.edges[edge].map(|vertex| mesh.points[vertex]);
        let length = mesh.edge_length(edge);
        let rooms: Vec<f64> = self.order[edge].iter().map(|&wire| Self::room(board, self.wires[wire].class)).collect();
        let total: f64 = rooms.iter().sum::<f64>() + extra;
        let before: f64 = rooms.iter().take(position / 2).sum::<f64>()
            + if position % 2 == 1 { rooms[position / 2] / 2.0 } else { extra / 2.0 };
        // Centre the bundle on the edge; squeeze it if it does not fit.
        let t = if total <= length { (length - total) / 2.0 + before } else { before * length / total };
        let t = (t / length.max(1.0e-12)).clamp(0.0, 1.0);
        [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
    }

    /// Cyclic coordinate around `face` of a position on one of its edges.
    fn coordinate(&self, mesh: &Mesh, face: usize, edge: usize, position: usize) -> f64 {
        let slot = mesh.face_edges[face].iter().position(|&e| e == edge).expect("edge of face");
        let vertices = mesh.faces[face];
        let forward = mesh.edges[edge] == [vertices[slot], vertices[(slot + 1) % 3]];
        let span = 2 * self.order[edge].len();
        let t = if forward { position } else { span - position };
        slot as f64 * 1.0e6 + t as f64
    }

    /// Wires of other nets whose chord through `face` separates the two
    /// positions (the new chord would cross them).
    fn crossings(&self, mesh: &Mesh, face: usize, net: NetId, from: (usize, usize), to: (usize, usize)) -> usize {
        let a = self.coordinate(mesh, face, from.0, from.1);
        let b = self.coordinate(mesh, face, to.0, to.1);
        let (low, high) = if a < b { (a, b) } else { (b, a) };
        let mut count = 0;
        for &wire in &self.face_wires[face] {
            if self.wires[wire].net == net {
                continue;
            }
            let Some((c, d)) = self.chord(mesh, face, wire) else {
                continue;
            };
            let inside = |x: f64| x > low && x < high;
            if inside(c) != inside(d) {
                count += 1;
            }
        }
        count
    }

    /// The boundary coordinates of a wire's chord through `face`, if it
    /// enters and leaves by edges there.
    pub fn chord(&self, mesh: &Mesh, face: usize, wire: usize) -> Option<(f64, f64)> {
        let path = &self.wires[wire];
        let step = path.faces.iter().position(|&f| f == face)?;
        if step == 0 || step + 1 >= path.faces.len() {
            return None;
        }
        let (entry, exit) = (path.edges[step - 1], path.edges[step]);
        let place = |edge: usize| 2 * self.order[edge].iter().position(|&w| w == wire).expect("wire on edge") + 1;
        Some((self.coordinate(mesh, face, entry, place(entry)), self.coordinate(mesh, face, exit, place(exit))))
    }

    /// Queues the gaps of `face`'s edges (other than the one entered by) as
    /// successors of `parent`.
    #[allow(clippy::too_many_arguments)]
    fn expand(
        &self,
        board: &Board,
        mesh: &Mesh,
        search: &mut Search,
        net: NetId,
        room: f64,
        target: Point,
        weights: Weights,
        from_point: Point,
        face: usize,
        entry: Option<(usize, usize)>,
        cost: f64,
        parent: usize,
    ) {
        for &edge in &mesh.face_edges[face] {
            if Some(edge) == entry.map(|entry| entry.0) {
                continue;
            }
            let next = mesh.across(edge, face);
            if next == NONE || mesh.face_layers(board, next, net) == 0 {
                continue;
            }
            // Never cross an edge twice.
            let mut ancestor = parent;
            let mut repeated = false;
            while ancestor != NONE {
                if search.nodes[ancestor].edge == edge {
                    repeated = true;
                    break;
                }
                ancestor = search.nodes[ancestor].parent;
            }
            if repeated {
                continue;
            }
            let used = self.used(board, edge);
            let overflow = (used + room - self.capacity[edge]).clamp(0.0, room);
            for gap in 0..=self.order[edge].len() {
                let point = self.portal(board, mesh, edge, 2 * gap, room);
                let crossed = match entry {
                    Some(entry) => self.crossings(mesh, face, net, entry, (edge, 2 * gap)),
                    None => 0,
                };
                let total = cost
                    + distance(from_point, point)
                    + weights.crossing * crossed as f64
                    + weights.overflow * overflow
                    + weights.history * self.history[edge];
                let key = (edge, gap);
                if search.best.get(&key).is_some_and(|&known| known <= total) {
                    continue;
                }
                search.best.insert(key, total);
                search.nodes.push(Node { edge, gap, face: next, point, cost: total, parent });
                search.queue.push(Queued { cost: total + distance(point, target), node: search.nodes.len() - 1 });
            }
        }
    }

    /// Finds the cheapest topological path for `wire` (which must not be
    /// routed) and inserts it. Returns false if the target is unreachable.
    pub fn route(&mut self, board: &Board, mesh: &Mesh, wire: usize, from_face: usize, to_face: usize, weights: Weights) -> bool {
        let (net, class, target, start) = (self.wires[wire].net, self.wires[wire].class, self.wires[wire].to, self.wires[wire].from);
        let room = Self::room(board, class);
        if from_face == to_face {
            self.wires[wire].faces = vec![from_face];
            self.wires[wire].edges = Vec::new();
            self.wires[wire].routed = true;
            self.face_wires[from_face].push(wire);
            return true;
        }
        let mut search = Search { nodes: Vec::new(), best: HashMap::new(), queue: BinaryHeap::new() };
        self.expand(board, mesh, &mut search, net, room, target, weights, start, from_face, None, 0.0, NONE);
        let mut found = None;
        let mut expansions = 0usize;
        while let Some(Queued { node, .. }) = search.queue.pop() {
            let current = search.nodes[node];
            if search.best.get(&(current.edge, current.gap)).is_some_and(|&known| known < current.cost) {
                continue;
            }
            if current.face == to_face {
                found = Some(node);
                break;
            }
            expansions += 1;
            if expansions > 400_000 {
                break;
            }
            self.expand(
                board,
                mesh,
                &mut search,
                net,
                room,
                target,
                weights,
                current.point,
                current.face,
                Some((current.edge, 2 * current.gap)),
                current.cost,
                node,
            );
        }
        let Some(mut node) = found else {
            return false;
        };
        let mut steps = Vec::new();
        while node != NONE {
            steps.push((search.nodes[node].edge, search.nodes[node].gap, search.nodes[node].face));
            node = search.nodes[node].parent;
        }
        steps.reverse();
        let mut faces = vec![from_face];
        let mut edges = Vec::new();
        for (edge, gap, face) in steps {
            self.order[edge].insert(gap, wire);
            edges.push(edge);
            faces.push(face);
        }
        for &face in &faces {
            if !self.face_wires[face].contains(&wire) {
                self.face_wires[face].push(wire);
            }
        }
        let path = &mut self.wires[wire];
        path.faces = faces;
        path.edges = edges;
        path.routed = true;
        true
    }

    /// Takes a wire out of the topology.
    pub fn rip_up(&mut self, wire: usize) {
        for &edge in &self.wires[wire].edges {
            self.order[edge].retain(|&w| w != wire);
        }
        for &face in &self.wires[wire].faces {
            self.face_wires[face].retain(|&w| w != wire);
        }
        let path = &mut self.wires[wire];
        path.faces.clear();
        path.edges.clear();
        path.routed = false;
    }

    /// Pairs of wires of different nets that cross, with the face where
    /// they do.
    pub fn crossing_pairs(&self, mesh: &Mesh) -> Vec<(usize, usize, usize)> {
        let mut pairs = Vec::new();
        for face in 0..mesh.faces.len() {
            let wires = &self.face_wires[face];
            let chords: Vec<(usize, f64, f64)> = wires
                .iter()
                .filter_map(|&wire| self.chord(mesh, face, wire).map(|(a, b)| (wire, a.min(b), a.max(b))))
                .collect();
            for i in 0..chords.len() {
                for j in i + 1..chords.len() {
                    let (a, low, high) = chords[i];
                    let (b, c, d) = chords[j];
                    if self.wires[a].net == self.wires[b].net {
                        continue;
                    }
                    let inside = |x: f64| x > low && x < high;
                    if inside(c) != inside(d) {
                        pairs.push((a, b, face));
                    }
                }
            }
        }
        pairs
    }

    /// Edges whose crossing wires need more room than the edge has.
    pub fn overflowing(&self, board: &Board) -> Vec<usize> {
        (0..self.order.len())
            .filter(|&edge| !self.order[edge].is_empty() && self.used(board, edge) > self.capacity[edge] + 1.0e-9)
            .collect()
    }
}

/// Whether `face` is free of track-blocking obstacles on `layer` (for any
/// net: own pads count as blocking).
pub fn free_on(board: &Board, mesh: &Mesh, face: usize, layer: usize) -> bool {
    mesh.inside[face]
        && mesh.face_blockers[face].iter().all(|&blocker| {
            let obstacle = &board.obstacles[blocker];
            !obstacle.blocks_tracks || obstacle.layers & (1 << layer) == 0
        })
}
