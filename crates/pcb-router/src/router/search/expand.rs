// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The expansion of one popped node: its eight steps along the layer and
//! its vias (through, or of a span), each step priced and offered to the
//! open list. Every function here runs once a node or once a step and is
//! inlined into the search's loop.

use super::*;

/// The popped node and what every step from it reads, looked up once per
/// expansion rather than once per step.
pub(super) struct Expansion<'a> {
    pub(super) state: usize,
    layer: usize,
    pub(super) cell: usize,
    x: usize,
    y: usize,
    /// The node's cost so far.
    here: f32,
    /// Steps since the path's last via (`SINCE_VIA_STEPS`), and what a
    /// step along the layer records.
    since_via: u8,
    next_since_via: u8,
    /// The node's parent: a direction, `8 + layer` for a via or
    /// `PARENT_SOURCE`.
    arrived: u8,
    /// The directions the static maps block for this net.
    blocked_edges: u8,
    /// This layer's packed maps.
    layer_hot: &'a [Hot],
    cleanup: bool,
    row_costs: &'a [f32],
    layer_costs: &'a [f32; 8],
    layer_factor: f32,
    thermal_guard_cost: f32,
    /// Whether every neighbour lies inside the window.
    interior: bool,
    /// Whether a neighbour may lie in a neck zone (else its class is the
    /// net's).
    near_zones: bool,
    /// The corridor's answer where every neighbour lies in this node's
    /// tile (0: no test, 1: test each, 2: none in the corridor).
    pub(super) corridor_test: u8,
}

impl Router {
    /// Expands the popped `state`: its steps along the layer, then its
    /// vias.
    #[inline(always)]
    pub(super) fn expand(
        &self,
        search: &Search,
        scratch: &mut Scratch,
        heap: &mut SearchHeap,
        state: usize,
    ) {
        let node = self.expansion(search, scratch, state);
        self.prefetch_neighbourhood(search, scratch, &node);
        // The eight steps unrolled (one copy each, the step's direction
        // known: whether it is diagonal and its bend cost are no longer
        // decided per step at run time).
        if node.corridor_test != 2 {
            self.layer_step::<0, 1, 0>(search, &node, scratch, heap);
            self.layer_step::<1, 1, 1>(search, &node, scratch, heap);
            self.layer_step::<2, 0, 1>(search, &node, scratch, heap);
            self.layer_step::<3, -1, 1>(search, &node, scratch, heap);
            self.layer_step::<4, -1, 0>(search, &node, scratch, heap);
            self.layer_step::<5, -1, -1>(search, &node, scratch, heap);
            self.layer_step::<6, 0, -1>(search, &node, scratch, heap);
            self.layer_step::<7, 1, -1>(search, &node, scratch, heap);
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

        let via_occupied =
            self.occupancy_seen(scratch, search.class, search.via_map, node.cell) as f32;
        if let Some(spans) = search.span_net {
            self.span_vias(search, &node, scratch, heap, spans, via_occupied);
        } else {
            self.through_vias(search, &node, scratch, heap, via_occupied);
        }
    }

    /// The popped node's `Expansion`.
    #[inline(always)]
    fn expansion<'a>(
        &'a self,
        search: &'a Search,
        scratch: &Scratch,
        state: usize,
    ) -> Expansion<'a> {
        let (class, layers, cells, nx) = (search.class, search.layers, search.cells, search.nx);
        let layer = state / cells;
        let cell = state % cells;
        let x = cell % nx;
        let y = cell / nx;
        let here = scratch.nodes[state].cost;
        let since_via = scratch.nodes[state].since_via & SINCE_VIA_STEPS;
        let next_since_via = since_via + (since_via < SINCE_VIA_STEPS) as u8;
        let arrived = scratch.nodes[state].parent;
        let here_statics = if search.necks {
            &self.statics[search.class_at(x, y)]
        } else {
            search.statics
        };
        let (edge_block, edge_owner) = here_statics.edges_at(layer, cell);
        let blocked_edges = if edge_block != 0 && edge_owner == search.own16 {
            0
        } else {
            edge_block
        };
        // This layer's packed maps, looked up once per expansion
        // rather than once per step.
        let layer_hot = &self.hot[self.hot_of[class] * layers + layer];
        let cleanup = self.cleanup;
        let row_costs = if self.cleanup_rows {
            &self.pin_row_cleanup_costs
        } else {
            &self.pin_row_costs
        };
        let layer_costs = &search.guide.direction_cost[layer];
        let mut layer_factor = if self.layer_bias.is_empty() {
            1.0
        } else {
            self.layer_bias[layer]
        };
        if search.preferred != 0 && search.preferred & (1 << layer) == 0 {
            layer_factor *= search.off_preferred;
        }
        let thermal_guard_cost = self.config.thermal_guard_cost as f32;

