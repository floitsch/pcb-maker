// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! A topological router in the manner of TopoR (Luzin & Polubasov; see
//! docs/reviews/2026-09-29-topor.md).
//!
//! 1. One constrained Delaunay triangulation of all obstacles, shared by
//!    the layers ([`mesh`]).
//! 2. Every connection routed topologically, crossings allowed, widest
//!    nets first, then the shortest: all connections are made at once,
//!    rules violated or not ([`topo`]).
//! 3. Layers assigned afterwards, with vias where crossing wires must part
//!    ([`layers`]).
//! 4. Geometry: each wire pulled tight around the obstacle corners as
//!    tangents and arcs ([`realize`]).
//! 5. Rip-up and reroute of the wires the exact verifier objects to, with
//!    congestion history and oscillating weights, keeping the best board.
//!    What still violates is taken out and reported open, so the copper
//!    that remains is legal.

pub mod layers;
pub mod mesh;
pub mod realize;
pub mod topo;

use pcb_router::grid::Grid;
use pcb_router::{Board, Diagnostics, NetRoute, NetStatus, Point, RoutingResult, verify};
use std::time::Instant;
use topo::{Topology, Weights, Wire};

#[derive(Clone, Debug)]
pub struct Config {
    /// Rip-up rounds after the first routing.
    pub iterations: usize,
    /// Wall-clock budget for the rounds.
    pub seconds: f64,
    pub seed: u64,
    pub verbose: bool,
    pub weights: Weights,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            iterations: 30,
            seconds: 300.0,
            seed: 1,
            verbose: false,
            weights: Weights { crossing: 2.0, overflow: 50.0, history: 5.0 },
        }
    }
}

fn distance(a: Point, b: Point) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// One realized state of the board.
struct Attempt {
    routes: Vec<NetRoute>,
    segment_wire: Vec<Vec<usize>>,
    via_wire: Vec<Vec<usize>>,
    bad: Vec<usize>,
    vias: usize,
    length: f64,
}

fn realize_all(board: &Board, mesh: &mesh::Mesh, topology: &Topology, seed: u64) -> Attempt {
    let assignment = layers::assign(board, mesh, topology, seed);
    let layout = realize::pieces(board, mesh, topology, &assignment);
    let realized = realize::realize(board, &layout, topology);
    let mut bad = realized.failed.clone();
    let violations = verify(board, &realized.routes);
    if std::env::var("PCB_TOPO_DEBUG").is_ok() {
        eprintln!("  failed realizations: {:?}", realized.failed);
        for violation in violations.iter().take(12) {
            eprintln!(
                "  violation net {} ({}) vs {}: at [{:.3}, {:.3}] layer {} required {:.3} actual {:.3}",
                violation.net,
                board.nets[violation.net as usize].name,
                violation.other,
                violation.at[0],
                violation.at[1],
                violation.layer,
                violation.required,
                violation.actual
            );
        }
    }
    for violation in violations {
        let net = violation.net as usize;
        let own = match violation.segment {
            Some(segment) => realized.segment_wire[net].get(segment).copied(),
            None => nearest_via(&realized.routes[net], &realized.via_wire[net], violation.at),
        };
        bad.extend(own);
        if let Some((other_net, other_segment)) = violation.other_segment {
            bad.extend(realized.segment_wire[other_net as usize].get(other_segment).copied());
        }
    }
    for (wire, path) in topology.wires.iter().enumerate() {
        if !path.routed {
            bad.push(wire);
        }
    }
    bad.sort_unstable();
    bad.dedup();
    let vias = realized.routes.iter().map(|route| route.vias.len()).sum();
    let length = realized
        .routes
        .iter()
        .flat_map(|route| &route.segments)
        .map(|segment| distance(segment.start, segment.end))
        .sum();
    Attempt { routes: realized.routes, segment_wire: realized.segment_wire, via_wire: realized.via_wire, bad, vias, length }
}

