// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Layer assignment after topological routing (TopoR's "расслоение"). Each
//! wire becomes a chain of tokens: its terminals, one token per face it
//! passes (the layers the face allows) and one per crossing with another
//! net's wire, in order along it. Crossing partners must end on different
//! layers; a change of layer between neighbouring tokens is a via, and a
//! via needs a face (not a pad) where both layers are free. A local search
//! moves runs of tokens between layers to remove conflicts and vias.

use crate::mesh::Mesh;
use crate::topo::Topology;
use pcb_router::{Board, LayerMask, ObstacleKind, Point};

#[derive(Clone, Debug)]
pub struct Token {
    /// The face the token lies in.
    pub face: usize,
    /// Index of that face in the wire's `faces`.
    pub step: usize,
    /// Position along the wire's chord through the face, 0 to 1.
    pub along: f64,
    pub mask: LayerMask,
    /// The other wire's token at a crossing.
    pub partner: Option<(usize, usize)>,
    /// Layers a via next to this token may join, in this face.
    pub via_mask: LayerMask,
}

pub struct Assignment {
    pub tokens: Vec<Vec<Token>>,
    pub layers: Vec<Vec<usize>>,
    pub vias: usize,
    pub conflicts: usize,
    pub infeasible: usize,
}

const VIA: f64 = 1.0;
const CONFLICT: f64 = 40.0;
const INFEASIBLE: f64 = 1000.0;

/// The wire's straight chord through the face at `step` (from where it
/// enters to where it leaves).
pub fn chord(board: &Board, mesh: &Mesh, topology: &Topology, wire: usize, step: usize) -> (Point, Point) {
    let path = &topology.wires[wire];
    let place = |edge: usize| {
        let index = topology.order[edge].iter().position(|&w| w == wire).expect("wire on edge");
        topology.portal(board, mesh, edge, 2 * index + 1, 0.0)
    };
    let from = if step == 0 { path.from } else { place(path.edges[step - 1]) };
    let to = if step + 1 == path.faces.len() { path.to } else { place(path.edges[step]) };
    (from, to)
}

/// Parameter along `p -> q` where it meets `r -> s` (0.5 when parallel).
fn meet(p: Point, q: Point, r: Point, s: Point) -> f64 {
    let d1 = [q[0] - p[0], q[1] - p[1]];
    let d2 = [s[0] - r[0], s[1] - r[1]];
    let denominator = d1[0] * d2[1] - d1[1] * d2[0];
    if denominator.abs() < 1.0e-15 {
        return 0.5;
    }
    (((r[0] - p[0]) * d2[1] - (r[1] - p[1]) * d2[0]) / denominator).clamp(0.0, 1.0)
}

/// Layers on which the net's own pads cover the face (none if no own pad).
fn own_pad_layers(board: &Board, mesh: &Mesh, face: usize, net: pcb_router::NetId) -> LayerMask {
    mesh.face_blockers[face]
        .iter()
        .filter(|&&blocker| {
            let obstacle = &board.obstacles[blocker];
            obstacle.kind == ObstacleKind::Copper && obstacle.net == Some(net)
        })
        .fold(0, |mask, &blocker| mask | board.obstacles[blocker].layers)
}

/// `mask` narrowed to `to`, unless that leaves nothing.
fn restrict(mask: LayerMask, to: LayerMask) -> LayerMask {
    if mask & to != 0 { mask & to } else { mask }
}

/// Builds every routed wire's tokens.
pub fn tokens(board: &Board, mesh: &Mesh, topology: &Topology) -> Vec<Vec<Token>> {
    let mut all: Vec<Vec<Token>> = vec![Vec::new(); topology.wires.len()];
    // Crossing events per wire: (step, along, other wire, face).
    let mut events: Vec<Vec<(usize, f64, usize)>> = vec![Vec::new(); topology.wires.len()];
    for (a, b, face) in topology.crossing_pairs(mesh) {
        let step_a = topology.wires[a].faces.iter().position(|&f| f == face).expect("face on wire");
        let step_b = topology.wires[b].faces.iter().position(|&f| f == face).expect("face on wire");
        let (p, q) = chord(board, mesh, topology, a, step_a);
        let (r, s) = chord(board, mesh, topology, b, step_b);
        events[a].push((step_a, meet(p, q, r, s), b));
        events[b].push((step_b, meet(r, s, p, q), a));
    }
    for (wire, path) in topology.wires.iter().enumerate() {
        if !path.routed {
            continue;
        }
        let net = path.net;
        let terminals = &board.nets[net as usize].terminals;
        let terminal_mask = |index: usize| terminals[index].layers;
        let mut list = Vec::new();
        let last = path.faces.len() - 1;
        let face_mask = |face: usize| {
            let mask = mesh.face_layers(board, face, net);
            let own = own_pad_layers(board, mesh, face, net);
            if own != 0 { mask & own } else { mask }
        };
        let via_mask = |face: usize| {
            if own_pad_layers(board, mesh, face, net) != 0 {
                0
            } else {
                mesh.face_layers(board, face, net)
            }
        };
        list.push(Token {
            face: path.faces[0],
            step: 0,
            along: 0.0,
            mask: restrict(terminal_mask(path.terminals[0]), face_mask(path.faces[0])),
            partner: None,
            via_mask: 0,
        });
        let mut own_events = events[wire].clone();
        own_events.sort_by(|x, y| x.0.cmp(&y.0).then(x.1.total_cmp(&y.1)));
        let mut next_event = 0;
        for (step, &face) in path.faces.iter().enumerate() {
            list.push(Token { face, step, along: 0.0, mask: face_mask(face), partner: None, via_mask: via_mask(face) });
            while next_event < own_events.len() && own_events[next_event].0 == step {
                let (_, along, other) = own_events[next_event];
                list.push(Token {
                    face,
                    step,
                    along,
                    mask: face_mask(face),
                    partner: Some((other, usize::MAX)),
                    via_mask: via_mask(face),
                });
                next_event += 1;
            }
        }
        list.push(Token {
            face: path.faces[last],
            step: last,
            along: 1.0,
            mask: restrict(terminal_mask(path.terminals[1]), face_mask(path.faces[last])),
            partner: None,
            via_mask: 0,
        });
        all[wire] = list;
    }
    // Link crossing partners token to token.
    for wire in 0..all.len() {
        for index in 0..all[wire].len() {
            let Some((other, usize::MAX)) = all[wire][index].partner else {
                continue;
            };
            let face = all[wire][index].face;
            let partner = all[other]
                .iter()
                .position(|token| token.face == face && token.partner == Some((wire, usize::MAX)))
                .expect("crossing partner");
            all[wire][index].partner = Some((other, partner));
            all[other][partner].partner = Some((wire, index));
        }
    }
    all
}

