// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The fan-out step (`Config::fanout`): before the negotiation, every pad
//! of a dense row (a neighbour of another net closer than `FINE_PITCH`, or
//! at most two straight ways out) gets one way out decided jointly with
//! its neighbours, and that way is kept free of other nets' copper by a
//! cost.
//!
//! A way out (`ExitRun`) is a straight lattice run from the pad's edge,
//! in one of the eight lattice directions from the lattice node at the
//! pad's centre, `fanout_depth` beyond the pad, every node and step legal
//! for the net. Its cost is how far it points from the net's nearest pad
//! on another part (the ratsnest's direction), more where it points into
//! its own part (under a QFN) and a little more on a diagonal. Two runs of different nets conflict when their
//! centrelines come closer than both half widths and the clearance; the
//! pads of a row choose one run each such that no two chosen runs
//! conflict, the most pads served first, the least cost second
//! (`assign_exits`, an exact search per group of pads whose runs
//! interact, bounded).
//!
//! The chosen runs are handed to the search as reservations: a node
//! within one track and clearance of a chosen run costs another net
//! `fanout_cost` mm per crossing (spread over the band's nodes, as a
//! pin-row gap's cost), and another net's via within reach of the run
//! costs it `fanout_cost` once. The pad's own net pays nothing, so the
//! run is its way out; a net that has no other way still crosses (the
//! cost is a price, not a wall: the usual fallback). Diagnosed on
//! 2026-10-28 (docs/router.md, "Fan-out before negotiation"): the last
//! open connections on fine-pitch rows were pads whose two straight exits
//! a neighbour's via (inward) and a neighbour's track turning across the
//! pad's tip (outward) had closed, where the row had an exit for every
//! pad.

use super::*;

/// `Config::fanout`'s default.
pub const FANOUT_DEFAULT: bool = false;

/// Pads closer than this (centre to centre, another net, the same layer)
/// make a row fine-pitch.
const FINE_PITCH: f64 = 0.65;

/// `PCB_ROUTER_FANOUT` when set: overrides `Config::fanout` (`0` or empty
/// off, anything else on).
pub fn fanout_override() -> Option<bool> {
    let value = std::env::var("PCB_ROUTER_FANOUT").ok()?;
    Some(!(value.is_empty() || value == "0"))
}

/// A way out into the pad's own part (under a QFN, towards its exposed
/// pad) costs this more: it leads under the part, where only a via goes
/// on, so it is taken where the outward way is closed or taken, not
/// because the net's other pad lies beyond the part (facade60's U4: two
/// pads given their inward way left the outward one to a neighbour's via
/// and stayed open).
const INWARD_COST: f64 = 2.0;

/// A part with fewer pads has no row.
const MIN_PART_PADS: usize = 4;

/// A way out that ends in a via costs this (the via, beside the run's
/// direction cost): taken where the row's order needs the pad's track
/// off its layer, or where the outward way leads far from the net's
/// other pad.
const VIA_EXIT_COST: f64 = 1.25;

/// A pad whose track the row's order cannot keep on its layer
/// (`mark_crossers`) pays this more on its ways without a via.
const ORDER_COST: f64 = 1.5;

/// How far beyond the pad a via site is looked for, in millimetres.
const VIA_REACH: f64 = 3.0;

/// A diagonal way out costs this more than an axis one (it passes
/// between the corners of two neighbours: an aQFN's inner row).
const DIAGONAL_COST: f64 = 0.25;

/// One way out of a pad: a straight lattice run from the first node
/// outside the pad to `end`.
#[derive(Clone, Debug)]
pub(super) struct ExitRun {
    pub(super) direction: usize,
    pub(super) start: crate::geometry::Point,
    pub(super) end: crate::geometry::Point,
    pub(super) cost: f64,
    /// The run ends in a via site (`end`): the pad's net leaves its layer
    /// there, so that the row's other ways cross its column beyond it.
    pub(super) via: bool,
}

/// A dense pad and its ways out.
#[derive(Clone, Debug)]
pub(super) struct DensePad {
    pub(super) net: NetId,
    pub(super) terminal: usize,
    pub(super) layer: usize,
    pub(super) half_width: f64,
    pub(super) clearance: f64,
    pub(super) via_radius: f64,
    pub(super) runs: Vec<ExitRun>,
    /// Ways out along the four axes that reach `fanout_depth` (the
    /// options), out of four.
    pub(super) free: usize,
    /// The nearest pad of another net on the layer, centre to centre.
    pub(super) pitch: f64,
    /// Its part's outward axis direction (an index of `DIRECTIONS`) and
    /// the part's name, for the rows' order.
    pub(super) row: Option<(String, usize)>,
    /// Where the ratsnest leads (`None` for no other pad).
    pub(super) target: Option<crate::geometry::Point>,
    /// The row's order cannot keep this pad's track on its layer
    /// (`mark_crossers`).
    pub(super) crosser: bool,
}

/// What the fan-out decided (`Router::fanout_plan`), for the report and
/// the tests.
#[derive(Clone, Debug, Default)]
pub(super) struct FanoutPlan {
    pub(super) pads: Vec<DensePad>,
    /// Per pad: the run chosen, if any.
    pub(super) chosen: Vec<Option<usize>>,
    /// Lattice nodes and search nodes the step spent.
    pub(super) work: u64,
}

impl FanoutPlan {
    /// Pads with runs that got none.
    pub(super) fn shortfall(&self) -> usize {
        self.pads
            .iter()
            .zip(&self.chosen)
            .filter(|(pad, chosen)| !pad.runs.is_empty() && chosen.is_none())
            .count()
    }
}

/// Two options in conflict: ((pad, option), (pad, option)).
pub(crate) type ExitConflict = ((usize, usize), (usize, usize));

/// The joint choice of `assign_exits`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ExitAssignment {
    /// Per pad: the option chosen.
    pub(crate) chosen: Vec<Option<usize>>,
    /// Search nodes spent.
    pub(crate) searched: u64,
}

impl ExitAssignment {
    /// Pads with at least one option that got none.
    #[cfg(test)]
    pub(crate) fn shortfall(&self, costs: &[Vec<f64>]) -> Vec<usize> {
        (0..costs.len())
            .filter(|pad| !costs[*pad].is_empty() && self.chosen[*pad].is_none())
            .collect()
    }
}

