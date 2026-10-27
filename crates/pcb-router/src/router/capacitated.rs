// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Capacitated corridors in the router (`Config::capacitated_corridors`,
//! `crate::capacitated`; docs/router.md, "Capacitated corridors
//! (2026-10-24)"): the plan's corridors for the negotiation's searches, a
//! miss located on the plan's boundaries, the learning and the re-plans.

use super::*;
use crate::capacitated::{CapacitatedPlan, Learn, PlanFrame, SearchKind, UNIT_EXPANSIONS, note};

/// The router's capacitated plan and how its tiles map onto the router's.
pub(super) struct Capacitated {
    plan: Box<dyn CapacitatedPlan>,
    frame: PlanFrame,
    /// Per plan tile: the router tiles it overlaps (x0, y0, x1, y1,
    /// inclusive).
    ranges: Vec<[u32; 4]>,
    /// The plan's work units charged to the search so far.
    charged: u64,
}

impl Clone for Capacitated {
    fn clone(&self) -> Self {
        Self {
            plan: self.plan.boxed_clone(),
            frame: self.frame,
            ranges: self.ranges.clone(),
            charged: self.charged,
        }
    }
}

/// Where a search that found nothing inside its capacitated corridor
/// stopped: the boundaries of the net's global route with one side
/// reached and the other not, and the tiles of its vias reached on one
/// of their layers only.
#[derive(Clone, Debug, Default)]
pub(super) struct Miss {
    boundaries: Vec<u32>,
    vias: Vec<u32>,
}

/// How a net's negotiation searches go under `capacitated_corridors`.
pub(super) enum CapacitatedStatus {
    /// Not at all (switch off, no plan, a hard or clean-up search).
    Off,
    /// The plan does not have the net (its terminals in one tile): the
    /// tile planner as without the switch.
    Unplanned,
    /// No global path under the learned capacities, or nothing could be
    /// learned from its miss: the window and the board.
    Fallback,
    /// Inside this corridor (per router tile).
    Band(Vec<bool>),
}

