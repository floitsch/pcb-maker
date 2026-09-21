// Copyright (C) 2026 Toit contributors.

use super::{DIRECTIONS, GridRouteRequest};
use std::{cmp::Reverse, collections::BinaryHeap};

pub(super) fn prepare(request: &GridRouteRequest) -> Result<(Vec<u64>, u32), String> {
    let mut distances = vec![u64::MAX; request.state_count()?];
    let finishes = request.endpoint_states(request.finish, &request.alternate_finishes)?;
    let mut queue = BinaryHeap::new();
    for finish in finishes {
        distances[finish] = 0;
        queue.push(Reverse((0_u64, finish)));
    }
    let plane = request.width * request.height;
    let mut expansions = 0_u32;
    while let Some(Reverse((cost, state))) = queue.pop() {
        if cost != distances[state] {
            continue;
        }
        expansions = expansions.saturating_add(1);
        let position = request.position(state)?;
        let mut relax = |previous: usize, step: u64| {
            let candidate = cost.saturating_add(step);
            if candidate < distances[previous] {
                distances[previous] = candidate;
                queue.push(Reverse((candidate, previous)));
            }
        };
        // Reverse traversal pays the FORWARD destination's cost and checks
        // the predecessor's outgoing edge. Neither edge masks nor entry
        // penalties are assumed symmetric.
        for (direction, (dx, dy)) in DIRECTIONS.iter().copied().enumerate() {
            let Some(x) = position.x.checked_add_signed(-dx) else {
                continue;
            };
            let Some(y) = position.y.checked_add_signed(-dy) else {
                continue;
            };
            if x >= request.width || y >= request.height {
                continue;
            }
            let previous = position.layer * plane + y * request.width + x;
            if request.blocked[previous]
                || request.blocked_planar_transitions[previous * DIRECTIONS.len() + direction]
            {
                continue;
            }
            let diagonal = dx != 0 && dy != 0;
            if diagonal {
                let horizontal = position.layer * plane + y * request.width + position.x;
                let vertical = position.layer * plane + position.y * request.width + x;
                if request.blocked[horizontal] || request.blocked[vertical] {
                    continue;
                }
            }
            let step = if diagonal {
                request.diagonal_cost
            } else {
                request.straight_cost
            };
            relax(
                previous,
                u64::from(step) + u64::from(request.trace_enter_costs[state]),
            );
        }
        if request.allow_vias && request.via_enter_costs[state] != u32::MAX {
            for layer in 0..request.layers {
                if layer == position.layer {
                    continue;
                }
                let previous = layer * plane + state % plane;
                if !request.blocked[previous] {
                    relax(
                        previous,
                        u64::from(request.via_cost) + u64::from(request.via_enter_costs[state]),
                    );
                }
            }
        }
    }
    Ok((distances, expansions))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GridPosition, GridRouteOutcome, route, route_with_obstacle_distances};

    fn request(width: usize, height: usize, layers: usize) -> GridRouteRequest {
        let states = width * height * layers;
        GridRouteRequest {
            width,
            height,
            layers,
            start: GridPosition {
                x: 0,
                y: 0,
                layer: 0,
            },
            finish: GridPosition {
                x: width - 1,
                y: height - 1,
                layer: 0,
            },
            alternate_starts: vec![],
            alternate_finishes: vec![],
            max_expansions: 10_000,
            straight_cost: 1,
            diagonal_cost: 2,
            bend_cost: 3,
            via_cost: 4,
            allow_vias: true,
            blocked: vec![false; states],
            blocked_planar_transitions: vec![false; states * 8],
            trace_enter_costs: vec![0; states],
            via_enter_costs: vec![0; states],
        }
    }

    #[test]
    fn reverse_distances_honor_directed_edges_and_destination_costs() {
        let mut r = request(3, 1, 1);
        r.trace_enter_costs = vec![99, 5, 7];
        let (d, expanded) = prepare(&r).unwrap();
        assert_eq!(d, [14, 8, 0]);
        assert_eq!(expanded, 3);
        r.blocked_planar_transitions[8] = true; // 1 -> 2 only.
        assert_eq!(prepare(&r).unwrap().0, [u64::MAX, u64::MAX, 0]);

        let mut r = request(1, 1, 2);
        r.finish.layer = 1;
        r.via_enter_costs = vec![u32::MAX, 9];
        assert_eq!(prepare(&r).unwrap().0, [13, 0]);
    }

    #[test]
    fn guided_cost_matches_zero_heuristic_search_on_varied_directed_grids() {
        let mut seed = 0x1742_9ab8_u64;
        let mut random = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..2048 {
            let mut r = request(3, 3, 2);
            r.alternate_starts.push(GridPosition {
                layer: 1,
                ..r.start
            });
            r.alternate_finishes.push(GridPosition {
                layer: 1,
                ..r.finish
            });
            r.straight_cost = 1 + (random() % 7) as u32;
            r.diagonal_cost = 1 + (random() % 11) as u32;
            r.bend_cost = (random() % 20) as u32;
            r.via_cost = (random() % 8) as u32;
            r.allow_vias = random() % 2 == 0;
            for i in 0..r.blocked.len() {
                r.blocked[i] = random() % 5 == 0;
                r.trace_enter_costs[i] = (random() % 9) as u32;
                r.via_enter_costs[i] = if random() % 4 == 0 {
                    u32::MAX
                } else {
                    (random() % 13) as u32
                };
            }
            for edge in &mut r.blocked_planar_transitions {
                *edge = random() % 7 == 0;
            }
            if r.validate().is_err() {
                continue;
            }
            let zeros = vec![0; r.state_count().unwrap()];
            let reference = route(&r, Some(&zeros)).unwrap();
            let (guided, work) = route_with_obstacle_distances(&r).unwrap();
            assert!(work as usize <= zeros.len());
            match (reference, guided) {
                (
                    GridRouteOutcome::Found { cost: a, .. },
                    GridRouteOutcome::Found { cost: b, .. },
                ) => assert_eq!(a, b),
                (GridRouteOutcome::NoPath { .. }, GridRouteOutcome::NoPath { .. }) => (),
                pair => panic!("guided/reference disagreement: {pair:?}"),
            }
        }
    }

    #[test]
    fn preparation_is_separate_from_search_budget_and_proves_disconnection() {
        let mut r = request(21, 1, 1);
        r.max_expansions = 1;
        let (result, prepared) = route_with_obstacle_distances(&r).unwrap();
        assert_eq!(prepared, 21);
        assert!(matches!(
            result,
            GridRouteOutcome::BudgetExhausted { expansions: 2, .. }
        ));
        r.blocked[10] = true;
        let (result, prepared) = route_with_obstacle_distances(&r).unwrap();
        assert_eq!(prepared, 10);
        assert!(matches!(
            result,
            GridRouteOutcome::NoPath { expansions: 0, .. }
        ));
    }
}
