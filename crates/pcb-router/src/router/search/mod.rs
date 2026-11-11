// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The lattice search: a priced A* over the lattice's states.
//!
//! `Router::search` runs in stages: the set-up (`setup`: the search's
//! invariants, the net's own vias and the targets marked, the sources
//! queued; `guide`: the step costs and the heuristic), the loop (the pop
//! and its stop test, `reached`), the expansion of each popped node
//! (`expand`: the steps along the layer, the vias) and the path's
//! reconstruction (`path_to`).

use super::*;

mod expand;
mod guide;
mod setup;
#[cfg(test)]
mod tests;

use guide::Guide;

/// One search: what it reads and does not change (the net, its costs,
/// its guide and the net's invariants), built by `search_setup`.
pub(super) struct Search<'a> {
    net: NetId,
    net_state: &'a NetState,
    class: usize,
    layers: usize,
    cells: usize,
    nx: usize,
    /// The scratch's generation of this search (`SearchNode::seen`).
    generation: u16,
    /// A hard search (reinsertion): no step onto another net's copper.
    hard: bool,
    /// The target is the net's pour (`NetState::plane`), not `targets`.
    plane_target: bool,
    /// The present-congestion factor.
    present: f32,
    /// The nodes the search may visit, `(x0, y0, x1, y1)` inclusive.
    window: (usize, usize, usize, usize),
    via_cost: f32,
    bend_cost: f32,
    guide: Guide<'a>,
    /// The span model when the net's vias take spans.
    span_net: Option<&'a SpanModel>,
    /// With spans, per via cell of the net the layers its via pierces.
    own_vias: HashMap<u32, crate::board::LayerMask>,
    /// `hop_reach` of the net's class.
    hop_steps: usize,
    hop_reach: f64,
    /// The net class's static maps.
    statics: &'a StaticMaps,
    /// The net's preferred layers (0: none) and the factor elsewhere.
    preferred: crate::board::LayerMask,
    off_preferred: f32,
    own: u32,
    own16: u16,
    /// The class's via occupancy map.
    via_map: usize,
    /// Whether the net has neck zones, their class and bounding box and
    /// the mask over it (taken from the scratch, given back at the end).
    necks: bool,
    neck_class: Option<usize>,
    zone_box: (usize, usize, usize, usize),
    neck_mask: ZoneMask,
}

impl Search<'_> {
    /// `class_at` from the coordinates the step has at hand (no
    /// division): the neck class inside one of the net's neck zones, else
    /// the net's class.
    #[inline(always)]
    fn class_at(&self, x: usize, y: usize) -> usize {
        let zone_box = self.zone_box;
        match self.neck_class {
            Some(neck)
                if x >= zone_box.0 && y >= zone_box.1 && x <= zone_box.2 && y <= zone_box.3 =>
            {
                let answer = if self.neck_mask.contains(x, y) {
                    neck
                } else {
                    self.class
                };
                debug_assert_eq!(
                    answer,
                    zone_class(self.class, neck, &self.net_state.neck_zones, x, y)
                );
                answer
            }
            _ => self.class,
        }
    }
}

