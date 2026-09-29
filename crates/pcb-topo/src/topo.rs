// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Topological wires on the shared triangulation (TopoR's model). A wire is
//! a sequence of portals, each a triangulation edge it crosses (with its
//! place in that edge's order of crossings, one order for all layers) or a
//! via site it passes through (a free vertex with room for a via; one wire
//! per site), and, once layers are assigned, the layer it takes in each
//! face. A wire changes layer only at a via site: that is a via.
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

/// Where a wire passes from one face to the next.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Portal {
    /// Across an edge.
    Edge(usize),
    /// Through a via site (a vertex).
    Vertex(usize),
}

#[derive(Clone, Debug)]
pub struct Wire {
    pub net: NetId,
    pub class: ClassId,
    pub from: Point,
    pub to: Point,
    /// Terminal indices within the net.
    pub terminals: [usize; 2],
    /// Faces traversed, `portals.len() + 1` of them.
    pub faces: Vec<usize>,
    pub portals: Vec<Portal>,
    /// The layer in each face, once assigned (else empty).
    pub layers: Vec<usize>,
    pub routed: bool,
}

impl Wire {
    /// Whether the wire changes layer at `portals[step]` (a via).
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
    /// Per unit of an edge's or site's history (how often it was in
    /// trouble).
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
    /// Per vertex: the wire passing through it (via sites only).
    pub occupant: Vec<Option<usize>>,
    /// Per face: wires passing through or ending in it.
    pub face_wires: Vec<Vec<usize>>,
    /// Per edge and per vertex: congestion history.
    pub history: Vec<f64>,
    pub vertex_history: Vec<f64>,
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

/// A search node: a portal (and gap, for an edge), on a layer, entering
/// `face`. The roots have no portal.
#[derive(Clone, Copy)]
struct Node {
    portal: Option<Portal>,
    gap: usize,
    layer: usize,
    face: usize,
    point: Point,
    cost: f64,
    parent: usize,
}

type Key = (Option<Portal>, usize, usize, usize);

struct Search {
    nodes: Vec<Node>,
    best: HashMap<Key, f64>,
    queue: BinaryHeap<Queued>,
}

/// What the search needs to know about the wire being routed.
struct Request {
    net: NetId,
    class: ClassId,
    room: f64,
    target: Point,
    mode: Mode,
    weights: Weights,
}

impl Topology {
    pub fn new(mesh: &Mesh) -> Self {
        Self {
            wires: Vec::new(),
            order: vec![Vec::new(); mesh.edges.len()],
            occupant: vec![None; mesh.points.len()],
            face_wires: vec![Vec::new(); mesh.faces.len()],
            history: vec![0.0; mesh.edges.len()],
            vertex_history: vec![0.0; mesh.points.len()],
        }
    }

    /// Room a track of `class` takes on an edge.
    pub fn track_room(board: &Board, class: ClassId) -> f64 {
        board.classes[class].trace_width + board.classes[class].clearance
    }

    /// The step at which `wire` crosses `edge`.
    pub fn step_of(&self, wire: usize, edge: usize) -> Option<usize> {
        self.wires[wire].portals.iter().position(|&portal| portal == Portal::Edge(edge))
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
            let counts = match layer {
                None => true,
                Some(layer) => path.layers.is_empty() || path.layers[self.step_of(wire, edge).expect("wire on edge")] == layer,
            };
            if counts {
                total += Self::track_room(board, path.class);
            }
        }
        total
    }

    /// How far tracks of `net`/`class` on `layer` stay from `vertex` for
    /// what never moves: an obstacle's corner or the outline there.
    fn fixed_keep_off(board: &Board, mesh: &Mesh, vertex: usize, layer: usize, net: NetId, class: ClassId) -> f64 {
        let rule = &board.classes[class];
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
        deduction
    }

