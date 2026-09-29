// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Layer assignment after topological routing (TopoR's "расслоение"). Every
//! routed wire gets a layer in each face it passes. Wires of different nets
//! that cross in a face must be on different layers there; a wire changes
//! layer only where it passes a via site (that is a via); a terminal's face
//! takes a layer of its pad; and each layer's room on each edge is shared
//! by the wires on it.
//!
//! Each wire's best layers, given everyone else's, are found exactly by
//! dynamic programming over its faces (a Viterbi pass); passes over all
//! wires in random order repeat until none improves. Paths never change,
//! and a wire only moves if the total cost drops.

use crate::mesh::Mesh;
use crate::topo::{Portal, Topology};
use pcb_router::Board;
use std::collections::HashMap;

/// Cost of a crossing of two wires on the same layer (illegal).
const CONFLICT: f64 = 1000.0;
/// Cost of a layer a face or pad does not allow, or a change of layer
/// across an edge.
const INFEASIBLE: f64 = 1.0e6;

#[derive(Clone, Copy, Debug)]
pub struct Costs {
    pub via: f64,
    /// Per millimetre of overflow of a layer's room on an edge.
    pub overflow: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stats {
    pub vias: usize,
    /// Crossings of two wires on the same layer.
    pub conflicts: usize,
    pub infeasible: usize,
    pub cost: f64,
}

/// Per wire, per step: the crossing partners (other wire, its step).
fn partners(mesh: &Mesh, topology: &Topology) -> Vec<Vec<Vec<(usize, usize)>>> {
    let mut list: Vec<Vec<Vec<(usize, usize)>>> = topology.wires.iter().map(|wire| vec![Vec::new(); wire.faces.len()]).collect();
    for (a, b, face) in topology.crossing_pairs(mesh) {
        let step_a = topology.wires[a].faces.iter().position(|&f| f == face).expect("face on wire");
        let step_b = topology.wires[b].faces.iter().position(|&f| f == face).expect("face on wire");
        list[a][step_a].push((b, step_b));
        list[b][step_b].push((a, step_a));
    }
    list
}

/// Layers a wire may take at each step.
fn allowed(board: &Board, mesh: &Mesh, topology: &Topology, wire: usize) -> Vec<u32> {
    let path = &topology.wires[wire];
    let terminals = &board.nets[path.net as usize].terminals;
    let last = path.faces.len() - 1;
    path.faces
        .iter()
        .enumerate()
        .map(|(step, &face)| {
            let mut mask = mesh.face_layers(board, face, path.net);
            if step == 0 {
                mask &= terminals[path.terminals[0]].layers;
            }
            if step == last {
                mask &= terminals[path.terminals[1]].layers;
            }
            mask
        })
        .collect()
}

/// Room used per edge and layer by the assigned wires.
#[derive(Default)]
struct Usage {
    /// (edge, layer) -> [(net, wire, room)]
    rooms: HashMap<(usize, usize), Vec<(u32, usize, f64)>>,
}

impl Usage {
    fn add(&mut self, board: &Board, topology: &Topology, wire: usize) {
        let path = &topology.wires[wire];
        let room = Topology::track_room(board, path.class);
        for (step, &portal) in path.portals.iter().enumerate() {
            if let Portal::Edge(edge) = portal {
                self.rooms.entry((edge, path.layers[step])).or_default().push((path.net, wire, room));
            }
        }
    }

    fn remove(&mut self, board: &Board, topology: &Topology, wire: usize) {
        let path = &topology.wires[wire];
        for &portal in &path.portals {
            if let Portal::Edge(edge) = portal {
                for layer in 0..board.layer_count {
                    if let Some(list) = self.rooms.get_mut(&(edge, layer)) {
                        list.retain(|&(_, owner, _)| owner != wire);
                    }
                }
            }
        }
    }

    fn used(&self, edge: usize, layer: usize, net: u32) -> f64 {
        self.rooms.get(&(edge, layer)).map_or(0.0, |list| list.iter().filter(|entry| entry.0 != net).map(|entry| entry.2).sum())
    }
}

/// Everything the cost of one wire's layers depends on.
struct Context<'a> {
    board: &'a Board,
    mesh: &'a Mesh,
    topology: &'a Topology,
    usage: &'a Usage,
    costs: Costs,
}

