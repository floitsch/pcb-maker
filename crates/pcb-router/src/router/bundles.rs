// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Bus bundles (`Config::bus_bundles`, off; docs/experiments/bus-bundles.md):
//! groups of similar nets routed together along one planned corridor,
//! side by side, and broken up where they cannot follow it.
//!
//! 1. Detection: four or more signal nets of one rule class with two to
//!    twelve pads each that join exactly the same set of footprints (the
//!    address and data lines of memories and their controller), ordered by
//!    their pads' positions along the row of the first footprint.
//! 2. One corridor per bus on the tile graph: a tree over the footprints'
//!    pad tiles whose tiles cost more where the bus's width does not fit
//!    beside what is there, grown by half the bus's width.
//! 3. The members route first (after the pour nets), in their order,
//!    inside that corridor, with a price on every step that does not lie
//!    beside a neighbouring member's copper (the lane): the members keep
//!    side by side and take their bends together.
//! 4. The break-up: a member whose search finds no path inside the
//!    corridor, or that the negotiation keeps rerouting, routes as any
//!    other net from then on; the hard repair and the cleanup always do.

use super::*;

/// One bus: its members in order and the corridor they share.
#[derive(Clone, Debug)]
pub(super) struct Bus {
    pub(super) members: Vec<NetId>,
    /// Per tile: inside the bus's corridor.
    pub(super) corridor: Vec<bool>,
}

/// The lane of the routed bus member (cells beside its neighbours' copper),
/// a bit per (layer, cell) state, kept in the search's scratch.
#[derive(Default)]
pub(super) struct Lane {
    pub(super) bits: Vec<u64>,
    /// Words of `bits` set, to clear them again.
    pub(super) words: Vec<usize>,
    pub(super) active: bool,
    /// The factor on a trace step off the lane.
    pub(super) off: f32,
}

/// How many members on either side of a member make its lane.
const LANE_NEIGHBOURS: usize = 2;
/// Tiles the corridor is grown by beyond half the bus's width
/// (`PCB_ROUTER_BUS_MARGIN`): a thinner tree of tiles passes where the
/// lattice does not (video's designer placement: 188 of 195 members found
/// no path in their corridor at 0 more tiles, 52 at 3, none at 4).
const BUS_MARGIN: usize = 4;
/// Reroutes after which a member is broken out of its bundle.
const BUS_PATIENCE: usize = 8;

impl Router {
    /// Whether bus bundles are on for this router.
    pub(super) fn bundles_on(&self) -> bool {
        self.config.bus_bundles
    }

    /// Detects the buses and plans their corridors, once per router (the
    /// switch off: nothing).
    pub(super) fn ensure_buses(&mut self) {
        if !self.bundles_on() || self.buses_planned {
            return;
        }
        self.buses_planned = true;
        let started = std::time::Instant::now();
        let groups = self.detect_buses();
        self.bus_of = vec![(u32::MAX, 0); self.board.nets.len()];
        self.buses.clear();
        for members in groups {
            let Some(corridor) = self.plan_bus_corridor(&members) else {
                continue;
            };
            let index = self.buses.len() as u32;
            for (position, net) in members.iter().enumerate() {
                self.bus_of[*net as usize] = (index, position as u32);
            }
            self.buses.push(Bus { members, corridor });
        }
        if self.config.verbose {
            let sizes: Vec<usize> = self.buses.iter().map(|bus| bus.members.len()).collect();
            eprintln!(
                "bus bundles: {} buses, {} nets {:?}, {:.2}s",
                self.buses.len(),
                sizes.iter().sum::<usize>(),
                sizes,
                started.elapsed().as_secs_f64()
            );
        }
        if std::env::var_os("PCB_ROUTER_BUS_WHY").is_some() {
            for bus in &self.buses {
                let names: Vec<&str> = bus
                    .members
                    .iter()
                    .map(|net| self.board.nets[*net as usize].name.as_str())
                    .collect();
                let tiles = bus.corridor.iter().filter(|inside| **inside).count();
                eprintln!("bus: {} tiles {:?}", tiles, names);
            }
        }
    }

