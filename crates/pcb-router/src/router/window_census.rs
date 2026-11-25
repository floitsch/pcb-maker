// Copyright (C) 2026 Florian Loitsch.

//! The window census, lab only (pcb-kicad-lab's `census-kicad-window`,
//! docs/experiments/window-census.md): the lattice of a small region as
//! the router's hard search sees it, for an offline joint solve of every
//! net with copper there. Nothing here changes a route.
//!
//! Per net: the nodes its static maps allow (`trace_allowed` of the class
//! at the node, the neck class in its neck zones), the layer steps neither
//! end's blocked edges forbid, the through vias `via_allowed` lets stand,
//! each terminal's nodes in the region (escape nodes with their stubs) and,
//! per connection the caller asks for, the nodes within a detour budget.
//! Per element (a node, a diagonal step, a via, an escape stub): the
//! occupancy cells it would stamp (`footprint_of`, the router's own
//! marking) and the cells a search reads to take it, both as keys of
//! (map, cell). Two elements of different nets conflict exactly when one
//! stamps a key the other reads: the hard search's test. Keys no other net
//! reads or stamps are left out.

use super::*;
use std::collections::{BinaryHeap, HashMap as Map, HashSet as Set};

/// One terminal's way in: a node of the net (an index into
/// `CensusNet::nodes`) and the escape stub behind it, if any (an index
/// into `CensusNet::escapes`).
pub struct CensusAccess {
    pub node: usize,
    pub escape: Option<usize>,
}

/// A net to census: the sets of its terminals that must end joined, and
/// its detour budget.
pub struct CensusRequest {
    pub net: NetId,
    pub requirements: Vec<Vec<usize>>,
    pub limits: CensusLimits,
}

/// The detour budget: a connection of length `d` (in lattice steps, a via
/// `via_steps`) may use the nodes on paths up to
/// `max(slack_steps, slack_ratio * d)` longer.
#[derive(Clone, Copy)]
pub struct CensusLimits {
    pub slack_steps: f64,
    pub slack_ratio: f64,
    pub via_steps: f64,
    /// A layer step is allowed when the search may take it from either end
    /// (else only when it may from both, the default).
    pub either_end: bool,
}

pub struct CensusEscape {
    pub terminal: usize,
    pub node: usize,
    pub width: f64,
    pub polyline: Vec<crate::geometry::Point>,
}

/// Two terminals to join and the nodes the budget allows them.
pub struct CensusConnection {
    pub a: usize,
    pub b: usize,
    pub distance: f64,
    pub nodes: Vec<usize>,
}

/// An element and its keys. `kind`: 0 node, 1 diagonal step (an index
/// into `edges`), 2 via (into `vias`), 3 escape stub (into `escapes`).
pub struct CensusElement {
    pub kind: u8,
    pub index: usize,
    pub stamps: Vec<u32>,
    pub reads: Vec<u32>,
}

pub struct CensusNet {
    pub net: NetId,
    pub class: usize,
    /// The class's via diameter and drill.
    pub via: [f64; 2],
    /// (layer, x, y) on the lattice.
    pub nodes: Vec<(usize, usize, usize)>,
    /// The track width at each node (the neck width in a neck zone).
    pub widths: Vec<f64>,
    /// Layer steps between nodes, and whether each is diagonal.
    pub edges: Vec<(usize, usize, bool)>,
    /// Through vias: the cell and the net's nodes there, a layer each.
    pub vias: Vec<(usize, Vec<usize>)>,
    /// Per terminal of the requirements: its ways in within the region.
    pub terminals: Vec<(usize, Vec<CensusAccess>)>,
    pub escapes: Vec<CensusEscape>,
    pub connections: Vec<CensusConnection>,
    /// Terminal pairs of a requirement that no path of the net's static
    /// lattice joins inside the region.
    pub separated: Vec<(usize, usize)>,
    pub elements: Vec<CensusElement>,
    /// Pairs of vias (indices into `vias`) closer than the class's via
    /// spacing (`mark_own_vias`).
    pub own_via_conflicts: Vec<(usize, usize)>,
}

pub struct CensusModel {
    /// The region's inclusive node range.
    pub range: (usize, usize, usize, usize),
    pub layers: usize,
    pub pitch: f64,
    pub warped: bool,
    pub nets: Vec<CensusNet>,
    pub keys: usize,
}

/// A node's index in the region's dense table.
struct Region {
    range: (usize, usize, usize, usize),
    width: usize,
    height: usize,
}

