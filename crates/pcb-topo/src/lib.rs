// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! A topological router in the manner of TopoR (Luzin & Polubasov; see
//! docs/reviews/2026-09-29-topor.md and docs/topological.md).
//!
//! 1. One constrained Delaunay triangulation of all obstacles, shared by
//!    the layers, with free points in open areas ([`mesh`]).
//! 2. Every connection routed topologically, crossings allowed, widest
//!    nets first, then the shortest: all connections at once, rules
//!    violated or not ([`topo`], planar mode).
//! 3. Layers assigned afterwards: crossing wires part, a change of layer is
//!    a via ([`layers`]).
//! 4. Geometry: each wire pulled tight around the obstacles as tangents
//!    and arcs ([`realize`], [`taut`]).
//! 5. Rip-up and reroute of the wires in trouble (same-layer crossings, no
//!    room, violations the exact verifier finds), now layer by layer, with
//!    congestion history and oscillating weights; layers reassigned; the
//!    best board kept. What still violates is taken out and reported open,
//!    so the copper that remains is legal.
//!
//! [`tighten`] applies step 4 to copper from any router.

pub mod layers;
pub mod mesh;
pub mod picture;
pub mod realize;
pub mod taut;
pub mod tighten;
pub mod topo;

use pcb_router::grid::Grid;
use pcb_router::{Board, Diagnostics, NetRoute, NetStatus, Point, RoutingResult, verify};
use realize::{Piece, PlacedVia};
use std::time::Instant;
use topo::{Mode, Topology, Weights, Wire};