/// Assigns each pad at most one of its options (`costs[pad][option]`, the
/// option's cost) so that no two chosen options conflict (`conflicts`:
/// pairs of (pad, option), of different pads): the most pads served first,
/// then the least total cost. Exact per group of pads linked by conflicts,
/// within `budget` search nodes a group (beyond it, the best found, at
/// least the greedy choice in pad order).
pub(crate) fn assign_exits(
    costs: &[Vec<f64>],
    conflicts: &[ExitConflict],
    budget: u64,
) -> ExitAssignment {
    let pads = costs.len();
    let mut chosen = vec![None; pads];
    let mut searched = 0u64;
    // Per (pad, option): the (pad, option)s it conflicts with.
    let mut against: Vec<Vec<Vec<(usize, usize)>>> = costs
        .iter()
        .map(|options| vec![Vec::new(); options.len()])
        .collect();
    let mut parent: Vec<usize> = (0..pads).collect();
    fn find(parent: &mut [usize], mut node: usize) -> usize {
        while parent[node] != node {
            parent[node] = parent[parent[node]];
            node = parent[node];
        }
        node
    }
    for &((a, i), (b, j)) in conflicts {
        if a == b || a >= pads || b >= pads || i >= costs[a].len() || j >= costs[b].len() {
            continue;
        }
        against[a][i].push((b, j));
        against[b][j].push((a, i));
        let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
        parent[ra] = rb;
    }
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut group_of: HashMap<usize, usize> = HashMap::new();
    for (pad, options) in costs.iter().enumerate() {
        if options.is_empty() {
            continue;
        }
        let root = find(&mut parent, pad);
        let index = *group_of.entry(root).or_insert_with(|| {
            groups.push(Vec::new());
            groups.len() - 1
        });
        groups[index].push(pad);
    }
    // Options in cost order, per pad.
    let order: Vec<Vec<usize>> = costs
        .iter()
        .map(|options| {
            let mut indices: Vec<usize> = (0..options.len()).collect();
            indices.sort_by(|a, b| options[*a].total_cmp(&options[*b]).then(a.cmp(b)));
            indices
        })
        .collect();
    struct Search<'a> {
        costs: &'a [Vec<f64>],
        order: &'a [Vec<usize>],
        against: &'a [Vec<Vec<(usize, usize)>>],
        group: &'a [usize],
        /// The least option cost of the group's pads from this position on.
        rest: Vec<f64>,
        blocked: Vec<Vec<u32>>,
        current: Vec<Option<usize>>,
        best: Vec<Option<usize>>,
        best_served: usize,
        best_cost: f64,
        nodes: u64,
        budget: u64,
    }
    impl Search<'_> {
        fn choose(&mut self, pad: usize, option: usize, delta: i32) {
            for &(other, other_option) in &self.against[pad][option] {
                let count = &mut self.blocked[other][other_option];
                *count = (*count as i32 + delta) as u32;
            }
        }
        fn run(&mut self, position: usize, served: usize, cost: f64) {
            if self.nodes >= self.budget {
                return;
            }
            self.nodes += 1;
            let left = self.group.len() - position;
            if served + left < self.best_served
                || (served + left == self.best_served
                    && cost + self.rest[position] >= self.best_cost - 1.0e-12)
            {
                return;
            }
            if position == self.group.len() {
                self.best_served = served;
                self.best_cost = cost;
                self.best.clone_from(&self.current);
                return;
            }
            let pad = self.group[position];
            for index in 0..self.order[pad].len() {
                let option = self.order[pad][index];
                if self.blocked[pad][option] != 0 {
                    continue;
                }
                self.current[position] = Some(option);
                self.choose(pad, option, 1);
                self.run(position + 1, served + 1, cost + self.costs[pad][option]);
                self.choose(pad, option, -1);
                self.current[position] = None;
            }
            self.run(position + 1, served, cost);
        }
    }
    let mut blocked: Vec<Vec<u32>> = costs.iter().map(|options| vec![0; options.len()]).collect();
    for group in &mut groups {
        // The most constrained pads first (fewest options), for the greedy
        // choice and the search alike.
        group.sort_by_key(|pad| (costs[*pad].len(), *pad));
        let group: &[usize] = group;
        // The greedy choice, repaired: the bound to beat.
        let greedy = greedy_with_repair(group, &order, &against, &mut blocked);
        let served = greedy.iter().filter(|chosen| chosen.is_some()).count();
        let cost: f64 = group
            .iter()
            .zip(&greedy)
            .filter_map(|(pad, chosen)| chosen.map(|option| costs[*pad][option]))
            .sum();
        let mut rest = vec![0.0; group.len() + 1];
        for position in (0..group.len()).rev() {
            let pad = group[position];
            let least = order[pad].first().map_or(0.0, |option| costs[pad][*option]);
            rest[position] = rest[position + 1] + least;
        }
        let mut search = Search {
            costs,
            order: &order,
            against: &against,
            group,
            rest,
            blocked: std::mem::take(&mut blocked),
            current: vec![None; group.len()],
            best: greedy,
            best_served: served,
            best_cost: cost,
            nodes: 0,
            budget,
        };
        // Strictly better than the greedy choice, or the greedy choice.
        search.best_cost += 1.0e-9;
        search.run(0, 0, 0.0);
        searched += search.nodes;
        for (position, &pad) in group.iter().enumerate() {
            chosen[pad] = search.best[position];
        }
        blocked = search.blocked;
    }
    ExitAssignment { chosen, searched }
}

