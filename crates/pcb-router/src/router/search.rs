// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The lattice search.

use super::*;

impl Router {
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
        let generation = scratch.next_generation();
        let class = self.board.nets[net as usize].class;
        let layers = self.board.layer_count;
        let cells = self.grid.cells();
        let nx = self.grid.nx;
        let via_cost = if plane_target {
            self.config.plane_via_cost
        } else if self.cleanup {
            self.config.cleanup_via_cost
        } else {
            self.config.via_cost
        } as f32;
        let bend_cost = self.config.bend_cost as f32;
        let pin_row_cost = self.config.pin_row_cost as f32;
        // Cleanup and hard reinsertion decide final geometry: search exactly.
        let weight = if self.cleanup || hard {
            0.999
        } else {
            self.config.heuristic_weight as f32 * 0.999
        };

        {
            let stamps = &self.stamps[class][class];
            let nx = self.grid.nx as i64;
            let ny = self.grid.ny as i64;
            let mut near = std::mem::take(&mut scratch.own_via_near);
            for branch in &net_state.branches {
                for pair in branch.nodes.windows(2) {
                    if pair[0].cell != pair[1].cell {
                        continue;
                    }
                    let (x, y) = (pair[0].cell as i64 % nx, pair[0].cell as i64 / nx);
                    for (dx, dy) in &stamps.via_to_via {
                        let (tx, ty) = (x + *dx as i64, y + *dy as i64);
                        if tx >= 0 && ty >= 0 && tx < nx && ty < ny && (*dx != 0 || *dy != 0) {
                            near[(ty * nx + tx) as usize] = generation;
                        }
                    }
                    near[pair[0].cell as usize] = 0;
                }
            }
            // A via cell itself stays allowed even inside another own via's
            // ring, so mark exact via cells last.
            for branch in &net_state.branches {
                for pair in branch.nodes.windows(2) {
                    if pair[0].cell == pair[1].cell {
                        near[pair[0].cell as usize] = 0;
                    }
                }
            }
            scratch.own_via_near = near;
        }
        scratch.targets.clear();
        for (node, element) in targets {
            if hard && !self.escape_is_free(scratch, net_state, net, *node) {
                continue;
            }
            let state = self.state(*node);
            scratch.nodes[state].target_mark = generation;
            scratch.targets.insert(state as u32, *element);
        }