        // Invariant per expansion: whether every neighbour lies inside
        // the window, and the corridor's answer where every neighbour
        // lies in this node's tile (0: no test, 1: test each, 2: none
        // in the corridor).
        let window = search.window;
        let interior = x > window.0 && y > window.1 && x < window.2 && y < window.3;
        // Whether a neighbour may lie in a neck zone (else its class is
        // the net's).
        let zone_box = search.zone_box;
        let near_zones = search.necks
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
        Expansion {
            state,
            layer,
            cell,
            x,
            y,
            here,
            since_via,
            next_since_via,
            arrived,
            blocked_edges,
            layer_hot,
            cleanup,
            row_costs,
            layer_costs,
            layer_factor,
            thermal_guard_cost,
            interior,
            near_zones,
            corridor_test,
        }
    }

    /// Asks for the cache lines the node's steps will read.
    #[inline(always)]
    fn prefetch_neighbourhood(&self, search: &Search, scratch: &Scratch, node: &Expansion) {
        let (class, layers, cells, nx) = (search.class, search.layers, search.cells, search.nx);
        let (layer, cell) = (node.layer, node.cell);
        // The rows below and above the node, asked for before the
        // steps read them: their search nodes, packed maps and
        // occupancy lie a row of the lattice away, on other cache
        // lines, and a step's branches would otherwise wait for each
        // load in turn (the profile round of 2026-10-25,
        // docs/profiling.md, "Router rounds", item 1).
        if node.corridor_test != 2 {
            let occupancy_map = class * (layers + 1) + layer;
            for row in [cell.wrapping_sub(nx + 1), cell + nx - 1] {
                for column in [row, row.wrapping_add(2)] {
                    prefetch_index(&scratch.nodes, column.wrapping_add(layer * cells));
                    prefetch_index(node.layer_hot, column);
                }
                self.occupancy.prefetch_index(occupancy_map, row);
            }
        }
        // The node's own cell on the other layers (the via's targets).
        let mut other_layers = (u64::MAX >> (64 - layers)) & !(1u64 << layer);
        while other_layers != 0 {
            let other = other_layers.trailing_zeros() as usize;
            other_layers &= other_layers - 1;
            prefetch_index(&scratch.nodes, other * cells + cell);
        }
        // The diagonal steps' clearance maps: both orientations at this
        // cell (the steps towards +x start here) and at the cells the
        // steps towards -x start from, a row below and above. Their
        // loads were the search's costliest line (6 to 8 % of the
        // route's CPU; the second round of 2026-10-27,
        // docs/profiling.md, "Router rounds", item 2).
        if node.corridor_test != 2 {
            let (rising, falling) = (
                self.diagonal_map(class, layer, 0),
                self.diagonal_map(class, layer, 1),
            );
            self.occupancy.prefetch_index(rising, cell);
            self.occupancy
                .prefetch_index(rising, cell.wrapping_sub(nx + 1));
            self.occupancy.prefetch_index(falling, cell);
            self.occupancy.prefetch_index(falling, cell + nx - 1);
        }
        // What the via step reads of this cell, after the steps.
        if layers > 1 {
            self.occupancy.prefetch_index(search.via_map, cell);
            search.statics.via_blocked.prefetch(cell);
            search.statics.free_layers.prefetch(cell);
            prefetch_index(&scratch.own_via_near, cell);
            if let Some(history) = self.history.get(layers) {
                prefetch_index(history, cell);
            }
        }
    }

    /// The step along the layer in `DIRECTION` (`DX`, `DY`): its tests
    /// (window, blocked edge, corridor, closed node, static map,
    /// clearance), its cost and its offer.
    #[inline(always)]
    fn layer_step<const DIRECTION: usize, const DX: i32, const DY: i32>(
        &self,
        search: &Search,
        node: &Expansion,
        scratch: &mut Scratch,
        heap: &mut SearchHeap,
    ) {
        let (class, layers, cells, nx) = (search.class, search.layers, search.cells, search.nx);
        let (layer, cell, x, y, window) = (node.layer, node.cell, node.x, node.y, search.window);
        let direction: usize = DIRECTION;
        let (dx, dy): (&i32, &i32) = (&DX, &DY);
        let tx = x as i64 + *dx as i64;
        let ty = y as i64 + *dy as i64;
        if !node.interior
            && (tx < window.0 as i64
                || ty < window.1 as i64
                || tx > window.2 as i64
                || ty > window.3 as i64)
        {
            return;
        }
        if node.blocked_edges & (1 << direction) != 0 {
            return;
        }
        if node.corridor_test == 1
            && let Some(corridor) = &scratch.corridor
            && !corridor[(ty as usize / TILE) * self.tiles_x + tx as usize / TILE]
        {
            return;
        }
        let target_cell = ty as usize * nx + tx as usize;
        let target_state = layer * cells + target_cell;
        // The search node is read for the cost update anyway: a
        // closed neighbour costs no other map's cache line.
        if scratch.nodes[target_state].closed(search.generation) {
            return;
        }
        let target_class = if node.near_zones {
            search.class_at(tx as usize, ty as usize)
        } else {
            class
        };
        let hot = if node.near_zones && target_class != class {
            // Only the static map's value is the class's.
            Hot {
                trace: self.statics[target_class].trace[layer][target_cell],
                ..node.layer_hot[target_cell]
            }
        } else {
            node.layer_hot[target_cell]
        };
        if hot.trace != crate::grid::FREE16 && hot.trace as u32 != search.own {
            return;
        }
        let map = target_class * (layers + 1) + layer;
        let mut occupied = self.occupancy_seen(scratch, class, map, target_cell) as f32;
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
        if search.hard && occupied > 0.0 {
            return;
        }
        let history = if node.cleanup { 0.0 } else { hot.history };
        let mut step =
            node.layer_costs[direction] * (1.0 + history) * (1.0 + search.present * occupied);
        let mut pin_row = 0.0;
        if hot.pour != 0 {
            let (guard, covered, gap) = self.hot_pour(hot.pour, layer, target_cell);
            if gap != 0 {
                let (a, b) = self.pin_row_nets[gap as usize];
                if a != search.own && b != search.own {
                    pin_row = node.row_costs[gap as usize];
                }
            }
            if guard != 0 && guard as u32 != search.own {
                step *= node.thermal_guard_cost;
            }
            if covered != 0 && covered as u32 != search.own {
                if self.exclusive[layer] {
                    return;
                }
                step *= self.layer_cut[layer] * self.sever_factor(layer, target_cell);
            }
        }
        step *= node.layer_factor;
        step += pin_row;
        if (node.arrived as usize) < 8 {
            let turn = (direction as i32 - node.arrived as i32).rem_euclid(8);
            step += search.bend_cost * turn.min(8 - turn) as f32;
        }
        let total = node.here + step;
        offer(
            search,
            scratch,
            heap,
            target_state,
            total,
            direction as u8,
            node.next_since_via,
            (layer, tx as i32, ty as i32),
        );
    }

    /// The vias of a net with via spans (`SpanModel`) from the popped
    /// node to each other layer.
    #[inline(always)]
    fn span_vias(
        &self,
        search: &Search,
        node: &Expansion,
        scratch: &mut Scratch,
        heap: &mut SearchHeap,
        spans: &SpanModel,
        via_occupied: f32,
    ) {
        let (class, layers, cells, net) = (search.class, search.layers, search.cells, search.net);
        let (state, layer, cell, x, y, arrived) = (
            node.state,
            node.layer,
            node.cell,
            node.x,
            node.y,
            node.arrived,
        );
        let (hard, statics, own) = (search.hard, search.statics, search.own);
        // A via of a span: no chain of vias at one cell (each would
        // be a hole of its own there: KiCad's co-located holes),
        // its layers' statics and occupancy alone, and at a cell of
        // the net's own via only within that via's layers.
        let via_arrived = arrived >= 8 && arrived != PARENT_SOURCE;
        if !via_arrived
            && scratch.own_via_near[cell] != search.generation
            && (node.since_via as usize > search.hop_steps
                || !self.hops_from_a_via(
                    scratch,
                    state,
                    x,
                    y,
                    arrived,
                    search.hop_steps,
                    search.hop_reach,
                ))
        {
            let history = if self.cleanup {
                0.0
            } else {
                self.history[layers][cell]
            };
            let marks = self.via_marks.get(cell).copied().unwrap_or(3);
            let own_via = search.own_vias.get(&(cell as u32)).copied();
            let cell_class = if search.necks {
                search.class_at(x, y)
            } else {
                class
            };
            let cell_statics = &self.statics[cell_class];
            let free_layers = cell_statics.free_layers.get(cell);
            let mut others = (u64::MAX >> (64 - layers)) & !(1u64 << layer);
            while others != 0 {
                let target_layer = others.trailing_zeros() as usize;
                others &= others - 1;
                let pierced = spans.mask(net, layer, target_layer);
                if own_via.is_some_and(|mask| pierced & !mask != 0) {
                    continue;
                }
                let through = pierced == spans.all();
                let occupied = if through {
                    if !statics.via_allowed(cell, net) {
                        continue;
                    }
                    via_occupied
                } else {
                    if !statics.span_allowed(cell, net, pierced) {
                        continue;
                    }
                    let mut most = 0u16;
                    let mut left = pierced;
                    while left != 0 {
                        let pierced_layer = left.trailing_zeros() as usize;
                        left &= left - 1;
                        most = most.max(self.occupancy_seen(
                            scratch,
                            class,
                            self.via_layer_map(spans, class, pierced_layer),
                            cell,
                        ));
                    }
                    most as f32
                };
                if hard && occupied > 0.0 {
                    continue;
                }
                let mut cut = 1.0f32;
                if marks & 1 != 0 {
                    for (cut_layer, (covered, factor)) in
                        self.covered.iter().zip(&self.layer_cut).enumerate()
                    {
                        if pierced & (1 << cut_layer) != 0
                            && !covered.is_empty()
                            && covered[cell] != 0
                            && covered[cell] as u32 != own
                        {
                            cut *= factor * self.sever_factor(cut_layer, cell);
                        }
                    }
                }
                if marks & 2 != 0 {
                    let foreign = self.guard.iter().enumerate().any(|(guard_layer, guard)| {
                        pierced & (1 << guard_layer) != 0
                            && !guard.is_empty()
                            && guard[cell] != 0
                            && guard[cell] as u32 != own
                    });
                    if foreign {
                        cut *= node.thermal_guard_cost;
                    }
                }
                let step =
                    search.via_cost * cut * (1.0 + history) * (1.0 + search.present * occupied);
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
                if scratch.nodes[target_state].closed(search.generation) {
                    continue;
                }
                let total = node.here + step;
                offer(
                    search,
                    scratch,
                    heap,
                    target_state,
                    total,
                    8 + layer as u8,
                    0,
                    (target_layer, x as i32, y as i32),
                );
            }
        }
    }

    /// The through vias (every layer pierced) from the popped node to
    /// each other layer, where a via may stand.
    #[inline(always)]
    fn through_vias(
        &self,
        search: &Search,
        node: &Expansion,
        scratch: &mut Scratch,
        heap: &mut SearchHeap,
        via_occupied: f32,
    ) {
        let (class, layers, cells, net) = (search.class, search.layers, search.cells, search.net);
        let (state, layer, cell, x, y, arrived) = (
            node.state,
            node.layer,
            node.cell,
            node.x,
            node.y,
            node.arrived,
        );
        let (hard, own) = (search.hard, search.own);
        if !(layers > 1
            && search.statics.via_allowed(cell, net)
            && !(hard && via_occupied > 0.0)
            && scratch.own_via_near[cell] != search.generation
            && (node.since_via as usize > search.hop_steps
                || !self.hops_from_a_via(
                    scratch,
                    state,
                    x,
                    y,
                    arrived,
                    search.hop_steps,
                    search.hop_reach,
                )))
        {
            return;
        }
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
                    if !covered.is_empty() && covered[cell] != 0 && covered[cell] as u32 != own {
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
        let step = search.via_cost * cut * (1.0 + history) * (1.0 + search.present * occupied);
        let cell_class = if search.necks {
            search.class_at(x, y)
        } else {
            class
        };
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
            if scratch.nodes[target_state].closed(search.generation) {
                continue;
            }
            let total = node.here + step;
            offer(
                search,
                scratch,
                heap,
                target_state,
                total,
                8 + layer as u8,
                0,
                (target_layer, x as i32, y as i32),
            );
        }
    }
}

/// Offers `target_state` the cost `total`: records it and queues the
/// node (or lowers its key) unless it is queued at a cost as low.
/// `parent` and `since_via` are what the node records (a target keeps
/// its mark); `(layer, x, y)` place it for the heuristic.
#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn offer(
    search: &Search,
    scratch: &mut Scratch,
    heap: &mut SearchHeap,
    target_state: usize,
    total: f32,
    parent: u8,
    since_via: u8,
    (layer, x, y): (usize, i32, i32),
) {
    let generation = search.generation;
    let node = &mut scratch.nodes[target_state];
    let seen = node.seen == generation;
    let queued = seen && node.slot != SLOT_UNQUEUED;
    if !queued || total < node.cost {
        let target = node.target_bit(generation);
        node.seen = generation;
        node.cost = total;
        node.parent = parent;
        node.since_via = since_via | target;
        let estimate = total + search.guide.heuristic(layer, x, y);
        let key = heap_key(estimate, target_state);
        if queued {
            heap.decrease(key, &mut scratch.nodes);
        } else {
            heap.push(key, &mut scratch.nodes);
        }
    }
}
