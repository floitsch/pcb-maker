// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Emitting the routes as segments and vias.

use super::*;

impl Router {
    /// The copper of `net` and the indices of its cosmetic pad-centre stubs.
    pub(super) fn materialize(&self, net: NetId) -> (NetRoute, Vec<usize>) {
        let description = &self.board.nets[net as usize];
        let rules = self.board.classes[description.class];
        let state = &self.nets[net as usize];
        let mut route = NetRoute::default();
        let mut stubs = Vec::new();
        let junctions = junction_nodes(&state.branches);
        for branch in &state.branches {
            let first = branch.nodes[0];
            let last = *branch.nodes.last().unwrap();
            let mut stub = |node: Node, terminal: u16| {
                if terminal == NO_TERMINAL || terminal == PLANE_TERMINAL || terminal == FREE_END {
                    return;
                }
                let anchor = description.terminals[terminal as usize].anchor;
                let center = self.grid.center_of(node.cell as usize);
                // A node on kept copper the terminal reaches
                // (`Board::terminal_copper`) and not on its pad is joined
                // there: no stub to the pad's centre.
                if !self.board.terminal_copper.is_empty() {
                    let own = &description.terminals[terminal as usize];
                    let pad = own
                        .contact
                        .as_ref()
                        .unwrap_or(&self.board.obstacles[own.pad].shape);
                    if !pad.contains(center)
                        && self.board.terminal_copper.iter().any(|extra| {
                            let copper = &self.board.obstacles[extra.obstacle];
                            extra.net == net
                                && extra.terminal == terminal as usize
                                && copper.layers & (1 << node.layer) != 0
                                && joins_copper(
                                    &copper.shape,
                                    center,
                                    self.width_on(description.class, node.layer as usize),
                                )
                        })
                    {
                        return;
                    }
                }
                if let Some((_, width, polyline)) = state.escapes.get(&node) {
                    for pair in polyline.windows(2) {
                        route.segments.push(Segment {
                            layer: node.layer as usize,
                            start: pair[0],
                            end: pair[1],
                            width: *width,
                        });
                    }
                } else if crate::geometry::distance(anchor, center) > 1.0e-7 {
                    // The pad stub at the node's width, or the neck if the
                    // full width does not fit.
                    let layer = node.layer as usize;
                    let mut widths = vec![self.width_at(net, node.cell, layer)];
                    let neck_width = self.neck_width_on(self.board.nets[net as usize].class, layer);
                    if neck_width < widths[0] {
                        widths.push(neck_width);
                    }
                    for width in widths {
                        if self.stub_is_clear(net, node.layer as usize, anchor, center, width)
                            && self.stub_keeps_edge(anchor, center, width)
                        {
                            stubs.push(route.segments.len());
                            route.segments.push(Segment {
                                layer: node.layer as usize,
                                start: anchor,
                                end: center,
                                width,
                            });
                            break;
                        }
                    }
                }
            };
            stub(first, branch.start_terminal);
            stub(last, branch.end_terminal);

            let mut run_start = 0;
            for index in 1..=branch.nodes.len() {
                let layer_change = index < branch.nodes.len()
                    && branch.nodes[index].layer != branch.nodes[index - 1].layer;
                if index < branch.nodes.len() && !layer_change {
                    continue;
                }
                self.emit_run(net, &branch.nodes[run_start..index], &junctions, &mut route);
                if layer_change {
                    // Through vias at one spot are one hole.
                    let at = self.grid.center_of(branch.nodes[index].cell as usize);
                    if !route.vias.iter().any(|via| via.at == at) {
                        route.vias.push(Via {
                            at,
                            diameter: rules.via_diameter,
                            drill: rules.via_drill,
                        });
                    }
                }
                run_start = index;
            }
        }
        for (layer, start, end, width) in &state.inner_spokes {
            route.segments.push(Segment {
                layer: *layer,
                start: *start,
                end: *end,
                width: *width,
            });
        }
        (route, stubs)
    }

    pub(super) fn emit_run(
        &self,
        net: NetId,
        nodes: &[Node],
        junctions: &HashSet<Node>,
        route: &mut NetRoute,
    ) {
        if nodes.len() < 2 {
            return;
        }
        let nx = self.grid.nx as i64;
        let delta = |a: Node, b: Node| {
            (
                b.cell as i64 % nx - a.cell as i64 % nx,
                b.cell as i64 / nx - a.cell as i64 / nx,
            )
        };
        // (A run keeps to one layer.)
        let layer = nodes[0].layer as usize;
        let wide = self.width_on(self.board.nets[net as usize].class, layer);
        let widths: Vec<f64> = if self.nets[net as usize].neck_zones.is_empty() {
            vec![wide; nodes.len()]
        } else {
            nodes
                .iter()
                .map(|node| self.width_at(net, node.cell, layer))
                .collect()
        };
        // A segment is as wide as its narrowest node: a run is cut at a
        // wide node next to a narrow one, so the wide segment ends at a
        // node checked at the full width and the narrow one reaches it.
        let mut start = 0;
        for index in 1..nodes.len() {
            let turning = index + 1 < nodes.len()
                && delta(nodes[index - 1], nodes[index]) != delta(nodes[index], nodes[index + 1]);
            let last = index + 1 == nodes.len();
            let width_edge = widths[index] >= wide
                && ((index + 1 < nodes.len() && widths[index + 1] < wide)
                    || widths[index - 1] < wide);
            if turning || last || width_edge || junctions.contains(&nodes[index]) {
                let width = widths[start..=index]
                    .iter()
                    .copied()
                    .fold(f64::INFINITY, f64::min);
                route.segments.push(Segment {
                    layer: nodes[start].layer as usize,
                    start: self.grid.center_of(nodes[start].cell as usize),
                    end: self.grid.center_of(nodes[index].cell as usize),
                    width,
                });
                start = index;
            }
        }
    }
}
