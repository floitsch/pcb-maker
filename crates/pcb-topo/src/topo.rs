// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Topological wires on the shared triangulation (TopoR's model). A wire is
//! the sequence of triangulation edges it crosses, its place in each edge's
//! order of crossings (one order for all layers), and, once layers are
//! assigned, the layer it takes in each face. A via is a change of layer
//! between two faces: it stands on the edge between them, at the wire's
//! place, and takes a via's room there on every layer.
//!
//! Two ways to search for a wire:
//! - [`Mode::Planar`], TopoR's first routing: layers ignored, crossings with
//!   other nets allowed at a cost, capacity summed over the layers;
//! - [`Mode::Layered`], for rip-up and reroute once layers exist: the
//!   search carries a layer, may not cross a wire on the same layer, and
//!   pays for vias and for each layer's capacity.

use crate::mesh::{Mesh, NONE};
use pcb_router::{Board, ClassId, LayerMask, NetId, ObstacleKind, Point};
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
    /// The layer in each face, once assigned (else empty).
    pub layers: Vec<usize>,
    pub routed: bool,
}

impl Wire {
    /// Whether the wire changes layer on `edges[step]`.
    pub fn via_at(&self, step: usize) -> bool {
        !self.layers.is_empty() && self.layers[step] != self.layers[step + 1]
    }
}

/// Cost weights of the search, in millimetres of length (TopoR tunes them
/// per wire and oscillates them; the optimizer does the same).
#[derive(Clone, Copy, Debug)]
pub struct Weights {
    /// One crossing with another net's wire (on another layer, if layered).
    pub crossing: f64,
    pub via: f64,
    /// Per millimetre of capacity overflow on an edge.
    pub overflow: f64,
    /// Per unit of an edge's history (how often it was in trouble).
    pub history: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Planar,
    Layered,
}

pub struct Topology {
    pub wires: Vec<Wire>,
    /// Per edge: the wires crossing it, ordered from `edges[e][0]`.
    pub order: Vec<Vec<usize>>,
    /// Per face: wires passing through or ending in it.
    pub face_wires: Vec<Vec<usize>>,
    /// Per edge: congestion history.
    pub history: Vec<f64>,
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

fn distance(a: Point, b: Point) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// A search node: a gap on an edge, on a layer, entering `face`.
#[derive(Clone, Copy)]
struct Node {
    edge: usize,
    gap: usize,
    layer: usize,
    face: usize,
    point: Point,
    cost: f64,
    parent: usize,
}

struct Search {
    nodes: Vec<Node>,
    best: HashMap<(usize, usize, usize, usize), f64>,
    queue: BinaryHeap<Queued>,
}

/// What the search needs to know about the wire being routed.
struct Request {
    net: NetId,
    class: ClassId,
    room: f64,
    via_room: f64,
    target: Point,
    mode: Mode,
    weights: Weights,
}

impl Topology {
    pub fn new(mesh: &Mesh) -> Self {
        Self {
            wires: Vec::new(),
            order: vec![Vec::new(); mesh.edges.len()],
            face_wires: vec![Vec::new(); mesh.faces.len()],
            history: vec![0.0; mesh.edges.len()],
        }
    }

    /// Room a track of `class` takes on an edge.
    pub fn track_room(board: &Board, class: ClassId) -> f64 {
        board.classes[class].trace_width + board.classes[class].clearance
    }

    pub fn via_room(board: &Board, class: ClassId) -> f64 {
        board.classes[class].via_diameter + board.classes[class].clearance
    }

    /// The step at which `wire` crosses `edge`.
    pub fn step_of(&self, wire: usize, edge: usize) -> Option<usize> {
        self.wires[wire].edges.iter().position(|&e| e == edge)
    }

    /// Room `wire` takes on `edge`: a via's if it changes layer there.
    pub fn room_on(&self, board: &Board, wire: usize, edge: usize) -> f64 {
        let path = &self.wires[wire];
        match self.step_of(wire, edge) {
            Some(step) if path.via_at(step) => Self::via_room(board, path.class),
            _ => Self::track_room(board, path.class),
        }
    }

