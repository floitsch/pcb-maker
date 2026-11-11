// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The search's set-up: its invariants, the net's own vias, the targets
//! marked and the sources queued.

use super::guide::Guide;
use super::*;

impl Router {
    /// What one search of `net` reads: its costs, its guide and the net's
    /// invariants, with the net's own vias and the targets marked in the
    /// scratch.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn search_setup<'a>(
        &'a self,
        scratch: &mut Scratch,
        net: NetId,
        net_state: &'a NetState,
        targets: &[(Node, u16)],
        points: Option<&'a [(i32, i32, f32)]>,
        plane_target: bool,
        present: f32,
        hard: bool,
        window: (usize, usize, usize, usize),
    ) -> Search<'a> {
        let generation = scratch.next_generation();
        let class = self.board.nets[net as usize].class;
        let layers = self.board.layer_count;
        let via_cost = if plane_target {
            self.config.plane_via_cost
        } else if self.cleanup {
            self.config.cleanup_via_cost
        } else {
            self.config.via_cost
        } as f32;
        let bend_cost = self.config.bend_cost as f32;
        // Cleanup and hard reinsertion decide final geometry: search exactly.
        let weight = if self.cleanup || hard {
            0.999
        } else {
            self.config.heuristic_weight as f32 * 0.999
        };

        // With via spans (`SpanModel`), a via of this net pierces only the
        // shortest allowed span holding the layers it joins.
        let span_net = self
            .spans
            .as_deref()
            .filter(|spans| spans.takes_spans(net) && layers > 2);
        let own_vias = self.mark_own_vias(scratch, net, net_state, class, generation, span_net);
        self.mark_targets(scratch, net, net_state, targets, hard, generation);

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
        let guide = Guide::new(
            points,
            plane_target,
            targets
                .iter()
                .map(|(node, _)| self.grid.xy(node.cell as usize)),
            (self.grid.nx, self.grid.ny),
            direction_cost,
            via_cost,
            weight,
        );

        let (hop_steps, hop_reach) = self.hop_reach(class);
        // The net's preferred layers (`Config::layer_preferences`): a step
        // elsewhere costs `layer_preference_cost` times as much.
        let preferred = self
            .preferred_layers
            .get(net as usize)
            .copied()
            .unwrap_or(0);
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
        // The zones as a mask over that box (`ZoneMask`: `zone_class`'s
        // answer, a bit a node).
        let mut neck_mask = std::mem::take(&mut scratch.neck_mask);
        if necks && neck_class.is_some() {
            neck_mask.build(&net_state.neck_zones, zone_box);
        }
        Search {
            net,
            net_state,
            class,
            layers,
            cells: self.grid.cells(),
            nx: self.grid.nx,
            generation,
            hard,
            plane_target,
            present,
            window,
            via_cost,
            bend_cost,
            guide,
            span_net,
            own_vias,
            hop_steps,
            hop_reach,
            statics: &self.statics[class],
            preferred,
            off_preferred: self.config.layer_preference_cost as f32,
            own: crate::grid::owner(net),
            own16: crate::grid::owner16(net),
            via_map: class * (layers + 1) + layers,
            necks,
            neck_class,
            zone_box,
            neck_mask,
        }
    }

    /// Marks in `scratch.own_via_near` the cells within via-to-via
    /// spacing of the net's routed vias (a via of the net may not stand
    /// there), the via cells themselves left free. With spans, returns per
    /// via cell the layers the net's via there pierces.
    fn mark_own_vias(
        &self,
        scratch: &mut Scratch,
        net: NetId,
        net_state: &NetState,
        class: usize,
        generation: u16,
        span_net: Option<&SpanModel>,
    ) -> HashMap<u32, crate::board::LayerMask> {
        let mut own_vias: HashMap<u32, crate::board::LayerMask> = HashMap::new();
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
                if self.grid.warp.is_some()
                    && !self
                        .grid
                        .uniform_around(x as usize, y as usize, stamps.fresh.reach)
                {
                    // Lines at the pins: the via's footprint measured.
                    self.grid
                        .disc_cells(x as usize, y as usize, stamps.radii.via_to_via, |cell| {
                            near[cell] = generation
                        });
                } else {
                    for (dx, dy) in &stamps.via_to_via {
                        let (tx, ty) = (x + *dx as i64, y + *dy as i64);
                        if tx >= 0 && ty >= 0 && tx < nx && ty < ny && (*dx != 0 || *dy != 0) {
                            near[(ty * nx + tx) as usize] = generation;
                        }
                    }
                }
                near[pair[0].cell as usize] = 0;
            }
        }
        // A via cell itself stays allowed even inside another own via's
        // ring, so mark exact via cells last.
        for branch in &net_state.branches {
            for (index, pair) in branch.nodes.windows(2).enumerate() {
                if pair[0].cell == pair[1].cell {
                    near[pair[0].cell as usize] = 0;
                    // With spans, the layers the net's via there
                    // pierces: another via at that cell is the same
                    // hole only within them.
                    if let Some(spans) = span_net {
                        let pierced = spans.chain_mask(net, chain_layers(&branch.nodes, index));
                        *own_vias.entry(pair[0].cell).or_insert(0) |= pierced;
                    }
                }
            }
        }
        scratch.own_via_near = near;
        own_vias
    }

    /// Marks the targets reached (`seen`) before the search starts, with
    /// no cost yet and no slot; a hard search skips those whose escape is
    /// not free.
    fn mark_targets(
        &self,
        scratch: &mut Scratch,
        net: NetId,
        net_state: &NetState,
        targets: &[(Node, u16)],
        hard: bool,
        generation: u16,
    ) {
        // (The element of the target reached is looked up afterwards in
        // `targets`: `Router::reached_element`.)
        for (node, _) in targets {
            if hard && !self.escape_is_free(scratch, net_state, net, *node) {
                continue;
            }
            let state = self.state(*node);
            // A target is reached (`seen`) before the search starts, with
            // no cost yet and no slot (`SLOT_UNQUEUED`: the first step to
            // it queues it).
            let target = &mut scratch.nodes[state];
            target.seen = generation;
            target.cost = f32::INFINITY;
            target.slot = SLOT_UNQUEUED;
            target.since_via = TARGET_MARK;
        }
    }

    /// Queues the sources at cost 0 (a hard search skips those inside
    /// another net's zone or with a blocked escape).
    pub(super) fn queue_sources(
        &self,
        search: &Search,
        scratch: &mut Scratch,
        heap: &mut SearchHeap,
        sources: &[Node],
    ) {
        let (class, generation) = (search.class, search.generation);
        for node in sources {
            // A hard route may not even start inside another net's zone.
            if search.hard
                && (self.occupancy_seen(
                    scratch,
                    class,
                    self.map_index(class, node.layer as usize),
                    node.cell as usize,
                ) > 0
                    || !self.escape_is_free(scratch, search.net_state, search.net, *node))
            {
                continue;
            }
            let state = self.state(*node);
            let (x, y) = self.grid.xy(node.cell as usize);
            // A source listed twice is queued once; a source that is a
            // target stays one.
            let source = &mut scratch.nodes[state];
            let seen = source.seen == generation;
            let queued = seen && source.slot != SLOT_UNQUEUED;
            let target = source.target_bit(generation);
            source.seen = generation;
            source.cost = 0.0;
            source.parent = PARENT_SOURCE;
            source.since_via = target;
            let key = heap_key(
                search
                    .guide
                    .heuristic(node.layer as usize, x as i32, y as i32),
                state,
            );
            if queued {
                heap.decrease(key, &mut scratch.nodes);
            } else {
                heap.push(key, &mut scratch.nodes);
            }
        }
    }
}
