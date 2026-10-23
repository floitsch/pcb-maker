// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Straightening: stair-steps and jogs taken out of the finished routes.
//!
//! The search costs a bend per 45 degrees of direction change against the
//! direction it arrived from, but it closes a lattice node at its first
//! arrival whatever the direction, so among the many shortest paths of an
//! octilinear lattice it often keeps one that steps sideways and back (a
//! stair-step), and the negotiation's detours leave jogs the clean-up does
//! not take out: it keeps a reroute only when it is shorter, and a path
//! with fewer bends is mostly as long. This pass works on each path of a
//! finished net, one layer's run at a time: between two of the run's
//! corners it tries the two shortest octilinear paths (the axis part first
//! or the diagonal first) and takes one where it has fewer bends (then
//! fewer stair-steps, then less turning in all) and every step is free as
//! the hard search judges it, against all other copper. A shortest path
//! is never longer, the ends and vias of every branch stay where they
//! were, so the net's connections and via count do not change.
//! `crates/pcb-kicad/src/aesthetics.rs` counts bends and stair-steps by
//! the same rule on the board's tracks.

use super::*;

/// Turns of one sense closer than this are one bend (mm): a corner cut at
/// 45 degrees.
const MERGE_MM: f64 = 1.0;
/// Two bends within this that return to the first direction are a
/// stair-step (mm).
const STAIR_MM: f64 = 2.0;
/// The most corners a replacement spans, and the longest path it replaces
/// (mm).
const SPAN_CORNERS: usize = 8;
const SPAN_MM: f64 = 15.0;

/// The bends, stair-steps and total turning (in 45 degrees) of a run of
/// lattice steps given by their directions (`DIRECTIONS` indices).
pub(super) fn bend_score(directions: &[u8], pitch: f64) -> (usize, usize, u32) {
    let step = |direction: u8| {
        if direction % 2 == 1 {
            pitch * std::f64::consts::SQRT_2
        } else {
            pitch
        }
    };
    // (turn in 45 degrees, where it starts, where it ends) along the run.
    let mut bends: Vec<(i32, f64, f64)> = Vec::new();
    let mut along = 0.0;
    let mut total = 0;
    for index in 1..directions.len() {
        along += step(directions[index - 1]);
        let turn = (directions[index] as i32 - directions[index - 1] as i32).rem_euclid(8);
        let turn = if turn > 4 { turn - 8 } else { turn };
        if turn == 0 {
            continue;
        }
        total += turn.unsigned_abs();
        match bends.last_mut() {
            Some(last)
                if last.0.signum() == turn.signum()
                    && along - last.2 < MERGE_MM
                    && (last.0 + turn).abs() <= 2 =>
            {
                last.0 += turn;
                last.2 = along;
            }
            _ => bends.push((turn, along, along)),
        }
    }
    let mut stairs = 0;
    let mut index = 1;
    while index < bends.len() {
        let (first, second) = (bends[index - 1], bends[index]);
        if first.0 + second.0 == 0 && second.1 - first.2 < STAIR_MM {
            stairs += 1;
            index += 2;
        } else {
            index += 1;
        }
    }
    (bends.len(), stairs, total)
}

/// The direction index of a unit step, if it is one.
fn direction_of(dx: i64, dy: i64) -> Option<u8> {
    crate::grid::DIRECTIONS
        .iter()
        .position(|(x, y)| *x as i64 == dx && *y as i64 == dy)
        .map(|index| index as u8)
}

/// The two shortest octilinear step sequences from (0, 0) to (dx, dy): the
/// axis part first, and the diagonal part first (one, if either is empty).
fn shortest_shapes(dx: i64, dy: i64) -> Vec<Vec<u8>> {
    let diagonal = dx.abs().min(dy.abs());
    let axis = dx.abs().max(dy.abs()) - diagonal;
    let diagonal_direction = direction_of(dx.signum(), dy.signum());
    let axis_direction = if dx.abs() > dy.abs() {
        direction_of(dx.signum(), 0)
    } else {
        direction_of(0, dy.signum())
    };
    let mut shapes = Vec::new();
    let axis_part = vec![axis_direction.unwrap_or(0); axis as usize];
    let diagonal_part = vec![diagonal_direction.unwrap_or(0); diagonal as usize];
    shapes.push([axis_part.clone(), diagonal_part.clone()].concat());
    if axis > 0 && diagonal > 0 {
        shapes.push([diagonal_part, axis_part].concat());
    }
    shapes
}