/// `assign_exits`' first answer for one group (`blocked` all zero before
/// and after): each pad in turn its cheapest option no chosen one
/// conflicts with; then each pad left without one takes an option whose
/// conflicts are at most two chosen pads that can move to another free
/// option each (an augmenting step), while that serves more pads.
fn greedy_with_repair(
    group: &[usize],
    order: &[Vec<usize>],
    against: &[Vec<Vec<(usize, usize)>>],
    blocked: &mut [Vec<u32>],
) -> Vec<Option<usize>> {
    let position_of: HashMap<usize, usize> = group
        .iter()
        .enumerate()
        .map(|(position, pad)| (*pad, position))
        .collect();
    let mut chosen: Vec<Option<usize>> = vec![None; group.len()];
    let mark = |blocked: &mut [Vec<u32>], pad: usize, option: usize, delta: i32| {
        for &(other, other_option) in &against[pad][option] {
            let count = &mut blocked[other][other_option];
            *count = (*count as i32 + delta) as u32;
        }
    };
    for (position, &pad) in group.iter().enumerate() {
        if let Some(&option) = order[pad].iter().find(|option| blocked[pad][**option] == 0) {
            chosen[position] = Some(option);
            mark(blocked, pad, option, 1);
        }
    }
    let mut progress = true;
    let mut rounds = 0;
    while progress && rounds < 4 {
        progress = false;
        rounds += 1;
        for position in 0..group.len() {
            if chosen[position].is_some() {
                continue;
            }
            let pad = group[position];
            'options: for &option in &order[pad] {
                // The chosen pads in the way.
                let mut in_way: Vec<usize> = Vec::new();
                for &(other, other_option) in &against[pad][option] {
                    let other_position = position_of[&other];
                    if chosen[other_position] == Some(other_option)
                        && !in_way.contains(&other_position)
                    {
                        in_way.push(other_position);
                    }
                }
                if in_way.is_empty() || in_way.len() > 2 {
                    continue;
                }
                // Lift them, take the option, and give each another one.
                for &other_position in &in_way {
                    let other = group[other_position];
                    mark(blocked, other, chosen[other_position].unwrap(), -1);
                }
                let lifted: Vec<(usize, usize)> = in_way
                    .iter()
                    .map(|other_position| (*other_position, chosen[*other_position].unwrap()))
                    .collect();
                mark(blocked, pad, option, 1);
                let mut moved: Vec<(usize, usize)> = Vec::new();
                for &(other_position, old) in &lifted {
                    let other = group[other_position];
                    match order[other]
                        .iter()
                        .find(|candidate| **candidate != old && blocked[other][**candidate] == 0)
                    {
                        Some(&candidate) => {
                            chosen[other_position] = Some(candidate);
                            mark(blocked, other, candidate, 1);
                            moved.push((other_position, candidate));
                        }
                        None => {
                            // Undo: the moves, the option, the lifted pads.
                            for &(moved_position, candidate) in &moved {
                                mark(blocked, group[moved_position], candidate, -1);
                            }
                            mark(blocked, pad, option, -1);
                            for &(lifted_position, old) in &lifted {
                                chosen[lifted_position] = Some(old);
                                mark(blocked, group[lifted_position], old, 1);
                            }
                            continue 'options;
                        }
                    }
                }
                chosen[position] = Some(option);
                progress = true;
                break;
            }
        }
    }
    // Back to all zero: the search keeps its own marks.
    for (position, &pad) in group.iter().enumerate() {
        if let Some(option) = chosen[position] {
            mark(blocked, pad, option, -1);
        }
    }
    chosen
}

/// The rows' order: per part, layer and outward direction, the pads in
/// their order along the row. Tracks leaving a row on its layer keep that
/// order (they fan out side by side), so the most pads whose ratsnest
/// targets follow the row's order stay on the layer (the longest run of
/// targets' coordinates along the row that does not fall), and the others
/// are crossers: their ways without a via cost `ORDER_COST` more, so that
/// they take a via site where one fits (facade60's U4: LED_1 above LED_0
/// heading down, LED_0 heading up, the one crossing the other's tip on
/// one layer). Returns the work (pairs compared).
fn mark_crossers(
    pads: &mut [DensePad],
    anchor: impl Fn(&DensePad) -> crate::geometry::Point,
) -> u64 {
    let mut rows: HashMap<(String, usize, usize), Vec<usize>> = HashMap::new();
    for (index, pad) in pads.iter().enumerate() {
        if let (Some((part, outward)), Some(_)) = (&pad.row, pad.target) {
            rows.entry((part.clone(), pad.layer, *outward))
                .or_default()
                .push(index);
        }
    }
    let mut work = 0u64;
    for ((_, _, outward), members) in rows {
        if members.len() < 3 {
            continue;
        }
        let (dx, dy) = crate::grid::DIRECTIONS[outward];
        let axis = [-dy as f64, dx as f64];
        let along = |point: crate::geometry::Point| point[0] * axis[0] + point[1] * axis[1];
        let mut order = members;
        order.sort_by(|a, b| {
            along(anchor(&pads[*a]))
                .total_cmp(&along(anchor(&pads[*b])))
                .then(a.cmp(b))
        });
        let targets: Vec<f64> = order
            .iter()
            .map(|index| along(pads[*index].target.unwrap_or_default()))
            .collect();
        let count = targets.len();
        let mut length = vec![1usize; count];
        let mut previous = vec![usize::MAX; count];
        for j in 0..count {
            for i in 0..j {
                work += 1;
                if targets[i] <= targets[j] + 1.0e-9 && length[i] + 1 > length[j] {
                    length[j] = length[i] + 1;
                    previous[j] = i;
                }
            }
        }
        let mut keep = vec![false; count];
        let mut at = (0..count)
            .max_by(|a, b| length[*a].cmp(&length[*b]).then(b.cmp(a)))
            .unwrap_or(usize::MAX);
        while at != usize::MAX {
            keep[at] = true;
            at = previous[at];
        }
        for (position, index) in order.into_iter().enumerate() {
            if keep[position] {
                continue;
            }
            let pad = &mut pads[index];
            pad.crosser = true;
            for run in &mut pad.runs {
                if !run.via {
                    run.cost += ORDER_COST;
                }
            }
        }
    }
    work
}

