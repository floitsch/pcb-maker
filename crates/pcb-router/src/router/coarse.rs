// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The coarse probe's ends (pcb-kicad's `probe-coarse-placement`, a
//! research measure; docs/placer.md, "A coarse whole first route as the
//! race probe (2026-10-27)"): after `probe`, the board as it stands once
//! the nets in conflict lose their conflicted branches (the hard repair's
//! rip-up without its searches), or the first route's finish (the hard
//! repair and the trouble repair as configured, no polish).

use super::*;

/// A router saved for the race's ripped count (`Router::saved_for_rip_up`):
/// only to go back to.
pub struct RipUpSaved(pub(super) Router);

/// A probe's router while the race's other probes run (`Router::park`):
/// without what `unpark` builds again from the rest (the search records,
/// the occupancy and the tiles' claims, the stamp marks). Its work so far
/// can be read; it routes only once unparked.
pub struct ParkedRouter {
    router: Router,
    /// The occupancy's maps (`Occupancy::len`).
    maps: usize,
}

impl ParkedRouter {
    /// The router's search expansions so far (`Router::expansions`).
    pub fn expansions(&self) -> u64 {
        self.router.expansions()
    }

    /// The router as it was parked: the occupancy and the tiles' claims
    /// counted again from the nets' stamps (the counts are their sum,
    /// whatever the order), the stamp marks clear (as between stamps),
    /// the search records built again (`build_hot`, as `restore_saved`).
    pub fn unpark(self) -> Router {
        let Self { mut router, maps } = self;
        let cells = router.grid.cells();
        router.occupancy = Occupancy::new(maps, cells);
        router.stamp_mark = vec![0; cells.div_ceil(64) * maps];
        router.tile_claimed = vec![vec![0; router.tiles_x * router.tiles_y]; maps];
        for net in 0..router.nets.len() {
            let stamped = std::mem::take(&mut router.nets[net].stamped);
            router.add_stamps(&stamped);
            router.nets[net].stamped = stamped;
        }
        router.build_hot();
        router.mem_update();
        router
    }
}

impl Router {
    /// The negotiation iterations so far and why the last negotiation
    /// ended (`RoutingResult::negotiation_stop`).
    pub fn negotiation_progress(&self) -> (usize, &'static str) {
        (self.iterations, self.negotiation_stop)
    }

    /// What the race's ripped count needs to undo `rip_up_standing` on the
    /// probe's own router (`restore_after_rip_up`): the router but for the
    /// negotiation's history (per layer and cell, and per tile), which
    /// neither the rip-up nor the standing reads or writes, and for what is
    /// rebuilt on the way back (`saved_copy`: the search records, the
    /// occupancy, the tiles' claims, the stamp marks). Until 2026-11-04 the
    /// race ripped a copy of each probe's router without its history
    /// (video's copies up to 381 MB, 2026-11-04): the copy held the
    /// occupancy and the search records as well. Not a router to route on.
    pub fn saved_for_rip_up(&mut self) -> RipUpSaved {
        RipUpSaved(self.saved_without(true))
    }

    /// The router parked while the race's other probes run (`ParkedRouter`):
    /// a finished probe's search records, occupancy, tiles' claims and stamp
    /// marks are built again only for the probe the race keeps. Until
    /// 2026-11-04 every probe's router stayed whole until the last probe
    /// ended (katia's three: 4.1 GB with the search arrays).
    pub fn park(mut self) -> ParkedRouter {
        let maps = self.occupancy.len();
        self.hot = Vec::new();
        self.occupancy = Occupancy::default();
        self.stamp_mark = Vec::new();
        self.tile_claimed = Vec::new();
        self.mem_update();
        ParkedRouter { router: self, maps }
    }

    /// Goes back to the router `saved` (`saved_for_rip_up`) after its
    /// `rip_up_standing`: the router as it was, as a copy ripped in its
    /// place would have left it.
    pub fn restore_after_rip_up(&mut self, saved: RipUpSaved) {
        self.restore_without(saved.0, true);
    }

