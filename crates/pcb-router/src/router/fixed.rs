// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Copper fixed before the negotiation: escape stubs, plane skeletons, plane stubs.

use super::*;

impl Router {
    /// Routes every pour net that has no copper yet as a plain tree, biased
    /// onto the layer its pours cover most, against all copper already on
    /// the board. The tree stays fixed for the rest of the run.
    /// Fixes a straight escape stub outward for every fine-pitch pad of a
    /// dense row (see `Config::escape_stub_mm`). The stubs are leading
    /// fixed branches, like a plane skeleton; a net that has branches
    /// already (kept through an update) is left alone.
    pub(super) fn fix_escapes(&mut self, order: &[NetId]) {
        let length = self.config.escape_stub_mm;
        if length <= 0.0 {
            return;
        }
        let pitch = self.grid.pitch;
        let steps = (length / pitch).round() as i64;
        let nx = self.grid.nx as i64;
        let ny = self.grid.ny as i64;
        // Fine-pitch pads on one layer, with the direction of their long
        // side: (net, terminal, layer, axis, aabb).
        let mut narrow: Vec<(NetId, usize, usize, usize, crate::geometry::Aabb)> = Vec::new();
        for net in order {
            let description = &self.board.nets[*net as usize];
            let rules = self.board.classes[description.class];
            let state = &self.nets[*net as usize];
            if !state.routable || !state.branches.is_empty() {
                continue;
            }
            for (index, terminal) in description.terminals.iter().enumerate() {
                if terminal.layers.count_ones() != 1 || state.terminal_nodes[index].is_empty() {
                    continue;
                }
                let bounds = self.board.obstacles[terminal.pad].shape.aabb();
                let size = [
                    bounds.maximum[0] - bounds.minimum[0],
                    bounds.maximum[1] - bounds.minimum[1],
                ];
                let short = size[0].min(size[1]);
                if short >= rules.trace_width + rules.clearance
                    || size[0].max(size[1]) < 1.5 * short
                {
                    continue;
                }
                let axis = usize::from(size[1] > size[0]);
                narrow.push((
                    *net,
                    index,
                    terminal.layers.trailing_zeros() as usize,
                    axis,
                    bounds,
                ));
            }
        }
        // A row: at least four such pads of the same direction whose
        // centres line up across the long side and follow each other along
        // it within three pad pitches.
        let mut in_row = vec![false; narrow.len()];
        for (i, (_, _, layer, axis, bounds)) in narrow.iter().enumerate() {
            let across = 1 - *axis;
            let centre = [
                (bounds.minimum[0] + bounds.maximum[0]) / 2.0,
                (bounds.minimum[1] + bounds.maximum[1]) / 2.0,
            ];
            let short = (bounds.maximum[across] - bounds.minimum[across]).max(1.0e-6);
            let mut alike = 0;
            for (j, (_, _, other_layer, other_axis, other)) in narrow.iter().enumerate() {
                if i == j || other_layer != layer || other_axis != axis {
                    continue;
                }
                let other_centre = [
                    (other.minimum[0] + other.maximum[0]) / 2.0,
                    (other.minimum[1] + other.maximum[1]) / 2.0,
                ];
                if (other_centre[*axis] - centre[*axis]).abs() < short
                    && (other_centre[across] - centre[across]).abs() <= 3.0 * (2.0 * short + 0.05)
                {
                    alike += 1;
                }
            }
            in_row[i] = alike >= 3;
            if std::env::var_os("PCB_ROUTER_DEBUG").is_some() && !in_row[i] {
                eprintln!(
                    "  narrow pad {} not in a row ({alike} alike)",
                    self.board.nets[narrow[i].0 as usize].terminals[narrow[i].1].label
                );
            }
        }
        if std::env::var_os("PCB_ROUTER_DEBUG").is_some() {
            eprintln!(
                "  {} narrow pads, {} in rows",
                narrow.len(),
                in_row.iter().filter(|r| **r).count()
            );
        }
        let mut fixed = 0;
        let mut by_net: HashMap<NetId, Vec<Branch>> = HashMap::new();
        for (i, (net, index, layer, axis, bounds)) in narrow.iter().enumerate() {
            if !in_row[i] {
                continue;
            }
            let net = *net;
            let nodes = &self.nets[net as usize].terminal_nodes[*index];
            let across = 1 - *axis;
            let middle = (bounds.minimum[across] + bounds.maximum[across]) / 2.0;
            let centre = (bounds.minimum[*axis] + bounds.maximum[*axis]) / 2.0;
            // Outward is the way with more room along the long side, from
            // the pad node nearest that end (a pad too narrow for a node
            // has escape nodes outside it: the straightest one that way).
            // Away from the part, when the part's other pads say where it
            // is (a QFN's inside is free of statics too).
            let (mut sum, mut count) = (0.0, 0usize);
            for (_, _, _, _, other) in &narrow {
                let other_centre = [
                    (other.minimum[0] + other.maximum[0]) / 2.0,
                    (other.minimum[1] + other.maximum[1]) / 2.0,
                ];
                if (other_centre[0] - (bounds.minimum[0] + bounds.maximum[0]) / 2.0).abs() < 8.0
                    && (other_centre[1] - (bounds.minimum[1] + bounds.maximum[1]) / 2.0).abs() < 8.0
                {
                    sum += other_centre[*axis];
                    count += 1;
                }
            }
            let offset = centre - sum / count.max(1) as f64;
            let signs: &[i64] = if offset > 1.0 {
                &[1]
            } else if offset < -1.0 {
                &[-1]
            } else {
                &[-1, 1]
            };
            let mut best: Option<(i64, Vec<Node>)> = None;
            for sign in signs.iter().copied() {
                let end = if sign > 0 {
                    bounds.maximum[*axis]
                } else {
                    bounds.minimum[*axis]
                };
                let Some(start) = nodes
                    .iter()
                    .filter(|node| node.layer as usize == *layer)
                    .filter(|node| {
                        let at = self.grid.center_of(node.cell as usize);
                        !self.nets[net as usize].escapes.contains_key(node)
                            || (at[*axis] - centre) * sign as f64 > 0.0
                    })
                    .min_by(|a, b| {
                        let key = |node: &Node| {
                            let at = self.grid.center_of(node.cell as usize);
                            (
                                (at[across] - middle).abs() * 4.0 + (at[*axis] - end).abs(),
                                0,
                            )
                        };
                        key(a).partial_cmp(&key(b)).unwrap()
                    })
                else {
                    continue;
                };
                let mut path = vec![*start];
                let (mut x, mut y) = (start.cell as i64 % nx, start.cell as i64 / nx);
                let inside = |cell: i64| {
                    let at = self.grid.center_of(cell as usize);
                    at[*axis] >= bounds.minimum[*axis] - 1.0e-6
                        && at[*axis] <= bounds.maximum[*axis] + 1.0e-6
                };
                let mut beyond = 0;
                while beyond < steps {
                    if *axis == 0 {
                        x += sign
                    } else {
                        y += sign
                    }
                    if x < 0 || y < 0 || x >= nx || y >= ny {
                        break;
                    }
                    let cell = y * nx + x;
                    let class = self.node_class(net, cell as u32);
                    if !self.statics[class].trace_allowed(*layer, cell as usize, net)
                        || self
                            .occupancy
                            .get(self.map_index(class, *layer), cell as usize)
                            > 0
                    {
                        break;
                    }
                    path.push(Node {
                        layer: *layer as u8,
                        cell: cell as u32,
                    });
                    if !inside(cell) {
                        beyond += 1;
                    }
                }
                if beyond >= 3 && best.as_ref().is_none_or(|(length, _)| beyond > *length) {
                    best = Some((beyond, path));
                }
            }
            if let Some((_, path)) = best {
                if std::env::var_os("PCB_ROUTER_DEBUG").is_some() {
                    let from = self.grid.center_of(path[0].cell as usize);
                    let to = self.grid.center_of(path.last().unwrap().cell as usize);
                    eprintln!(
                        "  escape stub {} ({}): ({:.2},{:.2}) -> ({:.2},{:.2}) on layer {layer}",
                        self.board.nets[net as usize].terminals[*index].label,
                        self.board.nets[net as usize].name,
                        from[0],
                        from[1],
                        to[0],
                        to[1]
                    );
                }
                by_net.entry(net).or_default().push(Branch {
                    nodes: path,
                    start_terminal: *index as u16,
                    end_terminal: FREE_END,
                });
                fixed += 1;
                // Stamp now so the next pad's stub sees this one.
                let branches = by_net.get(&net).unwrap().clone();
                let state = &mut self.nets[net as usize];
                state.branches = branches;
                state.fixed = state.branches.len();
                self.stamp(net);
            }
        }
        if self.config.verbose && fixed > 0 {
            eprintln!(
                "escape stubs fixed for {fixed} fine-pitch pads of {} nets",
                by_net.len()
            );
        }
    }