#[derive(Clone, Debug)]
pub struct Config {
    /// Rip-up rounds after the first routing.
    pub iterations: usize,
    /// Wall-clock budget for the rounds.
    pub seconds: f64,
    pub seed: u64,
    pub verbose: bool,
    pub weights: Weights,
    pub costs: layers::Costs,
    /// Spacing of the free points in open areas (0: none).
    pub spacing: f64,
    /// Price of a via, in millimetres, while improving a legal board.
    pub improve_via: f64,
    /// Passes over all wires while improving.
    pub improve_passes: usize,
    /// Route the first time layer by layer (each wire knowing the layers
    /// of those before it) instead of TopoR's way (all wires across, layers
    /// assigned afterwards).
    pub layered_start: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            iterations: 30,
            seconds: 300.0,
            seed: 1,
            verbose: false,
            weights: Weights { crossing: 2.0, via: 8.0, overflow: 50.0, history: 5.0, against: 1.6 },
            costs: layers::Costs { via: 8.0, overflow: 50.0, against: 1.6 },
            spacing: 2.0,
            improve_via: 25.0,
            improve_passes: 4,
            layered_start: false,
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

/// Every routed wire cut into per-layer pieces at its vias, each embedded
/// through its places on the edges it crosses.
fn layout(board: &Board, mesh: &mesh::Mesh, topology: &Topology) -> (Vec<Piece>, Vec<PlacedVia>) {
    let mut pieces = Vec::new();
    let mut vias = Vec::new();
    for (wire, path) in topology.wires.iter().enumerate() {
        if !path.routed || path.layers.len() != path.faces.len() {
            continue;
        }
        let width = board.classes[path.class].trace_width;
        let terminals = &board.nets[path.net as usize].terminals;
        let piece = |layer: usize, at: Point, pad: Option<usize>| Piece { wire, net: path.net, class: path.class, layer, width, points: vec![at], pads: [pad, None] };
        let mut current = piece(path.layers[0], path.from, Some(terminals[path.terminals[0]].pad));
        for (step, &portal) in path.portals.iter().enumerate() {
            let point = topology.place(board, mesh, wire, portal);
            current.points.push(point);
            if path.via_at(step) {
                pieces.push(current);
                vias.push(PlacedVia { wire, net: path.net, class: path.class, at: point });
                current = piece(path.layers[step + 1], point, None);
            }
        }
        current.points.push(path.to);
        current.pads[1] = Some(terminals[path.terminals[1]].pad);
        pieces.push(current);
    }
    (pieces, vias)
}

/// One realized state of the board.
struct Attempt {
    routes: Vec<NetRoute>,
    segment_wire: Vec<Vec<usize>>,
    via_wire: Vec<Vec<usize>>,
    /// Wires in trouble: not routed, same-layer crossings, no geometry, or
    /// violations.
    bad: Vec<usize>,
    vias: usize,
    length: f64,
    /// What the trouble was: unrouted, layer conflicts, overflowing,
    /// pieces without geometry, violations.
    trouble: [usize; 5],
    /// Where the geometry did not fit: wire, layer, place and millimetres
    /// missing.
    hotspots: Vec<(usize, usize, Point, f64)>,
}

fn realize_all(board: &Board, mesh: &mesh::Mesh, topology: &Topology, picture: Option<&str>) -> Attempt {
    let (pieces, vias) = layout(board, mesh, topology);
    let realized = realize::realize(board, &pieces, &vias);
    let violations = verify(board, &realized.routes);
    let mut bad: Vec<usize> = realized.failed.iter().map(|(piece, _)| pieces[*piece].wire).collect();
    let troubled = layers::troubled(board, mesh, topology);
    let (overflowing, overflowed) = topology.overflowing(board, mesh);
    if std::env::var_os("PCB_TOPO_DEBUG").is_some() {
        let free = |vertex: usize| mesh.via_site[vertex] && topology.occupant[vertex].is_none();
        let mut kinds = [0usize; 3];
        for &edge in &overflowed {
            kinds[mesh.edges[edge].iter().filter(|&&vertex| free(vertex)).count()] += 1;
        }
        eprintln!("  overflowing edges by free ends (0, 1, 2): {kinds:?}");
    }
    let unrouted = topology.wires.iter().filter(|wire| !wire.routed).count();
    let trouble = [unrouted, troubled.len(), overflowing.len(), realized.failed.len(), violations.len()];
    bad.extend(troubled);
    bad.extend(overflowing);
    if std::env::var_os("PCB_TOPO_DEBUG").is_some() {
        // What kinds of failure, in short.
        let mut kinds: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        for (_, failed) in &realized.failed {
            let reason = &failed.reason;
            let kind = if reason.starts_with("squeezed") {
                let vias = reason.matches("via of").count();
                let stacked = reason.contains("(r 1.9") || reason.contains("(r 2.") || reason.contains("(r 3.");
                format!("squeezed, {vias} vias{}", if stacked { ", nested" } else { "" })
            } else {
                reason.split(" (").next().unwrap_or(reason).split(" near").next().unwrap_or(reason).chars().take(40).collect::<String>()
            };
            *kinds.entry(kind).or_default() += 1;
        }
        for violation in &violations {
            let kind = if violation.other.starts_with("via of") {
                "violation: track-via".to_string()
            } else if violation.other_segment.is_some() {
                "violation: track-track".to_string()
            } else if violation.segment.is_none() {
                "violation: via-obstacle".to_string()
            } else {
                "violation: track-obstacle".to_string()
            };
            *kinds.entry(kind).or_default() += 1;
        }
        eprintln!("  failures: {kinds:?}");
        for (piece, failed) in realized.failed.iter().take(10) {
            let piece = &pieces[*piece];
            eprintln!("  piece of wire {} ({}) on layer {}: {}", piece.wire, board.nets[piece.net as usize].name, piece.layer, failed.reason);
        }
        for violation in violations.iter().take(10) {
            eprintln!(
                "  violation {} vs {} at [{:.3}, {:.3}] layer {}: {:.3} < {:.3}",
                board.nets[violation.net as usize].name,
                violation.other,
                violation.at[0],
                violation.at[1],
                violation.layer,
                violation.actual,
                violation.required
            );
        }
    }
    if let Some(name) = picture.filter(|_| picture::directory().is_some()) {
        let mut drawing = picture::Picture::new(board);
        drawing.obstacles(board);
        drawing.mesh(mesh);
        drawing.wires(board, mesh, topology, &|wire, step| topology.wires[wire].layers.get(step).copied());
        for via in &vias {
            drawing.circle(via.at, 0.3, "#e8c040", "#806010", 0.03);
        }
        drawing.save(&format!("{name}-layers"));
        let mut drawing = picture::Picture::new(board);
        drawing.obstacles(board);
        drawing.copper(&realized.routes, &violations);
        drawing.save(&format!("{name}-copper"));
    }
    let mut hotspots: Vec<(usize, usize, Point, f64)> = realized
        .failed
        .iter()
        .filter_map(|(piece, failed)| failed.at.map(|at| (pieces[*piece].wire, pieces[*piece].layer, at, failed.deficit)))
        .collect();
    for violation in &violations {
        let net = violation.net as usize;
        let own = match violation.segment {
            Some(segment) => realized.segment_wire[net].get(segment).copied(),
            None => nearest_via(&realized.routes[net], &realized.via_wire[net], violation.at),
        };
        let deficit = violation.required - violation.actual;
        bad.extend(own);
        hotspots.extend(own.map(|wire| (wire, violation.layer, violation.at, deficit)));
        if let Some((other_net, other_segment)) = violation.other_segment {
            let other = realized.segment_wire[other_net as usize].get(other_segment).copied();
            bad.extend(other);
            hotspots.extend(other.map(|wire| (wire, violation.layer, violation.at, deficit)));
        }
    }
    for (wire, path) in topology.wires.iter().enumerate() {
        if !path.routed {
            bad.push(wire);
        }
    }
    bad.sort_unstable();
    bad.dedup();
    let length = realized.routes.iter().flat_map(|route| &route.segments).map(|segment| distance(segment.start, segment.end)).sum();
    Attempt { vias: vias.len(), routes: realized.routes, segment_wire: realized.segment_wire, via_wire: realized.via_wire, bad, length, trouble, hotspots }
}

fn nearest_via(route: &NetRoute, owners: &[usize], at: Point) -> Option<usize> {
    route
        .vias
        .iter()
        .enumerate()
        .min_by(|a, b| distance(a.1.at, at).total_cmp(&distance(b.1.at, at)))
        .and_then(|(index, _)| owners.get(index).copied())
}

/// The connections to make: each net's terminals joined along a minimum
/// spanning tree, pad to pad, with the faces of their ends.
fn connections(board: &Board, mesh: &mesh::Mesh) -> (Vec<Wire>, Vec<(usize, usize)>) {
    let mut wires = Vec::new();
    let mut ends = Vec::new();
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
            let from = mesh.locate(terminals[a].anchor, Some(terminals[a].pad));
            let to = mesh.locate(terminals[b].anchor, Some(terminals[b].pad));
            let (Some(from_face), Some(to_face)) = (from, to) else {
                continue;
            };
            wires.push(Wire {
                net: net as u32,
                class: description.class,
                from: terminals[a].anchor,
                to: terminals[b].anchor,
                terminals: [a, b],
                faces: Vec::new(),
                portals: Vec::new(),
                layers: Vec::new(),
                routed: false,
            });
            ends.push((from_face, to_face));
        }
    }
    (wires, ends)
}