impl Context<'_> {
    fn overflow(&self, wire: usize, edge: usize, layer: usize, room: f64) -> f64 {
        let path = &self.topology.wires[wire];
        let capacity = self.topology.capacity(self.board, self.mesh, edge, Some(layer), path.net, path.class);
        (self.usage.used(edge, layer, path.net) + room - capacity).clamp(0.0, room)
    }

    fn state(&self, wire: usize, step: usize, layer: usize, partners: &[Vec<(usize, usize)>], allowed: &[u32]) -> f64 {
        let mut cost = if allowed[step] & (1 << layer) == 0 { INFEASIBLE } else { 0.0 };
        for &(other, other_step) in &partners[step] {
            if other != wire && self.topology.wires[other].layers.get(other_step) == Some(&layer) {
                cost += CONFLICT;
            }
        }
        cost
    }

    /// Cost of passing `portal` from layer `from` to layer `to`: the
    /// layer's room across an edge, a via at a via site.
    fn transition(&self, wire: usize, portal: Portal, from: usize, to: usize) -> f64 {
        let path = &self.topology.wires[wire];
        match portal {
            Portal::Edge(edge) if from == to => self.costs.overflow * self.overflow(wire, edge, to, Topology::track_room(self.board, path.class)),
            Portal::Edge(_) => INFEASIBLE,
            Portal::Vertex(_) if from == to => 0.0,
            Portal::Vertex(_) => self.costs.via,
        }
    }

    /// The cheapest layers for `wire` given everyone else's, and their cost.
    fn best(&self, wire: usize, partners: &[Vec<(usize, usize)>], allowed: &[u32]) -> (Vec<usize>, f64) {
        let path = &self.topology.wires[wire];
        let layers = self.board.layer_count;
        let steps = path.faces.len();
        let mut cost = vec![vec![f64::INFINITY; layers]; steps];
        let mut from = vec![vec![0usize; layers]; steps];
        for layer in 0..layers {
            cost[0][layer] = self.state(wire, 0, layer, partners, allowed);
        }
        for step in 1..steps {
            let portal = path.portals[step - 1];
            for layer in 0..layers {
                let here = self.state(wire, step, layer, partners, allowed);
                let mut best = (f64::INFINITY, 0);
                for previous in 0..layers {
                    let total = cost[step - 1][previous] + self.transition(wire, portal, previous, layer);
                    if total < best.0 {
                        best = (total, previous);
                    }
                }
                cost[step][layer] = best.0 + here;
                from[step][layer] = best.1;
            }
        }
        let mut layer = (0..layers).min_by(|&a, &b| cost[steps - 1][a].total_cmp(&cost[steps - 1][b])).unwrap_or(0);
        let total = cost[steps - 1][layer];
        let mut chosen = vec![0; steps];
        for step in (0..steps).rev() {
            chosen[step] = layer;
            layer = from[step][layer];
        }
        (chosen, total)
    }

    /// The cost of `wire`'s current layers, as `best` counts it.
    fn current(&self, wire: usize, partners: &[Vec<(usize, usize)>], allowed: &[u32]) -> f64 {
        let path = &self.topology.wires[wire];
        let mut total = 0.0;
        for step in 0..path.faces.len() {
            total += self.state(wire, step, path.layers[step], partners, allowed);
            if step > 0 {
                total += self.transition(wire, path.portals[step - 1], path.layers[step - 1], path.layers[step]);
            }
        }
        total
    }
}