fn nearest_via(route: &NetRoute, owners: &[usize], at: Point) -> Option<usize> {
    route
        .vias
        .iter()
        .enumerate()
        .min_by(|a, b| distance(a.1.at, at).total_cmp(&distance(b.1.at, at)))
        .and_then(|(index, _)| owners.get(index).copied())
}

/// Routes `board` topologically.
pub fn route(board: &Board, config: &Config) -> RoutingResult {
    let started = Instant::now();
    let index = mesh::Index::new(board);
    let mesh = mesh::Mesh::build(board, &index);
    let mut topology = Topology::new(board, &mesh);
    // Connections: every net's terminals joined along a minimum spanning
    // tree, pad to pad.
    let mut ends: Vec<(usize, usize)> = Vec::new();
    for (net, description) in board.nets.iter().enumerate() {
        let terminals = &description.terminals;
        if terminals.len() < 2 {
            continue;
        }
        let mut joined = vec![false; terminals.len()];
        joined[0] = true;
        for _ in 1..terminals.len() {
            let mut best = (f64::INFINITY, 0, 0);
            for (a, terminal) in terminals.iter().enumerate() {
                if !joined[a] {
                    continue;
                }
                for (b, other) in terminals.iter().enumerate() {
                    if joined[b] {
                        continue;
                    }
                    let d = distance(terminal.anchor, other.anchor);
                    if d < best.0 {
                        best = (d, a, b);
                    }
                }
            }
            let (_, a, b) = best;
            joined[b] = true;
            if terminals[a].pad == terminals[b].pad {
                continue;
            }
            let (Some(from_face), Some(to_face)) = (mesh.locate(terminals[a].anchor), mesh.locate(terminals[b].anchor)) else {
                continue;
            };
            topology.wires.push(Wire {
                net: net as u32,
                class: description.class,
                from: terminals[a].anchor,
                to: terminals[b].anchor,
                terminals: [a, b],
                faces: Vec::new(),
                edges: Vec::new(),
                routed: false,
            });
            ends.push((from_face, to_face));
        }
    }
    // TopoR's order: the widest nets first, then the shortest connections.
    let mut order: Vec<usize> = (0..topology.wires.len()).collect();
    order.sort_by(|&a, &b| {
        let width = |wire: usize| board.classes[topology.wires[wire].class].trace_width;
        let span = |wire: usize| distance(topology.wires[wire].from, topology.wires[wire].to);
        width(b).total_cmp(&width(a)).then(span(a).total_cmp(&span(b)))
    });
    for &wire in &order {
        topology.route(board, &mesh, wire, ends[wire].0, ends[wire].1, config.weights);
    }
    if config.verbose {
        eprintln!(
            "topological: {} vertices, {} faces, {} wires routed in {:.2}s, {} crossings",
            mesh.points.len(),
            mesh.faces.len(),
            topology.wires.iter().filter(|wire| wire.routed).count(),
            started.elapsed().as_secs_f64(),
            topology.crossing_pairs(&mesh).len()
        );
    }

    let mut random = Random(config.seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut best: Option<Attempt> = None;
    let mut iterations = 0;
    for round in 0..=config.iterations {
        iterations = round;
        let attempt = realize_all(board, &mesh, &topology, config.seed.wrapping_add(round as u64));
        if config.verbose {
            eprintln!(
                "round {round}: {} wires in violation, {} vias, {:.1} mm, {:.2}s",
                attempt.bad.len(),
                attempt.vias,
                attempt.length,
                started.elapsed().as_secs_f64()
            );
        }
        let better = best.as_ref().is_none_or(|known| {
            (attempt.bad.len(), attempt.vias, attempt.length) < (known.bad.len(), known.vias, known.length)
        });
        let bad = attempt.bad.clone();
        if better {
            best = Some(attempt);
        }
        if bad.is_empty() || round == config.iterations || started.elapsed().as_secs_f64() > config.seconds {
            break;
        }
        // Rip up the offenders, remember where they were, and reroute them
        // with weights oscillated around the configured ones.
        for &wire in &bad {
            for &edge in &topology.wires[wire].edges {
                topology.history[edge] += 1.0;
            }
            for edge in topology.overflowing(board) {
                topology.history[edge] += 0.5;
            }
            topology.rip_up(wire);
        }
        let weights = Weights {
            crossing: config.weights.crossing * (0.5 + random.unit()),
            overflow: config.weights.overflow * (0.5 + random.unit()),
            history: config.weights.history * (0.5 + random.unit()),
        };
        let mut again = bad.clone();
        // Shuffle, so the same net does not always go first.
        for index in (1..again.len()).rev() {
            let other = (random.next() % (index as u64 + 1)) as usize;
            again.swap(index, other);
        }
        for wire in again {
            topology.route(board, &mesh, wire, ends[wire].0, ends[wire].1, weights);
        }
    }

    // The best board, without the wires still in violation: drop them one
    // round at a time until the exact verifier is satisfied.
    let best = best.expect("at least one round");
    let mut dropped: Vec<usize> = best.bad.clone();
    let mut routes;
    loop {
        routes = keep(&best, &dropped);
        let violations = verify(board, &routes.0);
        if violations.is_empty() {
            break;
        }
        let before = dropped.len();
        for violation in violations {
            let net = violation.net as usize;
            let own = match violation.segment {
                Some(segment) => routes.1[net].get(segment).copied(),
                None => nearest_via(&routes.0[net], &routes.2[net], violation.at),
            };
            dropped.extend(own);
        }
        dropped.sort_unstable();
        dropped.dedup();
        if dropped.len() == before {
            break;
        }
    }
    let mut status: Vec<NetStatus> = board
        .nets
        .iter()
        .map(|net| if net.terminals.len() < 2 { NetStatus::Trivial } else { NetStatus::Routed })
        .collect();
    for (wire, path) in topology.wires.iter().enumerate() {
        if dropped.contains(&wire) || !path.routed {
            let entry = &mut status[path.net as usize];
            *entry = match entry {
                NetStatus::Partial { unconnected_terminals } => NetStatus::Partial { unconnected_terminals: *unconnected_terminals + 1 },
                _ => NetStatus::Partial { unconnected_terminals: 1 },
            };
        }
    }
    if config.verbose {
        eprintln!(
            "topological: {} of {} wires kept, {:.2}s",
            topology.wires.len() - dropped.len(),
            topology.wires.len(),
            started.elapsed().as_secs_f64()
        );
    }
    RoutingResult {
        grid: Grid { origin: [0.0, 0.0], pitch: 0.0, nx: 0, ny: 0 },
        congestion: Vec::new(),
        routes: routes.0,
        status,
        iterations,
        expansions: 0,
        searches: topology.wires.len() as u64,
        diagnostics: Diagnostics::default(),
    }
}

/// The attempt's copper without the dropped wires, with owners.
fn keep(attempt: &Attempt, dropped: &[usize]) -> (Vec<NetRoute>, Vec<Vec<usize>>, Vec<Vec<usize>>) {
    let mut routes = Vec::with_capacity(attempt.routes.len());
    let mut segment_wire = Vec::with_capacity(attempt.routes.len());
    let mut via_wire = Vec::with_capacity(attempt.routes.len());
    for (net, route) in attempt.routes.iter().enumerate() {
        let mut kept = NetRoute::default();
        let mut owners = Vec::new();
        for (index, segment) in route.segments.iter().enumerate() {
            let wire = attempt.segment_wire[net][index];
            if !dropped.contains(&wire) {
                kept.segments.push(segment.clone());
                owners.push(wire);
            }
        }
        let mut via_owners = Vec::new();
        for (index, via) in route.vias.iter().enumerate() {
            let wire = attempt.via_wire[net][index];
            if !dropped.contains(&wire) {
                kept.vias.push(via.clone());
                via_owners.push(wire);
            }
        }
        routes.push(kept);
        segment_wire.push(owners);
        via_wire.push(via_owners);
    }
    (routes, segment_wire, via_wire)
}
