// Copyright (C) 2026 Toit contributors.

//! The links may be given up in the negotiation: the pair's nets as they
//! are with only the links still standing, and the merging of terminals
//! the links join.

use super::*;
use pcb_router as core;

impl PairPlan {
    /// The net of `polarity` (0 positive, 1 negative) of pair `pair` as
    /// `plan_pairs` lowers it with only the links not in `released`: its
    /// terminals as lowered, each such link's stubs' ends added and joined
    /// with each other and with the pads whose stubs the link carries, then
    /// merged (`merge_net_terminals`). With nothing released it is the net
    /// as planned.
    pub(super) fn net_with_links(
        &self,
        board: &core::Board,
        pair: usize,
        polarity: usize,
        released: &BTreeSet<core::NetId>,
    ) -> Option<core::NetRewrite> {
        let net = [self.pairs[pair].positive, self.pairs[pair].negative][polarity];
        let (_, original) = self.originals.iter().find(|(owner, _)| *owner == net)?;
        let mut description = original.clone();
        let mut merged_pads: Vec<core::MergedPad> = self
            .merged_pads
            .iter()
            .filter(|merged| merged.net == net)
            .cloned()
            .collect();
        let mut terminal_copper: Vec<core::TerminalCopper> = self
            .terminal_copper
            .iter()
            .filter(|extra| extra.net == net)
            .copied()
            .collect();
        let mut parent: Vec<usize> = (0..description.terminals.len()).collect();
        for link in self
            .links
            .iter()
            .filter(|link| link.pair == pair && !released.contains(&link.net))
        {
            description.terminals.push(link.runs[0][polarity].clone());
            description.terminals.push(link.runs[1][polarity].clone());
            let count = description.terminals.len();
            parent.extend(parent.len()..count);
            let (a, b) = self.pairs[pair].links[link.link];
            let points = [self.pairs[pair].points[a], self.pairs[pair].points[b]];
            let mut joined = vec![count - 2, count - 1];
            for (end, point) in points.into_iter().enumerate() {
                if link.fixed[end] {
                    joined.push([point.0, point.1][polarity]);
                }
            }
            for item in &joined[1..] {
                let (x, y) = (root(&mut parent, joined[0]), root(&mut parent, *item));
                if x != y {
                    parent[x.max(y)] = x.min(y);
                }
            }
        }
        let host_of: Vec<usize> = (0..parent.len()).map(|t| root(&mut parent, t)).collect();
        merge_net_terminals(
            net,
            &mut description,
            &mut merged_pads,
            &mut terminal_copper,
            &board.obstacles,
            &host_of,
        );
        Some(core::NetRewrite {
            net,
            terminals: description.terminals,
            merged_pads,
            terminal_copper,
        })
    }
}

/// The links may be given up in the negotiation (docs/router.md, "Giving
/// up on a link"): each is a net that stands in for the pair's two nets
/// between the stubs' ends; given up, the pair's nets go back to their pads
/// as lowered, joined by the links still standing.
impl core::GiveUp for PairPlan {
    fn optional(&self, net: core::NetId) -> bool {
        net as usize >= self.nets && self.links.iter().any(|link| link.net == net)
    }

    fn released(&self, board: &core::Board, released: &[core::NetId]) -> Vec<core::NetRewrite> {
        let released: BTreeSet<core::NetId> = released.iter().copied().collect();
        let pairs: BTreeSet<usize> = self
            .links
            .iter()
            .filter(|link| released.contains(&link.net))
            .map(|link| link.pair)
            .collect();
        let mut rewrites = Vec::new();
        for pair in pairs {
            for polarity in 0..2 {
                rewrites.extend(self.net_with_links(board, pair, polarity, &released));
            }
        }
        rewrites
    }

    fn standing(
        &self,
        board: &core::Board,
        net: core::NetId,
        released: &[core::NetId],
    ) -> Vec<core::NetRewrite> {
        let released: BTreeSet<core::NetId> = released
            .iter()
            .copied()
            .filter(|other| *other != net)
            .collect();
        let Some(link) = self.links.iter().find(|link| link.net == net) else {
            return Vec::new();
        };
        (0..2)
            .filter_map(|polarity| self.net_with_links(board, link.pair, polarity, &released))
            .collect()
    }
}