        // Without a short list of target points, guide the search with an
        // exact octile distance transform over coarse blocks.
        const BLOCK: usize = 8;
        let blocks_x = nx.div_ceil(BLOCK);
        let blocks_y = self.grid.ny.div_ceil(BLOCK);
        let coarse: Vec<f32> = if points.is_some() || plane_target {
            Vec::new()
        } else {
            let mut distance = vec![f32::INFINITY; blocks_x * blocks_y];
            for (node, _) in targets {
                let (x, y) = self.grid.xy(node.cell as usize);
                distance[(y / BLOCK) * blocks_x + x / BLOCK] = 0.0;
            }
            let diagonal = std::f32::consts::SQRT_2;
            let relax = |x: usize, y: usize, dx: i64, dy: i64, distance: &mut Vec<f32>| {
                let sx = x as i64 + dx;
                let sy = y as i64 + dy;
                if sx < 0 || sy < 0 || sx >= blocks_x as i64 || sy >= blocks_y as i64 {
                    return;
                }
                let step = if dx != 0 && dy != 0 { diagonal } else { 1.0 };
                let candidate = distance[sy as usize * blocks_x + sx as usize] + step;
                if candidate < distance[y * blocks_x + x] {
                    distance[y * blocks_x + x] = candidate;
                }
            };
            for y in 0..blocks_y {
                for x in 0..blocks_x {
                    for (dx, dy) in [(-1, 0), (-1, -1), (0, -1), (1, -1)] {
                        relax(x, y, dx, dy, &mut distance);
                    }
                }
            }
            for y in (0..blocks_y).rev() {
                for x in (0..blocks_x).rev() {
                    for (dx, dy) in [(1, 0), (1, 1), (0, 1), (-1, 1)] {
                        relax(x, y, dx, dy, &mut distance);
                    }
                }
            }
            distance
        };
        let direction_cost: Vec<[f32; 8]> = if self.cleanup {
            let pitch = self.grid.pitch as f32;
            let mut costs = [pitch; 8];
            for direction in [1, 3, 5, 7] {
                costs[direction] = pitch * std::f32::consts::SQRT_2;
            }
            vec![costs; layers]
        } else {
            self.direction_cost.clone()
        };
        let minimum = |direction: usize| {
            direction_cost
                .iter()
                .map(|costs| costs[direction])
                .fold(f32::INFINITY, f32::min)
        };
        let (any_horizontal, any_diagonal, any_vertical) = (minimum(0), minimum(1), minimum(2));
        let heuristic = |layer: usize, x: i32, y: i32| -> f32 {
            if plane_target {
                // A pour is everywhere; the nearest free spot is close.
                return 0.0;
            }
            let Some(points) = points else {
                let blocks = coarse[(y as usize / BLOCK) * blocks_x + x as usize / BLOCK];
                return (blocks - std::f32::consts::SQRT_2).max(0.0)
                    * BLOCK as f32
                    * any_horizontal.min(any_vertical)
                    * weight;
            };
            let costs = &direction_cost[layer];
            let mut best = f32::INFINITY;
            for (tx, ty, reach) in points {
                let dx = (x - tx).abs() as f32;
                let dy = (y - ty).abs() as f32;
                let both = dx.min(dy);
                // Stay on this layer, or pay a via for the cheapest mix.
                let same_layer = both * costs[1] + (dx - both) * costs[0] + (dy - both) * costs[2];
                let any_layer = via_cost
                    + both * any_diagonal
                    + (dx - both) * any_horizontal
                    + (dy - both) * any_vertical;
                best = best.min(same_layer.min(any_layer) - reach * 2.0);
            }
            best.max(0.0) * weight
        };

        // The scratch's heap, its capacity kept from search to search.
        let mut heap = std::mem::take(&mut scratch.heap);
        // A bus member's lane (`bundles`; inactive otherwise).
        let lane = std::mem::take(&mut scratch.lane);
        heap.clear();
        for node in sources {
            // A hard route may not even start inside another net's zone.
            if hard
                && (self.occupancy_seen(
                    scratch,
                    class,
                    self.map_index(class, node.layer as usize),
                    node.cell as usize,
                ) > 0
                    || !self.escape_is_free(scratch, net_state, net, *node))
            {
                continue;
            }
            let state = self.state(*node);
            let (x, y) = self.grid.xy(node.cell as usize);
            scratch.nodes[state].seen = generation;
            scratch.nodes[state].cost = 0.0;
            scratch.nodes[state].parent = PARENT_SOURCE;
            heap.push(heap_key(
                heuristic(node.layer as usize, x as i32, y as i32),
                state,
            ));
        }