impl Region {
    fn slot(&self, layer: usize, x: usize, y: usize) -> Option<usize> {
        let (x0, y0, x1, y1) = self.range;
        (x >= x0 && x <= x1 && y >= y0 && y <= y1)
            .then(|| (layer * self.height + (y - y0)) * self.width + (x - x0))
    }
}

/// Dijkstra over one net's nodes from `sources`.
fn distances(adjacency: &[Vec<(usize, f64)>], sources: &[usize]) -> Vec<f64> {
    #[derive(PartialEq)]
    struct Item(f64, usize);
    impl Eq for Item {}
    impl PartialOrd for Item {
        fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
            Some(self.cmp(other))
        }
    }
    impl Ord for Item {
        fn cmp(&self, other: &Self) -> std::cmp::Ordering {
            other.0.total_cmp(&self.0).then(other.1.cmp(&self.1))
        }
    }
    let mut best = vec![f64::INFINITY; adjacency.len()];
    let mut heap = BinaryHeap::new();
    for &source in sources {
        best[source] = 0.0;
        heap.push(Item(0.0, source));
    }
    while let Some(Item(cost, node)) = heap.pop() {
        if cost > best[node] {
            continue;
        }
        for &(next, step) in &adjacency[node] {
            let total = cost + step;
            if total < best[next] {
                best[next] = total;
                heap.push(Item(total, next));
            }
        }
    }
    best
}

fn key(map: u32, cell: u32) -> u64 {
    ((map as u64) << 32) | cell as u64
}

impl Router {
    /// The census of `requests` in `region` (see the module's comment).
    /// An error for a board with via spans (only through vias are
    /// modelled).
    pub fn window_census(
        &self,
        region: crate::geometry::Aabb,
        requests: &[CensusRequest],
        max_nodes: usize,
    ) -> Result<CensusModel, String> {
        if let Some(spans) = &self.spans
            && requests
                .iter()
                .any(|request| spans.takes_spans(request.net))
        {
            return Err("via spans: the census models through vias only".into());
        }
        let range = self
            .grid
            .node_range(region)
            .ok_or("the region holds no lattice node")?;
        let layers = self.board.layer_count;
        let shape = Region {
            range,
            width: range.2 - range.0 + 1,
            height: range.3 - range.1 + 1,
        };
        let mut nets: Vec<CensusNet> = requests
            .iter()
            .map(|request| self.census_net(&shape, request))
            .collect();
        // Each node an element costs a footprint of a few hundred keys: a
        // window past `max_nodes` (the connections' nodes, all nets) is
        // too big for the offline solve.
        let used: usize = nets
            .iter()
            .map(|census| {
                let mut used = vec![false; census.nodes.len()];
                for connection in &census.connections {
                    for &node in &connection.nodes {
                        used[node] = true;
                    }
                }
                used.iter().filter(|used| **used).count()
            })
            .sum();
        if used > max_nodes {
            return Err(format!("window too big: {used} nodes in connections"));
        }
        let keys = self.census_elements(&mut nets);
        Ok(CensusModel {
            range,
            layers,
            pitch: self.grid.pitch,
            warped: self.grid.warp.is_some(),
            nets,
            keys,
        })
    }

