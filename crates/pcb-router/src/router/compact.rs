// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Compaction: straight runs slid beside their neighbours.
//!
//! A designer lays a bus as a bundle; our tracks spread over the free
//! board (docs/router.md, "What the routes look like (2026-11-05)"). A
//! run's segment S between two segments of one direction D (a Z: D, S, D)
//! can slide along D: the segment before grows by what the one after
//! loses, so the length, the bends and the ends stay as they were. This
//! pass, part of the straightening, slides such a segment toward other
//! nets' copper as far as every step stays free as the hard search judges
//! it, and keeps the position where most of the window's steps run beside
//! other copper (`hug_of_path`: an occupied node across the step, about
//! the minimum spacing), if that is more than before.

use super::straighten::{bend_score, direction_of, jogs_and_staircases};
use super::*;

/// The farthest a segment slides sideways (mm).
const SLIDE_MM: f64 = 1.0;
/// The shortest segment slid (mm): the census counts parallel runs from
/// 0.5 mm of overlap.
const SLIDE_RUN_MM: f64 = 0.5;
/// The positions tried before other copper, the nearest first: the one
/// beside it, else up to two steps short of it (a pad or a pour edge in
/// the way of the legs).
const SLIDE_TRIES: usize = 3;

/// What `compact_run` read: the steps it checked (`step_free`) and the
/// occupancy it loaded.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct SlideWork {
    pub checks: u64,
    pub loads: u64,
}

impl Router {
    /// The steps of a path of `net` (nodes in order) that run beside other
    /// copper: steps on one layer whose target has a node across the step
    /// occupied by another net's copper (with the net's own copper
    /// lifted).
    pub(super) fn hug_of_path(&self, net: NetId, nodes: &[Node]) -> usize {
        let (nx, ny) = (self.grid.nx as i64, self.grid.ny as i64);
        let mut count = 0;
        for pair in nodes.windows(2) {
            if pair[0].layer != pair[1].layer {
                continue;
            }
            let (a, b) = (pair[0].cell as i64, pair[1].cell as i64);
            let (tx, ty) = (b % nx, b / nx);
            let (dx, dy) = (tx - a % nx, ty - a / nx);
            let map = self.map_index(self.node_class(net, pair[1].cell), pair[1].layer as usize);
            if [(tx - dy, ty + dx), (tx + dy, ty - dx)]
                .iter()
                .any(|&(hx, hy)| {
                    hx >= 0
                        && hy >= 0
                        && hx < nx
                        && hy < ny
                        && self.occupancy.get(map, (hy * nx + hx) as usize) > 0
                })
            {
                count += 1;
            }
        }
        count
    }