        let (hop_steps, hop_reach) = self.hop_reach(class);
        let statics = &self.statics[class];
        let own = crate::grid::owner(net);
        let own16 = crate::grid::owner16(net);
        let trace_base = class * (layers + 1);
        let via_map = trace_base + layers;
        let necks = !net_state.neck_zones.is_empty();
        // `class_at` from the coordinates the step has at hand (no division).
        let neck_class = self.neck_of[class];
        // The zones' bounding box: outside it no zone needs asking.
        let zone_box = net_state
            .neck_zones
            .iter()
            .fold((usize::MAX, usize::MAX, 0, 0), |b, z| {
                (b.0.min(z.0), b.1.min(z.1), b.2.max(z.2), b.3.max(z.3))
            });
        let class_xy = |x: usize, y: usize| match neck_class {
            Some(neck)
                if x >= zone_box.0 && y >= zone_box.1 && x <= zone_box.2 && y <= zone_box.3 =>
            {
                zone_class(class, neck, &net_state.neck_zones, x, y)
            }
            _ => class,
        };
        let mut found = None;
        while let Some(key) = heap.pop() {
            let state = (key & u32::MAX as u64) as usize;
            if scratch.nodes[state].closed == generation {
                continue;
            }
            scratch.nodes[state].closed = generation;
            scratch.expansions += 1;
            if scratch.nodes[state].target_mark == generation {
                found = Some(state);
                break;
            }
            if plane_target {
                let state_net = &net_state;
                let mask = if state_net.plane_target.is_empty() {
                    &state_net.plane[state / cells]
                } else {
                    &state_net.plane_target[state / cells]
                };
                let pour_map = state_net.plane_class[state / cells] * (layers + 1) + state / cells;
                if !mask.is_empty()
                    && mask[state % cells]
                    && !(hard && self.occupancy_seen(scratch, class, pour_map, state % cells) > 0)
                {
                    found = Some(state);
                    break;
                }
            }
            let layer = state / cells;
            let cell = state % cells;
            let x = cell % nx;
            let y = cell / nx;
            let here = scratch.nodes[state].cost;
            let arrived = scratch.nodes[state].parent;
            let here_statics = if necks {
                &self.statics[class_xy(x, y)]
            } else {
                statics
            };
            let (edge_block, edge_owner) = here_statics.edges_at(layer, cell);
            let blocked_edges = if edge_block != 0 && edge_owner == own16 {
                0
            } else {
                edge_block
            };
            // This layer's packed maps, looked up once per expansion
            // rather than once per step.
            let layer_hot = &self.hot[self.hot_of[class] * layers + layer];
            let cleanup = self.cleanup;
            let layer_costs = &direction_cost[layer];
            let layer_factor = if self.layer_bias.is_empty() {
                1.0
            } else {
                self.layer_bias[layer]
            };
            let thermal_guard_cost = self.config.thermal_guard_cost as f32;

            // Invariant per expansion: whether every neighbour lies inside
            // the window, and the corridor's answer where every neighbour
            // lies in this node's tile (0: no test, 1: test each, 2: none
            // in the corridor).
            let interior = x > window.0 && y > window.1 && x < window.2 && y < window.3;
            // Whether a neighbour may lie in a neck zone (else its class is
            // the net's).
            let near_zones = necks
                && x + 1 >= zone_box.0
                && y + 1 >= zone_box.1
                && x <= zone_box.2 + 1
                && y <= zone_box.3 + 1;
            let corridor_test = match &scratch.corridor {
                None => 0,
                Some(corridor) => {
                    let (inner_x, inner_y) = (x % TILE, y % TILE);
                    if inner_x == 0 || inner_x == TILE - 1 || inner_y == 0 || inner_y == TILE - 1 {
                        1
                    } else if corridor[(y / TILE) * self.tiles_x + x / TILE] {
                        0
                    } else {
                        2
                    }
                }
            };
            // The eight steps unrolled (one copy each, the step's direction
            // known: whether it is diagonal and its bend cost are no longer
            // decided per step at run time).
            macro_rules! neighbour {
                ($direction:expr, $dx:expr, $dy:expr) => {
                    'step: {
                        let direction: usize = $direction;
                        let (dx, dy): (&i32, &i32) = (&$dx, &$dy);
                        let tx = x as i64 + *dx as i64;
                        let ty = y as i64 + *dy as i64;
                        if !interior
                            && (tx < window.0 as i64
                                || ty < window.1 as i64
                                || tx > window.2 as i64
                                || ty > window.3 as i64)
                        {
                            break 'step;
                        }
                        if blocked_edges & (1 << direction) != 0 {
                            break 'step;
                        }
                        if corridor_test == 1
                            && let Some(corridor) = &scratch.corridor
                            && !corridor[(ty as usize / TILE) * self.tiles_x + tx as usize / TILE]
                        {
                            break 'step;
                        }
                        let target_cell = ty as usize * nx + tx as usize;
                        let target_state = layer * cells + target_cell;
                        // The search node is read for the cost update anyway: a
                        // closed neighbour costs no other map's cache line.
                        if scratch.nodes[target_state].closed == generation {
                            break 'step;
                        }
                        let target_class = if near_zones {
                            class_xy(tx as usize, ty as usize)
                        } else {
                            class
                        };
                        let hot = if near_zones && target_class != class {
                            // Only the static map's value is the class's.
                            Hot {
                                trace: self.statics[target_class].trace[layer][target_cell],
                                ..layer_hot[target_cell]
                            }
                        } else {
                            layer_hot[target_cell]
                        };
                        if hot.trace != crate::grid::FREE16 && hot.trace as u32 != own {
                            break 'step;
                        }
                        let map = target_class * (layers + 1) + layer;
                        let mut occupied =
                            self.occupancy_seen(scratch, class, map, target_cell) as f32;
                        // A diagonal step passes the nodes on its bisector closer
                        // than its ends do: none of another net may lie there
                        // within the clearance (exact clearance; an axis step
                        // passes no node closer than its ends).
                        if *dx != 0 && *dy != 0 {
                            let (start, orientation) = if *dx > 0 {
                                (cell, (*dy < 0) as usize)
                            } else {
                                (target_cell, (*dy > 0) as usize)
                            };
                            let blocked = self.occupancy_seen(
                                scratch,
                                class,
                                self.diagonal_map(target_class, layer, orientation),
                                start,
                            ) as f32;
                            occupied = occupied.max(blocked);
                        }
                        if hard && occupied > 0.0 {
                            break 'step;
                        }
                        let history = if cleanup { 0.0 } else { hot.history };
                        let mut step =
                            layer_costs[direction] * (1.0 + history) * (1.0 + present * occupied);
                        let mut pin_row = 0.0;
                        if hot.pour != 0 {
                            let (guard, covered, gap) = self.hot_pour(hot.pour, layer, target_cell);
                            if gap != 0 {
                                let (a, b) = self.pin_row_nets[gap as usize];
                                if a != own && b != own {
                                    pin_row = pin_row_cost;
                                }
                            }
                            if guard != 0 && guard as u32 != own {
                                step *= thermal_guard_cost;
                            }
                            if covered != 0 && covered as u32 != own {
                                if self.exclusive[layer] {
                                    break 'step;
                                }
                                step *=
                                    self.layer_cut[layer] * self.sever_factor(layer, target_cell);
                            }
                        }
                        step *= layer_factor;
                        if lane.active
                            && lane.bits[target_state >> 6] & (1 << (target_state & 63)) == 0
                        {
                            step *= lane.off;
                        }
                        step += pin_row;
                        if (arrived as usize) < 8 {
                            let turn = (direction as i32 - arrived as i32).rem_euclid(8);
                            step += bend_cost * turn.min(8 - turn) as f32;
                        }
                        let total = here + step;
                        if scratch.nodes[target_state].seen != generation
                            || total < scratch.nodes[target_state].cost
                        {
                            scratch.nodes[target_state].seen = generation;
                            scratch.nodes[target_state].cost = total;
                            scratch.nodes[target_state].parent = direction as u8;
                            let estimate = total + heuristic(layer, tx as i32, ty as i32);
                            heap.push(heap_key(estimate, target_state));
                        }
                    }
                };
            }
            if corridor_test != 2 {
                neighbour!(0, 1, 0);
                neighbour!(1, 1, 1);
                neighbour!(2, 0, 1);
                neighbour!(3, -1, 1);
                neighbour!(4, -1, 0);
                neighbour!(5, -1, -1);
                neighbour!(6, 0, -1);
                neighbour!(7, 1, -1);
            }
            debug_assert_eq!(
                DIRECTIONS,
                [
                    (1, 0),
                    (1, 1),
                    (0, 1),
                    (-1, 1),
                    (-1, 0),
                    (-1, -1),
                    (0, -1),
                    (1, -1)
                ]
            );