impl Router {
    /// The fan-out's plan on the current board: the dense pads, their ways
    /// out and the choice (`assign_exits`).
    pub(super) fn fanout_plan(&self) -> FanoutPlan {
        let mut plan = FanoutPlan::default();
        let pitch = self.grid.pitch;
        let depth = self.config.fanout_depth.max(pitch);
        let nx = self.grid.nx as i64;
        let ny = self.grid.ny as i64;
        // Every single-layer terminal's anchor, by layer, for the pitch
        // and the parts' centres.
        struct Anchor {
            net: NetId,
            at: crate::geometry::Point,
            layer: usize,
        }
        let mut anchors: Vec<Anchor> = Vec::new();
        let mut parts: HashMap<&str, ([f64; 2], usize)> = HashMap::new();
        for (net, description) in self.board.nets.iter().enumerate() {
            for terminal in &description.terminals {
                let part = terminal
                    .label
                    .rsplit_once('.')
                    .map_or(terminal.label.as_str(), |(part, _)| part);
                let entry = parts.entry(part).or_insert(([0.0, 0.0], 0));
                entry.0[0] += terminal.anchor[0];
                entry.0[1] += terminal.anchor[1];
                entry.1 += 1;
                if terminal.layers.count_ones() == 1 {
                    anchors.push(Anchor {
                        net: net as NetId,
                        at: terminal.anchor,
                        layer: terminal.layers.trailing_zeros() as usize,
                    });
                }
            }
        }
        // A bucket grid of 1 mm for the nearest neighbour.
        let bucket =
            |point: crate::geometry::Point| (point[0].floor() as i64, point[1].floor() as i64);
        let mut buckets: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
        for (index, anchor) in anchors.iter().enumerate() {
            buckets.entry(bucket(anchor.at)).or_default().push(index);
        }
        let mut work = 0u64;
        // A pad of a coarser row takes part with at most this many
        // straight ways out (`PCB_ROUTER_FANOUT_FREE`, an experiment; 0:
        // fine-pitch rows only).
        let coarse_free: usize = std::env::var("PCB_ROUTER_FANOUT_FREE")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(2);
        for (net, description) in self.board.nets.iter().enumerate() {
            let net = net as NetId;
            let state = &self.nets[net as usize];
            // A pour net's pads join their pour by spokes and stubs: no
            // way out of theirs is reserved, and they pay for none
            // (`Router::search`).
            if !state.routable || !state.plane.is_empty() {
                continue;
            }
            let class = description.class;
            for (index, terminal) in description.terminals.iter().enumerate() {
                if terminal.layers.count_ones() != 1 {
                    continue;
                }
                let layer = terminal.layers.trailing_zeros() as usize;
                let (bx, by) = bucket(terminal.anchor);
                let mut nearest = f64::INFINITY;
                for y in by - 1..=by + 1 {
                    for x in bx - 1..=bx + 1 {
                        for other in buckets.get(&(x, y)).into_iter().flatten() {
                            let other = &anchors[*other];
                            if other.net != net && other.layer == layer {
                                nearest = nearest
                                    .min(crate::geometry::distance(other.at, terminal.anchor));
                            }
                        }
                    }
                }
                work += 9;
                let shape = &self.board.obstacles[terminal.pad].shape;
                let Some(centre) = self.grid.nearest_node(terminal.anchor) else {
                    continue;
                };
                if !shape.contains(self.grid.center_of(centre)) {
                    // The pad's centre line is off the lattice: its
                    // stubs are oblique, no straight run leaves it.
                    continue;
                }
                let (cx, cy) = self.grid.xy(centre);
                let part = terminal
                    .label
                    .rsplit_once('.')
                    .map_or(terminal.label.as_str(), |(part, _)| part);
                // Rows are a part's: a part of fewer pads (a 0201's two
                // pads closer than the fine pitch) has none.
                if parts
                    .get(part)
                    .is_none_or(|(_, count)| *count < MIN_PART_PADS)
                {
                    continue;
                }
                let part_centre = parts
                    .get(part)
                    .map(|(sum, count)| [sum[0] / *count as f64, sum[1] / *count as f64]);
                // The ratsnest's direction: the nearest pad of the net on
                // another part, else the nearest pad; none for a pour net.
                let target = if state.plane.is_empty() {
                    description
                        .terminals
                        .iter()
                        .enumerate()
                        .filter(|(other, _)| *other != index)
                        .map(|(_, other)| {
                            let other_part = other
                                .label
                                .rsplit_once('.')
                                .map_or(other.label.as_str(), |(part, _)| part);
                            (
                                other_part == part,
                                crate::geometry::distance(other.anchor, terminal.anchor),
                                other.anchor,
                            )
                        })
                        .min_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)))
                        .map(|(_, _, at)| at)
                } else {
                    None
                };
                let mut runs = Vec::new();
                for (direction, (dx, dy)) in crate::grid::DIRECTIONS.iter().enumerate() {
                    let (dx, dy) = (*dx as i64, *dy as i64);
                    let (mut x, mut y) = (cx as i64, cy as i64);
                    let mut start: Option<crate::geometry::Point> = None;
                    let mut end = None;
                    loop {
                        x += dx;
                        y += dy;
                        work += 1;
                        if x < 0 || y < 0 || x >= nx || y >= ny {
                            break;
                        }
                        let cell = (y * nx + x) as usize;
                        let point = self.grid.center_of(cell);
                        if start.is_none() && shape.contains(point) {
                            continue;
                        }
                        let node_class = self.node_class(net, cell as u32);
                        if !self.statics[node_class].trace_allowed(layer, cell, net) {
                            break;
                        }
                        // The step into this node (a diagonal one may
                        // pass a corner closer than its nodes do).
                        let previous = ((y - dy) * nx + x - dx) as usize;
                        let (blocked, owner) = self.statics[self.node_class(net, previous as u32)]
                            .edges_at(layer, previous);
                        if blocked & (1 << direction) != 0 && owner != crate::grid::owner16(net) {
                            break;
                        }
                        let first = *start.get_or_insert(point);
                        // Beyond the pad by `depth` (the first node outside
                        // lies within a pitch of its edge).
                        if crate::geometry::distance(first, point) + pitch >= depth - 1.0e-9 {
                            end = Some(point);
                            break;
                        }
                    }
                    let (Some(start), Some(end)) = (start, end) else {
                        continue;
                    };
                    let diagonal = dx != 0 && dy != 0;
                    let outward = part_centre.is_some_and(|centre| {
                        (terminal.anchor[0] - centre[0]) * dx as f64
                            + (terminal.anchor[1] - centre[1]) * dy as f64
                            > 1.0e-6
                    });
                    // Via sites further along an outward axis: beyond the
                    // neighbours' runs (a via there keeps its clearance
                    // from their tracks), two at most, a via's spacing
                    // apart (staggered for neighbours that both take one).
                    let mut vias: Vec<crate::geometry::Point> = Vec::new();
                    if outward && !diagonal && self.board.layer_count > 1 {
                        let via_radius = self.board.classes[class].via_diameter / 2.0;
                        let clearance = self.board.classes[class].clearance;
                        let half_width = self.width_on(class, layer) / 2.0;
                        let first_site = depth + via_radius + clearance + half_width - pitch;
                        let spacing = 2.0 * via_radius + clearance;
                        let statics = &self.statics[class];
                        let (mut vx, mut vy) =
                            self.grid.xy(self.grid.nearest_node(end).unwrap_or(0));
                        loop {
                            vx = (vx as i64 + dx) as usize;
                            vy = (vy as i64 + dy) as usize;
                            work += 1;
                            if vx >= self.grid.nx || vy >= self.grid.ny {
                                break;
                            }
                            let cell = self.grid.index(vx, vy);
                            let point = self.grid.center_of(cell);
                            let along_run = crate::geometry::distance(start, point);
                            if along_run > VIA_REACH
                                || !self.statics[self.node_class(net, cell as u32)]
                                    .trace_allowed(layer, cell, net)
                            {
                                break;
                            }
                            if along_run + 1.0e-9 < first_site
                                || vias.last().is_some_and(|last| {
                                    crate::geometry::distance(*last, point) + 1.0e-9 < spacing
                                })
                                || !statics.via_allowed(cell, net)
                            {
                                continue;
                            }
                            vias.push(point);
                            if vias.len() == 2 {
                                break;
                            }
                        }
                    }
                    let along = if diagonal {
                        [
                            dx as f64 * std::f64::consts::FRAC_1_SQRT_2,
                            dy as f64 * std::f64::consts::FRAC_1_SQRT_2,
                        ]
                    } else {
                        [dx as f64, dy as f64]
                    };
                    let mut cost = match target {
                        Some(at) => {
                            let (tx, ty) = (at[0] - terminal.anchor[0], at[1] - terminal.anchor[1]);
                            let length = tx.hypot(ty);
                            if length < 1.0e-9 {
                                0.5
                            } else {
                                1.0 - (tx * along[0] + ty * along[1]) / length
                            }
                        }
                        None => 0.5,
                    };
                    if let Some(centre) = part_centre {
                        let (ox, oy) = (
                            terminal.anchor[0] - centre[0],
                            terminal.anchor[1] - centre[1],
                        );
                        if ox * along[0] + oy * along[1] < -1.0e-6 {
                            cost += INWARD_COST;
                        }
                    }
                    if diagonal {
                        cost += DIAGONAL_COST;
                    }
                    runs.push(ExitRun {
                        direction,
                        start,
                        end,
                        cost,
                        via: false,
                    });
                    for (index, site) in vias.into_iter().enumerate() {
                        runs.push(ExitRun {
                            direction,
                            start,
                            end: site,
                            cost: VIA_EXIT_COST + 0.05 * index as f64,
                            via: true,
                        });
                    }
                }
                // The straight ways along the axes: a pad with at most
                // two is in a row whatever its pitch.
                let free = runs
                    .iter()
                    .filter(|run| run.direction % 2 == 0 && !run.via)
                    .count();
                // A pad with no straight way out at all is listed (the
                // report names it) where the row is fine-pitch; it takes
                // no part in the choice.
                if nearest >= FINE_PITCH && (free == 0 || free > coarse_free) {
                    continue;
                }
                let neck = self.node_class(net, centre as u32);
                plan.pads.push(DensePad {
                    net,
                    terminal: index,
                    layer,
                    half_width: self.width_on(neck, layer) / 2.0,
                    clearance: self.board.classes[class].clearance,
                    via_radius: self.board.classes[class].via_diameter / 2.0,
                    row: part_centre.map(|centre| {
                        // The axis direction closest to the way from the
                        // part's centre to the pad.
                        let away = [
                            terminal.anchor[0] - centre[0],
                            terminal.anchor[1] - centre[1],
                        ];
                        let outward = (0..8)
                            .step_by(2)
                            .max_by(|a, b| {
                                let dot = |direction: usize| {
                                    let (dx, dy) = crate::grid::DIRECTIONS[direction];
                                    away[0] * dx as f64 + away[1] * dy as f64
                                };
                                dot(*a).total_cmp(&dot(*b)).then(b.cmp(a))
                            })
                            .unwrap_or(0);
                        (part.to_string(), outward)
                    }),
                    target,
                    crosser: false,
                    runs,
                    free,
                    pitch: nearest,
                });
            }
        }
        work += mark_crossers(&mut plan.pads, |pad| {
            self.board.nets[pad.net as usize].terminals[pad.terminal].anchor
        });
        // Conflicts between the runs of pads of different nets on one
        // layer, through a bucket grid of 1 mm over the runs' ends.
        let mut cells: HashMap<(usize, i64, i64), Vec<(usize, usize)>> = HashMap::new();
        for (pad_index, pad) in plan.pads.iter().enumerate() {
            for (run_index, run) in pad.runs.iter().enumerate() {
                let reach = 2.0 * pad.half_width.max(pad.via_radius) + pad.clearance;
                let (x0, y0) = bucket([
                    run.start[0].min(run.end[0]) - reach,
                    run.start[1].min(run.end[1]) - reach,
                ]);
                let (x1, y1) = bucket([
                    run.start[0].max(run.end[0]) + reach,
                    run.start[1].max(run.end[1]) + reach,
                ]);
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        cells
                            .entry((pad.layer, x, y))
                            .or_default()
                            .push((pad_index, run_index));
                    }
                }
            }
        }
        let mut conflicts: Vec<ExitConflict> = Vec::new();
        let mut seen: HashSet<ExitConflict> = HashSet::new();
        for members in cells.values() {
            for (i, &(a, ra)) in members.iter().enumerate() {
                for &(b, rb) in &members[i + 1..] {
                    let (pa, pb) = (&plan.pads[a], &plan.pads[b]);
                    if a == b || pa.net == pb.net {
                        continue;
                    }
                    let key = if a < b {
                        ((a, ra), (b, rb))
                    } else {
                        ((b, rb), (a, ra))
                    };
                    if !seen.insert(key) {
                        continue;
                    }
                    work += 1;
                    let (run_a, run_b) = (&pa.runs[ra], &pb.runs[rb]);
                    let clearance = pa.clearance.max(pb.clearance) - 1.0e-9;
                    let required = pa.half_width + pb.half_width + clearance;
                    // A via site against the other run, and two sites
                    // against each other.
                    let via_against = |via: &ExitRun, radius: f64, other: &ExitRun, half: f64| {
                        via.via
                            && crate::geometry::point_segment_distance(
                                via.end,
                                other.start,
                                other.end,
                            ) < radius + half + clearance
                    };
                    let both_vias = run_a.via
                        && run_b.via
                        && crate::geometry::distance(run_a.end, run_b.end)
                            < pa.via_radius + pb.via_radius + clearance;
                    if both_vias
                        || via_against(run_a, pa.via_radius, run_b, pb.half_width)
                        || via_against(run_b, pb.via_radius, run_a, pa.half_width)
                        || crate::geometry::segment_segment_distance(
                            run_a.start,
                            run_a.end,
                            run_b.start,
                            run_b.end,
                        ) < required
                    {
                        conflicts.push(key);
                    }
                }
            }
        }
        conflicts.sort_unstable();
        let costs: Vec<Vec<f64>> = plan
            .pads
            .iter()
            .map(|pad| pad.runs.iter().map(|run| run.cost).collect())
            .collect();
        let assignment = assign_exits(&costs, &conflicts, 20_000);
        plan.chosen = assignment.chosen;
        plan.work = work + assignment.searched;
        plan
    }

    /// Marks the chosen runs of the fan-out (`fanout_plan`) as reservations
    /// after the pin-row gaps (`mark_pin_rows`): the nodes within one track
    /// and clearance of a run as gap entries of the run's net (the search
    /// charges another net `fanout_cost` per crossing), and the via sites
    /// within reach of the run (`fanout_via`, bit 4 of `via_marks`).
    pub(super) fn mark_fanout(&mut self) {
        self.fanout_via.clear();
        if !self.config.fanout || self.config.fanout_cost <= 0.0 {
            return;
        }
        let plan = self.fanout_plan();
        let why = std::env::var_os("PCB_ROUTER_FANOUT_WHY").is_some();
        if why {
            self.report_fanout(&plan);
        }
        let pitch = self.grid.pitch;
        let owner = crate::grid::owner;
        // Per layer and node: the reserving nets (two at most).
        let mut bands: Vec<HashMap<u32, (u32, u32, f64)>> =
            vec![HashMap::new(); self.board.layer_count];
        let mut vias: HashMap<u32, u32> = HashMap::new();
        let mut work = plan.work;
        // `PCB_ROUTER_FANOUT_VIAS=0` (an experiment): no via sites.
        let via_band = std::env::var("PCB_ROUTER_FANOUT_VIAS").map_or(true, |value| value != "0");
        for (pad, chosen) in plan.pads.iter().zip(&plan.chosen) {
            let Some(run) = chosen.map(|index| &pad.runs[index]) else {
                continue;
            };
            let trace_reach = 2.0 * pad.half_width + pad.clearance;
            let via_reach = pad.half_width + pad.clearance + pad.via_radius;
            // Around a via site: another net's track within the via's
            // clearance, another via within both.
            let (site_trace, site_via) = if run.via {
                (
                    pad.via_radius + pad.clearance + pad.half_width,
                    2.0 * pad.via_radius + pad.clearance,
                )
            } else {
                (0.0, 0.0)
            };
            let reach = trace_reach.max(via_reach).max(site_via);
            let bounds = crate::geometry::Aabb {
                minimum: [run.start[0].min(run.end[0]), run.start[1].min(run.end[1])],
                maximum: [run.start[0].max(run.end[0]), run.start[1].max(run.end[1])],
            }
            .inflated(reach);
            let Some((x0, y0, x1, y1)) = self.grid.node_range(bounds) else {
                continue;
            };
            let per_node = self.config.fanout_cost * pitch / (2.0 * trace_reach);
            let own = owner(pad.net);
            for y in y0..=y1 {
                for x in x0..=x1 {
                    work += 1;
                    let cell = self.grid.index(x, y) as u32;
                    let distance = crate::geometry::point_segment_distance(
                        self.grid.center(x, y),
                        run.start,
                        run.end,
                    );
                    let at_site = if run.via {
                        crate::geometry::distance(self.grid.center(x, y), run.end)
                    } else {
                        f64::INFINITY
                    };
                    // The run on its layer; the via site on every layer.
                    let layers = if at_site < site_trace - 1.0e-9 {
                        0..self.board.layer_count
                    } else if distance < trace_reach - 1.0e-9 {
                        pad.layer..pad.layer + 1
                    } else {
                        0..0
                    };
                    for layer in layers {
                        let entry = bands[layer].entry(cell).or_insert((own, own, per_node));
                        if entry.0 != own && entry.1 == entry.0 {
                            entry.1 = own;
                        }
                        entry.2 = entry.2.max(per_node);
                    }
                    if via_band && (distance < via_reach - 1.0e-9 || at_site < site_via - 1.0e-9) {
                        vias.entry(cell).or_insert(own);
                    }
                }
            }
        }
        let mut entries: HashMap<(u32, u32, u64), u16> = HashMap::new();
        if self.pin_row_nets.is_empty() {
            self.pin_row_nets.push((0, 0));
            self.pin_row_costs.push(0.0);
        }
        for (layer, band) in bands.into_iter().enumerate() {
            let mut nodes: Vec<(u32, (u32, u32, f64))> = band.into_iter().collect();
            nodes.sort_unstable_by_key(|(cell, _)| *cell);
            for (cell, (a, b, cost)) in nodes {
                let pair = (a.min(b), a.max(b));
                let key = (pair.0, pair.1, cost.to_bits());
                let index = match entries.get(&key) {
                    Some(index) => *index,
                    None => {
                        if self.pin_row_nets.len() > u16::MAX as usize {
                            continue;
                        }
                        let index = self.pin_row_nets.len() as u16;
                        self.pin_row_nets.push(pair);
                        self.pin_row_costs.push(cost as f32);
                        entries.insert(key, index);
                        index
                    }
                };
                self.pin_row[layer].push((cell, index));
            }
        }
        let mut vias: Vec<(u32, u32)> = vias.into_iter().collect();
        vias.sort_unstable();
        self.fanout_via = vias;
        self.apply_fanout_via_marks();
        // The step's own work, on the work clock (as the straightening's
        // checks: eight a search node).
        self.scratch.expansions += work.div_ceil(8);
        if self.config.verbose || why {
            let assigned = plan.chosen.iter().filter(|chosen| chosen.is_some()).count();
            eprintln!(
                "fanout: {} dense pads, {assigned} with a way out reserved, {} short, {} without a straight way out, {} search nodes",
                plan.pads.len(),
                plan.shortfall(),
                plan.pads.iter().filter(|pad| pad.runs.is_empty()).count(),
                plan.work
            );
        }
    }

    /// Sets bit 4 of `via_marks` on the fan-out's via sites.
    pub(super) fn apply_fanout_via_marks(&mut self) {
        for &(cell, _) in &self.fanout_via {
            if let Some(mark) = self.via_marks.get_mut(cell as usize) {
                *mark |= 4;
            }
        }
    }

    /// The fan-out's via cost of a via at `cell` by the net `own` (an
    /// `owner`): `fanout_cost` where another net's run is within reach.
    pub(super) fn fanout_via_cost(&self, own: u32, cell: usize) -> f32 {
        match self
            .fanout_via
            .binary_search_by_key(&(cell as u32), |(node, _)| *node)
        {
            Ok(position) if self.fanout_via[position].1 != own => self.config.fanout_cost as f32,
            _ => 0.0,
        }
    }

    /// `PCB_ROUTER_FANOUT_WHY`: per part with dense pads, each pad's ways
    /// out and the choice.
    fn report_fanout(&self, plan: &FanoutPlan) {
        const NAMES: [&str; 8] = ["+x", "+x+y", "+y", "-x+y", "-x", "-x-y", "-y", "+x-y"];
        let mut by_part: Vec<(String, usize)> = plan
            .pads
            .iter()
            .enumerate()
            .map(|(index, pad)| {
                let label = &self.board.nets[pad.net as usize].terminals[pad.terminal].label;
                (label.clone(), index)
            })
            .collect();
        by_part.sort();
        for (label, index) in by_part {
            let pad = &plan.pads[index];
            let options: Vec<String> = pad
                .runs
                .iter()
                .enumerate()
                .map(|(run_index, run)| {
                    format!(
                        "{}{}{} {:.2}",
                        if plan.chosen[index] == Some(run_index) {
                            "*"
                        } else {
                            ""
                        },
                        NAMES[run.direction],
                        if run.via { " via" } else { "" },
                        run.cost
                    )
                })
                .collect();
            eprintln!(
                "fanout {label} {} layer {} pitch {:.2}{}: {} free [{}]{}",
                self.board.nets[pad.net as usize].name,
                pad.layer,
                pad.pitch,
                if pad.crosser { " crosser" } else { "" },
                pad.free,
                options.join(", "),
                if pad.runs.is_empty() {
                    " NO WAY OUT"
                } else if plan.chosen[index].is_none() {
                    " SHORT"
                } else {
                    ""
                }
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{Net, Obstacle, ObstacleKind, RuleClass, Terminal};
    use crate::geometry::Shape;

    /// The conflicts of options that use the same exit (`slots[pad]
    /// [option]`).
    fn sharing(slots: &[Vec<&str>]) -> Vec<ExitConflict> {
        let mut conflicts = Vec::new();
        for (a, options) in slots.iter().enumerate() {
            for (i, slot) in options.iter().enumerate() {
                for (b, others) in slots.iter().enumerate().skip(a + 1) {
                    for (j, other) in others.iter().enumerate() {
                        if slot == other {
                            conflicts.push(((a, i), (b, j)));
                        }
                    }
                }
            }
        }
        conflicts
    }

    /// A row of six pads under four exits U0 to U3, each pad reaching the
    /// one or two above it, the first and the last pad also one at the
    /// row's end (L, R). Taking each pad's cheapest free exit in order
    /// leaves the fifth pad without one; the joint choice serves all six
    /// (the end pads leave sideways). Without R two pads compete for U3
    /// and one is short; without L and R, four exits serve four pads.
    #[test]
    fn a_row_assigns_every_pad_an_exit_when_it_can_and_names_the_shortfall() {
        let slots = vec![
            vec!["U0", "L"],
            vec!["U0", "U1"],
            vec!["U1", "U2"],
            vec!["U2", "U3"],
            vec!["U3"],
            vec!["U3", "R"],
        ];
        let costs: Vec<Vec<f64>> = vec![
            vec![0.0, 0.5],
            vec![0.0, 0.2],
            vec![0.0, 0.2],
            vec![0.0, 0.2],
            vec![0.0],
            vec![0.0, 0.5],
        ];
        let assignment = assign_exits(&costs, &sharing(&slots), 10_000);
        assert!(assignment.shortfall(&costs).is_empty());
        let used: Vec<&str> = assignment
            .chosen
            .iter()
            .enumerate()
            .map(|(pad, chosen)| slots[pad][chosen.unwrap()])
            .collect();
        assert_eq!(used, ["L", "U0", "U1", "U2", "U3", "R"]);

        // The row's right end closed: U3 serves one of the last two pads.
        let mut closed = slots.clone();
        closed[5].pop();
        let mut closed_costs = costs.clone();
        closed_costs[5].pop();
        let assignment = assign_exits(&closed_costs, &sharing(&closed), 10_000);
        let short = assignment.shortfall(&closed_costs);
        assert_eq!(short.len(), 1);
        assert!(short[0] == 4 || short[0] == 5);
        let served = assignment.chosen.iter().filter(|c| c.is_some()).count();
        assert_eq!(served, 5);

        // Four exits for six pads.
        let mut four = closed;
        four[0].pop();
        let mut four_costs = closed_costs;
        four_costs[0].pop();
        let assignment = assign_exits(&four_costs, &sharing(&four), 10_000);
        assert_eq!(assignment.shortfall(&four_costs).len(), 2);
        let mut taken: Vec<&str> = assignment
            .chosen
            .iter()
            .enumerate()
            .filter_map(|(pad, chosen)| chosen.map(|option| four[pad][option]))
            .collect();
        taken.sort_unstable();
        assert_eq!(taken, ["U0", "U1", "U2", "U3"]);
    }

    fn pad(shape: Shape, net: Option<NetId>, kind: ObstacleKind, label: &str) -> Obstacle {
        Obstacle {
            shape,
            layers: 1,
            kind,
            net,
            clearance: 0.0,
            clearance_override: None,
            blocks_tracks: true,
            blocks_vias: true,
            label: label.into(),
        }
    }

    fn rectangle(x0: f64, y0: f64, x1: f64, y1: f64) -> Shape {
        Shape::Polygon {
            points: vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]],
        }
    }

    /// A row of five 0.3 mm pads at 0.5 mm pitch (nets P0 to P4, each
    /// with a pad 4 mm above the row, 2 mm apart) and net S from one side of the row
    /// to the other, its pads 0.6 mm beyond the row's tips. `walls`
    /// closes everything but the strip between the tips and a keepout
    /// 0.8 mm beyond them.
    fn row_and_crossing(fanout: bool, walls: bool) -> Router {
        let class = RuleClass {
            trace_width: 0.2,
            clearance: 0.2,
            via_diameter: 0.6,
            via_drill: 0.3,
            layers: 0,
        };
        let mut obstacles = Vec::new();
        let mut nets = Vec::new();
        let terminal = |pad: usize, at: [f64; 2], label: &str| Terminal {
            anchor: at,
            layers: 1,
            pad,
            contact: None,
            label: label.into(),
        };
        for index in 0..5 {
            let x = 4.0 + 0.5 * index as f64;
            let net = index as NetId;
            obstacles.push(pad(
                rectangle(x - 0.15, 4.5, x + 0.15, 5.5),
                Some(net),
                ObstacleKind::Copper,
                &format!("U1.{index}"),
            ));
            let row = obstacles.len() - 1;
            let far = 1.0 + 2.0 * index as f64;
            obstacles.push(pad(
                Shape::Circle {
                    center: [far, 9.0],
                    radius: 0.2,
                },
                Some(net),
                ObstacleKind::Copper,
                &format!("J{index}.1"),
            ));
            nets.push(Net {
                name: format!("P{index}"),
                class: 0,
                terminals: vec![
                    terminal(row, [x, 5.0], &format!("U1.{index}")),
                    terminal(row + 1, [far, 9.0], &format!("J{index}.1")),
                ],
            });
        }
        let s = nets.len() as NetId;
        let mut s_terminals = Vec::new();
        for (index, x) in [1.0, 9.0].iter().enumerate() {
            obstacles.push(pad(
                Shape::Circle {
                    center: [*x, 6.1],
                    radius: 0.15,
                },
                Some(s),
                ObstacleKind::Copper,
                &format!("K{index}.1"),
            ));
            s_terminals.push(terminal(
                obstacles.len() - 1,
                [*x, 6.1],
                &format!("K{index}.1"),
            ));
        }
        nets.push(Net {
            name: "S".into(),
            class: 0,
            terminals: s_terminals,
        });
        if walls {
            for (x0, y0, x1, y1) in [(0.0, 6.3, 10.0, 8.0), (0.0, 0.0, 10.0, 4.3)] {
                obstacles.push(pad(
                    rectangle(x0, y0, x1, y1),
                    None,
                    ObstacleKind::Keepout,
                    "wall",
                ));
            }
        }
        let board = Board {
            layer_count: 1,
            outline: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
            edge_clearance: 0.1,
            hole_clearance: 0.25,
            hole_to_hole: 0.25,
            classes: vec![class],
            neck_width: 0.2,
            obstacles,
            nets,
            planes: Vec::new(),
            solid_pads: Vec::new(),
            isolated_pads: Vec::new(),
            solid_pad_obstacles: Vec::new(),
            isolated_pad_obstacles: Vec::new(),
            pad_spokes: Default::default(),
            merged_pads: Vec::new(),
            no_via_pads: Vec::new(),
            fill_passes: Vec::new(),
            pin_row_gaps: Vec::new(),
            terminal_copper: Vec::new(),
            attachment: Default::default(),
        };
        let config = Config {
            pitches: vec![0.1],
            fanout,
            fanout_cost: 6.0,
            fanout_depth: 0.5,
            ..Config::default()
        };
        Router::new(&board, &config)
    }

    /// Net S's route alone: complete, its nodes, and how many lie in
    /// another net's reserved run.
    fn route_s(router: &mut Router) -> (Vec<Node>, usize) {
        let s = 5;
        router.route_net_seq(s, 0.0, false, 1.0);
        router.stamp(s);
        assert!(router.nets[s as usize].complete);
        let nodes: Vec<Node> = router.nets[s as usize]
            .branches
            .iter()
            .flat_map(|branch| branch.nodes.iter().copied())
            .collect();
        let reserved = nodes
            .iter()
            .filter(|node| router.pin_row_foreign(s, node.layer as usize, node.cell as usize))
            .count();
        (nodes, reserved)
    }

    /// The row's pads all leave upwards (towards their pads above), and
    /// S, whose straight line runs 0.6 mm beyond the row's tips, keeps out
    /// of the reserved runs with the fan-out (round them, beyond their
    /// band) and runs straight without it; walled into the strip beside
    /// the tips, S still completes through the runs (a price, not a
    /// wall).
    #[test]
    fn the_search_keeps_out_of_a_reserved_way_out_and_crosses_it_when_it_must() {
        let mut router = row_and_crossing(true, false);
        let plan = router.fanout_plan();
        assert_eq!(plan.pads.len(), 5);
        for (pad, chosen) in plan.pads.iter().zip(&plan.chosen) {
            let run = &pad.runs[chosen.expect("every row pad has a way out")];
            assert_eq!(run.direction, 2, "towards the pads above");
        }
        let highest = |router: &Router, nodes: &[Node]| {
            nodes
                .iter()
                .map(|node| router.grid.center_of(node.cell as usize)[1])
                .fold(f64::NEG_INFINITY, f64::max)
        };
        let (nodes, reserved) = route_s(&mut router);
        assert_eq!(reserved, 0);
        assert!(highest(&router, &nodes) > 6.35);

        let mut plain = row_and_crossing(false, false);
        let (nodes, reserved) = route_s(&mut plain);
        assert_eq!(reserved, 0, "no reservations without the switch");
        assert!(highest(&plain, &nodes) < 6.15);

        let mut walled = row_and_crossing(true, true);
        let (_, reserved) = route_s(&mut walled);
        assert!(reserved > 0);
    }
}