    /// The bus a net belongs to, with its position there.
    pub(super) fn bus_member(&self, net: NetId) -> Option<(usize, usize)> {
        let (bus, position) = *self.bus_of.get(net as usize)?;
        (bus != u32::MAX).then_some((bus as usize, position as usize))
    }

    /// Groups of at least four signal nets of one class with two to twelve
    /// pads that join the same set of (at least two) footprints, each in
    /// the order of its pads along the first footprint's row.
    fn detect_buses(&self) -> Vec<Vec<NetId>> {
        let footprint = |label: &str| {
            label
                .split_once('.')
                .map(|(reference, _)| reference.to_string())
        };
        let mut groups: std::collections::BTreeMap<(usize, Vec<String>), Vec<NetId>> =
            std::collections::BTreeMap::new();
        for (net, state) in self.nets.iter().enumerate() {
            let terminals = &self.board.nets[net].terminals;
            if !state.routable || !state.plane.is_empty() || !(2..=12).contains(&terminals.len()) {
                continue;
            }
            let mut set: Vec<String> = Vec::new();
            for terminal in terminals {
                let Some(reference) = footprint(&terminal.label) else {
                    set.clear();
                    break;
                };
                set.push(reference);
            }
            set.sort();
            set.dedup();
            if set.len() < 2 {
                continue;
            }
            groups
                .entry((self.board.nets[net].class, set))
                .or_default()
                .push(net as NetId);
        }
        let mut buses = Vec::new();
        for ((_, set), mut members) in groups {
            if members.len() < 4 {
                continue;
            }
            // The members' pads on the first footprint, along its longer
            // extent.
            let anchor = |net: NetId| {
                self.board.nets[net as usize]
                    .terminals
                    .iter()
                    .find(|terminal| footprint(&terminal.label).as_deref() == Some(set[0].as_str()))
                    .map(|terminal| terminal.anchor)
                    .unwrap_or([0.0, 0.0])
            };
            let points: Vec<[f64; 2]> = members.iter().map(|net| anchor(*net)).collect();
            let extent = |axis: usize| {
                let values = points.iter().map(|point| point[axis]);
                values.clone().fold(f64::MIN, f64::max) - values.fold(f64::MAX, f64::min)
            };
            let (major, minor) = if extent(0) >= extent(1) {
                (0, 1)
            } else {
                (1, 0)
            };
            members.sort_by(|a, b| {
                let (pa, pb) = (anchor(*a), anchor(*b));
                pa[major]
                    .total_cmp(&pb[major])
                    .then(pa[minor].total_cmp(&pb[minor]))
                    .then(a.cmp(b))
            });
            buses.push(members);
        }
        // The widest buses first; ties by their first member.
        buses.sort_by_key(|members| (std::cmp::Reverse(members.len()), members[0]));
        buses
    }