    /// Room another net's wire through `vertex` takes from tracks of
    /// `net`/`class` on `layer` (a via's on every layer).
    fn occupant_keep_off(&self, board: &Board, vertex: usize, layer: usize, net: NetId, class: ClassId) -> f64 {
        let Some(wire) = self.occupant[vertex] else { return 0.0 };
        let path = &self.wires[wire];
        if path.net == net {
            return 0.0;
        }
        let (rule, other) = (&board.classes[class], &board.classes[path.class]);
        let gap = rule.clearance.max(other.clearance);
        let step = path.portals.iter().position(|&portal| portal == Portal::Vertex(vertex)).expect("occupant passes");
        if path.via_at(step) {
            other.via_diameter / 2.0 + gap
        } else if path.layers.is_empty() || path.layers[step] == layer {
            other.trace_width / 2.0 + gap
        } else {
            0.0
        }
    }

    /// The cut of `edge` on `layer` less what its ends keep off for good.
    fn fixed_capacity(board: &Board, mesh: &Mesh, edge: usize, layer: usize, net: NetId, class: ClassId) -> f64 {
        // A net's wires may run over its own pads: an edge that ends on one
        // does not limit them.
        let own = mesh.edges[edge].iter().any(|&vertex| {
            mesh.vertex_obstacles[vertex].iter().any(|&obstacle| {
                let obstacle = &board.obstacles[obstacle];
                obstacle.kind == ObstacleKind::Copper && obstacle.net == Some(net) && obstacle.layers & (1 << layer) != 0
            })
        });
        if own {
            return f64::INFINITY;
        }
        // n tracks take n rooms (width plus clearance) and the ends'
        // clearances, less the one clearance the rooms count too many.
        let mut room = mesh.cut[edge][layer] + board.classes[class].clearance;
        for &vertex in &mesh.edges[edge] {
            room -= Self::fixed_keep_off(board, mesh, vertex, layer, net, class);
        }
        room
    }

    /// Room for tracks of `net`/`class` on `edge` on `layer` (summed over
    /// the layers with `None`): its cut less what each end keeps off.
    pub fn capacity(&self, board: &Board, mesh: &Mesh, edge: usize, layer: Option<usize>, net: NetId, class: ClassId) -> f64 {
        let layers: Vec<usize> = match layer {
            Some(layer) => vec![layer],
            None => (0..board.layer_count).collect(),
        };
        let mut total = 0.0;
        for layer in layers {
            let mut room = Self::fixed_capacity(board, mesh, edge, layer, net, class);
            for &vertex in &mesh.edges[edge] {
                room -= self.occupant_keep_off(board, vertex, layer, net, class);
            }
            total += room.max(0.0);
        }
        total
    }

