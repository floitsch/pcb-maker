// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! A net's copper on the shared occupancy: stamping, rip-up, conflicts and the masks of a net's own stamps.

use super::*;

impl Router {
    pub(super) fn unstamp(&mut self, net: NetId) {
        let state = &mut self.nets[net as usize];
        for (map, cell) in state.stamped.drain(..) {
            self.occupancy[map as usize][cell as usize] -= 1;
            if self.occupancy[map as usize][cell as usize] == 0 {
                let cell = cell as usize;
                let tile =
                    (cell / self.grid.nx / TILE) * self.tiles_x + (cell % self.grid.nx) / TILE;
                self.tile_claimed[map as usize][tile] -= 1;
            }
        }
    }

    /// Removes the net's copper but its fixed branches, which stay stamped:
    /// a stub or skeleton must hold its ground while other nets are routed
    /// against it, whether or not its net is ripped up at the time.
    pub(super) fn rip_up(&mut self, net: NetId) {
        let state = &mut self.nets[net as usize];
        let fixed = state.fixed;
        state.branches.truncate(fixed);
        state.connected.fill(false);
        state.complete = false;
        self.stamp(net);
    }

    /// Whether a rip-up of `net` takes every branch: every `STUCK_WHOLE`th
    /// iteration of a net stuck in conflict. The branches a partial rip-up
    /// keeps may be what leaves the other net no way through.
    pub(super) fn rip_up_whole(&self, net: NetId) -> bool {
        const STUCK_WHOLE: usize = 8;
        let stuck = self.nets[net as usize].stuck;
        // A pour net ripped up whole loses the vias that join its layers'
        // fill: the next refresh finds its pads scattered over a dozen
        // pieces, cuts them off from the main piece and routes them as
        // tracks (Sisu's GND every eighth iteration: 217 pads in the main
        // piece, then 35, then 261 again, about 100 pads cut off each time).
        if !self.nets[net as usize].plane.is_empty() && !self.config.pour_whole_rip_up {
            return false;
        }
        stuck > 0 && stuck.is_multiple_of(STUCK_WHOLE)
    }

    /// Removes only the branches of `net` that are in conflict, plus any
    /// branch left dangling at a junction on a removed branch.
    pub(super) fn rip_up_conflicted(&mut self, net: NetId) {
        let keep = self.unconflicted_branches(net);
        let mut keep = keep.into_iter();
        let state = &mut self.nets[net as usize];
        state.branches.retain(|_| keep.next().unwrap());
        state.connected.fill(false);
        state.complete = false;
        // Kept branches stay stamped and are masked in the net's own search.
        self.stamp(net);
    }