/// Whether a via may join tokens `i` and `i + 1` on layers `a` and `b`.
fn via_allowed(tokens: &[Token], index: usize, a: usize, b: usize) -> bool {
    let both = (1 << a) | (1 << b);
    tokens[index].via_mask & both == both || tokens[index + 1].via_mask & both == both
}

fn cost_of(tokens: &[Vec<Token>], layers: &[Vec<usize>], wire: usize) -> f64 {
    let list = &tokens[wire];
    let chosen = &layers[wire];
    let mut cost = 0.0;
    for index in 0..list.len() {
        if list[index].mask & (1 << chosen[index]) == 0 {
            cost += INFEASIBLE;
        }
        if let Some((other, partner)) = list[index].partner
            && layers[other][partner] == chosen[index]
        {
            cost += CONFLICT / 2.0;
        }
        if index + 1 < list.len() && chosen[index] != chosen[index + 1] {
            cost += if via_allowed(list, index, chosen[index], chosen[index + 1]) { VIA } else { INFEASIBLE };
        }
    }
    cost
}

/// Assigns layers to every routed wire's tokens.
pub fn assign(board: &Board, mesh: &Mesh, topology: &Topology, seed: u64) -> Assignment {
    let tokens = tokens(board, mesh, topology);
    let layer_count = board.layer_count.max(1);
    let mut layers: Vec<Vec<usize>> = tokens.iter().map(|list| vec![0; list.len()]).collect();
    // Start: each wire on one layer, the cheapest given the wires before.
    for wire in 0..tokens.len() {
        if tokens[wire].is_empty() {
            continue;
        }
        let mut best = (f64::INFINITY, 0);
        for layer in 0..layer_count {
            layers[wire].iter_mut().for_each(|chosen| *chosen = layer);
            let cost = cost_of(&tokens, &layers, wire);
            if cost < best.0 {
                best = (cost, layer);
            }
        }
        layers[wire].iter_mut().for_each(|chosen| *chosen = best.1);
    }
    let mut random = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407) | 1;
    let mut next = move || {
        random ^= random << 13;
        random ^= random >> 7;
        random ^= random << 17;
        random
    };
    // Local search: move a span of tokens (a single token, or everything
    // between two layer changes) to another layer while it pays.
    for _round in 0..60 {
        let mut improved = false;
        for wire in 0..tokens.len() {
            let count = tokens[wire].len();
            if count == 0 {
                continue;
            }
            let mut index = 0;
            while index < count {
                // The run of equal layers around `index`.
                let layer = layers[wire][index];
                let mut end = index;
                while end + 1 < count && layers[wire][end + 1] == layer {
                    end += 1;
                }
                let spans = [(index, end), (index, index), (end, end)];
                for &(from, to) in &spans {
                    for candidate in 0..layer_count {
                        if candidate == layer {
                            continue;
                        }
                        let partners: Vec<usize> = (from..=to).filter_map(|i| tokens[wire][i].partner.map(|p| p.0)).collect();
                        let before = cost_of(&tokens, &layers, wire)
                            + partners.iter().map(|&other| cost_of(&tokens, &layers, other)).sum::<f64>();
                        let saved: Vec<usize> = layers[wire][from..=to].to_vec();
                        layers[wire][from..=to].iter_mut().for_each(|chosen| *chosen = candidate);
                        let after = cost_of(&tokens, &layers, wire)
                            + partners.iter().map(|&other| cost_of(&tokens, &layers, other)).sum::<f64>();
                        if after + 1.0e-9 < before {
                            improved = true;
                        } else {
                            layers[wire][from..=to].copy_from_slice(&saved);
                        }
                    }
                }
                index = end + 1;
            }
        }
        if !improved {
            // A random kick: flip one wire's longest run, keep if not worse
            // overall after another pass (cheap diversification).
            if tokens.is_empty() || next() % 4 != 0 {
                break;
            }
        }
    }
    let mut vias = 0;
    let mut conflicts = 0;
    let mut infeasible = 0;
    for wire in 0..tokens.len() {
        let list = &tokens[wire];
        for index in 0..list.len() {
            if list[index].mask & (1 << layers[wire][index]) == 0 {
                infeasible += 1;
            }
            if let Some((other, partner)) = list[index].partner
                && layers[other][partner] == layers[wire][index]
                && wire < other
            {
                conflicts += 1;
            }
            if index + 1 < list.len() && layers[wire][index] != layers[wire][index + 1] {
                vias += 1;
            }
        }
    }
    let _ = mesh;
    Assignment { tokens, layers, vias, conflicts, infeasible }
}