    pub(super) fn route_skeletons(&mut self, order: &[NetId]) {
        if !self.config.plane_skeleton {
            return;
        }
        let layers = self.board.layer_count;
        for net in order {
            let state = &self.nets[*net as usize];
            if state.plane.is_empty()
                || state.fixed > 0
                || !state.branches.is_empty()
                || self.board.nets[*net as usize].terminals.len() < 2
            {
                continue;
            }
            let started = std::time::Instant::now();
            let layer = (0..layers)
                .max_by_key(|layer| state.plane[*layer].iter().filter(|inside| **inside).count())
                .unwrap_or(0);
            self.layer_bias = vec![self.config.skeleton_bias as f32; layers];
            self.layer_bias[layer] = 1.0;
            let mut scratch = std::mem::take(&mut self.scratch);
            let mut net_state = std::mem::take(&mut self.nets[*net as usize]);
            let plane = std::mem::take(&mut net_state.plane);
            self.route_net(&mut scratch, *net, &mut net_state, 0.0, true, 4.0);
            net_state.plane = plane;
            net_state.fixed = net_state.branches.len();
            net_state.blocked = false;
            self.nets[*net as usize] = net_state;
            self.scratch = scratch;
            self.layer_bias.clear();
            self.stamp(*net);
            if self.config.verbose {
                let state = &self.nets[*net as usize];
                let vias = state
                    .branches
                    .iter()
                    .map(|branch| {
                        branch
                            .nodes
                            .windows(2)
                            .filter(|pair| pair[0].cell == pair[1].cell)
                            .count()
                    })
                    .sum::<usize>();
                eprintln!(
                    "skeleton {} on layer {layer}: {} branches, {vias} vias, complete {}, {:.2}s",
                    self.board.nets[*net as usize].name,
                    state.fixed,
                    state.complete,
                    started.elapsed().as_secs_f64()
                );
            }
        }
    }