    /// Which branches of `net` are free of conflict and not left dangling
    /// at a junction on a conflicted branch (the skeleton always is).
    pub(super) fn unconflicted_branches(&self, net: NetId) -> Vec<bool> {
        let branches = &self.nets[net as usize].branches;
        let whole = self.rip_up_whole(net);
        let mut found = Vec::new();
        let mut keep: Vec<bool> = branches
            .iter()
            .map(|branch| {
                found.clear();
                self.branch_conflicts(net, branch, &mut found);
                found.is_empty() && !whole
            })
            .collect();
        for flag in keep.iter_mut().take(self.nets[net as usize].fixed) {
            *flag = true;
        }
        loop {
            let hosts = junction_hosts(branches, &keep);
            let mut changed = false;
            for (index, branch) in branches.iter().enumerate() {
                if !keep[index] {
                    continue;
                }
                let dangling = (branch.start_terminal == NO_TERMINAL
                    && !hosts.contains_key(&branch.nodes[0]))
                    || (branch.end_terminal == NO_TERMINAL
                        && !hosts.contains_key(branch.nodes.last().unwrap()));
                if dangling {
                    keep[index] = false;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        keep
    }

    pub(super) fn stamp(&mut self, net: NetId) {
        // A full restamp: the new list is built first (marked, counts left
        // alone), then only the stamps that went or came change their
        // counts. The counts end as with the old list unstamped and the new
        // one stamped, without touching the stamps a net keeps (most of
        // them, when only a branch was rerouted).
        let old = std::mem::take(&mut self.nets[net as usize].stamped);
        let class = self.board.nets[net as usize].class;
        let layers = self.board.layer_count;
        let mut stamped = Vec::with_capacity(old.len());
        let nx = self.grid.nx as i64;
        let ny = self.grid.ny as i64;
        let branches = std::mem::take(&mut self.nets[net as usize].branches);
        let escapes = std::mem::take(&mut self.nets[net as usize].escapes);
        // Each node's class (narrower in a neck zone), all branches in one
        // list.
        let mut node_classes: Vec<usize> =
            Vec::with_capacity(branches.iter().map(|branch| branch.nodes.len()).sum());
        for branch in &branches {
            node_classes.extend(
                branch
                    .nodes
                    .iter()
                    .map(|node| self.node_class(net, node.cell)),
            );
        }
        let diagonal_base = self.board.classes.len() * (layers + 1);
        let size = (nx, ny);
        let words = self.grid.cells().div_ceil(64);
        let class_rules = &self.class_rules;
        let variant_of = |layer: usize| {
            class_rules
                .as_ref()
                .map_or(0, |split| split.resolved.layer_variant[layer])
        };
        for querying in 0..self.board.classes.len() {
            let marks = &mut self.stamp_mark;
            let base = querying * (layers + 1);
            let mut first = 0;
            for branch in &branches {
                let classes = &node_classes[first..first + branch.nodes.len()];
                first += branch.nodes.len();
                for end in [branch.nodes[0], *branch.nodes.last().unwrap()] {
                    if let Some((cells, width, _)) = escapes.get(&end) {
                        // A stub at the neck width stamps the neck's footprint.
                        let stub_class = match self.neck_of[class] {
                            Some(neck)
                                if *width < self.board.classes[class].trace_width - 1.0e-9 =>
                            {
                                neck
                            }
                            _ => class,
                        };
                        let stub_stamps = match &self.class_rules {
                            None => &self.stamps[stub_class][querying],
                            Some(split) => {
                                &self.stamps[stub_class][split.resolved.layer_variant
                                    [end.layer as usize]
                                    * self.board.classes.len()
                                    + querying]
                            }
                        };
                        let reach = stub_stamps.fresh.reach;
                        for cell in cells {
                            let map = base + end.layer as usize;
                            mark_footprint(
                                marks,
                                words,
                                &mut stamped,
                                map,
                                *cell,
                                &stub_stamps.stub_to_trace,
                                (1, 1),
                                size,
                                reach,
                            );
                            let map = base + layers;
                            mark_footprint(
                                marks,
                                words,
                                &mut stamped,
                                map,
                                *cell,
                                &stub_stamps.stub_to_via,
                                (1, 1),
                                size,
                                reach,
                            );
                            for orientation in 0..2 {
                                let map = diagonal_base
                                    + (querying * layers + end.layer as usize) * 2
                                    + orientation;
                                mark_footprint(
                                    marks,
                                    words,
                                    &mut stamped,
                                    map,
                                    *cell,
                                    &stub_stamps.stub_diagonal_block[orientation],
                                    (1, 1),
                                    size,
                                    reach,
                                );
                            }
                        }
                    }
                }
                for (index, node) in branch.nodes.iter().enumerate() {
                    // A node in a neck zone stamps its narrower footprint.
                    let node_class = classes[index];
                    let variant = variant_of(node.layer as usize);
                    let own =
                        &self.stamps[node_class][variant * self.board.classes.len() + querying];
                    let reach = own.fresh.reach;
                    let x = node.cell as i64 % nx;
                    let y = node.cell as i64 / nx;
                    // The step from the node before, when that node stamped
                    // the same footprints: only their leading edge is new.
                    let mut step = None;
                    let mut diagonal = None;
                    let mut is_via = false;
                    let mut via_before = false;
                    if index > 0 {
                        let previous = branch.nodes[index - 1];
                        let (dx, dy) =
                            (x - previous.cell as i64 % nx, y - previous.cell as i64 / nx);
                        is_via = previous.cell == node.cell && previous.layer != node.layer;
                        // (A step from a layer of another stamp variant
                        // stamps whole footprints.)
                        if classes[index - 1] == node_class
                            && dx.abs() <= 1
                            && dy.abs() <= 1
                            && variant_of(previous.layer as usize) == variant
                        {
                            step = Some(((dx + 1) * 3 + dy + 1) as usize);
                        }
                        if !is_via && dx != 0 && dy != 0 {
                            diagonal = Some((dx, dy));
                        }
                        via_before = index > 1
                            && previous.cell == node.cell
                            && branch.nodes[index - 2].cell == previous.cell
                            && branch.nodes[index - 2].layer != previous.layer;
                    }
                    let same_layer = index > 0 && branch.nodes[index - 1].layer == node.layer;
                    let map = base + node.layer as usize;
                    let offsets = match step {
                        Some(step) if same_layer => &own.fresh.trace_to_trace[step],
                        _ => &own.trace_to_trace,
                    };
                    mark_footprint(
                        marks,
                        words,
                        &mut stamped,
                        map,
                        node.cell,
                        offsets,
                        (1, 1),
                        size,
                        reach,
                    );
                    let map = base + layers;
                    let offsets = match step {
                        Some(step) => &own.fresh.trace_to_via[step],
                        None => &own.trace_to_via,
                    };
                    mark_footprint(
                        marks,
                        words,
                        &mut stamped,
                        map,
                        node.cell,
                        offsets,
                        (1, 1),
                        size,
                        reach,
                    );
                    for orientation in 0..2 {
                        let map = diagonal_base
                            + (querying * layers + node.layer as usize) * 2
                            + orientation;
                        let offsets = match step {
                            Some(step) if same_layer => {
                                &own.fresh.diagonal_block[orientation][step]
                            }
                            _ => &own.diagonal_block[orientation],
                        };
                        mark_footprint(
                            marks,
                            words,
                            &mut stamped,
                            map,
                            node.cell,
                            offsets,
                            (1, 1),
                            size,
                            reach,
                        );
                    }
                    // A via through several layers at one cell: the via
                    // before stamped the same footprints at the same cell.
                    if is_via && !via_before {
                        for layer in 0..layers {
                            let stamps = match &self.class_rules {
                                None => &self.stamps[class][querying],
                                Some(split) => {
                                    &self.stamps[class][split.resolved.layer_variant[layer]
                                        * self.board.classes.len()
                                        + querying]
                                }
                            };
                            let map = base + layer;
                            mark_footprint(
                                marks,
                                words,
                                &mut stamped,
                                map,
                                node.cell,
                                &stamps.via_to_trace,
                                (1, 1),
                                size,
                                stamps.fresh.reach,
                            );
                        }
                        // Via to via is the largest of every layer's.
                        let stamps = &self.stamps[class][querying];
                        let reach = stamps.fresh.reach;
                        let map = base + layers;
                        mark_footprint(
                            marks,
                            words,
                            &mut stamped,
                            map,
                            node.cell,
                            &stamps.via_to_via,
                            (1, 1),
                            size,
                            reach,
                        );
                        for layer in 0..layers {
                            let stamps = match &self.class_rules {
                                None => &self.stamps[class][querying],
                                Some(split) => {
                                    &self.stamps[class][split.resolved.layer_variant[layer]
                                        * self.board.classes.len()
                                        + querying]
                                }
                            };
                            for orientation in 0..2 {
                                let map =
                                    diagonal_base + (querying * layers + layer) * 2 + orientation;
                                mark_footprint(
                                    marks,
                                    words,
                                    &mut stamped,
                                    map,
                                    node.cell,
                                    &stamps.diagonal_block_via[orientation],
                                    (1, 1),
                                    size,
                                    stamps.fresh.reach,
                                );
                            }
                        }
                    }
                    // A diagonal step covers the nodes on its bisector too
                    // (the bisectors of a diagonal run are parallel lines
                    // that share no cell: no leading edge to take).
                    if let Some(step) = diagonal {
                        let previous = branch.nodes[index - 1];
                        let map = base + node.layer as usize;
                        mark_footprint(
                            marks,
                            words,
                            &mut stamped,
                            map,
                            previous.cell,
                            &own.diagonal_to_trace,
                            step,
                            size,
                            reach,
                        );
                        let map = base + layers;
                        mark_footprint(
                            marks,
                            words,
                            &mut stamped,
                            map,
                            previous.cell,
                            &own.diagonal_to_via,
                            step,
                            size,
                            reach,
                        );
                    }
                }
            }
        }
        // An old stamp still marked is a new one too: its count stays, and
        // its mark goes so that the new list passes it over; the others
        // go (the count down, the tile's claim with the last of a cell).
        // The count a stamp AHEAD entries on is asked for early: the maps
        // are larger than the core's cache and a list visits dozens of them.
        let nxu = self.grid.nx;
        const AHEAD: usize = 32;
        for (index, (map, cell)) in old.iter().enumerate() {
            if let Some((map, cell)) = old.get(index + AHEAD) {
                prefetch(&self.occupancy[*map as usize][*cell as usize]);
            }
            let (map, cell) = (*map as usize, *cell as usize);
            let bit = 1u64 << (cell % 64);
            if self.stamp_mark[map * words + cell / 64] & bit != 0 {
                self.stamp_mark[map * words + cell / 64] &= !bit;
                continue;
            }
            self.occupancy[map][cell] -= 1;
            if self.occupancy[map][cell] == 0 {
                self.tile_claimed[map][(cell / nxu / TILE) * self.tiles_x + (cell % nxu) / TILE] -=
                    1;
            }
        }
        // A new stamp still marked was not stamped before: it counts.
        for (index, (map, cell)) in stamped.iter().enumerate() {
            if let Some((map, cell)) = stamped.get(index + AHEAD) {
                prefetch(&self.occupancy[*map as usize][*cell as usize]);
            }
            let (map, cell) = (*map as usize, *cell as usize);
            let bit = 1u64 << (cell % 64);
            if self.stamp_mark[map * words + cell / 64] & bit == 0 {
                continue;
            }
            self.stamp_mark[map * words + cell / 64] &= !bit;
            self.occupancy[map][cell] += 1;
            if self.occupancy[map][cell] == 1 {
                self.tile_claimed[map][(cell / nxu / TILE) * self.tiles_x + (cell % nxu) / TILE] +=
                    1;
            }
        }
        let state = &mut self.nets[net as usize];
        state.branches = branches;
        state.escapes = escapes;
        state.stamped = stamped;
    }

    /// Nodes of `net` that lie inside another net's clearance zone.
    pub(super) fn conflicts(&self, net: NetId) -> Vec<(usize, u32)> {
        let mut result = Vec::new();
        // Skeleton branches are fixed: an overlap is the other net's conflict.
        for branch in self.nets[net as usize]
            .branches
            .iter()
            .skip(self.nets[net as usize].fixed)
        {
            self.branch_conflicts(net, branch, &mut result);
        }
        result
    }

    /// The cells of `branch` that another net's clearance zone covers
    /// (`(layer, cell)`, the layer count standing for the via map): its
    /// escape stubs, its nodes and its vias. Another net's diagonal step
    /// passing a node too closely shows here as well, through the stamps
    /// of that step's bisector. The one judge of a conflict: what rips a
    /// branch up must be what the negotiation counts.
    pub(super) fn branch_conflicts(
        &self,
        net: NetId,
        branch: &Branch,
        result: &mut Vec<(usize, u32)>,
    ) {
        let class = self.board.nets[net as usize].class;
        let layers = self.board.layer_count;
        let state = &self.nets[net as usize];
        for end in [branch.nodes[0], *branch.nodes.last().unwrap()] {
            if let Some((cells, width, _)) = state.escapes.get(&end) {
                let stub_class = match self.neck_of[class] {
                    Some(neck) if *width < self.board.classes[class].trace_width - 1.0e-9 => neck,
                    _ => class,
                };
                let map = self.map_index(stub_class, end.layer as usize);
                for cell in cells {
                    if self.occupancy[map][*cell as usize] > 1 {
                        result.push((end.layer as usize, *cell));
                    }
                }
            }
        }
        for (index, node) in branch.nodes.iter().enumerate() {
            let map = self.map_index(self.node_class(net, node.cell), node.layer as usize);
            if self.occupancy[map][node.cell as usize] > 1 {
                result.push((node.layer as usize, node.cell));
            }
            let is_via = index > 0
                && branch.nodes[index - 1].cell == node.cell
                && branch.nodes[index - 1].layer != node.layer;
            if is_via && self.occupancy[self.map_index(class, layers)][node.cell as usize] > 1 {
                result.push((layers, node.cell));
            }
        }
    }

    /// Where a map of `net`'s own class (0) or neck class (1) is marked in
    /// `own_mark`: `(which, kind)`, the kind being the layer (or the via
    /// map, or a diagonal map's layer and orientation) within that class.
    pub(super) fn own_kind(&self, class: usize, map: usize) -> Option<(usize, usize)> {
        let layers = self.board.layer_count;
        let diagonal_base = self.board.classes.len() * (layers + 1);
        let (of_class, kind) = if map < diagonal_base {
            (map / (layers + 1), map % (layers + 1))
        } else {
            let relative = map - diagonal_base;
            (
                relative / (2 * layers),
                layers + 1 + relative % (2 * layers),
            )
        };
        let which = if of_class == class {
            0
        } else if Some(of_class) == self.neck_of[class] {
            1
        } else {
            return None;
        };
        Some((which, kind))
    }

    /// The occupancy of a cell as `net`'s own search sees it: without the
    /// net's own stamps when they are still in place (`mask_own`).
    #[inline(always)]
    pub(super) fn occupancy_seen(
        &self,
        scratch: &Scratch,
        class: usize,
        map: usize,
        cell: usize,
    ) -> u16 {
        let value = self.occupancy[map][cell];
        if !scratch.own_active || value == 0 {
            return value;
        }
        let Some((which, kind)) = self.own_kind(class, map) else {
            return value;
        };
        let kinds = 3 * self.board.layer_count + 1;
        let bit = (which * kinds + kind) * self.grid.cells() + cell;
        if scratch.own_mark[bit / 64] & (1 << (bit % 64)) != 0 {
            value - 1
        } else {
            value
        }
    }

    /// Marks the net's own stamps in `scratch.own_mark` so that its search
    /// subtracts them from the occupancy it sees (see `own_kind` for the
    /// layout). The caller calls `Scratch::unmask_own` afterwards.
    pub(super) fn mask_own(&self, scratch: &mut Scratch, net: NetId, net_state: &NetState) {
        let layers = self.board.layer_count;
        let cells = self.grid.cells();
        let kinds = 3 * layers + 1;
        let words = (2 * kinds * cells).div_ceil(64);
        if scratch.own_mark.len() != words {
            scratch.own_mark = vec![0; words];
            scratch.own_set.clear();
        }
        scratch.unmask_own();
        scratch.own_active = true;
        let class = self.board.nets[net as usize].class;
        let tiles = self.tiles_x * self.tiles_y;
        scratch.own_claimed.clear();
        scratch.own_claimed.resize(layers * tiles, 0);
        let class_base = class * (layers + 1);
        for (map, cell) in &net_state.stamped {
            let map = *map as usize;
            // The corridor planner's tile fill: cells only this net claims.
            if map >= class_base
                && map < class_base + layers
                && self.occupancy[map][*cell as usize] == 1
            {
                let tile = (*cell as usize / self.grid.nx / TILE) * self.tiles_x
                    + (*cell as usize % self.grid.nx) / TILE;
                scratch.own_claimed[(map - class_base) * tiles + tile] += 1;
            }
            if let Some((which, kind)) = self.own_kind(class, map) {
                let bit = (which * kinds + kind) * cells + *cell as usize;
                scratch.own_mark[bit / 64] |= 1 << (bit % 64);
                scratch.own_set.push(bit);
            }
        }
    }
}