    /// The layer `wire` has in `face` (if assigned).
    pub fn layer_in(&self, wire: usize, face: usize) -> Option<usize> {
        let path = &self.wires[wire];
        if path.layers.is_empty() {
            return None;
        }
        path.faces.iter().position(|&f| f == face).map(|step| path.layers[step])
    }

    /// Room other nets' wires take on `edge` on `layer` (all layers
    /// together with `None`).
    pub fn used(&self, board: &Board, edge: usize, layer: Option<usize>, net: NetId) -> f64 {
        let mut total = 0.0;
        for &wire in &self.order[edge] {
            let path = &self.wires[wire];
            if path.net == net {
                continue;
            }
            let step = self.step_of(wire, edge).expect("wire on edge");
            let counts = match layer {
                None => true,
                Some(layer) => path.layers.is_empty() || path.via_at(step) || path.layers[step] == layer,
            };
            if counts {
                total += self.room_on(board, wire, edge);
            }
        }
        total
    }

    /// Room for tracks of `net`/`class` on `edge` on `layer` (summed over
    /// the layers with `None`): its length less the clearance at each end
    /// that is an obstacle's corner on that layer (not the net's own
    /// copper) or the outline.
    pub fn capacity(board: &Board, mesh: &Mesh, edge: usize, layer: Option<usize>, net: NetId, class: ClassId) -> f64 {
        let layers: Vec<usize> = match layer {
            Some(layer) => vec![layer],
            None => (0..board.layer_count).collect(),
        };
        let rule = &board.classes[class];
        let mut total = 0.0;
        for layer in layers {
            let mut room = mesh.edge_length(edge);
            for &vertex in &mesh.edges[edge] {
                let mut deduction: f64 = if mesh.vertex_on_outline[vertex] { board.edge_clearance } else { 0.0 };
                for &obstacle in &mesh.vertex_obstacles[vertex] {
                    let obstacle = &board.obstacles[obstacle];
                    if !obstacle.blocks_tracks || obstacle.layers & (1 << layer) == 0 {
                        continue;
                    }
                    let gap = match obstacle.kind {
                        ObstacleKind::Copper if obstacle.net == Some(net) => 0.0,
                        ObstacleKind::Copper => board.copper_clearance(rule, obstacle),
                        ObstacleKind::Hole => board.hole_clearance.max(obstacle.clearance),
                        ObstacleKind::Keepout => 0.0,
                    };
                    deduction = deduction.max(gap);
                }
                room -= deduction;
            }
            total += room.max(0.0);
        }
        total
    }

