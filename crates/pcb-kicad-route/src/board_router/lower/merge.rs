// Copyright (C) 2026 Toit contributors.

//! Pads that are one item to KiCad's connectivity, merged into one
//! terminal (`core::MergedPad`).

use super::*;
use pcb_router as core;
use std::collections::HashMap;

/// Per terminal of `net`, the terminal it is merged into (itself for a
/// terminal that stays). Pads of one number that overlap on a copper layer
/// they share are grouped, transitively, as are the pads of one jumper
/// group (`jumper_groups`, by pad obstacle); a group's terminal is its
/// largest pad by box area, the first in the net's order among equals.
pub(super) fn terminal_hosts(
    net: &core::Net,
    obstacles: &[core::Obstacle],
    jumper_groups: &HashMap<usize, (usize, String)>,
) -> Vec<usize> {
    let area = |terminal: &core::Terminal| {
        let bounds = obstacles[terminal.pad].shape.aabb();
        (bounds.maximum[0] - bounds.minimum[0]) * (bounds.maximum[1] - bounds.minimum[1])
    };
    let count = net.terminals.len();
    let mut order: Vec<usize> = (0..count).collect();
    order.sort_by(|a, b| {
        area(&net.terminals[*b])
            .total_cmp(&area(&net.terminals[*a]))
            .then(a.cmp(b))
    });
    // Union by overlap; the root of a group is its first pad in `order`
    // (the largest), so it is the terminal.
    let mut rank = vec![0; count];
    for (position, &index) in order.iter().enumerate() {
        rank[index] = position;
    }
    let mut parent: Vec<usize> = (0..count).collect();
    fn root(parent: &mut [usize], mut index: usize) -> usize {
        while parent[index] != index {
            parent[index] = parent[parent[index]];
            index = parent[index];
        }
        index
    }
    // The group's root stays its first pad in `order`.
    let join = |parent: &mut [usize], a: usize, b: usize| {
        let (ra, rb) = (root(parent, a), root(parent, b));
        if rank[ra] < rank[rb] {
            parent[rb] = ra;
        } else {
            parent[ra] = rb;
        }
    };
    let overlap = |a: &core::Terminal, b: &core::Terminal| {
        let (pa, pb) = (&obstacles[a.pad], &obstacles[b.pad]);
        pa.layers & pb.layers != 0 && (pa.shape.contains(b.anchor) || pb.shape.contains(a.anchor))
    };
    let mut by_label: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (index, terminal) in net.terminals.iter().enumerate() {
        by_label.entry(&terminal.label).or_default().push(index);
    }
    for group in by_label.values().filter(|group| group.len() > 1) {
        for (position, &a) in group.iter().enumerate() {
            for &b in &group[position + 1..] {
                if !overlap(&net.terminals[a], &net.terminals[b]) {
                    continue;
                }
                join(&mut parent, a, b);
            }
        }
    }
    // Pads of a jumper group are one item to KiCad's connectivity.
    let mut by_jumper: BTreeMap<(usize, &str), Vec<usize>> = BTreeMap::new();
    for (index, terminal) in net.terminals.iter().enumerate() {
        if let Some((footprint, group)) = jumper_groups.get(&terminal.pad) {
            by_jumper
                .entry((*footprint, group.as_str()))
                .or_default()
                .push(index);
        }
    }
    for group in by_jumper.values() {
        for &other in &group[1..] {
            join(&mut parent, group[0], other);
        }
    }
    (0..count).map(|index| root(&mut parent, index)).collect()
}

impl Builder {
    /// Each net's pads of one item merged into one terminal: the others
    /// become merged pads of it. Pads of one number that overlap on a
    /// copper layer they share are one pad to KiCad (a solder pad with
    /// holes in it, an exposed pad's thermal vias): the largest is the
    /// terminal, the others just its copper. OpenRX's panel hangs half of
    /// each wire pad's holes over the edge, unreachable as terminals. Pads
    /// of one number at one place on different layers without a hole (an
    /// edge connector's or a USB-C shell's top and bottom pads) are two
    /// items to KiCad's connectivity, which wants them joined by copper:
    /// two terminals (Knox Key's P1.A1: a track on B.Cu reached the bottom
    /// pad, KiCad found the top one unconnected). The overlap is
    /// transitive: an exposed pad on F.Cu and its copy on B.Cu are one item
    /// through the thermal vias between them (MSign's U1.21, Sisu's U2.11),
    /// a reversible diode's SMD pads on both sides one through its
    /// through-hole pad (urchin's and eternal-keypad's D_SMD_TH), so the
    /// pads of one number are grouped by overlap and the largest of each
    /// group is its terminal.
    pub(super) fn merge_pads(&mut self) {
        for (net_id, net) in self.nets.iter_mut().enumerate() {
            let host = terminal_hosts(net, &self.obstacles, &self.jumper_groups);
            let count = net.terminals.len();
            let keep: Vec<bool> = (0..count).map(|index| host[index] == index).collect();
            let mut renumbered = vec![usize::MAX; net.terminals.len()];
            let mut next = 0;
            for (index, kept) in keep.iter().enumerate() {
                if *kept {
                    renumbered[index] = next;
                    next += 1;
                }
            }
            for (index, terminal) in net.terminals.iter().enumerate() {
                if !keep[index] {
                    self.merged_pads.push(core::MergedPad {
                        net: net_id as core::NetId,
                        host: renumbered[host[index]],
                        pad: terminal.pad,
                        anchor: terminal.anchor,
                        layers: self.obstacles[terminal.pad].layers,
                        padstack: false,
                    });
                }
            }
            let mut index = 0;
            net.terminals.retain(|_| {
                index += 1;
                keep[index - 1]
            });
        }
    }

    /// A padstack pad's copper on the layers its stack names joins its
    /// pad's terminal (the terminal the pad was merged into, if it was).
    pub(super) fn join_padstack_copper(&mut self) {
        for (own, variant) in std::mem::take(&mut self.padstack_copper) {
            let Some(net) = self.obstacles[own].net else {
                continue;
            };
            if let Some((host, anchor)) = self.terminal_host(net, own) {
                self.merged_pads.push(core::MergedPad {
                    net,
                    host,
                    pad: variant,
                    anchor,
                    layers: self.obstacles[variant].layers,
                    padstack: true,
                });
            }
        }
    }
}