            let via_occupied = self.occupancy_seen(scratch, class, via_map, cell) as f32;
            if layers > 1
                && statics.via_allowed(cell, net)
                && !(hard && via_occupied > 0.0)
                && scratch.own_via_near[cell] != generation
                && !self.hops_from_a_via(scratch, state, x, y, arrived, hop_steps, hop_reach)
            {
                let occupied = via_occupied;
                let history = if self.cleanup {
                    0.0
                } else {
                    self.history[layers][cell]
                };
                let mut cut = 1.0f32;
                let marks = self.via_marks.get(cell).copied().unwrap_or(3);
                if marks & 1 != 0 {
                    let cover = self.via_cover.get(cell).copied().unwrap_or(VIA_COVER_MIXED);
                    if cover == VIA_COVER_MIXED {
                        for (layer, (covered, factor)) in
                            self.covered.iter().zip(&self.layer_cut).enumerate()
                        {
                            if !covered.is_empty()
                                && covered[cell] != 0
                                && covered[cell] as u32 != own
                            {
                                cut *= factor * self.sever_factor(layer, cell);
                            }
                        }
                    } else if cover & 0xffff != own {
                        // The same products in the same (layer) order.
                        let mut layers_covered = cover >> 16;
                        while layers_covered != 0 {
                            let layer = layers_covered.trailing_zeros() as usize;
                            cut *= self.layer_cut[layer] * self.sever_factor(layer, cell);
                            layers_covered &= layers_covered - 1;
                        }
                    }
                }
                // A via is copper on every layer: inside another net's
                // thermal ring on any of them it starves the spokes there.
                if marks & 2 != 0 {
                    let foreign = |guard: &Vec<u16>| {
                        !guard.is_empty() && guard[cell] != 0 && guard[cell] as u32 != own
                    };
                    if self.guard.iter().any(foreign) {
                        cut *= self.config.thermal_guard_cost as f32;
                    }
                }
                let step = via_cost * cut * (1.0 + history) * (1.0 + present * occupied);
                let cell_class = if necks { class_xy(x, y) } else { class };
                let cell_statics = &self.statics[cell_class];
                let free_layers = cell_statics.free_layers.get(cell);
                // The other layers as a mask (a test against this layer
                // in the loop mispredicted, its place changing per node).
                let mut others = (u64::MAX >> (64 - layers)) & !(1u64 << layer);
                while others != 0 {
                    let target_layer = others.trailing_zeros() as usize;
                    others &= others - 1;
                    if free_layers & (1 << target_layer) == 0 {
                        let allowed = cell_statics.trace_at(target_layer, cell);
                        if allowed != crate::grid::FREE && allowed != own {
                            continue;
                        }
                    }
                    if hard
                        && self.occupancy_seen(
                            scratch,
                            class,
                            cell_class * (layers + 1) + target_layer,
                            cell,
                        ) > 0
                    {
                        continue;
                    }
                    let target_state = target_layer * cells + cell;
                    if scratch.nodes[target_state].closed == generation {
                        continue;
                    }
                    let total = here + step;
                    if scratch.nodes[target_state].seen != generation
                        || total < scratch.nodes[target_state].cost
                    {
                        scratch.nodes[target_state].seen = generation;
                        scratch.nodes[target_state].cost = total;
                        scratch.nodes[target_state].parent = 8 + layer as u8;
                        let estimate = total + heuristic(target_layer, x as i32, y as i32);
                        heap.push(heap_key(estimate, target_state));
                    }
                }
            }
        }

        scratch.heap = heap;
        scratch.lane = lane;
        if let Some(started) = timed {
            use std::sync::atomic::Ordering::Relaxed;
            self.profile
                .search_nanos
                .fetch_add(started.elapsed().as_nanos() as u64, Relaxed);
            self.profile
                .search_expansions
                .fetch_add(scratch.expansions - expansions_before, Relaxed);
        }
        if scratch.corridor.is_none() {
            scratch.open_searches += 1;
            scratch.open_expansions += scratch.expansions - expansions_before;
        }
        if found.is_none() {
            scratch.failed_searches += 1;
        }
        let mut state = found?;
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
        Some(path)
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
    /// follows the coordinates rather than dividing the states (until
    /// 2026-10-25 two divisions a step back and two for the cell's centre:
    /// 5 to 8 % of the route's CPU on Castor; the profile round,
    /// docs/profiling.md).
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
        ((reach / self.grid.pitch).ceil() as usize, reach)
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