    /// One layer's run of lattice nodes with its Z segments slid beside
    /// other copper (see the module's comment), or None if nothing moved.
    /// `taken`: the net's other nodes, which a new path may not touch;
    /// `fixed`: nodes that must stay (branch ends, junctions). Counts the
    /// steps it checks and the occupancy it reads in `work`.
    pub(super) fn compact_run(
        &self,
        net: NetId,
        run: &[Node],
        taken: &HashSet<Node>,
        fixed: &HashSet<Node>,
        corridors: &HashMap<(u8, u32), Vec<NetId>>,
        work: &mut SlideWork,
    ) -> Option<Vec<Node>> {
        let nx = self.grid.nx as i64;
        let pitch = self.grid.pitch;
        let layer = run[0].layer;
        let xy = |node: &Node| (node.cell as i64 % nx, node.cell as i64 / nx);
        let length = |direction: u8| {
            if direction % 2 == 1 {
                pitch * std::f64::consts::SQRT_2
            } else {
                pitch
            }
        };
        let mut nodes = run.to_vec();
        let mut changed = false;
        'improve: for _ in 0..run.len() {
            let mut directions = Vec::with_capacity(nodes.len());
            for pair in nodes.windows(2) {
                let (a, b) = (xy(&pair[0]), xy(&pair[1]));
                directions.push(direction_of(b.0 - a.0, b.1 - a.1)?);
            }
            // Segment k runs from node corners[k] to corners[k + 1].
            let mut corners = vec![0];
            corners.extend((1..directions.len()).filter(|k| directions[k - 1] != directions[*k]));
            corners.push(nodes.len() - 1);
            if corners.len() < 4 {
                break;
            }
            let score = bend_score(&directions, pitch);
            let steps = jogs_and_staircases(&directions, pitch);
            for k in 1..corners.len() - 2 {
                let (p0, p1, p2, p3) = (corners[k - 1], corners[k], corners[k + 1], corners[k + 2]);
                let (along, side) = (directions[p0], directions[p1]);
                if directions[p2] != along
                    || (p2 - p1) as f64 * length(side) < SLIDE_RUN_MM
                    || nodes[p0 + 1..p3].iter().any(|node| fixed.contains(node))
                {
                    continue;
                }
                // How far a step along D moves S sideways: the sine
                // between D and S times D's step.
                let turn = (side as i32 - along as i32).rem_euclid(8);
                let sine = match turn.min(8 - turn) {
                    1 | 3 => std::f64::consts::FRAC_1_SQRT_2,
                    2 => 1.0,
                    _ => continue,
                };
                let reach = (SLIDE_MM / (sine * length(along)).max(1.0e-9)).floor() as i64;
                let window = &nodes[p0 + 1..p3];
                let (before, after) = (p1 - p0, p3 - p2);
                let old = self.hug_of_path(net, &nodes[p0..=p3]);
                let mut best: Option<(usize, Vec<Node>)> = None;
                let (ax, ay) = crate::grid::DIRECTIONS[along as usize];
                let segment = &nodes[p1..=p2];
                for sign in [1i64, -1] {
                    // The first slide that puts S on other copper (the
                    // occupancy alone, a load a node): the positions just
                    // before it run beside that copper. None within reach:
                    // nothing to slide to on this side.
                    let shifted = |slide: i64| {
                        let (sx, sy) = (sign * slide * ax as i64, sign * slide * ay as i64);
                        segment.iter().map(move |node| {
                            let (x, y) = xy(node);
                            (x + sx, y + sy)
                        })
                    };
                    let mut blocked = None;
                    for slide in 1..=reach + 1 {
                        let shift = sign * slide;
                        if before as i64 + shift < 1 || after as i64 - shift < 1 {
                            break;
                        }
                        work.loads += segment.len() as u64;
                        if shifted(slide).any(|(x, y)| {
                            x < 0
                                || y < 0
                                || x >= nx
                                || y >= self.grid.ny as i64
                                || self.occupancy.get(
                                    self.map_index(
                                        self.node_class(net, (y * nx + x) as u32),
                                        layer as usize,
                                    ),
                                    (y * nx + x) as usize,
                                ) > 0
                        }) {
                            blocked = Some(slide);
                            break;
                        }
                    }
                    let Some(blocked) = blocked else {
                        continue;
                    };
                    for slide in (1..blocked).rev().take(SLIDE_TRIES) {
                        let shift = sign * slide;
                        let (first, last) = (before as i64 + shift, after as i64 - shift);
                        let shape: Vec<u8> = std::iter::repeat_n(along, first as usize)
                            .chain(std::iter::repeat_n(side, p2 - p1))
                            .chain(std::iter::repeat_n(along, last as usize))
                            .collect();
                        let mut cell = nodes[p0].cell as usize;
                        let mut path = vec![nodes[p0]];
                        let free = shape.iter().all(|direction| {
                            work.checks += 1;
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
                        if !free || *path.last().unwrap() != nodes[p3] {
                            continue;
                        }
                        let interior = &path[1..path.len() - 1];
                        if interior.iter().any(|node| {
                            fixed.contains(node)
                                || (taken.contains(node) && !window.contains(node))
                                || nodes[..=p0].contains(node)
                                || nodes[p3..].contains(node)
                        }) {
                            continue;
                        }
                        let mut cut = 0isize;
                        let mut enters = false;
                        for node in interior {
                            let (guarded, on_pour) = self.guarded_and_cut(net, *node, corridors);
                            enters |= guarded && !window.contains(node);
                            cut += on_pour as isize;
                        }
                        for node in window {
                            cut -= self.guarded_and_cut(net, *node, corridors).1 as isize;
                        }
                        if enters || cut > 0 {
                            continue;
                        }
                        let hug = self.hug_of_path(net, &path);
                        if hug <= best.as_ref().map_or(old, |(hug, _)| *hug) {
                            continue;
                        }
                        // The bends merge by distance: no more bends,
                        // stair-steps, jogs or staircases on the run.
                        let candidate: Vec<u8> = directions[..p0]
                            .iter()
                            .chain(&shape)
                            .chain(&directions[p3..])
                            .copied()
                            .collect();
                        let (bends, stairs, _) = bend_score(&candidate, pitch);
                        let (jogs, staircases) = jogs_and_staircases(&candidate, pitch);
                        if bends > score.0
                            || stairs > score.1
                            || staircases > steps.1
                            || jogs + staircases > steps.0 + steps.1
                        {
                            continue;
                        }
                        best = Some((hug, path));
                        break;
                    }
                }
                if let Some((_, path)) = best {
                    let mut next = nodes[..p0].to_vec();
                    next.extend(&path);
                    next.extend(&nodes[p3 + 1..]);
                    nodes = next;
                    changed = true;
                    continue 'improve;
                }
            }
            break;
        }
        changed.then_some(nodes)
    }
}

/// What the compaction did, for the verbose line.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Compacted {
    pub nets: usize,
    pub runs: usize,
    pub hug_before: usize,
    pub hug_after: usize,
    pub taken_back: usize,
}