    /// Whether a via of `net`/`class` at `vertex` leaves every wire of
    /// another net crossing an edge around it its room.
    pub fn via_fits(&self, board: &Board, mesh: &Mesh, vertex: usize, net: NetId, class: ClassId) -> bool {
        let rule = &board.classes[class];
        for &face in &mesh.vertex_faces[vertex] {
            for &edge in &mesh.face_edges[face] {
                if !mesh.edges[edge].contains(&vertex) {
                    continue;
                }
                for layer in 0..board.layer_count {
                    let mut crossing = 0.0;
                    let mut widest_gap: f64 = 0.0;
                    for &wire in &self.order[edge] {
                        let path = &self.wires[wire];
                        if path.net == net {
                            continue;
                        }
                        let step = self.step_of(wire, edge).expect("wire on edge");
                        if !path.layers.is_empty() && path.layers[step] != layer {
                            continue;
                        }
                        crossing += Self::track_room(board, path.class);
                        widest_gap = widest_gap.max(board.classes[path.class].clearance);
                    }
                    if crossing == 0.0 {
                        continue;
                    }
                    let other = mesh.edges[edge][0] + mesh.edges[edge][1] - vertex;
                    let fixed = mesh.cut[edge][layer] - Self::fixed_keep_off(board, mesh, other, layer, net, class) + widest_gap;
                    let via = rule.via_diameter / 2.0 + rule.clearance.max(widest_gap);
                    let occupied = self.occupant_keep_off(board, other, layer, u32::MAX, class);
                    if crossing > fixed - via - occupied + 1.0e-9 {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// Whether a track of `net`/`class` fits across `edge` at all, on
    /// `layer` (on some layer with `None`), whatever else is there.
    pub fn passable(board: &Board, mesh: &Mesh, edge: usize, layer: Option<usize>, net: NetId, class: ClassId) -> bool {
        let room = Self::track_room(board, class) - 1.0e-9;
        match layer {
            Some(layer) => Self::fixed_capacity(board, mesh, edge, layer, net, class) >= room,
            None => (0..board.layer_count).any(|layer| Self::fixed_capacity(board, mesh, edge, layer, net, class) >= room),
        }
    }

    /// Where on `edge` the wire at `position` of its order runs, spreading
    /// all crossing wires (every layer) by their room from the edge's first
    /// vertex; `extra` is room for a wire being placed at a gap.
    /// `position` counts in halves: gap `g` is `2g`, wire `j` is `2j + 1`.
    pub fn portal(&self, board: &Board, mesh: &Mesh, edge: usize, position: usize, extra: f64) -> Point {
        let [a, b] = mesh.edges[edge].map(|vertex| mesh.points[vertex]);
        let length = mesh.edge_length(edge);
        let rooms: Vec<f64> = self.order[edge].iter().map(|&wire| Self::track_room(board, self.wires[wire].class)).collect();
        let total: f64 = rooms.iter().sum::<f64>() + extra;
        let before: f64 = rooms.iter().take(position / 2).sum::<f64>()
            + if position % 2 == 1 { rooms[position / 2] / 2.0 } else { extra / 2.0 };
        // Centre the bundle on the edge; squeeze it if it does not fit.
        let t = if total <= length { (length - total) / 2.0 + before } else { before * length / total };
        let t = (t / length.max(1.0e-12)).clamp(0.0, 1.0);
        [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
    }

    /// Where `wire` passes `portal`.
    pub fn place(&self, board: &Board, mesh: &Mesh, wire: usize, portal: Portal) -> Point {
        match portal {
            Portal::Edge(edge) => {
                let index = self.order[edge].iter().position(|&w| w == wire).expect("wire on edge");
                self.portal(board, mesh, edge, 2 * index + 1, 0.0)
            }
            Portal::Vertex(vertex) => mesh.points[vertex],
        }
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

    /// Cyclic coordinate of a corner of `face` (just before the edge that
    /// starts there).
    fn corner(mesh: &Mesh, face: usize, vertex: usize) -> f64 {
        let slot = mesh.faces[face].iter().position(|&v| v == vertex).expect("corner of face");
        slot as f64 * 1.0e6 - 0.5
    }

    /// Coordinate of where `wire` passes `portal`, seen from `face`.
    fn portal_coordinate(&self, mesh: &Mesh, face: usize, wire: usize, portal: Portal) -> f64 {
        match portal {
            Portal::Edge(edge) => {
                let place = 2 * self.order[edge].iter().position(|&w| w == wire).expect("wire on edge") + 1;
                self.coordinate(mesh, face, edge, place)
            }
            Portal::Vertex(vertex) => Self::corner(mesh, face, vertex),
        }
    }

    /// The boundary coordinates of a wire's chord through `face`, if it
    /// enters and leaves there (not a terminal's face).
    pub fn chord(&self, mesh: &Mesh, face: usize, wire: usize) -> Option<(f64, f64)> {
        let path = &self.wires[wire];
        let step = path.faces.iter().position(|&f| f == face)?;
        if step == 0 || step + 1 >= path.faces.len() {
            return None;
        }
        Some((
            self.portal_coordinate(mesh, face, wire, path.portals[step - 1]),
            self.portal_coordinate(mesh, face, wire, path.portals[step]),
        ))
    }

    /// Wires of other nets whose chord through `face` separates the two
    /// coordinates (a chord between them would cross those wires).
    fn separating(&self, mesh: &Mesh, face: usize, net: NetId, a: f64, b: f64) -> Vec<usize> {
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

    /// Crossing cost of a chord in `face` between two coordinates, or
    /// `None` if it would cross a wire on the same layer.
    fn crossing_cost(&self, mesh: &Mesh, request: &Request, face: usize, layer: usize, a: Option<f64>, b: f64) -> Option<f64> {
        let Some(a) = a else { return Some(0.0) };
        let mut cost = 0.0;
        for other in self.separating(mesh, face, request.net, a, b) {
            if request.mode == Mode::Layered && self.layer_in(other, face) == Some(layer) {
                return None;
            }
            cost += request.weights.crossing;
        }
        Some(cost)
    }

    fn is_ancestor(search: &Search, mut node: usize, portal: Portal) -> bool {
        while node != NONE {
            if search.nodes[node].portal == Some(portal) {
                return true;
            }
            node = search.nodes[node].parent;
        }
        false
    }

    fn push(search: &mut Search, request: &Request, node: Node) {
        let key = (node.portal, node.gap, node.layer, node.face);
        if search.best.get(&key).is_some_and(|&known| known <= node.cost) {
            return;
        }
        search.best.insert(key, node.cost);
        search.nodes.push(node);
        search.queue.push(Queued { cost: node.cost + distance(node.point, request.target), node: search.nodes.len() - 1 });
    }

    /// Queues the successors of `parent`, which entered `face` on `layer`.
    fn expand(&self, board: &Board, mesh: &Mesh, search: &mut Search, request: &Request, parent: usize) {
        let Node { face, layer, point: from_point, cost, portal: entry, gap: entry_gap, .. } = search.nodes[parent];
        let layered = request.mode == Mode::Layered;
        let entry_coordinate = entry.map(|portal| match portal {
            Portal::Edge(edge) => self.coordinate(mesh, face, edge, 2 * entry_gap),
            Portal::Vertex(vertex) => Self::corner(mesh, face, vertex),
        });
        let entry_vertex = match entry {
            Some(Portal::Vertex(vertex)) => Some(vertex),
            _ => None,
        };
        // Across an edge, on the same layer.
        for &edge in &mesh.face_edges[face] {
            if entry == Some(Portal::Edge(edge)) || entry_vertex.is_some_and(|vertex| mesh.edges[edge].contains(&vertex)) {
                continue;
            }
            let next = mesh.across(edge, face);
            let wanted: u32 = if layered { 1 << layer } else { u32::MAX };
            if next == NONE || mesh.face_layers(board, next, request.net) & wanted == 0 {
                continue;
            }
            if Self::is_ancestor(search, parent, Portal::Edge(edge)) {
                continue;
            }
            let on = if layered { Some(layer) } else { None };
            if !Self::passable(board, mesh, edge, on, request.net, request.class) {
                continue;
            }
            // Wires through the ends (vias above all) may leave no room.
            let capacity = self.capacity(board, mesh, edge, on, request.net, request.class);
            if layered && capacity + 1.0e-9 < request.room {
                continue;
            }
            let over = (self.used(board, edge, on, request.net) + request.room - capacity).clamp(0.0, request.room);
            let fixed = request.weights.overflow * over + request.weights.history * self.history[edge];
            for gap in 0..=self.order[edge].len() {
                let target = self.coordinate(mesh, face, edge, 2 * gap);
                let Some(crossings) = self.crossing_cost(mesh, request, face, layer, entry_coordinate, target) else {
                    continue;
                };
                let point = self.portal(board, mesh, edge, 2 * gap, request.room);
                let total = cost + distance(from_point, point) + crossings + fixed;
                Self::push(search, request, Node { portal: Some(Portal::Edge(edge)), gap, layer, face: next, point, cost: total, parent });
            }
        }
        // Through a free via site at a corner, into any face around it, on
        // any layer (a change of layer is a via).
        for &vertex in &mesh.faces[face] {
            if !mesh.via_site[vertex] || self.occupant[vertex].is_some() || entry_vertex == Some(vertex) {
                continue;
            }
            // A via here must leave room for the wires already passing it.
            if layered && !self.via_fits(board, mesh, vertex, request.net, request.class) {
                continue;
            }
            if Self::is_ancestor(search, parent, Portal::Vertex(vertex)) {
                continue;
            }
            let Some(crossings) = self.crossing_cost(mesh, request, face, layer, entry_coordinate, Self::corner(mesh, face, vertex)) else {
                continue;
            };
            let point = mesh.points[vertex];
            let base = cost + distance(from_point, point) + crossings + request.weights.history * self.vertex_history[vertex];
            for &next in &mesh.vertex_faces[vertex] {
                if next == face {
                    continue;
                }
                let mask = mesh.face_layers(board, next, request.net);
                let layers: Vec<usize> = if layered { (0..board.layer_count).filter(|&l| mask & (1 << l) != 0).collect() } else if mask != 0 { vec![0] } else { Vec::new() };
                for next_layer in layers {
                    let total = if next_layer == layer { base } else { base + request.weights.via };
                    Self::push(search, request, Node { portal: Some(Portal::Vertex(vertex)), gap: 0, layer: next_layer, face: next, point, cost: total, parent });
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
        let request = Request { net: path.net, class: path.class, room: Self::track_room(board, path.class), target: path.to, mode, weights };
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
            path.portals = Vec::new();
            path.layers = layer.map(|layer| vec![layer]).unwrap_or_default();
            path.routed = true;
            self.face_wires[from_face].push(wire);
            return true;
        }
        let mut search = Search { nodes: Vec::new(), best: HashMap::new(), queue: BinaryHeap::new() };
        for &layer in &starts {
            search.nodes.push(Node { portal: None, gap: 0, layer, face: from_face, point: start, cost: 0.0, parent: NONE });
            search.queue.push(Queued { cost: distance(start, request.target), node: search.nodes.len() - 1 });
        }
        let mut found = None;
        let mut expansions = 0usize;
        while let Some(Queued { node, .. }) = search.queue.pop() {
            let current = search.nodes[node];
            if current.portal.is_some() && search.best.get(&(current.portal, current.gap, current.layer, current.face)).is_some_and(|&known| known < current.cost) {
                continue;
            }
            if current.portal.is_some() && current.face == to_face && (mode == Mode::Planar || end_mask & (1 << current.layer) != 0) {
                found = Some(node);
                break;
            }
            expansions += 1;
            if expansions > 300_000 {
                break;
            }
            self.expand(board, mesh, &mut search, &request, node);
        }
        let Some(mut node) = found else {
            return false;
        };
        let mut steps = Vec::new();
        while search.nodes[node].portal.is_some() {
            steps.push(search.nodes[node]);
            node = search.nodes[node].parent;
        }
        let first_layer = search.nodes[node].layer;
        steps.reverse();
        let mut faces = vec![from_face];
        let mut portals = Vec::new();
        let mut layers = vec![first_layer];
        for step in &steps {
            let portal = step.portal.expect("portal");
            match portal {
                Portal::Edge(edge) => self.order[edge].insert(step.gap, wire),
                Portal::Vertex(vertex) => self.occupant[vertex] = Some(wire),
            }
            portals.push(portal);
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
        path.portals = portals;
        path.layers = if mode == Mode::Layered { layers } else { Vec::new() };
        path.routed = true;
        true
    }

    /// Takes a wire out of the topology.
    pub fn rip_up(&mut self, wire: usize) {
        for &portal in &self.wires[wire].portals {
            match portal {
                Portal::Edge(edge) => self.order[edge].retain(|&w| w != wire),
                Portal::Vertex(vertex) => self.occupant[vertex] = None,
            }
        }
        for &face in &self.wires[wire].faces {
            self.face_wires[face].retain(|&w| w != wire);
        }
        let path = &mut self.wires[wire];
        path.faces.clear();
        path.portals.clear();
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
}