    /// Connects every pad of a pour net to its plane on the empty board (a
    /// stub and a via next to the pad, as a designer places them) and keeps
    /// those connections fixed: signals route around them instead of
    /// pushing them off the via sites next to the pads.
    pub(super) fn fix_plane_stubs(&mut self, order: &[NetId]) {
        if !self.config.fixed_plane_stubs {
            return;
        }
        let started = std::time::Instant::now();
        let pour_nets: Vec<NetId> = order
            .iter()
            .copied()
            .filter(|net| {
                let state = &self.nets[*net as usize];
                !state.plane.is_empty() && state.fixed == 0 && state.branches.is_empty()
            })
            .collect();
        if pour_nets.is_empty() {
            return;
        }
        // On the empty board every pad touches its layer's pour; the pours
        // on the outer layers are what the signals cut to pieces later. So
        // only the inner planes count here: a pad on an outer layer has to
        // reach one through a via, as a designer places one next to every
        // ground pad. Boards without an inner plane are left alone.
        let layers = self.board.layer_count;
        let mut saved = Vec::new();
        for net in &pour_nets {
            let state = &mut self.nets[*net as usize];
            let inner = (1..layers.saturating_sub(1)).any(|layer| !state.plane[layer].is_empty());
            if std::env::var_os("PCB_ROUTER_DEBUG").is_some() {
                let covered: Vec<usize> = (0..layers)
                    .filter(|layer| !state.plane[*layer].is_empty())
                    .collect();
                eprintln!(
                    "  plane stubs {}: pour layers {covered:?}, inner {inner}",
                    self.board.nets[*net as usize].name
                );
            }
            if !inner {
                continue;
            }
            saved.push((*net, state.plane.clone(), state.on_plane.clone()));
            state.plane[0] = Vec::new();
            state.plane[layers - 1] = Vec::new();
            for (index, nodes) in state.terminal_nodes.iter().enumerate() {
                state.on_plane[index] = nodes.iter().any(|node| {
                    state
                        .plane
                        .get(node.layer as usize)
                        .is_some_and(|mask| !mask.is_empty() && mask[node.cell as usize])
                });
            }
        }
        if saved.is_empty() {
            return;
        }
        let pour_nets: Vec<NetId> = saved.iter().map(|(net, _, _)| *net).collect();
        // The pour nets negotiate among themselves (their vias compete for
        // the same spots next to alternating pads), then stay.
        // At most a minute: of work where the wall clock guards (route
        // mode; a minute of wall clock made it depend on the load), as
        // before under a caller's deadline (the layout: stall alone).
        let limits = (
            self.config.negotiation_seconds,
            self.config.negotiation_expansions,
        );
        if self.wall_guard() {
            let minute = (60.0 * expansions_per_second()) as u64;
            self.config.negotiation_expansions = if limits.1 == 0 {
                minute
            } else {
                limits.1.min(minute)
            };
            self.config.negotiation_seconds = limits.0.min(60.0 * guard());
        } else {
            self.config.negotiation_seconds = limits.0.min(60.0);
        }
        self.negotiate(&pour_nets, pour_nets.clone());
        (
            self.config.negotiation_seconds,
            self.config.negotiation_expansions,
        ) = limits;
        for (net, plane, on_plane) in saved {
            let state = &mut self.nets[net as usize];
            state.plane = plane;
            state.on_plane = on_plane;
        }
        for net in &pour_nets {
            // Only what is in conflict with nothing stays fixed: the
            // branches still contested (ColdFire's alternating GND and
            // +3.3V pads, whose vias compete for the same spots) go, and
            // those pads are left to the stitching at the end.
            let keep = self.unconflicted_branches(*net);
            let mut keep = keep.into_iter();
            let state = &mut self.nets[*net as usize];
            state.branches.retain(|_| keep.next().unwrap());
            state.fixed = state.branches.len();
            state.blocked = false;
            state.complete = false;
            self.stamp(*net);
            let state = &mut self.nets[*net as usize];
            if self.config.verbose {
                eprintln!(
                    "plane stubs {}: {} fixed branches, {} of {} terminals connected, {:.2}s",
                    self.board.nets[*net as usize].name,
                    state.fixed,
                    state.connected.iter().filter(|done| **done).count(),
                    state.connected.len(),
                    started.elapsed().as_secs_f64()
                );
            }
        }
    }
}