/// Assigns layers to every routed wire; wires that already have layers
/// start from them.
pub fn assign(board: &Board, mesh: &Mesh, topology: &mut Topology, costs: Costs, seed: u64) -> Stats {
    let partners = partners(mesh, topology);
    let routed: Vec<usize> = (0..topology.wires.len()).filter(|&wire| topology.wires[wire].routed).collect();
    let allowed: Vec<Vec<u32>> = (0..topology.wires.len())
        .map(|wire| if topology.wires[wire].routed { allowed(board, mesh, topology, wire) } else { Vec::new() })
        .collect();
    let mut usage = Usage::default();
    for &wire in &routed {
        if topology.wires[wire].layers.len() == topology.wires[wire].faces.len() {
            usage.add(board, topology, wire);
        } else {
            topology.wires[wire].layers.clear();
        }
    }
    // First the unassigned wires, most crossed first.
    let mut fresh: Vec<usize> = routed.iter().copied().filter(|&wire| topology.wires[wire].layers.is_empty()).collect();
    fresh.sort_by_key(|&wire| std::cmp::Reverse(partners[wire].iter().map(Vec::len).sum::<usize>()));
    for wire in fresh {
        let context = Context { board, mesh, topology, usage: &usage, costs };
        let (layers, _) = context.best(wire, &partners[wire], &allowed[wire]);
        topology.wires[wire].layers = layers;
        usage.add(board, topology, wire);
    }
    // Descent: each wire re-chooses given the others, in random order,
    // while anything improves.
    let mut random = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut order = routed.clone();
    for _round in 0..30 {
        for index in (1..order.len()).rev() {
            random ^= random << 13;
            random ^= random >> 7;
            random ^= random << 17;
            order.swap(index, (random % (index as u64 + 1)) as usize);
        }
        let mut improved = false;
        for &wire in &order {
            usage.remove(board, topology, wire);
            let context = Context { board, mesh, topology, usage: &usage, costs };
            let current = context.current(wire, &partners[wire], &allowed[wire]);
            let (layers, cost) = context.best(wire, &partners[wire], &allowed[wire]);
            if cost + 1.0e-9 < current {
                topology.wires[wire].layers = layers;
                improved = true;
            }
            usage.add(board, topology, wire);
        }
        if !improved {
            break;
        }
    }
    // On two layers, whole webs of crossing stretches flip together: the
    // moves one wire at a time cannot make.
    if board.layer_count == 2 {
        for _ in 0..4 {
            if !flip_clusters(topology, &partners, &allowed, costs, &mut random) {
                break;
            }
            let mut improved = false;
            for &wire in &order {
                usage.remove(board, topology, wire);
                let context = Context { board, mesh, topology, usage: &usage, costs };
                let current = context.current(wire, &partners[wire], &allowed[wire]);
                let (layers, cost) = context.best(wire, &partners[wire], &allowed[wire]);
                if cost + 1.0e-9 < current {
                    topology.wires[wire].layers = layers;
                    improved = true;
                }
                usage.add(board, topology, wire);
            }
            if !improved {
                break;
            }
        }
    }
    let context = Context { board, mesh, topology, usage: &usage, costs };
    let mut stats = Stats::default();
    for &wire in &routed {
        let path = &topology.wires[wire];
        stats.cost += context.current(wire, &partners[wire], &allowed[wire]);
        for step in 0..path.faces.len() {
            if allowed[wire][step] & (1 << path.layers[step]) == 0 {
                stats.infeasible += 1;
            }
            for &(other, other_step) in &partners[wire][step] {
                if other > wire && topology.wires[other].layers.get(other_step) == Some(&path.layers[step]) {
                    stats.conflicts += 1;
                }
            }
            if step + 1 < path.faces.len() && path.via_at(step) {
                stats.vias += 1;
                if matches!(path.portals[step], Portal::Edge(_)) {
                    stats.infeasible += 1;
                }
            }
        }
    }
    stats
}