/// A net the pass changed: the net, its old branches, its (bends,
/// stair-steps) before and after, and the runs changed.
type Changed = (NetId, Vec<Branch>, (usize, usize), (usize, usize), usize);

/// What the pass did.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Straightened {
    pub nets: usize,
    pub runs: usize,
    pub bends_before: usize,
    pub bends_after: usize,
    pub stairs_before: usize,
    pub stairs_after: usize,
    pub pour_rejected: usize,
}

impl Router {
    /// Whether the hard search could step from `cell` in `direction` on
    /// `layer` for `net` with the net's own stamps lifted: the checks of
    /// `search`'s step (the static map and its edge blocks, the other
    /// nets' clearance zones at the node and beside a diagonal, exclusive
    /// pours).
    fn step_free(&self, net: NetId, layer: usize, cell: usize, direction: u8) -> Option<usize> {
        let nx = self.grid.nx;
        let (x, y) = self.grid.xy(cell);
        let (dx, dy) = crate::grid::DIRECTIONS[direction as usize];
        let (tx, ty) = (x as i64 + dx as i64, y as i64 + dy as i64);
        if tx < 0 || ty < 0 || tx >= nx as i64 || ty >= self.grid.ny as i64 {
            return None;
        }
        let target = ty as usize * nx + tx as usize;
        let own = crate::grid::owner(net);
        let own16 = crate::grid::owner16(net);
        let here = &self.statics[self.node_class(net, cell as u32)];
        let (edge_block, edge_owner) = here.edges_at(layer, cell);
        if edge_block != 0 && edge_owner != own16 && edge_block & (1 << direction) != 0 {
            return None;
        }
        let class = self.node_class(net, target as u32);
        let hot = self.hot[class * self.board.layer_count + layer][target];
        if hot.trace != crate::grid::FREE16 && hot.trace as u32 != own {
            return None;
        }
        if self.occupancy[self.map_index(class, layer)][target] > 0 {
            return None;
        }
        if dx != 0 && dy != 0 {
            let (start, orientation) = if dx > 0 {
                (cell, (dy < 0) as usize)
            } else {
                (target, (dy > 0) as usize)
            };
            if self.occupancy[self.diagonal_map(class, layer, orientation)][start] > 0 {
                return None;
            }
        }
        if hot.pour != 0 && self.exclusive[layer] && {
            let (_, covered) = self.hot_pour(hot.pour, layer, target);
            covered != 0 && covered as u32 != own
        } {
            return None;
        }
        Some(target)
    }

    /// How many of `nodes` lie in another net's thermal guard (a pad's
    /// spoke corridors) and how many on another net's pour.
    fn guarded_and_cut(&self, net: NetId, nodes: &[Node]) -> (usize, usize) {
        let own = crate::grid::owner(net);
        let (mut guarded, mut cut) = (0, 0);
        for node in nodes {
            let class = self.node_class(net, node.cell);
            let hot =
                self.hot[class * self.board.layer_count + node.layer as usize][node.cell as usize];
            if hot.pour == 0 {
                continue;
            }
            let (guard, covered) = self.hot_pour(hot.pour, node.layer as usize, node.cell as usize);
            guarded += (guard != 0 && guard as u32 != own) as usize;
            cut += (covered != 0 && covered as u32 != own) as usize;
        }
        (guarded, cut)
    }