    /// Where on `edge` the wire at `position` of its order runs, spreading
    /// all crossing wires (every layer) by their room from the edge's first
    /// vertex; `extra` is room for a wire being placed at a gap.
    /// `position` counts in halves: gap `g` is `2g`, wire `j` is `2j + 1`.
    pub fn portal(&self, board: &Board, mesh: &Mesh, edge: usize, position: usize, extra: f64) -> Point {
        let [a, b] = mesh.edges[edge].map(|vertex| mesh.points[vertex]);
        let length = mesh.edge_length(edge);
        let rooms: Vec<f64> = self.order[edge].iter().map(|&wire| self.room_on(board, wire, edge)).collect();
        let total: f64 = rooms.iter().sum::<f64>() + extra;
        let before: f64 = rooms.iter().take(position / 2).sum::<f64>()
            + if position % 2 == 1 { rooms[position / 2] / 2.0 } else { extra / 2.0 };
        // Centre the bundle on the edge; squeeze it if it does not fit.
        let t = if total <= length { (length - total) / 2.0 + before } else { before * length / total };
        let t = (t / length.max(1.0e-12)).clamp(0.0, 1.0);
        [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
    }

    /// Where `wire` runs on `edge` (its place in the order).
    pub fn place(&self, board: &Board, mesh: &Mesh, wire: usize, edge: usize) -> Point {
        let index = self.order[edge].iter().position(|&w| w == wire).expect("wire on edge");
        self.portal(board, mesh, edge, 2 * index + 1, 0.0)
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
    /// positions (a chord between them would cross those wires).
    fn separating(&self, mesh: &Mesh, face: usize, net: NetId, from: (usize, usize), to: (usize, usize)) -> Vec<usize> {
        let a = self.coordinate(mesh, face, from.0, from.1);
        let b = self.coordinate(mesh, face, to.0, to.1);
        let (low, high) = if a < b { (a, b) } else { (b, a) };
        let mut found = Vec::new();
        for &wire in &self.face_wires[face] {
            if self.wires[wire].net == net {
                continue;
            }
            let Some((c, d)) = self.chord(mesh, face, wire) else {
                continue;
            };
            let inside = |x: f64| x > low && x < high;
            if inside(c) != inside(d) {
                found.push(wire);
            }
        }
        found
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
        request: &Request,
        from_point: Point,
        face: usize,
        layer: usize,
        entry: Option<(usize, usize)>,
        cost: f64,
        parent: usize,
    ) {
        let layered = request.mode == Mode::Layered;
        for &edge in &mesh.face_edges[face] {
            if Some(edge) == entry.map(|entry| entry.0) {
                continue;
            }
            let next = mesh.across(edge, face);
            if next == NONE {
                continue;
            }
            let mask = mesh.face_layers(board, next, request.net);
            if mask == 0 {
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
            let via_allowed = layered && mesh.via_allowed(board, edge);
            // Overflow on this edge per layer the wire may continue on.
            let overflow = |layer: Option<usize>, room: f64| {
                let used = self.used(board, edge, layer, request.net);
                let capacity = Self::capacity(board, mesh, edge, layer, request.net, request.class);
                (used + room - capacity).clamp(0.0, room)
            };
            let planar_overflow = if layered { 0.0 } else { overflow(None, request.room) };
            let mut layer_overflow = [0.0f64; 32];
            let mut via_overflow = 0.0f64;
            if layered {
                for next_layer in 0..board.layer_count {
                    layer_overflow[next_layer] = overflow(Some(next_layer), request.room);
                    if via_allowed {
                        via_overflow = via_overflow.max(overflow(Some(next_layer), request.via_room));
                    }
                }
            }
            for gap in 0..=self.order[edge].len() {
                let point = self.portal(board, mesh, edge, 2 * gap, request.room);
                let separated = match entry {
                    Some(entry) => self.separating(mesh, face, request.net, entry, (edge, 2 * gap)),
                    None => Vec::new(),
                };
                let mut crossings = 0usize;
                let mut blocked = false;
                for &other in &separated {
                    if layered && self.layer_in(other, face) == Some(layer) {
                        blocked = true;
                        break;
                    }
                    crossings += 1;
                }
                if blocked {
                    continue;
                }
                let base = cost + distance(from_point, point) + request.weights.crossing * crossings as f64 + request.weights.history * self.history[edge];
                let layers: Vec<usize> = if layered { (0..board.layer_count).filter(|&l| mask & (1 << l) != 0).collect() } else { vec![0] };
                for next_layer in layers {
                    let total = if !layered {
                        base + request.weights.overflow * planar_overflow
                    } else if next_layer == layer {
                        base + request.weights.overflow * layer_overflow[next_layer]
                    } else if via_allowed {
                        base + request.weights.via + request.weights.overflow * via_overflow
                    } else {
                        continue;
                    };
                    let key = (edge, gap, next_layer, next);
                    if search.best.get(&key).is_some_and(|&known| known <= total) {
                        continue;
                    }
                    search.best.insert(key, total);
                    search.nodes.push(Node { edge, gap, layer: next_layer, face: next, point, cost: total, parent });
                    search.queue.push(Queued { cost: total + distance(point, request.target), node: search.nodes.len() - 1 });
                }
            }
        }
    }

    /// Finds the cheapest path for `wire` (which must not be routed) and
    /// inserts it; with [`Mode::Layered`] it starts on a layer of
    /// `start_layers` and ends on one of `end_layers`. Returns false if the
    /// target is unreachable.
    #[allow(clippy::too_many_arguments)]
    pub fn route(
        &mut self,
        board: &Board,
        mesh: &Mesh,
        wire: usize,
        from_face: usize,
        to_face: usize,
        weights: Weights,
        mode: Mode,
        start_layers: LayerMask,
        end_layers: LayerMask,
    ) -> bool {
        let path = &self.wires[wire];
        let request = Request {
            net: path.net,
            class: path.class,
            room: Self::track_room(board, path.class),
            via_room: Self::via_room(board, path.class),
            target: path.to,
            mode,
            weights,
        };
        let start = path.from;
        let start_mask = start_layers & mesh.face_layers(board, from_face, request.net);
        let end_mask = end_layers & mesh.face_layers(board, to_face, request.net);
        let starts: Vec<usize> = match mode {
            Mode::Planar => vec![0],
            Mode::Layered => (0..board.layer_count).filter(|&l| start_mask & (1 << l) != 0).collect(),
        };
        if from_face == to_face {
            let layer = match mode {
                Mode::Planar => None,
                Mode::Layered => match (0..board.layer_count).find(|&l| start_mask & end_mask & (1 << l) != 0) {
                    Some(layer) => Some(layer),
                    None => return false,
                },
            };
            let path = &mut self.wires[wire];
            path.faces = vec![from_face];
            path.edges = Vec::new();
            path.layers = layer.map(|layer| vec![layer]).unwrap_or_default();
            path.routed = true;
            self.face_wires[from_face].push(wire);
            return true;
        }
        let mut search = Search { nodes: Vec::new(), best: HashMap::new(), queue: BinaryHeap::new() };
        for &layer in &starts {
            // A root node per start layer (edge NONE: not an edge).
            search.nodes.push(Node { edge: NONE, gap: 0, layer, face: from_face, point: start, cost: 0.0, parent: NONE });
            let root = search.nodes.len() - 1;
            self.expand(board, mesh, &mut search, &request, start, from_face, layer, None, 0.0, root);
        }
        let mut found = None;
        let mut expansions = 0usize;
        while let Some(Queued { node, .. }) = search.queue.pop() {
            let current = search.nodes[node];
            if search.best.get(&(current.edge, current.gap, current.layer, current.face)).is_some_and(|&known| known < current.cost) {
                continue;
            }
            if current.face == to_face && (mode == Mode::Planar || end_mask & (1 << current.layer) != 0) {
                found = Some(node);
                break;
            }
            expansions += 1;
            if expansions > 300_000 {
                break;
            }
            self.expand(
                board,
                mesh,
                &mut search,
                &request,
                current.point,
                current.face,
                current.layer,
                Some((current.edge, 2 * current.gap)),
                current.cost,
                node,
            );
        }
        let Some(mut node) = found else {
            return false;
        };
        let mut steps = Vec::new();
        while search.nodes[node].edge != NONE {
            steps.push(search.nodes[node]);
            node = search.nodes[node].parent;
        }
        let first_layer = search.nodes[node].layer;
        steps.reverse();
        let mut faces = vec![from_face];
        let mut edges = Vec::new();
        let mut layers = vec![first_layer];
        for step in &steps {
            self.order[step.edge].insert(step.gap, wire);
            edges.push(step.edge);
            faces.push(step.face);
            layers.push(step.layer);
        }
        for &face in &faces {
            if !self.face_wires[face].contains(&wire) {
                self.face_wires[face].push(wire);
            }
        }
        let path = &mut self.wires[wire];
        path.faces = faces;
        path.edges = edges;
        path.layers = if mode == Mode::Layered { layers } else { Vec::new() };
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
        path.layers.clear();
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

    /// Edges where the wires on some layer need more room than there is,
    /// with the wires on them.
    pub fn overflowing(&self, board: &Board, mesh: &Mesh) -> Vec<usize> {
        let mut found = Vec::new();
        for edge in 0..self.order.len() {
            let Some(&first) = self.order[edge].first() else { continue };
            let wire = &self.wires[first];
            let layers: Vec<Option<usize>> = if wire.layers.is_empty() { vec![None] } else { (0..board.layer_count).map(Some).collect() };
            // Measured for the first wire's net and class: an estimate.
            let over = layers.iter().any(|&layer| {
                let used = self.used(board, edge, layer, u32::MAX);
                used > Self::capacity(board, mesh, edge, layer, wire.net, wire.class) + 1.0e-9
            });
            if over {
                found.push(edge);
            }
        }
        found
    }
}