    /// The corridor of a bus on the tile graph: from the footprint with
    /// most member pads, a tree that reaches each other footprint's pad
    /// tiles in turn (Dijkstra from the tree so far), a tile costing more
    /// the less room the bus's width finds there beside what is there,
    /// grown by half the bus's width; the members' pad tiles with a tile
    /// around them are in it too.
    fn plan_bus_corridor(&self, members: &[NetId]) -> Option<Vec<bool>> {
        let class = self.board.nets[members[0] as usize].class;
        let rules = self.board.classes[class];
        let layers = self.board.layer_count;
        let tiles = self.tiles_x * self.tiles_y;
        let nx = self.grid.nx;
        let tile_of =
            |cell: u32| (cell as usize / nx / TILE) * self.tiles_x + (cell as usize % nx) / TILE;
        // The pad tiles per footprint.
        let mut by_footprint: std::collections::BTreeMap<String, Vec<usize>> =
            std::collections::BTreeMap::new();
        for net in members {
            let state = &self.nets[*net as usize];
            for (terminal, nodes) in self.board.nets[*net as usize]
                .terminals
                .iter()
                .zip(&state.terminal_nodes)
            {
                let reference = terminal
                    .label
                    .split_once('.')
                    .map(|(reference, _)| reference.to_string())
                    .unwrap_or_default();
                let list = by_footprint.entry(reference).or_default();
                list.extend(nodes.iter().map(|node| tile_of(node.cell)));
            }
        }
        let mut groups: Vec<Vec<usize>> = by_footprint
            .into_values()
            .map(|mut list| {
                list.sort_unstable();
                list.dedup();
                list
            })
            .filter(|list| !list.is_empty())
            .collect();
        if groups.len() < 2 {
            return None;
        }
        groups.sort_by_key(|list| std::cmp::Reverse(list.len()));
        // Room per tile over all layers, in cells, and the bus's demand:
        // its width across one tile on one layer.
        let room: Vec<f32> = (0..tiles)
            .map(|tile| {
                (0..layers)
                    .map(|layer| {
                        let routable = self.tile_routable[class * layers + layer][tile] as f32;
                        let claimed = self.tile_claimed[self.map_index(class, layer)][tile] as f32;
                        (routable - claimed).max(0.0)
                    })
                    .sum()
            })
            .collect();
        let width_mm = members.len() as f64 * (rules.trace_width + rules.clearance);
        let demand = (width_mm / self.grid.pitch) as f32 * TILE as f32;
        let tile_mm = (TILE as f64 * self.grid.pitch) as f32;
        let mut in_tree = vec![false; tiles];
        let mut endpoint = vec![false; tiles];
        for list in &groups {
            for tile in list {
                endpoint[*tile] = true;
            }
        }
        for tile in &groups[0] {
            in_tree[*tile] = true;
        }
        let mut reached = vec![false; groups.len()];
        reached[0] = true;
        let mut group_of = vec![usize::MAX; tiles];
        for (index, list) in groups.iter().enumerate().skip(1) {
            for tile in list {
                if group_of[*tile] == usize::MAX {
                    group_of[*tile] = index;
                }
            }
        }
        while reached.iter().any(|done| !done) {
            let mut cost = vec![f32::INFINITY; tiles];
            let mut from = vec![u32::MAX; tiles];
            let mut heap = KeyHeap::default();
            for tile in 0..tiles {
                if in_tree[tile] {
                    cost[tile] = 0.0;
                    heap.push(tile as u64);
                }
            }
            let mut found = None;
            while let Some(key) = heap.pop() {
                let tile = (key & u32::MAX as u64) as usize;
                let here = f32::from_bits((key >> 32) as u32);
                if here > cost[tile] {
                    continue;
                }
                if group_of[tile] != usize::MAX && !reached[group_of[tile]] {
                    found = Some(tile);
                    break;
                }
                let (x, y) = (tile % self.tiles_x, tile / self.tiles_x);
                for (dx, dy) in DIRECTIONS {
                    let (tx, ty) = (x as i64 + dx as i64, y as i64 + dy as i64);
                    if tx < 0 || ty < 0 || tx >= self.tiles_x as i64 || ty >= self.tiles_y as i64 {
                        continue;
                    }
                    let next = ty as usize * self.tiles_x + tx as usize;
                    if room[next] <= 0.0 && !endpoint[next] {
                        continue;
                    }
                    let fill = (demand / room[next].max(1.0)).min(1.0);
                    let length = if dx != 0 && dy != 0 {
                        std::f32::consts::SQRT_2
                    } else {
                        1.0
                    };
                    let total = here + tile_mm * length * (1.0 + 12.0 * fill * fill * fill);
                    if total < cost[next] {
                        cost[next] = total;
                        from[next] = tile as u32;
                        heap.push(((total.to_bits() as u64) << 32) | next as u64);
                    }
                }
            }
            let tile = found?;
            let group = group_of[tile];
            reached[group] = true;
            let mut at = tile;
            while !in_tree[at] {
                in_tree[at] = true;
                if from[at] == u32::MAX {
                    break;
                }
                at = from[at] as usize;
            }
            for tile in &groups[group] {
                in_tree[*tile] = true;
            }
        }
        let margin = (width_mm as f32 / 2.0 / tile_mm).ceil() as usize + 1;
        let mut corridor = vec![false; tiles];
        for tile in (0..tiles).filter(|tile| in_tree[*tile]) {
            let extra: usize = std::env::var("PCB_ROUTER_BUS_MARGIN")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(BUS_MARGIN);
            let grow = extra + if endpoint[tile] { 1 } else { margin };
            let (x, y) = (tile % self.tiles_x, tile / self.tiles_x);
            for ty in y.saturating_sub(grow)..=(y + grow).min(self.tiles_y - 1) {
                for tx in x.saturating_sub(grow)..=(x + grow).min(self.tiles_x - 1) {
                    corridor[ty * self.tiles_x + tx] = true;
                }
            }
        }
        Some(corridor)
    }