impl Router {
    /// Slides the Z segments of every complete signal net's branches
    /// beside other copper (`compact_run`), after the straightening. With
    /// `pour_aware_polish`, if a pour net then has more islands
    /// (`pour_islands`) or the spoke model more starved pads than before,
    /// the shortest prefix of the nets changed that grows it ends with the
    /// culprit (found by halving); it and the nets after it go back.
    pub(super) fn compact(&mut self, order: &[NetId]) -> Compacted {
        let mut report = Compacted::default();
        let corridors = self.corridor_map();
        let mut work = SlideWork::default();
        // (net, old branches, new branches), in order.
        let mut changed: Vec<(NetId, Vec<Branch>, Vec<Branch>)> = Vec::new();
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
            self.unstamp(net);
            let mut branches = saved.clone();
            let mut runs = 0;
            let (mut hug_before, mut hug_after) = (0, 0);
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
                    match (run.len() > 3)
                        .then(|| self.compact_run(net, run, &taken, &fixed, &corridors, &mut work))
                        .flatten()
                    {
                        Some(moved) => {
                            hug_before += self.hug_of_path(net, run);
                            hug_after += self.hug_of_path(net, &moved);
                            nodes.extend(moved);
                            runs += 1;
                        }
                        None => nodes.extend_from_slice(run),
                    }
                    start = index;
                }
                branch.nodes = nodes;
            }
            self.stamp(net);
            if runs == 0 {
                continue;
            }
            report.runs += runs;
            report.hug_before += hug_before;
            report.hug_after += hug_after;
            self.nets[net as usize].branches = branches.clone();
            self.stamp(net);
            changed.push((net, saved, branches));
        }
        if !changed.is_empty() {
            // The counts before: every change taken back for the count.
            for (net, old, _) in changed.iter().rev() {
                self.put_back(*net, old.clone());
            }
            let islands_before = if self.config.pour_aware_polish {
                self.pour_islands()
            } else {
                Vec::new()
            };
            let starved_before = self.starved_pads();
            let grown = |router: &Self| {
                router
                    .pour_islands()
                    .iter()
                    .zip(&islands_before)
                    .any(|(after, before)| after > before)
                    || router.starved_pads() > starved_before
            };
            let apply =
                |router: &mut Self, changed: &[(NetId, Vec<Branch>, Vec<Branch>)], kept: usize| {
                    for (index, (net, old, new)) in changed.iter().enumerate().rev() {
                        let branches = if index < kept { new } else { old };
                        if router.nets[*net as usize].branches != *branches {
                            router.put_back(*net, branches.clone());
                        }
                    }
                };
            apply(self, &changed, changed.len());
            if grown(self) {
                let (mut low, mut high) = (0, changed.len());
                while high - low > 1 {
                    let middle = (low + high) / 2;
                    apply(self, &changed, middle);
                    if grown(self) {
                        high = middle;
                    } else {
                        low = middle;
                    }
                }
                apply(self, &changed, low);
                report.taken_back = changed.len() - low;
                changed.truncate(low);
            }
        }
        report.nets = changed.len();
        // Eight step checks are about one expansion's work, as are 64
        // occupancy loads.
        let SlideWork { checks, loads } = work;
        self.scratch.expansions += checks.div_ceil(8) + loads.div_ceil(64);
        if self.config.verbose {
            eprintln!(
                "compaction: {} nets, {} runs slid, steps beside other copper {} -> {}, {} nets taken back for pour islands or starved pads, {checks} steps checked, {loads} occupancy loads",
                report.nets, report.runs, report.hug_before, report.hug_after, report.taken_back
            );
        }
        report
    }
}