/// Two-layer moves on whole clusters. A wire's stretch between via sites
/// (or its ends) has a single layer, and stretches that cross must differ,
/// so the stretches linked by crossings flip together, keeping every
/// crossing apart. Simulated annealing over the clusters minimizes vias and
/// layers not allowed. Returns whether the layers changed (for the better).
fn flip_clusters(topology: &mut Topology, partners: &[Vec<Vec<(usize, usize)>>], allowed: &[Vec<u32>], costs: Costs, random: &mut u64) -> bool {
    let mut next = || {
        *random ^= *random << 13;
        *random ^= *random >> 7;
        *random ^= *random << 17;
        (*random >> 11) as f64 / (1u64 << 53) as f64
    };
    // Stretches: (wire, first step, last step), and which one each step is in.
    let mut stretches: Vec<(usize, usize, usize)> = Vec::new();
    let mut stretch_of: Vec<Vec<usize>> = vec![Vec::new(); topology.wires.len()];
    for (wire, path) in topology.wires.iter().enumerate() {
        if !path.routed || path.layers.len() != path.faces.len() {
            continue;
        }
        stretch_of[wire] = vec![0; path.faces.len()];
        let mut first = 0;
        for step in 0..path.faces.len() {
            stretch_of[wire][step] = stretches.len();
            if step + 1 == path.faces.len() || matches!(path.portals[step], Portal::Vertex(_)) {
                stretches.push((wire, first, step));
                first = step + 1;
            }
        }
    }
    let count = stretches.len();
    if count == 0 {
        return false;
    }
    // Clusters: stretches joined by crossings.
    let mut parent: Vec<usize> = (0..count).collect();
    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }
    for (wire, list) in partners.iter().enumerate() {
        if stretch_of[wire].is_empty() {
            continue;
        }
        for (step, pairs) in list.iter().enumerate() {
            for &(other, other_step) in pairs {
                if stretch_of[other].is_empty() {
                    continue;
                }
                let (a, b) = (find(&mut parent, stretch_of[wire][step]), find(&mut parent, stretch_of[other][other_step]));
                parent[a] = b;
            }
        }
    }
    let mut cluster_of = vec![0; count];
    let mut clusters: Vec<Vec<usize>> = Vec::new();
    let mut index: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    for stretch in 0..count {
        let root = find(&mut parent, stretch);
        let cluster = *index.entry(root).or_insert_with(|| {
            clusters.push(Vec::new());
            clusters.len() - 1
        });
        cluster_of[stretch] = cluster;
        clusters[cluster].push(stretch);
    }
    // Per stretch: its layer, the cost of each layer, and its neighbours
    // along its wire (a via if their layers differ).
    let mut layer: Vec<usize> = stretches.iter().map(|&(wire, first, _)| topology.wires[wire].layers[first].min(1)).collect();
    let own_cost: Vec<[f64; 2]> = stretches
        .iter()
        .map(|&(wire, first, last)| {
            let mut cost = [0.0; 2];
            for step in first..=last {
                for (side, entry) in cost.iter_mut().enumerate() {
                    if allowed[wire][step] & (1 << side) == 0 {
                        *entry += INFEASIBLE;
                    }
                }
            }
            cost
        })
        .collect();
    let mut neighbours: Vec<Vec<usize>> = vec![Vec::new(); count];
    for (stretch, &(wire, _, last)) in stretches.iter().enumerate() {
        if last + 1 < topology.wires[wire].faces.len() {
            let following = stretch_of[wire][last + 1];
            neighbours[stretch].push(following);
            neighbours[following].push(stretch);
        }
    }
    let energy = |layer: &[usize]| -> f64 {
        let mut total = 0.0;
        for stretch in 0..count {
            total += own_cost[stretch][layer[stretch]];
            for &other in &neighbours[stretch] {
                if other > stretch && layer[other] != layer[stretch] {
                    total += costs.via;
                }
            }
        }
        total
    };
    let delta = |layer: &[usize], cluster: usize| -> f64 {
        let mut change = 0.0;
        for &stretch in &clusters[cluster] {
            change += own_cost[stretch][1 - layer[stretch]] - own_cost[stretch][layer[stretch]];
            for &other in &neighbours[stretch] {
                if cluster_of[other] == cluster {
                    continue;
                }
                let before = (layer[other] != layer[stretch]) as usize as f64;
                let after = (layer[other] != 1 - layer[stretch]) as usize as f64;
                change += costs.via * (after - before);
            }
        }
        change
    };
    let start = energy(&layer);
    let mut current = start;
    let (mut best, mut best_layer) = (start, layer.clone());
    let moves = 60 * clusters.len();
    for step in 0..moves {
        let temperature = costs.via * (1.0 - step as f64 / moves as f64).powi(2) + 1.0e-6;
        let cluster = ((next() * clusters.len() as f64) as usize).min(clusters.len() - 1);
        let change = delta(&layer, cluster);
        if change < 0.0 || next() < (-change / temperature).exp() {
            for &stretch in &clusters[cluster] {
                layer[stretch] = 1 - layer[stretch];
            }
            current += change;
            if current < best - 1.0e-9 {
                best = current;
                best_layer.clone_from(&layer);
            }
        }
    }
    if best + 1.0e-9 >= start {
        return false;
    }
    for (stretch, &(wire, first, last)) in stretches.iter().enumerate() {
        for step in first..=last {
            topology.wires[wire].layers[step] = best_layer[stretch];
        }
    }
    true
}

/// Wires in trouble after assignment: same-layer crossings, layers or vias
/// not allowed.
pub fn troubled(board: &Board, mesh: &Mesh, topology: &Topology) -> Vec<usize> {
    let partners = partners(mesh, topology);
    let mut found = Vec::new();
    for (wire, path) in topology.wires.iter().enumerate() {
        if !path.routed || path.layers.len() != path.faces.len() {
            continue;
        }
        let allowed = allowed(board, mesh, topology, wire);
        let bad = (0..path.faces.len()).any(|step| {
            allowed[step] & (1 << path.layers[step]) == 0
                || partners[wire][step].iter().any(|&(other, other_step)| topology.wires[other].layers.get(other_step) == Some(&path.layers[step]))
                || (step + 1 < path.faces.len() && path.via_at(step) && matches!(path.portals[step], Portal::Edge(_)))
        });
        if bad {
            found.push(wire);
        }
    }
    found
}