    /// Whether `net` routes as a bundle member in this search: the switch
    /// on, a member not broken out, in the negotiation (not the hard
    /// repair, the cleanup or the via reduction).
    pub(super) fn bundled(&self, net: NetId, net_state: &NetState, hard: bool) -> Option<usize> {
        if hard || self.cleanup || self.in_via_reduction || net_state.bus_broken {
            return None;
        }
        let (bus, _) = self.bus_member(net)?;
        if net_state.reroutes >= BUS_PATIENCE {
            return None;
        }
        Some(bus)
    }

    /// Sets the lane of `net` in `scratch`: the cells on its layers beside
    /// the copper of its neighbours in the bus's order (closer than one
    /// track pitch and a pitch and a half). Inactive when they have none.
    pub(super) fn set_lane(&self, scratch: &mut Scratch, net: NetId) {
        self.clear_lane(scratch);
        let Some((bus, position)) = self.bus_member(net) else {
            return;
        };
        let members = &self.buses[bus].members;
        let class = self.board.classes[self.board.nets[net as usize].class];
        let pitch = self.grid.pitch;
        let radius = ((class.trace_width + class.clearance) / pitch + 1.5).ceil() as i64;
        let cells = self.grid.cells();
        let states = cells * self.board.layer_count;
        let lane = &mut scratch.lane;
        if lane.bits.len() != states.div_ceil(64) {
            lane.bits = vec![0; states.div_ceil(64)];
            lane.words.clear();
        }
        let (nx, ny) = (self.grid.nx as i64, self.grid.ny as i64);
        let from = position.saturating_sub(LANE_NEIGHBOURS);
        let to = (position + LANE_NEIGHBOURS).min(members.len() - 1);
        let mut any = false;
        for other in &members[from..=to] {
            if *other == net {
                continue;
            }
            for branch in &self.nets[*other as usize].branches {
                for node in &branch.nodes {
                    any = true;
                    let (x, y) = (node.cell as i64 % nx, node.cell as i64 / nx);
                    let base = node.layer as usize * cells;
                    for dy in -radius..=radius {
                        let ty = y + dy;
                        if ty < 0 || ty >= ny {
                            continue;
                        }
                        for dx in -radius..=radius {
                            let tx = x + dx;
                            if tx < 0 || tx >= nx || dx * dx + dy * dy > radius * radius {
                                continue;
                            }
                            let state = base + (ty * nx + tx) as usize;
                            let word = state >> 6;
                            if lane.bits[word] == 0 {
                                lane.words.push(word);
                            }
                            lane.bits[word] |= 1 << (state & 63);
                        }
                    }
                }
            }
        }
        lane.active = any;
        lane.off = self.config.bus_lane_cost as f32;
    }

    /// Clears the lane again.
    pub(super) fn clear_lane(&self, scratch: &mut Scratch) {
        let lane = &mut scratch.lane;
        for word in lane.words.drain(..) {
            lane.bits[word] = 0;
        }
        lane.active = false;
    }
}
