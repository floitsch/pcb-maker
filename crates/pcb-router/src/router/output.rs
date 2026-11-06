// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Emitting the routes as segments and vias.

use super::*;

impl Router {
    /// The allowed span of exactly the layers `mask` (`Board::via_spans`),
    /// or None for a through via.
    pub(super) fn span_of_mask(
        &self,
        mask: crate::board::LayerMask,
    ) -> Option<crate::board::ViaSpan> {
        self.board
            .via_spans
            .iter()
            .find(|span| span.mask() == mask)
            .copied()
    }

    /// The copper of `net` and the indices of its cosmetic pad-centre stubs.
    pub(super) fn materialize(&self, net: NetId) -> (NetRoute, Vec<usize>) {
        let (route, stubs, _) = self.materialize_with_escapes(net);
        (route, stubs)
    }

    /// `materialize`, and per escape stub the branch ends use the indices
    /// of its segments (`finish` necks one down where it comes too close
    /// to another net's copper: `neck_crowded_escapes`).
    pub(super) fn materialize_with_escapes(
        &self,
        net: NetId,
    ) -> (NetRoute, Vec<usize>, Vec<Vec<usize>>) {
        let description = &self.board.nets[net as usize];
        let rules = self.board.classes[description.class];
        let state = &self.nets[net as usize];
        let mut route = NetRoute::default();
        let mut stubs = Vec::new();
        let mut escapes: Vec<Vec<usize>> = Vec::new();
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
                    escapes.push(
                        (route.segments.len()..route.segments.len() + polyline.len() - 1).collect(),
                    );
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
                    // With spans, the via's layers (`SpanModel`): those of
                    // its chain at the cell, joined with any other via of
                    // the net there (the search keeps a second one within
                    // the first's layers).
                    let pierced = self.spans.as_deref().and_then(|spans| {
                        let mask = spans.chain_mask(net, chain_layers(&branch.nodes, index));
                        (mask != spans.all()).then_some((spans, mask))
                    });
                    // A span's own size (a micro via's) where it is
                    // within the class's via, which the maps kept clear.
                    let sized = |via: &mut Via| {
                        via.diameter = rules.via_diameter;
                        via.drill = rules.via_drill;
                        if let Some([diameter, drill]) = via.span.and_then(|span| span.size)
                            && diameter <= rules.via_diameter
                            && drill <= rules.via_drill
                        {
                            via.diameter = diameter;
                            via.drill = drill;
                        }
                    };
                    match route.vias.iter_mut().find(|via| via.at == at) {
                        None => {
                            let mut via = Via {
                                at,
                                diameter: rules.via_diameter,
                                drill: rules.via_drill,
                                span: pierced.and_then(|(_, mask)| self.span_of_mask(mask)),
                            };
                            if via.span.is_some() {
                                sized(&mut via);
                            }
                            route.vias.push(via);
                        }
                        Some(via) => {
                            if let (Some(span), Some((spans, mask))) = (via.span, pierced) {
                                let joined = spans.chain_mask(net, span.mask() | mask);
                                via.span = self.span_of_mask(joined);
                            } else {
                                via.span = None;
                            }
                            if pierced.is_some() {
                                sized(via);
                            }
                        }
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
        (route, stubs, escapes)
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