impl Router {
    /// The cheapest path of `net` from one of `sources` to one of
    /// `targets` (or, with `plane_target`, to a free node of the net's
    /// pour), source first; None when there is none in `window`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn search(
        &self,
        scratch: &mut Scratch,
        net: NetId,
        net_state: &NetState,
        sources: &[Node],
        targets: &[(Node, u16)],
        points: Option<&[(i32, i32, f32)]>,
        plane_target: bool,
        present: f32,
        hard: bool,
        window: (usize, usize, usize, usize),
    ) -> Option<Vec<Node>> {
        scratch.searches += 1;
        let expansions_before = scratch.expansions;
        let timed = search_rate_enabled().then(std::time::Instant::now);
        let search = self.search_setup(
            scratch,
            net,
            net_state,
            targets,
            points,
            plane_target,
            present,
            hard,
            window,
        );
        // The scratch's heap, its capacity kept from search to search.
        let mut heap = std::mem::take(&mut scratch.heap);
        heap.clear();
        self.queue_sources(&search, scratch, &mut heap, sources);
        let found = self.search_loop(&search, scratch, &mut heap);
        scratch.heap = heap;
        scratch.neck_mask = search.neck_mask;
        self.count_search(scratch, net, expansions_before, timed, found.is_some());
        Some(self.path_to(scratch, found?))
    }

    /// The A* loop: pops the least estimate and expands it until a
    /// target is popped (the popped state) or the open list runs dry.
    #[inline(always)]
    fn search_loop(
        &self,
        search: &Search,
        scratch: &mut Scratch,
        heap: &mut SearchHeap,
    ) -> Option<usize> {
        let (cells, generation) = (search.cells, search.generation);
        while let Some(key) = heap.pop(&mut scratch.nodes) {
            let state = (key & u32::MAX as u64) as usize;
            // The search node the next pop most likely takes (unless this
            // expansion pushes a lesser key), asked for an expansion ahead:
            // the popped node's own record was a cache miss (2 to 4 % of
            // the route's CPU; the second round, docs/profiling.md, "Router rounds", item 2).
            let next = heap.next_hint();
            if next != NO_KEY {
                let next = (next & u32::MAX as u64) as usize;
                prefetch_index(&scratch.nodes, next);
                // And its blocked steps' bits, the expansion's first map.
                let next_layer = next / cells;
                search
                    .statics
                    .prefetch_edges(next_layer, next - next_layer * cells);
            }
            // The heap holds each queued node once, at its least key: no
            // pop is stale (until 2026-10-27, 39 to 41 % were).
            debug_assert!(
                scratch.nodes[state].seen == generation
                    && scratch.nodes[state].slot < SLOT_UNQUEUED
            );
            scratch.nodes[state].slot = SLOT_CLOSED;
            scratch.expansions += 1;
            if self.reached(search, scratch, state) {
                return Some(state);
            }
            self.expand(search, scratch, heap, state);
        }
        None
    }

    /// Whether the popped `state` ends the search: a target, or with a
    /// plane target a node of the net's pour (a hard search's free of
    /// other nets).
    #[inline(always)]
    fn reached(&self, search: &Search, scratch: &Scratch, state: usize) -> bool {
        if scratch.nodes[state].since_via & TARGET_MARK != 0 {
            return true;
        }
        if search.plane_target {
            let (cells, layers) = (search.cells, search.layers);
            let state_net = search.net_state;
            let mask = if state_net.plane_target.is_empty() {
                &state_net.plane[state / cells]
            } else {
                &state_net.plane_target[state / cells]
            };
            let pour_map = state_net.plane_class[state / cells] * (layers + 1) + state / cells;
            if !mask.is_empty()
                && mask[state % cells]
                && !(search.hard
                    && self.occupancy_seen(scratch, search.class, pour_map, state % cells) > 0)
            {
                return true;
            }
        }
        false
    }

    /// Books the search in the scratch's counters and the profiles.
    fn count_search(
        &self,
        scratch: &mut Scratch,
        net: NetId,
        expansions_before: u64,
        timed: Option<std::time::Instant>,
        found: bool,
    ) {
        if let Some(started) = timed {
            use std::sync::atomic::Ordering::Relaxed;
            self.profile
                .search_nanos
                .fetch_add(started.elapsed().as_nanos() as u64, Relaxed);
            self.profile
                .search_expansions
                .fetch_add(scratch.expansions - expansions_before, Relaxed);
        }
        if net_work_path().is_some() {
            note_net_work(
                &self.board.nets[net as usize].name,
                scratch.expansions - expansions_before,
            );
        }
        if scratch.corridor.is_none() {
            scratch.open_searches += 1;
            scratch.open_expansions += scratch.expansions - expansions_before;
        }
        if !found {
            scratch.failed_searches += 1;
        }
    }

    /// The path to the reached `state`, from its source, by the parents
    /// the search recorded.
    fn path_to(&self, scratch: &Scratch, mut state: usize) -> Vec<Node> {
        let cells = self.grid.cells();
        let nx = self.grid.nx;
        let mut path = Vec::new();
        loop {
            let layer = state / cells;
            let cell = state % cells;
            path.push(Node {
                layer: layer as u8,
                cell: cell as u32,
            });
            let parent = scratch.nodes[state].parent;
            if parent == PARENT_SOURCE {
                break;
            }
            if parent < 8 {
                let (dx, dy) = DIRECTIONS[parent as usize];
                let x = (cell % nx) as i64 - dx as i64;
                let y = (cell / nx) as i64 - dy as i64;
                state = layer * cells + y as usize * nx + x as usize;
            } else {
                state = (parent as usize - 8) * cells + cell;
            }
        }
        path.reverse();
        path
    }

    /// Whether a via at `state`'s cell would lie within the hole-to-hole
    /// spacing of the previous via of the path being searched: a hop
    /// (through a via, a step or two, back through another), which KiCad
    /// reports as hole_to_hole whatever the nets (Castor's
    /// Net-(C33-Pad1): two vias 0.25 mm apart once vias stayed out of
    /// SMD pads). The net's routed branches keep their vias apart by
    /// `own_via_near`; this walks back the few steps that can be that
    /// near (`steps`, `reach`: `hop_reach` of the search). A via arriving
    /// here is the same via (another layer of it).
    ///
    /// `x`, `y` are the state's cell and `arrived` its parent: the walk
    /// follows the coordinates rather than dividing the states. The search
    /// asks only where the node's `since_via` is at most `steps` (else
    /// every node the walk would reach arrived by a step, and it would
    /// find nothing): until 2026-10-25 every expansion walked back with
    /// two divisions a step, 3 to 6.5 % of the route's CPU (the profile
    /// round, docs/profiling.md, "Router rounds", item 1).
    #[allow(clippy::too_many_arguments)]
    #[inline(always)]
    fn hops_from_a_via(
        &self,
        scratch: &Scratch,
        state: usize,
        x: usize,
        y: usize,
        arrived: u8,
        steps: usize,
        reach: f64,
    ) -> bool {
        let nx = self.grid.nx as i64;
        let (mut back, mut back_x, mut back_y) = (state as i64, x as i64, y as i64);
        let mut parent = arrived;
        for step in 0..=steps {
            if step > 0 {
                parent = scratch.nodes[back as usize].parent;
            }
            if parent == PARENT_SOURCE {
                return false;
            }
            if parent >= 8 {
                return back != state as i64
                    && crate::geometry::distance(
                        self.grid.center(back_x as usize, back_y as usize),
                        self.grid.center(x, y),
                    ) < reach - 1.0e-9;
            }
            let (dx, dy) = DIRECTIONS[parent as usize];
            back_x -= dx as i64;
            back_y -= dy as i64;
            back -= dy as i64 * nx + dx as i64;
        }
        false
    }

    /// The reach of `hops_from_a_via` for `class` and the steps back it
    /// walks, once a search.
    fn hop_reach(&self, class: usize) -> (usize, f64) {
        let rules = self.board.classes[class];
        let reach = rules.via_drill + self.board.hole_to_hole.max(0.0);
        // The shortest step: lines at the pins may lie closer than the
        // pitch.
        ((reach / self.grid.steps().0).ceil() as usize, reach)
    }

    /// Whether a retained branch is still legal against the fixed copper.
    pub(super) fn branch_is_legal(&self, net: NetId, branch: &Branch) -> bool {
        let nx = self.grid.nx as i64;
        for (index, node) in branch.nodes.iter().enumerate() {
            let statics = &self.statics[self.node_class(net, node.cell)];
            if !statics.trace_allowed(node.layer as usize, node.cell as usize, net) {
                return false;
            }
            if index == 0 {
                continue;
            }
            let previous = branch.nodes[index - 1];
            if previous.cell == node.cell {
                if !statics.via_allowed(node.cell as usize, net) {
                    return false;
                }
                continue;
            }
            let (dx, dy) = (
                node.cell as i64 % nx - previous.cell as i64 % nx,
                node.cell as i64 / nx - previous.cell as i64 / nx,
            );
            let Some(direction) = DIRECTIONS
                .iter()
                .position(|(sx, sy)| *sx as i64 == dx && *sy as i64 == dy)
            else {
                return false;
            };
            if !statics.edge_allowed(
                previous.layer as usize,
                previous.cell as usize,
                direction,
                net,
            ) {
                return false;
            }
        }
        true
    }
}