/// Routes `board` topologically.
/// Everything the phases after the first routing share.
struct Run<'a> {
    board: &'a Board,
    mesh: &'a mesh::Mesh,
    config: &'a Config,
    ends: Vec<(usize, usize)>,
    started: Instant,
    random: Random,
    rounds: usize,
}

impl Run<'_> {
    fn terminal_layers(&self, topology: &Topology, wire: usize) -> (u32, u32) {
        let path = &topology.wires[wire];
        let terminals = &self.board.nets[path.net as usize].terminals;
        (terminals[path.terminals[0]].layers, terminals[path.terminals[1]].layers)
    }

    fn out_of_time(&self) -> bool {
        self.started.elapsed().as_secs_f64() > self.config.seconds
    }

    fn report(&self, label: &str, attempt: &Attempt) {
        if self.config.verbose {
            eprintln!(
                "{label}: {} wires in trouble ({} unrouted, {} in layer conflict, {} overflowing, {} pieces without geometry, {} violations), {} vias, {:.1} mm, {:.2}s",
                attempt.bad.len(),
                attempt.trouble[0],
                attempt.trouble[1],
                attempt.trouble[2],
                attempt.trouble[3],
                attempt.trouble[4],
                attempt.vias,
                attempt.length,
                self.started.elapsed().as_secs_f64()
            );
        }
    }

    /// Rounds of rip-up and reroute of the wires in trouble, layer by
    /// layer, with congestion history, a rising price of overflow and
    /// oscillating weights; layers reassigned after each. Returns the best
    /// state seen and its topology.
    fn repair(&mut self, topology: &mut Topology, rounds: usize, label: &str) -> (Attempt, Topology) {
        let (board, mesh) = (self.board, self.mesh);
        let mut best: Option<(Attempt, Topology)> = None;
        for round in 0..=rounds {
            // The model first: no overflow, no same-layer crossing.
            let left = self.negotiate(topology, 12);
            let attempt = realize_all(board, mesh, topology, Some(&format!("{label}{:02}", round + 1)));
            self.report(&format!("{label}round {round} (model: {left} wires left in overflow or conflict)"), &attempt);
            self.rounds += 1;
            let better = best.as_ref().is_none_or(|(known, _)| {
                (attempt.bad.len(), attempt.vias, attempt.length) < (known.bad.len(), known.vias, known.length)
            });
            let bad = attempt.bad.clone();
            let hotspots = attempt.hotspots.clone();
            if better {
                best = Some((attempt, topology.clone()));
            }
            if bad.is_empty() || round == rounds || self.out_of_time() {
                break;
            }
            // Where the geometry did not fit, the model had room it did not
            // have: take it off the edges the wire crosses there; reroute
            // those wires.
            for &(wire, layer, at, deficit) in &hotspots {
                topology.learn(mesh, wire, layer, at, deficit);
            }
            self.reroute(topology, &bad, 1.0);
        }
        best.expect("at least one round")
    }

    /// Rips up `wires` and routes them again, layer by layer, in random
    /// order, with overflow priced `present` times the configured weight.
    fn reroute(&mut self, topology: &mut Topology, wires: &[usize], present: f64) {
        let (board, mesh, config) = (self.board, self.mesh, self.config);
        for &wire in wires {
            if topology.wires[wire].routed {
                topology.rip_up(wire);
            }
        }
        let weights = Weights {
            crossing: config.weights.crossing * (0.5 + self.random.unit()),
            via: config.weights.via * (0.5 + self.random.unit()),
            overflow: config.weights.overflow * present,
            history: config.weights.history,
            against: config.weights.against,
        };
        let mut again = wires.to_vec();
        for index in (1..again.len()).rev() {
            let other = (self.random.next() % (index as u64 + 1)) as usize;
            again.swap(index, other);
        }
        for wire in again {
            let (start, end) = self.terminal_layers(topology, wire);
            let (from, to) = self.ends[wire];
            if !topology.route(board, mesh, wire, from, to, weights, Mode::Layered, start, end) {
                // No legal way on the layers as they are: route across and
                // let the layer assignment part the crossings.
                topology.route(board, mesh, wire, from, to, weights, Mode::Planar, start, end);
            }
        }
        layers::assign(board, mesh, topology, config.costs, config.seed.wrapping_add(self.rounds as u64));
        self.rounds += 1;
    }

    /// Negotiated congestion in the model (PathFinder): the wires that
    /// overflow an edge, cross on one layer or are not routed are rerouted
    /// with the price of overflow rising and the history of overflowed
    /// edges growing, until none is left or `iterations` run out. Returns
    /// how many are left.
    fn negotiate(&mut self, topology: &mut Topology, iterations: usize) -> usize {
        let (board, mesh) = (self.board, self.mesh);
        let mut present = 1.0;
        let mut left = 0;
        for _ in 0..iterations {
            let (mut troubled, overflowed) = topology.overflowing(board, mesh);
            troubled.extend(layers::troubled(board, mesh, topology));
            troubled.extend((0..topology.wires.len()).filter(|&wire| !topology.wires[wire].routed));
            troubled.sort_unstable();
            troubled.dedup();
            left = troubled.len();
            if troubled.is_empty() || self.out_of_time() {
                break;
            }
            for edge in overflowed {
                topology.history[edge] += 1.0;
            }
            self.reroute(topology, &troubled, present);
            present *= 1.6;
        }
        left
    }

    /// TopoR's optimization: every wire in turn ripped up and rerouted with
    /// vias priced high (and no overflow), kept if it gets cheaper; then the
    /// layers reassigned. Returns how many wires changed.
    fn improve(&mut self, topology: &mut Topology, pass: usize) -> usize {
        let (board, mesh, config) = (self.board, self.mesh, self.config);
        let weights = Weights { crossing: 0.5, via: config.improve_via, overflow: 0.0, history: 0.0, against: config.weights.against };
        let mut order: Vec<usize> = (0..topology.wires.len()).filter(|&wire| topology.wires[wire].routed).collect();
        for index in (1..order.len()).rev() {
            let other = (self.random.next() % (index as u64 + 1)) as usize;
            order.swap(index, other);
        }
        let mut changed = 0;
        for wire in order {
            let old = topology.cost(board, mesh, wire, config.improve_via);
            let saved = topology.save(wire);
            topology.rip_up(wire);
            let (start, end) = self.terminal_layers(topology, wire);
            let (from, to) = self.ends[wire];
            let routed = topology.route(board, mesh, wire, from, to, weights, Mode::Strict, start, end);
            if routed && topology.cost(board, mesh, wire, config.improve_via) + 1.0e-6 < old {
                changed += 1;
            } else {
                topology.restore(saved);
            }
        }
        layers::assign(board, mesh, topology, layers::Costs { via: config.improve_via, ..config.costs }, config.seed.wrapping_add(1000 + pass as u64));
        changed
    }
}

