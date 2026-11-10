// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The routing order.

use super::*;

impl Router {
    /// Nets in routing order: pour nets first (their connections are local
    /// stubs and vias that must claim the sites next to their pads), then
    /// small nets by span, then the large ones.
    pub(super) fn routing_order(&self) -> Vec<NetId> {
        let mut order: Vec<NetId> = (0..self.board.nets.len() as NetId)
            .filter(|net| self.nets[*net as usize].routable)
            .collect();
        let span = |router: &Self, net: NetId| {
            let (x0, y0, x1, y1) = router.window(net, 0.0);
            (x1 - x0) + (y1 - y0)
        };
        let seed = self.config.seed.or_else(experiment_seed);
        let mut state = seed.unwrap_or(0) ^ 0x5DEE_CE66;
        let jitter: Vec<f64> = (0..self.board.nets.len())
            .map(|_| match seed {
                Some(_) => 0.6 + 0.8 * unit_noise(&mut state),
                None => 1.0,
            })
            .collect();
        order.sort_by_key(|net| {
            (
                self.nets[*net as usize].plane.is_empty(),
                self.board.nets[*net as usize].terminals.len() > 8,
                (span(self, *net) as f64 * jitter[*net as usize]) as u64,
                *net,
            )
        });
        order
    }
}