/// Whether the router may give up a pair's link in the negotiation
/// (`core::GiveUp`, docs/router.md "Giving up on a link"): a test's
/// override, else the settings' `diff_pair_give_up`, else
/// `PCB_ROUTER_DIFF_PAIRS_GIVE_UP` (`1` or `0`), else `GIVE_UP_BY_DEFAULT`.
pub(super) fn give_up_links(config: &KiCadBoardRouterConfig) -> bool {
    GIVE_UP.with(|cell| cell.get()).unwrap_or_else(|| {
        config.diff_pair_give_up.unwrap_or_else(|| {
            std::env::var("PCB_ROUTER_DIFF_PAIRS_GIVE_UP")
                .map_or(GIVE_UP_BY_DEFAULT, |value| value == "1")
        })
    })
}

/// The links' give-up's default (`give_up_links`): off. Measured
/// 2026-11-05 (docs/router.md, "Giving up on a link"): on jetson-nano-
/// baseboard it brings the layout's unconnected from 23 and 44 to 10 and 16
/// with as many pairs coupled, but on ducktop2's left_io seed 101 it costs
/// 3 of 6 coupled pairs and 16 connections, beyond the evidence rule's one
/// pair.
pub(super) const GIVE_UP_BY_DEFAULT: bool = false;

thread_local! {
    /// A test's override of `give_up_links` (this thread only).
    pub(super) static GIVE_UP: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

/// Merges terminals of `net` into their hosts (`host_of[t]`, the smallest
/// terminal of its group): they become merged pads of the host, which a
/// route may end on (`Board::terminal_copper`), as the board's own copper
/// merges the pads it joins (`keep_existing_copper`).
pub(super) fn merge_terminals(board: &mut core::Board, net: core::NetId, host_of: &[usize]) {
    let mut description = std::mem::replace(
        &mut board.nets[net as usize],
        core::Net {
            name: String::new(),
            class: 0,
            terminals: Vec::new(),
        },
    );
    merge_net_terminals(
        net,
        &mut description,
        &mut board.merged_pads,
        &mut board.terminal_copper,
        &board.obstacles,
        host_of,
    );
    board.nets[net as usize] = description;
}

/// `merge_terminals` on a net of its own: `description` is the net
/// `net`, `merged_pads` and `terminal_copper` hold (at least) its entries.
pub(super) fn merge_net_terminals(
    net: core::NetId,
    description: &mut core::Net,
    merged_pads: &mut Vec<core::MergedPad>,
    terminal_copper: &mut Vec<core::TerminalCopper>,
    obstacles: &[core::Obstacle],
    host_of: &[usize],
) {
    let terminals = description.terminals.len();
    let mut renumbered = vec![usize::MAX; terminals];
    let mut next = 0;
    for terminal in 0..terminals {
        if host_of[terminal] == terminal {
            renumbered[terminal] = next;
            next += 1;
        }
    }
    if next == terminals {
        return;
    }
    let mut merged = Vec::new();
    for terminal in 0..terminals {
        if host_of[terminal] != terminal {
            let terminal_description = &description.terminals[terminal];
            merged.push(core::MergedPad {
                net,
                host: renumbered[host_of[terminal]],
                pad: terminal_description.pad,
                anchor: terminal_description.anchor,
                layers: obstacles[terminal_description.pad].layers,
                padstack: false,
            });
        }
    }
    for existing in merged_pads
        .iter_mut()
        .filter(|existing| existing.net == net && existing.host < terminals)
    {
        existing.padstack &= host_of[existing.host] == existing.host;
        existing.host = renumbered[host_of[existing.host]];
    }
    for pad in &merged {
        terminal_copper.push(core::TerminalCopper {
            net,
            terminal: pad.host,
            obstacle: pad.pad,
        });
    }
    let mut index = 0;
    description.terminals.retain(|_| {
        index += 1;
        host_of[index - 1] == index - 1
    });
    merged_pads.extend(merged);
}

pub(super) fn root(parent: &mut [usize], mut item: usize) -> usize {
    while parent[item] != item {
        parent[item] = parent[parent[item]];
        item = parent[item];
    }
    item
}