/// Routes `board` topologically.
pub fn route(board: &Board, config: &Config) -> RoutingResult {
    let started = Instant::now();
    let index = mesh::Index::new(board);
    let mesh = mesh::Mesh::build(board, &index, config.spacing);
    let mut topology = Topology::new(&mesh);
    let (wires, ends) = connections(board, &mesh);
    topology.wires = wires;
    let mut run = Run { board, mesh: &mesh, config, ends, started, random: Random(config.seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1), rounds: 0 };
    // TopoR's order: the widest nets first, then the shortest connections.
    let mut order: Vec<usize> = (0..topology.wires.len()).collect();
    order.sort_by(|&a, &b| {
        let width = |wire: usize| board.classes[topology.wires[wire].class].trace_width;
        let span = |wire: usize| distance(topology.wires[wire].from, topology.wires[wire].to);
        width(b).total_cmp(&width(a)).then(span(a).total_cmp(&span(b)))
    });
    for &wire in &order {
        let (start, end) = run.terminal_layers(&topology, wire);
        let (from, to) = run.ends[wire];
        let routed = config.layered_start && topology.route(board, &mesh, wire, from, to, config.weights, Mode::Layered, start, end);
        if !routed {
            topology.route(board, &mesh, wire, from, to, config.weights, Mode::Planar, start, end);
        }
    }
    if std::env::var_os("PCB_TOPO_DEBUG").is_some() {
        for (wire, path) in topology.wires.iter().enumerate() {
            if !path.routed {
                let terminals = &board.nets[path.net as usize].terminals;
                eprintln!("  unrouted wire {wire} ({}): {} to {}", board.nets[path.net as usize].name, terminals[path.terminals[0]].label, terminals[path.terminals[1]].label);
            }
        }
    }
    let crossings = topology.crossing_pairs(&mesh).len();
    if picture::directory().is_some() {
        let mut drawing = picture::Picture::new(board);
        drawing.obstacles(board);
        drawing.mesh(&mesh);
        drawing.wires(board, &mesh, &topology, &|_, _| None);
        drawing.save("00-topology");
    }
    let first_seconds = started.elapsed().as_secs_f64();
    let assigned = layers::assign(board, &mesh, &mut topology, config.costs, config.seed);
    if config.verbose {
        eprintln!(
            "topological: {} vertices, {} faces; {} of {} wires routed in {:.2}s with {} crossings; layers: {} vias, {} conflicts, {} infeasible ({:.2}s)",
            mesh.points.len(),
            mesh.faces.len(),
            topology.wires.iter().filter(|wire| wire.routed).count(),
            topology.wires.len(),
            first_seconds,
            crossings,
            assigned.vias,
            assigned.conflicts,
            assigned.infeasible,
            started.elapsed().as_secs_f64()
        );
    }

    let (mut best, mut best_topology) = run.repair(&mut topology, config.iterations, "");
    // Improve, repair what that broke, keep it if the board got better.
    let price = |attempt: &Attempt| (attempt.bad.len(), attempt.vias as f64 * config.improve_via + attempt.length);
    for pass in 0..config.improve_passes {
        if run.out_of_time() {
            break;
        }
        let mut topology = best_topology.clone();
        let changed = run.improve(&mut topology, pass);
        if changed == 0 {
            break;
        }
        let (attempt, repaired) = run.repair(&mut topology, 8, &format!("improve {pass} "));
        let better = price(&attempt).0 < price(&best).0 || (price(&attempt).0 == price(&best).0 && price(&attempt).1 < price(&best).1 - 1.0e-6);
        if config.verbose {
            eprintln!("improve {pass}: {changed} wires rerouted, {}", if better { "kept" } else { "dropped" });
        }
        if better {
            best = attempt;
            best_topology = repaired;
        }
    }
    let topology = best_topology;

    // The best board, without the wires still in trouble: drop them one
    // round at a time until the exact verifier is satisfied.
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
        if dropped.contains(&wire) {
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
        iterations: run.rounds,
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