impl Router {
    /// `search` without a plane target, counted by `kind` in the
    /// process's corridor counters (`crate::capacitated::corridor_totals`).
    #[allow(
        clippy::too_many_arguments,
        reason = "the search's own arguments and the kind it is counted as"
    )]
    pub(super) fn counted_search(
        &self,
        kind: SearchKind,
        scratch: &mut Scratch,
        net: NetId,
        net_state: &NetState,
        sources: &[Node],
        targets: &[(Node, u16)],
        points: Option<&[(i32, i32, f32)]>,
        present: f32,
        hard: bool,
        window: CellWindow,
    ) -> Option<Vec<Node>> {
        let before = scratch.expansions;
        let found = self.search(
            scratch, net, net_state, sources, targets, points, false, present, hard, window,
        );
        crate::capacitated::note_search(hard, kind, found.is_some(), scratch.expansions - before);
        found
    }

    /// Builds the capacitated plan when the switch is on, a planner is
    /// registered and none is built (after `update`: the board changed).
    pub(super) fn ensure_capacitated(&mut self) {
        if !self.config.capacitated_corridors || self.capacitated.is_some() {
            return;
        }
        let Some(factory) = crate::capacitated::planner() else {
            return;
        };
        let plan = factory(&self.board);
        let frame = plan.frame();
        let pitch = self.grid.pitch;
        let origin = self.grid.origin;
        // A node covers half a pitch on either side of its centre.
        let router_tile = |mm: f64, axis: usize, count: usize| -> u32 {
            let node = ((mm - origin[axis]) / pitch + 0.5).floor().max(0.0) as usize;
            (node / TILE).min(count - 1) as u32
        };
        let epsilon = 1.0e-6;
        let mut ranges = Vec::with_capacity(frame.tiles());
        for ty in 0..frame.tiles_y {
            for tx in 0..frame.tiles_x {
                let x0 = frame.origin[0] + tx as f64 * frame.tile_mm;
                let y0 = frame.origin[1] + ty as f64 * frame.tile_mm;
                ranges.push([
                    router_tile(x0 + epsilon, 0, self.tiles_x),
                    router_tile(y0 + epsilon, 1, self.tiles_y),
                    router_tile(x0 + frame.tile_mm - epsilon, 0, self.tiles_x),
                    router_tile(y0 + frame.tile_mm - epsilon, 1, self.tiles_y),
                ]);
            }
        }
        self.capacitated = Some(Capacitated {
            plan,
            frame,
            ranges,
            charged: 0,
        });
        note(Learn::PlansBuilt, 1);
        let expansions = self.charge_plan();
        note(Learn::PlanExpansions, expansions);
    }

    /// Charges the plan's work since the last charge to the search's
    /// expansions (every budget counts them); returns the expansions.
    fn charge_plan(&mut self) -> u64 {
        let Some(state) = self.capacitated.as_mut() else {
            return 0;
        };
        let units = state.plan.work_units();
        let expansions = |units: u64| (units as f64 * UNIT_EXPANSIONS) as u64;
        let added = expansions(units) - expansions(state.charged);
        state.charged = units;
        self.scratch.expansions += added;
        added
    }

    /// How `net`'s search goes now (`CapacitatedStatus`).
    pub(super) fn capacitated_status(
        &self,
        net: NetId,
        net_state: &NetState,
        hard: bool,
    ) -> CapacitatedStatus {
        let Some(state) = self.capacitated.as_ref() else {
            return CapacitatedStatus::Off;
        };
        if hard || self.cleanup {
            return CapacitatedStatus::Off;
        }
        let net = net as usize;
        if !state.plan.planned(net) {
            return CapacitatedStatus::Unplanned;
        }
        if net_state.capacitated_fallback || !state.plan.routed(net) {
            return CapacitatedStatus::Fallback;
        }
        let frame = state.frame;
        let tiles = frame.tiles();
        // One plan tile around the route; with `capacitated_growth`, one
        // more every that many reroutes (as the tile planner widens).
        let margin = match self.config.capacitated_growth {
            0 => 1,
            every => (1 + net_state.reroutes / every).min(10),
        };
        let mut marked = vec![false; tiles];
        for state_index in state.plan.states(net) {
            let tile = *state_index as usize % tiles;
            let (x, y) = (tile % frame.tiles_x, tile / frame.tiles_x);
            for ty in y.saturating_sub(margin)..=(y + margin).min(frame.tiles_y - 1) {
                for tx in x.saturating_sub(margin)..=(x + margin).min(frame.tiles_x - 1) {
                    marked[ty * frame.tiles_x + tx] = true;
                }
            }
        }
        let mut corridor = vec![false; self.tiles_x * self.tiles_y];
        for (tile, inside) in marked.iter().enumerate() {
            if !inside {
                continue;
            }
            let [x0, y0, x1, y1] = state.ranges[tile];
            for ry in y0..=y1 {
                for rx in x0..=x1 {
                    corridor[ry as usize * self.tiles_x + rx as usize] = true;
                }
            }
        }
        CapacitatedStatus::Band(corridor)
    }

    /// Per lattice column and row: the plan tile's column and row.
    fn plan_columns(&self, frame: &PlanFrame) -> (Vec<u32>, Vec<u32>) {
        let along = |count: usize, axis: usize, tiles: usize| -> Vec<u32> {
            (0..count)
                .map(|index| {
                    let mm = self.grid.origin[axis] + index as f64 * self.grid.pitch;
                    (((mm - frame.origin[axis]) / frame.tile_mm).floor().max(0.0) as usize)
                        .min(tiles - 1) as u32
                })
                .collect()
        };
        (
            along(self.grid.nx, 0, frame.tiles_x),
            along(self.grid.ny, 1, frame.tiles_y),
        )
    }

    /// After a search inside `band` found nothing: the boundaries and via
    /// tiles of `net`'s global route between the plan tiles the search
    /// reached (its closed nodes, per layer) and those it did not.
    #[allow(
        clippy::needless_range_loop,
        reason = "a router tile's cells by row and column, the plan's columns and rows parallel"
    )]
    pub(super) fn capacitated_miss(&self, scratch: &Scratch, net: NetId, band: &[bool]) -> Miss {
        let Some(state) = self.capacitated.as_ref() else {
            return Miss::default();
        };
        let frame = state.frame;
        let tiles = frame.tiles();
        let layers = self.board.layer_count.min(frame.layers);
        let cells = self.grid.cells();
        let (nx, ny) = (self.grid.nx, self.grid.ny);
        let generation = scratch.generation;
        let (columns, rows) = self.plan_columns(&frame);
        let mut reached = vec![false; tiles * frame.layers];
        for (router_tile, inside) in band.iter().enumerate() {
            if !inside {
                continue;
            }
            let (rx, ry) = (router_tile % self.tiles_x, router_tile / self.tiles_x);
            for y in ry * TILE..((ry + 1) * TILE).min(ny) {
                for x in rx * TILE..((rx + 1) * TILE).min(nx) {
                    let cell = y * nx + x;
                    let tile = rows[y] as usize * frame.tiles_x + columns[x] as usize;
                    for layer in 0..layers {
                        if scratch.nodes[layer * cells + cell].closed(generation) {
                            reached[layer * tiles + tile] = true;
                        }
                    }
                }
            }
        }
        let net = net as usize;
        let per = frame.layers * tiles;
        let mut miss = Miss::default();
        for &boundary in state.plan.crossings(net) {
            let (direction, rest) = (boundary as usize / per, boundary as usize % per);
            let (layer, tile) = (rest / tiles, rest % tiles);
            let other = if direction == 0 {
                tile + 1
            } else {
                tile + frame.tiles_x
            };
            if layer < layers && other < tiles && reached[rest] != reached[layer * tiles + other] {
                miss.boundaries.push(boundary);
            }
        }
        for &tile in state.plan.vias(net) {
            let tile = tile as usize;
            let on: Vec<bool> = state
                .plan
                .states(net)
                .iter()
                .map(|state| *state as usize)
                .filter(|state| state % tiles == tile && state / tiles < layers)
                .map(|state| reached[state])
                .collect();
            if on.iter().any(|value| *value) && on.iter().any(|value| !*value) {
                miss.vias.push(tile as u32);
            }
        }
        miss.boundaries.sort_unstable();
        miss.boundaries.dedup();
        miss.vias.sort_unstable();
        miss.vias.dedup();
        miss
    }

    /// The tracks (in reference tracks, each net's demand) the routed
    /// copper puts across each of `boundaries`, and the vias it puts in
    /// each of `via_tiles`: the floors the learning keeps.
    fn routed_through(
        &mut self,
        boundaries: &[u32],
        via_tiles: &[u32],
    ) -> (FastMap<u32, f32>, FastMap<u32, f32>) {
        let mut across: FastMap<u32, f32> = boundaries.iter().map(|b| (*b, 0.0)).collect();
        let mut vias: FastMap<u32, f32> = via_tiles.iter().map(|t| (*t, 0.0)).collect();
        let Some(state) = self.capacitated.as_ref() else {
            return (across, vias);
        };
        let frame = state.frame;
        let tiles = frame.tiles();
        let per = frame.layers * tiles;
        let (columns, rows) = self.plan_columns(&frame);
        let nx = self.grid.nx;
        let mut visited = 0u64;
        for (net, net_state) in self.nets.iter().enumerate() {
            if net_state.branches.is_empty() {
                continue;
            }
            let demand = state.plan.demand(net);
            for branch in &net_state.branches {
                visited += branch.nodes.len() as u64;
                for pair in branch.nodes.windows(2) {
                    let (a, b) = (pair[0], pair[1]);
                    let (ax, ay) = (a.cell as usize % nx, a.cell as usize / nx);
                    if a.cell == b.cell {
                        let tile =
                            (rows[ay] as usize * frame.tiles_x + columns[ax] as usize) as u32;
                        if let Some(count) = vias.get_mut(&tile) {
                            *count += 1.0;
                        }
                        continue;
                    }
                    if a.layer != b.layer || a.layer as usize >= frame.layers {
                        continue;
                    }
                    let (bx, by) = (b.cell as usize % nx, b.cell as usize / nx);
                    let (cax, cay) = (columns[ax] as usize, rows[ay] as usize);
                    let (cbx, cby) = (columns[bx] as usize, rows[by] as usize);
                    let layer = a.layer as usize;
                    if cax != cbx {
                        let boundary = (layer * tiles + cay * frame.tiles_x + cax.min(cbx)) as u32;
                        if let Some(count) = across.get_mut(&boundary) {
                            *count += demand;
                        }
                    }
                    if cay != cby {
                        let boundary =
                            (per + layer * tiles + cay.min(cby) * frame.tiles_x + cbx) as u32;
                        if let Some(count) = across.get_mut(&boundary) {
                            *count += demand;
                        }
                    }
                }
            }
        }
        // A step costs a fraction of a search expansion (no heap, no maps).
        let expansions = visited / 16;
        self.scratch.expansions += expansions;
        note(Learn::FloorExpansions, expansions);
        (across, vias)
    }

    /// Lowers the boundaries and via sites of each entry of `wanted` by
    /// one (each once, not below its floor). Returns per entry whether any
    /// of its own was lowered (or had been by an entry before), and the
    /// boundaries and via tiles lowered.
    fn lower(
        &mut self,
        wanted: &[(Vec<u32>, Vec<u32>)],
        conflicts: bool,
    ) -> (Vec<bool>, Vec<u32>, Vec<u32>) {
        let all_boundaries: Vec<u32> = wanted.iter().flat_map(|w| w.0.clone()).collect();
        let all_vias: Vec<u32> = wanted.iter().flat_map(|w| w.1.clone()).collect();
        let (floors, via_floors) = self.routed_through(&all_boundaries, &all_vias);
        let mut lowered: Vec<u32> = Vec::new();
        let mut lowered_vias: Vec<u32> = Vec::new();
        let mut learned = Vec::with_capacity(wanted.len());
        let Some(state) = self.capacitated.as_mut() else {
            return (vec![false; wanted.len()], lowered, lowered_vias);
        };
        let reduction = if conflicts {
            Learn::ConflictReductions
        } else {
            Learn::Reductions
        };
        for (boundaries, vias) in wanted {
            let mut any = false;
            for boundary in boundaries {
                if lowered.contains(boundary) {
                    any = true;
                } else if state.plan.lower(*boundary, floors[boundary]) {
                    lowered.push(*boundary);
                    note(reduction, 1);
                    any = true;
                } else {
                    note(Learn::FloorHeld, 1);
                }
            }
            for tile in vias {
                if lowered_vias.contains(tile) {
                    any = true;
                } else if state.plan.lower_via(*tile, via_floors[tile]) {
                    lowered_vias.push(*tile);
                    note(reduction, 1);
                    any = true;
                } else {
                    note(Learn::FloorHeld, 1);
                }
            }
            learned.push(any);
        }
        (learned, lowered, lowered_vias)
    }

    /// Plans the nets through `lowered` and `lowered_vias` again, then
    /// `last` (the nets that taught the plan), in that order.
    fn replan_after(&mut self, lowered: &[u32], lowered_vias: &[u32], last: &[usize]) {
        let Some(state) = self.capacitated.as_mut() else {
            return;
        };
        if lowered.is_empty() && lowered_vias.is_empty() && last.is_empty() {
            return;
        }
        let mut nets = state.plan.nets_through(lowered, lowered_vias);
        nets.retain(|net| !last.contains(net));
        nets.extend_from_slice(last);
        state.plan.replan(&nets);
        note(Learn::Replans, 1);
        note(Learn::ReplannedNets, nets.len() as u64);
        let expansions = self.charge_plan();
        note(Learn::ReplanExpansions, expansions);
    }

    /// After a batch: its nets that missed their capacitated corridor
    /// teach the plan (`lower`, `replan_after`) and are routed again in their
    /// new corridors, up to `capacitated_retries` rounds; a net nothing
    /// could be learned from is routed again through the window and the
    /// board (`NetState::capacitated_fallback`).
    pub(super) fn learn_from_misses(&mut self, batch: &[NetId], present: f32, growth: f64) {
        if self.capacitated.is_none() {
            return;
        }
        let retries = self.config.capacitated_retries;
        for round in 0..=retries {
            let missed: Vec<(NetId, Miss)> = batch
                .iter()
                .filter_map(|net| {
                    self.nets[*net as usize]
                        .capacitated_miss
                        .take()
                        .map(|miss| (*net, miss))
                })
                .collect();
            if missed.is_empty() {
                return;
            }
            if round == retries {
                note(Learn::Deferred, missed.len() as u64);
                return;
            }
            for (_, miss) in &missed {
                if miss.boundaries.is_empty() && miss.vias.is_empty() {
                    note(Learn::NoBoundary, 1);
                }
            }
            let wanted: Vec<(Vec<u32>, Vec<u32>)> = missed
                .iter()
                .map(|(_, miss)| (miss.boundaries.clone(), miss.vias.clone()))
                .collect();
            let (learned, lowered, lowered_vias) = self.lower(&wanted, false);
            let learners: Vec<usize> = missed
                .iter()
                .zip(&learned)
                .filter(|(_, learned)| **learned)
                .map(|((net, _), _)| *net as usize)
                .collect();
            if !learners.is_empty() {
                self.replan_after(&lowered, &lowered_vias, &learners);
            }
            for ((net, _), learned) in missed.iter().zip(&learned) {
                if !learned {
                    self.nets[*net as usize].capacitated_fallback = true;
                    note(Learn::FallbackNothingLearned, 1);
                }
                self.route_net_seq(*net, present, false, growth);
                self.stamp(*net);
                note(Learn::Retries, 1);
            }
        }
    }

    /// At an iteration's end (every second): nets in conflict for
    /// `capacitated_stuck` iterations running teach the plan where their
    /// conflicts are: the boundary of their global route with the most
    /// conflict cells in its two tiles loses a track, and the nets through
    /// it and those nets are planned again.
    pub(super) fn learn_from_conflicts(&mut self, conflicted: &[NetId]) {
        let stuck = self.config.capacitated_stuck;
        let Some(state) = self.capacitated.as_ref() else {
            return;
        };
        if stuck == 0 {
            return;
        }
        let frame = state.frame;
        let tiles = frame.tiles();
        let per = frame.layers * tiles;
        let (columns, rows) = self.plan_columns(&frame);
        let nx = self.grid.nx;
        let mut wanted: Vec<(Vec<u32>, Vec<u32>)> = Vec::new();
        let mut teachers: Vec<usize> = Vec::new();
        for net in conflicted {
            let index = *net as usize;
            if self.nets[index].stuck < stuck
                || !state.plan.planned(index)
                || !state.plan.routed(index)
            {
                continue;
            }
            let mut hits: FastMap<usize, u32> = FastMap::default();
            for (layer, cell) in self.conflicts(*net) {
                if layer >= frame.layers {
                    continue;
                }
                let (x, y) = (cell as usize % nx, cell as usize / nx);
                let tile = rows[y] as usize * frame.tiles_x + columns[x] as usize;
                *hits.entry(layer * tiles + tile).or_default() += 1;
            }
            let mut best: Option<(u32, u32)> = None;
            for &boundary in state.plan.crossings(index) {
                let rest = boundary as usize % per;
                let (layer, tile) = (rest / tiles, rest % tiles);
                let other = if boundary as usize / per == 0 {
                    tile + 1
                } else {
                    tile + frame.tiles_x
                };
                let score = hits.get(&rest).copied().unwrap_or(0)
                    + hits.get(&(layer * tiles + other)).copied().unwrap_or(0);
                if score > 0 && best.is_none_or(|(kept, _)| score > kept) {
                    best = Some((score, boundary));
                }
            }
            if let Some((_, boundary)) = best {
                wanted.push((vec![boundary], Vec::new()));
                teachers.push(index);
            }
        }
        if wanted.is_empty() {
            return;
        }
        let (learned, lowered, lowered_vias) = self.lower(&wanted, true);
        let teachers: Vec<usize> = teachers
            .into_iter()
            .zip(learned)
            .filter(|(_, learned)| *learned)
            .map(|(net, _)| net)
            .collect();
        if !teachers.is_empty() {
            self.replan_after(&lowered, &lowered_vias, &teachers);
        }
    }
}