    /// One layer's run of lattice nodes straightened (`straighten`), or
    /// None if nothing changed. `taken`: the net's other nodes, which a new
    /// path may not touch; `fixed`: nodes that must stay (branch ends,
    /// junctions). Counts the steps it checks in `checks`.
    fn straighten_run(
        &self,
        net: NetId,
        run: &[Node],
        taken: &HashSet<Node>,
        fixed: &HashSet<Node>,
        checks: &mut u64,
    ) -> Option<Vec<Node>> {
        let nx = self.grid.nx as i64;
        let pitch = self.grid.pitch;
        let layer = run[0].layer;
        let xy = |node: &Node| (node.cell as i64 % nx, node.cell as i64 / nx);
        let mut nodes = run.to_vec();
        let mut changed = false;
        'improve: for _ in 0..4 * run.len() {
            let mut directions = Vec::with_capacity(nodes.len());
            for pair in nodes.windows(2) {
                let (a, b) = (xy(&pair[0]), xy(&pair[1]));
                directions.push(direction_of(b.0 - a.0, b.1 - a.1)?);
            }
            if directions.len() < 2 {
                break;
            }
            let mut corners = vec![0];
            corners.extend((1..directions.len()).filter(|k| directions[k - 1] != directions[*k]));
            corners.push(nodes.len() - 1);
            if corners.len() < 3 {
                break;
            }
            let mut along = Vec::with_capacity(directions.len() + 1);
            along.push(0.0);
            for direction in &directions {
                let step = if direction % 2 == 1 {
                    pitch * std::f64::consts::SQRT_2
                } else {
                    pitch
                };
                along.push(along.last().unwrap() + step);
            }
            let last = corners.len() - 1;
            for a in 0..corners.len() {
                for b in (a + 2..corners.len().min(a + SPAN_CORNERS + 1)).rev() {
                    let (i, j) = (corners[a], corners[b]);
                    if along[j] - along[i] > SPAN_MM
                        || nodes[i + 1..j].iter().any(|node| fixed.contains(node))
                    {
                        continue;
                    }
                    // Bends interact within a few corners: the score is
                    // compared on the window and three corners either side.
                    let (low, high) = (corners[a.saturating_sub(3)], corners[(b + 3).min(last)]);
                    let score = bend_score(&directions[low..high], pitch);
                    let (from, to) = (xy(&nodes[i]), xy(&nodes[j]));
                    for shape in shortest_shapes(to.0 - from.0, to.1 - from.1) {
                        if shape[..] == directions[i..j] {
                            continue;
                        }
                        let candidate: Vec<u8> = directions[low..i]
                            .iter()
                            .chain(&shape)
                            .chain(&directions[j..high])
                            .copied()
                            .collect();
                        if bend_score(&candidate, pitch) >= score {
                            continue;
                        }
                        let mut cell = nodes[i].cell as usize;
                        let mut path = Vec::with_capacity(shape.len());
                        let free = shape.iter().all(|direction| {
                            *checks += 1;
                            match self.step_free(net, layer as usize, cell, *direction) {
                                Some(next) => {
                                    cell = next;
                                    path.push(Node {
                                        layer,
                                        cell: next as u32,
                                    });
                                    true
                                }
                                None => false,
                            }
                        });
                        if !free || *path.last().unwrap() != nodes[j] {
                            continue;
                        }
                        // The new nodes may touch none of the net's others
                        // (the window's own excepted) nor the run's.
                        let interior = &path[..path.len() - 1];
                        let window = &nodes[i + 1..j];
                        if interior.iter().any(|node| {
                            fixed.contains(node)
                                || (taken.contains(node) && !window.contains(node))
                                || nodes[..=i].contains(node)
                                || nodes[j..].contains(node)
                        }) {
                            continue;
                        }
                        // Nor enter more of other pads' spoke corridors
                        // (the thermal spoke repair would move it out
                        // again, with vias) nor cut more of a pour.
                        let (now, was) = (
                            self.guarded_and_cut(net, interior),
                            self.guarded_and_cut(net, window),
                        );
                        if now.0 > was.0 || now.1 > was.1 {
                            continue;
                        }
                        let mut next = nodes[..=i].to_vec();
                        next.extend(&path);
                        next.extend(&nodes[j + 1..]);
                        nodes = next;
                        changed = true;
                        continue 'improve;
                    }
                }
            }
            break;
        }
        changed.then_some(nodes)
    }

    /// The bend score of a branch's runs, summed.
    fn branch_bends(&self, branch: &Branch) -> (usize, usize) {
        let nx = self.grid.nx as i64;
        let mut directions = Vec::new();
        let (mut bends, mut stairs) = (0, 0);
        let mut flush = |directions: &mut Vec<u8>| {
            let (b, s, _) = bend_score(directions, self.grid.pitch);
            bends += b;
            stairs += s;
            directions.clear();
        };
        for pair in branch.nodes.windows(2) {
            let (a, b) = (pair[0].cell as i64, pair[1].cell as i64);
            match direction_of(b % nx - a % nx, b / nx - a / nx) {
                Some(direction) if pair[0].layer == pair[1].layer => directions.push(direction),
                _ => flush(&mut directions),
            }
        }
        flush(&mut directions);
        (bends, stairs)
    }

    /// Takes stair-steps and jogs out of every complete signal net's
    /// branches (see the module's comment). With `pour_aware_polish` the
    /// pour nets' stranded pads are counted before the first change and
    /// once after the last (the pour nets within a stamp's reach of what
    /// changed); if a count grew, the changed nets go back to their old
    /// branches, the last changed first, until none has grown.
    pub(super) fn straighten(&mut self, order: &[NetId]) -> Straightened {
        let mut report = Straightened::default();
        let pour_nets = if self.config.pour_aware_polish {
            self.pour_net_ids()
        } else {
            Vec::new()
        };
        let mut pour_stranded: Option<Vec<usize>> = None;
        let mut checks = 0u64;
        // The changed nets: (net, old branches, (bends, stairs) before and
        // after, runs).
        let mut changed: Vec<Changed> = Vec::new();
        for &net in order {
            let state = &self.nets[net as usize];
            if !state.routable
                || !state.complete
                || !state.plane.is_empty()
                || state.branches.len() <= state.fixed
            {
                continue;
            }
            let fixed: HashSet<Node> = state
                .branches
                .iter()
                .filter(|branch| !branch.nodes.is_empty())
                .flat_map(|branch| [branch.nodes[0], *branch.nodes.last().unwrap()])
                .chain(
                    state.branches[..state.fixed]
                        .iter()
                        .flat_map(|branch| branch.nodes.iter().copied()),
                )
                .collect();
            let taken: HashSet<Node> = state
                .branches
                .iter()
                .flat_map(|branch| branch.nodes.iter().copied())
                .collect();
            let first = state.fixed;
            let saved = state.branches.clone();
            // Lifted for the checks: the net's own copper is no obstacle.
            self.unstamp(net);
            let mut branches = saved.clone();
            let mut runs = 0;
            for branch in branches.iter_mut().skip(first) {
                let mut nodes = Vec::with_capacity(branch.nodes.len());
                let mut start = 0;
                for index in 1..=branch.nodes.len() {
                    if index < branch.nodes.len()
                        && branch.nodes[index].layer == branch.nodes[start].layer
                    {
                        continue;
                    }
                    let run = &branch.nodes[start..index];
                    match (run.len() > 2)
                        .then(|| self.straighten_run(net, run, &taken, &fixed, &mut checks))
                        .flatten()
                    {
                        Some(straight) => {
                            nodes.extend(straight);
                            runs += 1;
                        }
                        None => nodes.extend_from_slice(run),
                    }
                    start = index;
                }
                branch.nodes = nodes;
            }
            // The net's old copper back for now (its branches unchanged).
            self.stamp(net);
            if runs == 0 {
                continue;
            }
            if pour_stranded.is_none() && !pour_nets.is_empty() {
                pour_stranded = Some(self.pour_stranded());
            }
            let score = |router: &Self, branches: &[Branch]| {
                branches[first..]
                    .iter()
                    .map(|branch| router.branch_bends(branch))
                    .fold((0, 0), |sum, b| (sum.0 + b.0, sum.1 + b.1))
            };
            let (before, after) = (score(self, &saved), score(self, &branches));
            self.nets[net as usize].branches = branches;
            self.stamp(net);
            changed.push((net, saved, before, after, runs));
        }
        if let Some(stranded) = pour_stranded {
            let pour_reach = {
                let widest = self
                    .board
                    .classes
                    .iter()
                    .map(|class| class.via_diameter + class.trace_width + class.clearance)
                    .fold(0.0f64, f64::max)
                    + self.board.hole_clearance.max(self.board.hole_to_hole);
                (2.0 * widest / self.grid.pitch).ceil() as usize + 3
            };
            let mut area: Option<(usize, usize, usize, usize)> = None;
            for (net, saved, ..) in &changed {
                for node in saved
                    .iter()
                    .chain(&self.nets[*net as usize].branches)
                    .flat_map(|branch| &branch.nodes)
                {
                    let (x, y) = self.grid.xy(node.cell as usize);
                    area = Some(area.map_or((x, y, x, y), |(x0, y0, x1, y1)| {
                        (x0.min(x), y0.min(y), x1.max(x), y1.max(y))
                    }));
                }
            }
            let near: Vec<bool> = pour_nets
                .iter()
                .map(|pour| match (area, self.plane_bounds(*pour)) {
                    (Some((x0, y0, x1, y1)), Some((px0, py0, px1, py1))) => {
                        x0 <= px1 + pour_reach
                            && px0 <= x1 + pour_reach
                            && y0 <= py1 + pour_reach
                            && py0 <= y1 + pour_reach
                    }
                    _ => false,
                })
                .collect();
            let grown = |router: &Self| {
                pour_nets
                    .iter()
                    .zip(&near)
                    .zip(&stranded)
                    .any(|((pour, near), was)| *near && router.pour_stranded_of(*pour) > *was)
            };
            while !changed.is_empty() && grown(self) {
                let (net, saved, ..) = changed.pop().unwrap();
                self.nets[net as usize].branches = saved;
                self.stamp(net);
                report.pour_rejected += 1;
            }
        }
        for (_, _, before, after, runs) in &changed {
            report.nets += 1;
            report.runs += runs;
            report.bends_before += before.0;
            report.bends_after += after.0;
            report.stairs_before += before.1;
            report.stairs_after += after.1;
        }
        // Eight step checks are about one expansion's work.
        self.scratch.expansions += checks.div_ceil(8);
        if self.config.verbose {
            eprintln!(
                "straightening: {} nets, {} runs, bends {} -> {}, stair-steps {} -> {}, {} nets kept out for stranding pour pads, {checks} steps checked",
                report.nets,
                report.runs,
                report.bends_before,
                report.bends_after,
                report.stairs_before,
                report.stairs_after,
                report.pour_rejected
            );
        }
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_jog_is_two_bends_and_a_stair_step_and_a_cut_corner_one_bend() {
        // East, one step north-east, east: a jog.
        assert_eq!(bend_score(&[0, 0, 0, 1, 0, 0, 0], 0.1), (2, 1, 2));
        // East, two steps north-east, north: a corner cut at 45 degrees.
        assert_eq!(bend_score(&[0, 0, 1, 1, 2, 2], 0.1), (1, 0, 2));
        // The same cut 20 steps long (2.8 mm) is two bends.
        let mut long = vec![0, 0];
        long.extend([1; 20]);
        long.extend([2, 2]);
        assert_eq!(bend_score(&long, 0.1), (2, 0, 2));
        assert_eq!(bend_score(&[0, 0, 0], 0.1), (0, 0, 0));
    }

    #[test]
    fn the_shortest_shapes_lead_to_the_end() {
        for (dx, dy) in [(5, 2), (-3, 7), (4, 0), (-2, -2), (0, -6)] {
            for shape in shortest_shapes(dx, dy) {
                let end = shape.iter().fold((0i64, 0i64), |(x, y), d| {
                    let (sx, sy) = crate::grid::DIRECTIONS[*d as usize];
                    (x + sx as i64, y + sy as i64)
                });
                assert_eq!(end, (dx, dy));
                assert_eq!(shape.len() as i64, dx.abs().max(dy.abs()));
            }
        }
    }
}