    /// One net's lattice, terminals and connections (no elements yet).
    fn census_net(&self, shape: &Region, request: &CensusRequest) -> CensusNet {
        let net = request.net;
        let limits = request.limits;
        let layers = self.board.layer_count;
        let class = self.board.nets[net as usize].class;
        let state = &self.nets[net as usize];
        let (x0, y0, x1, y1) = shape.range;
        let own16 = crate::grid::owner16(net);
        let mut slots = vec![usize::MAX; shape.width * shape.height * layers];
        let mut nodes = Vec::new();
        let mut widths = Vec::new();
        for layer in 0..layers {
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let cell = self.grid.index(x, y);
                    let node_class = self.class_at(net, state, cell as u32);
                    if self.statics[node_class].trace_allowed(layer, cell, net) {
                        slots[shape.slot(layer, x, y).unwrap()] = nodes.len();
                        nodes.push((layer, x, y));
                        widths.push(self.width_on(node_class, layer));
                    }
                }
            }
        }
        // Layer steps: E, NE, N, NW from each node; neither end's blocked
        // edges may forbid it (the search tests the end it leaves).
        let mut edges = Vec::new();
        let mut adjacency: Vec<Vec<(usize, f64)>> = vec![Vec::new(); nodes.len()];
        for (index, &(layer, x, y)) in nodes.iter().enumerate() {
            for direction in 0..4 {
                let (dx, dy) = crate::grid::DIRECTIONS[direction];
                let (tx, ty) = (x as i64 + dx as i64, y as i64 + dy as i64);
                if tx < 0 || ty < 0 {
                    continue;
                }
                let Some(slot) = shape.slot(layer, tx as usize, ty as usize) else {
                    continue;
                };
                let other = slots[slot];
                if other == usize::MAX {
                    continue;
                }
                let blocked = |at: usize, way: usize| {
                    let cell = self.grid.index(nodes[at].1, nodes[at].2);
                    let statics = &self.statics[self.class_at(net, state, cell as u32)];
                    let (block, owner) = statics.edges_at(layer, cell);
                    block & (1 << way) != 0 && owner != own16
                };
                let (from, to) = (blocked(index, direction), blocked(other, direction + 4));
                if (from && to) || (!limits.either_end && (from || to)) {
                    continue;
                }
                let diagonal = dx != 0 && dy != 0;
                let cost = if diagonal {
                    std::f64::consts::SQRT_2
                } else {
                    1.0
                };
                edges.push((index, other, diagonal));
                adjacency[index].push((other, cost));
                adjacency[other].push((index, cost));
            }
        }
        // Through vias.
        let mut vias = Vec::new();
        if layers > 1 {
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let cell = self.grid.index(x, y);
                    let here: Vec<usize> = (0..layers)
                        .map(|layer| slots[shape.slot(layer, x, y).unwrap()])
                        .filter(|slot| *slot != usize::MAX)
                        .collect();
                    if here.len() < 2 || !self.statics[class].via_allowed(cell, net) {
                        continue;
                    }
                    for (i, &a) in here.iter().enumerate() {
                        for &b in &here[i + 1..] {
                            adjacency[a].push((b, limits.via_steps));
                            adjacency[b].push((a, limits.via_steps));
                        }
                    }
                    vias.push((cell, here));
                }
            }
        }
        // Terminals and their escapes.
        let mut terminals = Vec::new();
        let mut escapes = Vec::new();
        let mut wanted: Vec<usize> = request.requirements.iter().flatten().copied().collect();
        wanted.sort_unstable();
        wanted.dedup();
        for terminal in wanted {
            let mut access = Vec::new();
            for node in state.terminal_nodes.get(terminal).into_iter().flatten() {
                let (x, y) = self.grid.xy(node.cell as usize);
                let Some(slot) = shape.slot(node.layer as usize, x, y) else {
                    continue;
                };
                let index = slots[slot];
                if index == usize::MAX {
                    continue;
                }
                let escape = state.escapes.get(node).map(|(_, width, polyline)| {
                    escapes.push(CensusEscape {
                        terminal,
                        node: index,
                        width: *width,
                        polyline: polyline.clone(),
                    });
                    escapes.len() - 1
                });
                access.push(CensusAccess {
                    node: index,
                    escape,
                });
            }
            terminals.push((terminal, access));
        }
        // Connections: per requirement, a spanning tree of its terminals
        // by lattice distance; each with the nodes its budget allows.
        let mut from: Map<usize, Vec<f64>> = Map::new();
        for (terminal, access) in &terminals {
            if !access.is_empty() {
                let sources: Vec<usize> = access.iter().map(|way| way.node).collect();
                from.insert(*terminal, distances(&adjacency, &sources));
            }
        }
        let reach = |a: usize, b: usize| -> f64 {
            let (Some(distance), Some((_, access))) = (
                from.get(&a),
                terminals.iter().find(|(terminal, _)| *terminal == b),
            ) else {
                return f64::INFINITY;
            };
            access
                .iter()
                .map(|way| distance[way.node])
                .fold(f64::INFINITY, f64::min)
        };
        let mut connections = Vec::new();
        let mut separated = Vec::new();
        for requirement in &request.requirements {
            // Prim's tree over the requirement's terminals.
            let mut inside = vec![requirement[0]];
            let mut left: Vec<usize> = requirement[1..].to_vec();
            while !left.is_empty() {
                let mut best: Option<(f64, usize, usize)> = None;
                for &a in &inside {
                    for (position, &b) in left.iter().enumerate() {
                        let distance = reach(a, b);
                        if best.is_none_or(|(least, _, _)| distance < least) {
                            best = Some((distance, a, position));
                        }
                    }
                }
                let (distance, a, position) = best.expect("terminals left");
                let b = left.remove(position);
                inside.push(b);
                if !distance.is_finite() {
                    separated.push((a, b));
                    continue;
                }
                let slack = limits.slack_steps.max(limits.slack_ratio * distance);
                let (da, db) = (&from[&a], &from[&b]);
                let allowed: Vec<usize> = (0..nodes.len())
                    .filter(|&node| da[node] + db[node] <= distance + slack + 1.0e-9)
                    .collect();
                connections.push(CensusConnection {
                    a,
                    b,
                    distance,
                    nodes: allowed,
                });
            }
        }
        CensusNet {
            net,
            class,
            via: [
                self.board.classes[class].via_diameter,
                self.board.classes[class].via_drill,
            ],
            nodes,
            widths,
            edges,
            vias,
            terminals,
            escapes,
            connections,
            separated,
            elements: Vec::new(),
            own_via_conflicts: Vec::new(),
        }
    }

    /// The elements of every net (only where its connections may go) and
    /// their keys, shared between the nets; returns the number of keys.
    fn census_elements(&self, nets: &mut [CensusNet]) -> usize {
        let layers = self.board.layer_count;
        let nx = self.grid.nx;
        let mut marks = Vec::new();
        let no_escapes: HashMap<Node, Escape> = HashMap::default();
        let lone = |layer: usize, cell: usize| Node {
            layer: layer as u8,
            cell: cell as u32,
        };
        // Per net, its elements with raw (map, cell) stamps and reads.
        type Raw = (u8, usize, Vec<u64>, Vec<u64>);
        let mut raw: Vec<Vec<Raw>> = Vec::new();
        for census in nets.iter() {
            let net = census.net;
            let class = census.class;
            let state = &self.nets[net as usize];
            let mut used = vec![false; census.nodes.len()];
            for connection in &census.connections {
                for &node in &connection.nodes {
                    used[node] = true;
                }
            }
            for (_, access) in &census.terminals {
                for way in access {
                    used[way.node] = true;
                }
            }
            let cell_of = |node: usize| {
                let (_, x, y) = census.nodes[node];
                self.grid.index(x, y)
            };
            let mut node_stamps: Map<usize, Set<u64>> = Map::new();
            let mut elements: Vec<Raw> = Vec::new();
            for (node, _) in used.iter().enumerate().filter(|(_, used)| **used) {
                let (layer, _, _) = census.nodes[node];
                let cell = cell_of(node);
                let stamps: Set<u64> = self
                    .footprint_of(
                        &mut marks,
                        net,
                        &[Branch {
                            nodes: vec![lone(layer, cell)],
                            start_terminal: NO_TERMINAL,
                            end_terminal: NO_TERMINAL,
                        }],
                        &no_escapes,
                    )
                    .into_iter()
                    .map(|(map, cell)| key(map, cell))
                    .collect();
                let node_class = self.class_at(net, state, cell as u32);
                let reads = vec![key(self.map_index(node_class, layer) as u32, cell as u32)];
                elements.push((0, node, stamps.iter().copied().collect(), reads));
                node_stamps.insert(node, stamps);
            }
            let minus = |list: Vec<(u32, u32)>, parts: &[&Set<u64>]| -> Vec<u64> {
                list.into_iter()
                    .map(|(map, cell)| key(map, cell))
                    .filter(|stamp| parts.iter().all(|part| !part.contains(stamp)))
                    .collect()
            };
            for (index, &(a, b, diagonal)) in census.edges.iter().enumerate() {
                if !diagonal || !used[a] || !used[b] {
                    continue;
                }
                let (layer, ax, ay) = census.nodes[a];
                let (_, bx, by) = census.nodes[b];
                let (ca, cb) = (cell_of(a), cell_of(b));
                let stamps = minus(
                    self.footprint_of(
                        &mut marks,
                        net,
                        &[Branch {
                            nodes: vec![lone(layer, ca), lone(layer, cb)],
                            start_terminal: NO_TERMINAL,
                            end_terminal: NO_TERMINAL,
                        }],
                        &no_escapes,
                    ),
                    &[&node_stamps[&a], &node_stamps[&b]],
                );
                // The step read either way: from the end with the smaller
                // x, the start is that end (`layer_step`).
                let (dx, dy) = (bx as i64 - ax as i64, by as i64 - ay as i64);
                let (start, orientation) = if dx > 0 {
                    (ca, (dy < 0) as usize)
                } else {
                    (cb, (dy > 0) as usize)
                };
                let mut reads = Vec::new();
                for end in [ca, cb] {
                    let target_class = self.class_at(net, state, end as u32);
                    reads.push(key(
                        self.diagonal_map(target_class, layer, orientation) as u32,
                        start as u32,
                    ));
                }
                reads.sort_unstable();
                reads.dedup();
                elements.push((1, index, stamps, reads));
            }
            let via_map = self.map_index(class, layers) as u32;
            for (index, (cell, here)) in census.vias.iter().enumerate() {
                if here.iter().filter(|node| used[**node]).count() < 2 {
                    continue;
                }
                let (first, second) = (census.nodes[here[0]].0, census.nodes[here[1]].0);
                let stamps = minus(
                    self.footprint_of(
                        &mut marks,
                        net,
                        &[Branch {
                            nodes: vec![lone(first, *cell), lone(second, *cell)],
                            start_terminal: NO_TERMINAL,
                            end_terminal: NO_TERMINAL,
                        }],
                        &no_escapes,
                    ),
                    &[
                        node_stamps.get(&here[0]).unwrap_or(&Set::new()),
                        node_stamps.get(&here[1]).unwrap_or(&Set::new()),
                    ],
                );
                elements.push((2, index, stamps, vec![key(via_map, *cell as u32)]));
            }
            for (index, escape) in census.escapes.iter().enumerate() {
                let (layer, _, _) = census.nodes[escape.node];
                let cell = cell_of(escape.node);
                let end = lone(layer, cell);
                let Some(stub) = state.escapes.get(&end) else {
                    continue;
                };
                let mut escapes: HashMap<Node, Escape> = HashMap::default();
                escapes.insert(end, stub.clone());
                let stamps = minus(
                    self.footprint_of(
                        &mut marks,
                        net,
                        &[Branch {
                            nodes: vec![end],
                            start_terminal: NO_TERMINAL,
                            end_terminal: NO_TERMINAL,
                        }],
                        &escapes,
                    ),
                    &[node_stamps.get(&escape.node).unwrap_or(&Set::new())],
                );
                let stub_class = match self.neck_of[class] {
                    Some(neck) if stub.1 < self.width_on(class, layer) - 1.0e-9 => neck,
                    _ => class,
                };
                let map = self.map_index(stub_class, layer) as u32;
                let reads = stub.0.iter().map(|cell| key(map, *cell)).collect();
                elements.push((3, index, stamps, reads));
            }
            raw.push(elements);
        }
        // Keys read and stamped by different nets.
        let mut readers: Map<u64, Vec<usize>> = Map::new();
        let mut stampers: Map<u64, Vec<usize>> = Map::new();
        for (which, elements) in raw.iter().enumerate() {
            for (_, _, stamps, reads) in elements {
                for read in reads {
                    let list = readers.entry(*read).or_default();
                    if list.last() != Some(&which) {
                        list.push(which);
                    }
                }
                for stamp in stamps {
                    let list = stampers.entry(*stamp).or_default();
                    if list.last() != Some(&which) {
                        list.push(which);
                    }
                }
            }
        }
        let shared = |key: &u64| -> bool {
            match (readers.get(key), stampers.get(key)) {
                (Some(read), Some(stamped)) => read
                    .iter()
                    .any(|reader| stamped.iter().any(|stamper| stamper != reader)),
                _ => false,
            }
        };
        let mut ids: Map<u64, u32> = Map::new();
        for (census, elements) in nets.iter_mut().zip(raw) {
            for (kind, index, stamps, reads) in elements {
                let mut id = |list: Vec<u64>| -> Vec<u32> {
                    let mut out: Vec<u32> = list
                        .into_iter()
                        .filter(|key| shared(key))
                        .map(|key| {
                            let next = ids.len() as u32;
                            *ids.entry(key).or_insert(next)
                        })
                        .collect();
                    out.sort_unstable();
                    out.dedup();
                    out
                };
                let stamps = id(stamps);
                let reads = id(reads);
                census.elements.push(CensusElement {
                    kind,
                    index,
                    stamps,
                    reads,
                });
            }
            // The net's own vias keep their spacing (`mark_own_vias`).
            let stamps = &self.stamps[census.class][census.class];
            let at: Map<usize, usize> = census
                .vias
                .iter()
                .enumerate()
                .map(|(index, (cell, _))| (*cell, index))
                .collect();
            for (index, (cell, _)) in census.vias.iter().enumerate() {
                let (x, y) = (cell % nx, cell / nx);
                let mut near = Vec::new();
                if self.grid.warp.is_some() && !self.grid.uniform_around(x, y, stamps.fresh.reach) {
                    self.grid
                        .disc_cells(x, y, stamps.radii.via_to_via, |cell| near.push(cell));
                } else {
                    for (dx, dy) in &stamps.via_to_via {
                        let (tx, ty) = (x as i64 + *dx as i64, y as i64 + *dy as i64);
                        if tx >= 0 && ty >= 0 && (tx as usize) < nx && (ty as usize) < self.grid.ny
                        {
                            near.push(ty as usize * nx + tx as usize);
                        }
                    }
                }
                for other in near {
                    if let Some(&other) = at.get(&other)
                        && other > index
                    {
                        census.own_via_conflicts.push((index, other));
                    }
                }
            }
        }
        ids.len()
    }
}