    /// After `probe`: every net in conflict loses its conflicted branches,
    /// the most conflicted first and the others judged again after each
    /// (`resolve_remaining`'s rip-up, without its hard searches), and the
    /// standing then: no net is left in conflict, so its open terminals
    /// are the board's (`board_open_terminals`) and the pour pads off
    /// their pour. Returns the nets ripped and the standing.
    ///
    /// The most conflicted net is found from a heap of each net's count
    /// as last seen, an upper bound: a rip-up only takes branches away, so
    /// the occupancy only falls and no other net's count rises. The top is
    /// counted again; where it still holds that count it is the most
    /// conflicted (ties to the larger net id, as the scan's maximum of
    /// (count, net) takes them), else it goes back with its count of now.
    /// The same nets in the same order as the scan of every net after
    /// every rip-up (`rip_up_standing_scan`), which took the race's ripped
    /// count up to 11 s on MegaDrive (2026-11-04); checked against it at
    /// every rip-up in debug builds and with `PCB_ROUTER_CHECK_RIP_UP`.
    pub fn rip_up_standing(&mut self) -> (usize, ProbeStanding) {
        let removed = self.rip_up_most_conflicted();
        (removed.len(), self.probe_standing())
    }

    /// `rip_up_standing`'s rip-ups: the nets ripped, in order.
    pub(super) fn rip_up_most_conflicted(&mut self) -> Vec<NetId> {
        let check = cfg!(debug_assertions) || std::env::var_os("PCB_ROUTER_CHECK_RIP_UP").is_some();
        let order = self.routing_order();
        let mut heap: std::collections::BinaryHeap<(usize, NetId)> = order
            .iter()
            .map(|net| (self.conflicts(*net).len(), *net))
            .filter(|(count, _)| *count > 0)
            .collect();
        let mut removed: Vec<NetId> = Vec::new();
        while let Some((bound, net)) = heap.pop() {
            let count = self.conflicts(net).len();
            if check {
                assert!(
                    count <= bound,
                    "net {net}: {count} conflicts after a rip-up, {bound} before"
                );
            }
            if count < bound {
                if count > 0 {
                    heap.push((count, net));
                }
                continue;
            }
            if check {
                assert_eq!(
                    self.most_conflicted(&order, &removed),
                    Some((count, net)),
                    "the heap's most conflicted net against the scan's"
                );
            }
            self.nets[net as usize].stuck = 0;
            self.rip_up_conflicted(net);
            removed.push(net);
        }
        if check {
            assert_eq!(self.most_conflicted(&order, &removed), None);
        }
        removed
    }

    /// The most conflicted net of `order` not in `removed`, by (count,
    /// net), with its count; None where none is in conflict.
    fn most_conflicted(&self, order: &[NetId], removed: &[NetId]) -> Option<(usize, NetId)> {
        order
            .iter()
            .filter(|net| !removed.contains(*net))
            .map(|net| (self.conflicts(*net).len(), *net))
            .filter(|(count, _)| *count > 0)
            .max()
    }

    /// `rip_up_most_conflicted` as it was: every net counted again after
    /// every rip-up (for the tests).
    #[cfg(test)]
    pub(super) fn rip_up_most_conflicted_scan(&mut self) -> Vec<NetId> {
        let order = self.routing_order();
        let mut removed: Vec<NetId> = Vec::new();
        while let Some((_, net)) = self.most_conflicted(&order, &removed) {
            self.nets[net as usize].stuck = 0;
            self.rip_up_conflicted(net);
            removed.push(net);
        }
        removed
    }

    /// After `probe`: the finish of the layout's first route (the hard
    /// repair, the trouble repair where `Config::trouble_repair` has it,
    /// the pours stitched; no clean-up, no via reduction), within the
    /// router's work deadline (`Config::deadline_work`).
    pub fn finish_probe(&mut self) -> RoutingResult {
        let order = self.routing_order();
        self.finish_polished(&order, false)
    }
}
