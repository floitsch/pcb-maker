// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The global route kept up to date while parts move (the placer's
//! measured escape demand, docs/global-route.md, "Where it runs"): a move
//! re-measures the capacities of the tiles around the part's old and new
//! place and routes only the part's nets again, against the usage, the
//! history and the present price the rest's negotiation left.

use crate::GlobalConfig;
use crate::negotiate::{GlobalRoute, Graph, NetRoute, plan, pour_owned};
use pcb_router::Board;
use pcb_router::geometry::Aabb;

/// A global route that follows moves.
#[derive(Clone, Debug)]
pub struct Incremental {
    pub config: GlobalConfig,
    pub route: GlobalRoute,
    /// Per board net its index in `route.nets` (`None`: nothing routed).
    index: Vec<Option<usize>>,
    /// The work of everything done so far (`Capacity::work`'s distance
    /// tests and the states popped), cumulative.
    pub capacity_tests: u64,
    pub pops: u64,
}

impl Incremental {
    /// The full global route of `board`.
    pub fn new(board: &Board, config: &GlobalConfig) -> Self {
        let route = crate::route(board, config);
        let mut index = vec![None; board.nets.len()];
        for (slot, net) in route.nets.iter().enumerate() {
            index[net.net] = Some(slot);
        }
        Self {
            config: config.clone(),
            capacity_tests: route.capacity.work,
            pops: route.pops,
            route,
            index,
        }
    }

    /// The tiles (x0, y0, x1, y1, inclusive) a box in millimetres
    /// touches, grown by `margin` tiles.
    pub fn tiles_of(&self, area: Aabb, margin: usize) -> [usize; 4] {
        let frame = &self.route.frame;
        let tile = |value: f64, axis: usize, count: usize| -> usize {
            (((value - frame.origin[axis]) / frame.tile_mm)
                .floor()
                .max(0.0) as usize)
                .min(count.saturating_sub(1))
        };
        let (x0, y0) = (
            tile(area.minimum[0], 0, frame.tiles_x),
            tile(area.minimum[1], 1, frame.tiles_y),
        );
        let (x1, y1) = (
            tile(area.maximum[0], 0, frame.tiles_x),
            tile(area.maximum[1], 1, frame.tiles_y),
        );
        [
            x0.saturating_sub(margin),
            y0.saturating_sub(margin),
            (x1 + margin).min(frame.tiles_x.saturating_sub(1)),
            (y1 + margin).min(frame.tiles_y.saturating_sub(1)),
        ]
    }

    /// The state after a move: `board` is the board with the parts moved
    /// (the same nets, the same outline and pours), `area` the boxes of
    /// their copper before and after (millimetres), `nets` the nets with a
    /// terminal on them. The capacities of the tiles the area touches
    /// (and one around) are measured again, and the nets are routed again
    /// one by one in the routing order against the others' usage, at the
    /// history and the present price the negotiation ended with.
    pub fn moved(&self, board: &Board, area: Aabb, nets: &[usize]) -> Self {
        let mut next = self.clone();
        let region = self.tiles_of(area, 1);
        let frame = next.route.frame.clone();
        next.capacity_tests +=
            next.route
                .capacity
                .rebuild_region(board, &frame, &self.config, region);
        let capacity = next.route.capacity.clone();
        let owned = pour_owned(board, &capacity);
        let mut plans: Vec<_> = nets
            .iter()
            .filter_map(|net| plan(board, &frame, &capacity, &self.config, &owned, *net))
            .collect();
        plans.sort_by_key(|(key, _)| *key);
        let mut graph = Graph::new(&frame, &capacity, &self.config);
        graph.usage = std::mem::take(&mut next.route.usage);
        graph.history = std::mem::take(&mut next.route.history);
        graph.via_usage = std::mem::take(&mut next.route.via_usage);
        graph.via_history = std::mem::take(&mut next.route.via_history);
        // Every moved net's old route leaves first.
        for net in nets {
            if let Some(slot) = next.index[*net] {
                let old = std::mem::take(&mut next.route.nets[slot]);
                let demand = plans
                    .iter()
                    .find(|(_, plan)| plan.net == *net)
                    .map_or(1.0, |(_, plan)| plan.demand);
                graph.take(&old, demand, -1.0);
                next.route.nets[slot] = NetRoute {
                    net: *net,
                    routed: true,
                    ..NetRoute::default()
                };
            }
        }
        let present = next.route.present;
        for (_, plan) in &plans {
            let route = graph.route(plan, present);
            graph.take(&route, plan.demand, 1.0);
            match next.index[plan.net] {
                Some(slot) => next.route.nets[slot] = route,
                None => {
                    next.index[plan.net] = Some(next.route.nets.len());
                    next.route.nets.push(route);
                }
            }
        }
        next.pops += graph.pops;
        next.route.usage = std::mem::take(&mut graph.usage);
        next.route.history = std::mem::take(&mut graph.history);
        next.route.via_usage = std::mem::take(&mut graph.via_usage);
        next.route.via_history = std::mem::take(&mut graph.via_history);
        drop(graph);
        next.route.capacity = capacity;
        next
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capacity::tests::{empty_board, wall};

    #[test]
    fn a_part_moved_and_back_gives_the_same_capacities() {
        // A wall with a gap the net must pass; the wall moves away and
        // back: the capacities measured again equal the full build's.
        let mut board = empty_board(9.0, 6.0, 1);
        board.nets[0].terminals[1].anchor = [8.5, 0.5];
        board.obstacles.push(wall([4.5, 2.0], [0.3, 2.0], 1));
        let config = GlobalConfig::default();
        let first = Incremental::new(&board, &config);
        let mut moved = board.clone();
        moved.obstacles[0] = wall([4.5, 4.5], [0.3, 1.0], 1);
        let area = Aabb {
            minimum: [4.0, 0.0],
            maximum: [5.0, 6.0],
        };
        let after = first.moved(&moved, area, &[0]);
        let full = Incremental::new(&moved, &config);
        assert_eq!(after.route.capacity.edges, full.route.capacity.edges);
        assert_eq!(after.route.capacity.vias, full.route.capacity.vias);
        let back = after.moved(&board, area, &[0]);
        assert_eq!(back.route.capacity.edges, first.route.capacity.edges);
        assert!(back.route.nets.iter().all(|net| net.routed));
    }
}
