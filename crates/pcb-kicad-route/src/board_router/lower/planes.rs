// Copyright (C) 2026 Toit contributors.

//! The board's pours as the router's planes, one per pour and layer.

use super::super::geometry::polygon_area;
use super::*;
use pcb_router as core;

impl Builder {
    /// A plane per layer of each pour of a net with pads, filled with a
    /// class of the pour's own width and clearance (added if new), cut by
    /// the pours above it and the fill keepouts.
    pub(super) fn planes(&mut self, ctx: &Context, pours: &[Pour]) -> Vec<core::Plane> {
        let layers = &ctx.layers;
        let board_area = polygon_area(&ctx.loops.outline);
        let mut planes = Vec::new();
        for pour in pours {
            // A pour without pads has nothing to connect.
            let Some(net) = net_ids_lookup(&self.nets, &pour.net) else {
                continue;
            };
            let net_class = self.classes[self.nets[net as usize].class];
            let brush = core::RuleClass {
                trace_width: pour.min_thickness.max(0.05),
                clearance: pour.clearance.max(net_class.clearance),
                ..net_class
            };
            let class = self
                .classes
                .iter()
                .position(|existing| *existing == brush)
                .unwrap_or_else(|| {
                    self.classes.push(brush);
                    self.classes.len() - 1
                });
            for layer in 0..layers.len() {
                if pour.layers & (1 << layer) == 0 {
                    continue;
                }
                planes.push(core::Plane {
                    net,
                    class,
                    layer,
                    polygon: pour.polygon.clone(),
                    // Another net's zone of a higher priority, of an equal
                    // one the larger uuid (KiCad's ZONE::HigherPriority:
                    // USB-C 3-way splitter's +5V and GND, both of priority
                    // 1).
                    excluded: pours
                        .iter()
                        .filter(|other| {
                            other.net != pour.net
                                && other.layers & (1 << layer) != 0
                                && (other.priority, &other.uuid) > (pour.priority, &pour.uuid)
                        })
                        .map(|other| other.polygon.clone())
                        .chain(
                            self.pour_keepouts
                                .iter()
                                .filter(|(mask, _)| mask & (1 << layer) != 0)
                                .map(|(_, polygon)| polygon.clone()),
                        )
                        .collect(),
                    // KiCad cuts a same-net zone of a strictly higher
                    // priority out of this one's fill (an equal one
                    // overlaps it).
                    above: pours
                        .iter()
                        .filter(|other| {
                            other.net == pour.net
                                && other.layers & (1 << layer) != 0
                                && other.priority > pour.priority
                        })
                        .map(|other| other.polygon.clone())
                        .collect(),
                    connect: ctx.connect_pours,
                    thermal_reach: pour.thermal_reach,
                    thermal_gap: pour.thermal_gap,
                    // An inner layer poured over nearly the whole board is
                    // a plane: it carries no signals.
                    exclusive: layers.names[layer].starts_with("In")
                        && polygon_area(&pour.polygon) >= 0.9 * board_area,
                    solid: pour.solid,
                    hatch: pour.hatch,
                    island_min_area: pour.island_min_area,
                });
            }
        }
        planes
    }
}

/// With `exclusive_layers`, each net keeps at most that many exclusive
/// planes, the outermost layers first; the others carry signals again.
pub(super) fn limit_exclusive_planes(ctx: &Context, planes: &mut [core::Plane]) {
    let Some(limit) = ctx.config.tuning.exclusive_layers else {
        return;
    };
    let mut by_net: BTreeMap<core::NetId, Vec<usize>> = BTreeMap::new();
    for (index, plane) in planes.iter().enumerate() {
        if plane.exclusive {
            by_net.entry(plane.net).or_default().push(index);
        }
    }
    let last = ctx.layers.len().saturating_sub(1);
    for (_, mut indices) in by_net {
        indices.sort_by_key(|index| {
            (
                planes[*index].layer.min(last - planes[*index].layer),
                planes[*index].layer,
            )
        });
        for index in indices.into_iter().skip(limit) {
            planes[index].exclusive = false;
        }
    }
}
