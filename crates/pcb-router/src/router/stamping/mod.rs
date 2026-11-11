// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! A net's copper on the shared occupancy: stamping (`stamp`), rip-up,
//! conflicts and the masks of a net's own stamps.

use super::*;

mod stamp;

impl Router {
    pub(super) fn unstamp(&mut self, net: NetId) {
        self.copper_changes += 1;
        let state = &mut self.nets[net as usize];
        let stamped = std::mem::take(&mut state.stamped);
        for (map, cell) in stamped.iter() {
            if self.occupancy.decrement(map as usize, cell as usize) {
                let cell = cell as usize;
                let tile =
                    (cell / self.grid.nx / TILE) * self.tiles_x + (cell % self.grid.nx) / TILE;
                self.tile_claimed[map as usize][tile] -= 1;
            }
        }
    }

    /// Counts `stamped` on the occupancy (as `stamp` counts a new list).
    pub(super) fn add_stamps(&mut self, stamped: &Stamped) {
        for (map, cell) in stamped.iter() {
            let (map, cell) = (map as usize, cell as usize);
            if self.occupancy.increment(map, cell) {
                let tile =
                    (cell / self.grid.nx / TILE) * self.tiles_x + (cell % self.grid.nx) / TILE;
                self.tile_claimed[map][tile] += 1;
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
        self.branch_conflicts_with(net, branch, &mut |layer, _, cell| {
            result.push((layer, cell))
        });
    }

    /// The conflicts of `net` (`conflicts`) by the occupancy map that
    /// counts each: `(map, cell)`, so that the nets stamping them can be
    /// named (the trouble repair's `blockers`). A stub's cells are in its
    /// stub's class map (the neck class where the stub is narrower than
    /// the track), which the node's class at that cell need not be.
    pub(super) fn conflict_maps(&self, net: NetId) -> Vec<(u32, u32)> {
        let mut result = Vec::new();
        for branch in self.nets[net as usize]
            .branches
            .iter()
            .skip(self.nets[net as usize].fixed)
        {
            self.branch_conflicts_with(net, branch, &mut |_, map, cell| {
                result.push((map as u32, cell))
            });
        }
        result
    }

    /// `branch_conflicts`, each conflict given to `found` as its layer
    /// (the layer count for the via map), the occupancy map that counts
    /// it and its cell.
    fn branch_conflicts_with(
        &self,
        net: NetId,
        branch: &Branch,
        found: &mut dyn FnMut(usize, usize, u32),
    ) {
        let class = self.board.nets[net as usize].class;
        let layers = self.board.layer_count;
        let state = &self.nets[net as usize];
        for end in [branch.nodes[0], *branch.nodes.last().unwrap()] {
            if let Some((cells, width, _)) = state.escapes.get(&end) {
                let stub_class = match self.neck_of[class] {
                    Some(neck) if *width < self.width_on(class, end.layer as usize) - 1.0e-9 => {
                        neck
                    }
                    _ => class,
                };
                let map = self.map_index(stub_class, end.layer as usize);
                for cell in cells {
                    if self.occupancy.get(map, *cell as usize) > 1 {
                        found(end.layer as usize, map, *cell);
                    }
                }
            }
        }
        for (index, node) in branch.nodes.iter().enumerate() {
            let map = self.map_index(self.node_class(net, node.cell), node.layer as usize);
            if self.occupancy.get(map, node.cell as usize) > 1 {
                found(node.layer as usize, map, node.cell);
            }
            let is_via = index > 0
                && branch.nodes[index - 1].cell == node.cell
                && branch.nodes[index - 1].layer != node.layer;
            let via_map = self.map_index(class, layers);
            if !is_via {
                continue;
            }
            match &self.spans {
                // A via of a span meets other copper on its layers only.
                Some(spans) if spans.takes_spans(net) => {
                    let pierced = spans.chain_mask(net, chain_layers(&branch.nodes, index));
                    if pierced == spans.all() {
                        if self.occupancy.get(via_map, node.cell as usize) > 1 {
                            found(layers, via_map, node.cell);
                        }
                        continue;
                    }
                    for layer in 0..layers {
                        let map = self.via_layer_map(spans, class, layer);
                        if pierced & (1 << layer) != 0
                            && self.occupancy.get(map, node.cell as usize) > 1
                        {
                            found(layers, map, node.cell);
                            break;
                        }
                    }
                }
                _ => {
                    if self.occupancy.get(via_map, node.cell as usize) > 1 {
                        found(layers, via_map, node.cell);
                    }
                }
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
        } else if let Some(spans) = self.spans.as_ref().filter(|spans| map >= spans.base) {
            let relative = map - spans.base;
            (relative / layers, 3 * layers + 1 + relative % layers)
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
        let value = self.occupancy.get(map, cell);
        if !scratch.own_active || value == 0 {
            return value;
        }
        let Some((which, kind)) = self.own_kind(class, map) else {
            return value;
        };
        let kinds = self.own_kinds();
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
        let kinds = self.own_kinds();
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
        for (map, cell) in net_state.stamped.iter() {
            let map = map as usize;
            // The corridor planner's tile fill: cells only this net claims.
            if map >= class_base
                && map < class_base + layers
                && self.occupancy.get(map, cell as usize) == 1
            {
                let tile = (cell as usize / self.grid.nx / TILE) * self.tiles_x
                    + (cell as usize % self.grid.nx) / TILE;
                scratch.own_claimed[(map - class_base) * tiles + tile] += 1;
            }
            if let Some((which, kind)) = self.own_kind(class, map) {
                let bit = (which * kinds + kind) * cells + cell as usize;
                scratch.own_mark[bit / 64] |= 1 << (bit % 64);
                scratch.own_set.push(bit);
            }
        }
    }
}
