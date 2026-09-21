// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! A deterministic, bounded A* strategy ported from `testing-esp32-duts`.
//!
//! Transport and policy are deliberately outside this crate. Callers provide
//! already-rasterized obstacles and costs; the same kernel can therefore be
//! compared with corridor or visibility strategies on a common candidate.

use std::{
    cmp::Reverse,
    collections::{BTreeSet, BinaryHeap},
};

mod obstacle_distances;

/// Prepare exact distances to the equivalent finishes on the physical grid,
/// excluding bend costs, then guide the heading-aware search with them.
/// Preparation visits each reachable physical state at most once and is
/// reported separately from the original A* expansion budget.
pub fn route_with_obstacle_distances(
    request: &GridRouteRequest,
) -> Result<(GridRouteOutcome, u32), String> {
    request.validate()?;
    let (distances, preparation_expansions) = obstacle_distances::prepare(request)?;
    Ok((route(request, Some(&distances))?, preparation_expansions))
}

const DIRECTIONS: [(isize, isize); 8] = [
    (1, 0),
    (-1, 0),
    (0, 1),
    (0, -1),
    (1, 1),
    (1, -1),
    (-1, 1),
    (-1, -1),
];
const VIA_DIRECTION: usize = DIRECTIONS.len();
const INCOMING_DIRECTION_COUNT: usize = DIRECTIONS.len() + 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GridPosition {
    pub x: usize,
    pub y: usize,
    pub layer: usize,
}

#[derive(Clone, Debug)]
pub struct GridRouteRequest {
    pub width: usize,
    pub height: usize,
    pub layers: usize,
    pub start: GridPosition,
    pub finish: GridPosition,
    /// Additional electrically equivalent endpoints. They share one search
    /// budget with the primary endpoints and do not incur a via transition.
    pub alternate_starts: Vec<GridPosition>,
    pub alternate_finishes: Vec<GridPosition>,
    pub max_expansions: u32,
    pub straight_cost: u32,
    pub diagonal_cost: u32,
    pub bend_cost: u32,
    pub via_cost: u32,
    pub allow_vias: bool,
    pub blocked: Vec<bool>,
    /// Directional planar edges rejected after exact continuous-geometry
    /// checking. Indexed by `grid_state * 8 + direction`.
    pub blocked_planar_transitions: Vec<bool>,
    /// Additional cost paid when copper enters a state.
    pub trace_enter_costs: Vec<u32>,
    /// Additional cost paid when a via enters a layer at this cell.
    /// `u32::MAX` forbids that destination.
    pub via_enter_costs: Vec<u32>,
}

impl GridRouteRequest {
    pub fn state_count(&self) -> Result<usize, String> {
        self.width
            .checked_mul(self.height)
            .and_then(|count| count.checked_mul(self.layers))
            .ok_or_else(|| "grid dimensions overflow".into())
    }

    pub fn state_index(&self, position: GridPosition) -> Result<usize, String> {
        if position.x >= self.width || position.y >= self.height || position.layer >= self.layers {
            return Err("grid position is outside the routing volume".into());
        }
        Ok(position.layer * self.width * self.height + position.y * self.width + position.x)
    }

    pub fn position(&self, state: usize) -> Result<GridPosition, String> {
        if state >= self.state_count()? {
            return Err("grid state is outside the routing volume".into());
        }
        let plane = self.width * self.height;
        let cell = state % plane;
        Ok(GridPosition {
            x: cell % self.width,
            y: cell / self.width,
            layer: state / plane,
        })
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.width == 0 || self.height == 0 || self.layers == 0 {
            return Err("grid dimensions and layer count must be non-zero".into());
        }
        let states = self.state_count()?;
        let transitions = states
            .checked_mul(DIRECTIONS.len())
            .ok_or_else(|| "grid transition count overflow".to_string())?;
        if self.blocked.len() != states
            || self.blocked_planar_transitions.len() != transitions
            || self.trace_enter_costs.len() != states
            || self.via_enter_costs.len() != states
        {
            return Err("routing state arrays do not match the routing volume".into());
        }
        let starts = self.endpoint_states(self.start, &self.alternate_starts)?;
        let finishes = self.endpoint_states(self.finish, &self.alternate_finishes)?;
        if starts.is_empty() || finishes.is_empty() {
            return Err("route terminal is blocked".into());
        }
        Ok(())
    }

    fn endpoint_states(
        &self,
        primary: GridPosition,
        alternatives: &[GridPosition],
    ) -> Result<BTreeSet<usize>, String> {
        std::iter::once(primary)
            .chain(alternatives.iter().copied())
            .map(|position| self.state_index(position))
            .collect::<Result<BTreeSet<_>, _>>()
            .map(|states| {
                states
                    .into_iter()
                    .filter(|state| !self.blocked[*state])
                    .collect()
            })
    }

    pub fn block_planar_transition(
        &mut self,
        from: GridPosition,
        to: GridPosition,
    ) -> Result<(), String> {
        if from.layer != to.layer {
            return Err("planar transition cannot change layers".into());
        }
        let dx = to.x as isize - from.x as isize;
        let dy = to.y as isize - from.y as isize;
        let direction = DIRECTIONS
            .iter()
            .position(|candidate| *candidate == (dx, dy))
            .ok_or_else(|| "planar transition must join neighboring grid cells".to_string())?;
        let reverse = DIRECTIONS
            .iter()
            .position(|candidate| *candidate == (-dx, -dy))
            .expect("direction table is symmetric");
        let from_state = self.state_index(from)?;
        let to_state = self.state_index(to)?;
        self.blocked_planar_transitions[from_state * DIRECTIONS.len() + direction] = true;
        self.blocked_planar_transitions[to_state * DIRECTIONS.len() + reverse] = true;
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GridRouteOutcome {
    Found {
        path: Vec<GridPosition>,
        cost: u32,
        expansions: u32,
    },
    NoPath {
        expansions: u32,
        /// Unique blocked grid states adjacent to the explored search volume.
        /// This is solver-neutral failure evidence; semantic ownership remains
        /// the adapter's responsibility.
        blocked_frontier: Vec<GridPosition>,
    },
    BudgetExhausted {
        expansions: u32,
        blocked_frontier: Vec<GridPosition>,
    },
}

pub trait GridRouter {
    fn name(&self) -> &'static str;
    fn route(&mut self, request: &GridRouteRequest) -> Result<GridRouteOutcome, String>;
}

/// Cost-independent connectivity result over the exact routing graph.
///
/// Unlike A*, this search visits each physical grid state at most once. It is
/// useful as a conservative preflight before a more expensive search whose
/// state also includes heading, cost, or other optimization history.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GridReachabilityOutcome {
    Reachable { expansions: u32 },
    Unreachable { expansions: u32 },
}

pub trait GridReachabilityChecker {
    fn name(&self) -> &'static str;
    fn check(&mut self, request: &GridRouteRequest) -> Result<GridReachabilityOutcome, String>;
}

/// Deterministic CPU flood fill. The flat state arrays and frontier make this
/// deliberately replaceable by a parallel/GPU reachability implementation.
#[derive(Clone, Copy, Debug, Default)]
pub struct GridFloodFill;

impl GridReachabilityChecker for GridFloodFill {
    fn name(&self) -> &'static str {
        "grid-flood-fill-v1"
    }

    fn check(&mut self, request: &GridRouteRequest) -> Result<GridReachabilityOutcome, String> {
        request.validate()?;
        reachability(request)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DutAStar;

impl GridRouter for DutAStar {
    fn name(&self) -> &'static str {
        "testing-esp32-duts-astar-v2"
    }

    fn route(&mut self, request: &GridRouteRequest) -> Result<GridRouteOutcome, String> {
        request.validate()?;
        route(request, None)
    }
}

fn route(
    request: &GridRouteRequest,
    lower_bounds: Option<&[u64]>,
) -> Result<GridRouteOutcome, String> {
    let grid_states = request.state_count()?;
    let search_states = grid_states
        .checked_mul(INCOMING_DIRECTION_COUNT)
        .ok_or_else(|| "search state count overflow".to_string())?;
    let starts = request.endpoint_states(request.start, &request.alternate_starts)?;
    let finishes = request.endpoint_states(request.finish, &request.alternate_finishes)?;
    let mut frontier = SearchFrontier::new(request, search_states, &starts, lower_bounds);
    let mut expansions = 0_u32;
    let mut blocked_frontier = BTreeSet::new();

    while let Some(Reverse((_estimate, cost, current_search))) = frontier.open.pop() {
        if cost != frontier.scores[current_search] {
            continue;
        }
        expansions = expansions.saturating_add(1);
        if expansions > request.max_expansions {
            return Ok(GridRouteOutcome::BudgetExhausted {
                expansions,
                blocked_frontier: materialize_frontier(request, &blocked_frontier)?,
            });
        }
        let state = grid_state(current_search);
        if finishes.contains(&state) {
            let path = reconstruct(&frontier.parents, current_search)
                .into_iter()
                .map(|state| request.position(state))
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(GridRouteOutcome::Found {
                path,
                cost,
                expansions,
            });
        }

        let position = request.position(state)?;
        let plane = request.width * request.height;
        for (direction, (dx, dy)) in DIRECTIONS.iter().copied().enumerate() {
            let Some(x) = position.x.checked_add_signed(dx) else {
                continue;
            };
            let Some(y) = position.y.checked_add_signed(dy) else {
                continue;
            };
            if x >= request.width || y >= request.height {
                continue;
            }
            let next = position.layer * plane + y * request.width + x;
            if request.blocked_planar_transitions[state * DIRECTIONS.len() + direction] {
                continue;
            }
            if request.blocked[next] {
                blocked_frontier.insert(next);
                continue;
            }
            let diagonal = dx != 0 && dy != 0;
            if diagonal {
                let horizontal = position.layer * plane + position.y * request.width + x;
                let vertical = position.layer * plane + y * request.width + position.x;
                if request.blocked[horizontal] || request.blocked[vertical] {
                    if request.blocked[horizontal] {
                        blocked_frontier.insert(horizontal);
                    }
                    if request.blocked[vertical] {
                        blocked_frontier.insert(vertical);
                    }
                    continue;
                }
            }
            let mut step = if diagonal {
                request.diagonal_cost
            } else {
                request.straight_cost
            };
            if frontier.parents[current_search] != u32::MAX
                && incoming_direction(current_search) != direction
            {
                step = step.saturating_add(request.bend_cost);
            }
            frontier.relax(
                current_search,
                next,
                direction,
                cost.saturating_add(step)
                    .saturating_add(request.trace_enter_costs[next]),
            );
        }

        if request.allow_vias {
            let cell = state % plane;
            for layer in 0..request.layers {
                if layer == position.layer {
                    continue;
                }
                let next = layer * plane + cell;
                let entry = request.via_enter_costs[next];
                if request.blocked[next] || entry == u32::MAX {
                    continue;
                }
                frontier.relax(
                    current_search,
                    next,
                    VIA_DIRECTION,
                    cost.saturating_add(request.via_cost).saturating_add(entry),
                );
            }
        }
    }

    Ok(GridRouteOutcome::NoPath {
        expansions,
        blocked_frontier: materialize_frontier(request, &blocked_frontier)?,
    })
}

fn reachability(request: &GridRouteRequest) -> Result<GridReachabilityOutcome, String> {
    flood(request, true).map(|(outcome, _)| outcome)
}

/// Inspect every state reachable from the supplied equivalent starts, using
/// exactly the same transition and corner rules as the reachability gate.
/// Costs are ignored; forbidden via entries remain forbidden.
pub fn reachable_states(request: &GridRouteRequest) -> Result<Vec<bool>, String> {
    request.validate()?;
    flood(request, false).map(|(_, states)| states)
}

fn flood(
    request: &GridRouteRequest,
    stop_at_finish: bool,
) -> Result<(GridReachabilityOutcome, Vec<bool>), String> {
    let state_count = request.state_count()?;
    let starts = request.endpoint_states(request.start, &request.alternate_starts)?;
    let finishes = request.endpoint_states(request.finish, &request.alternate_finishes)?;
    let plane = request.width * request.height;
    let mut visited = vec![false; state_count];
    let mut frontier: Vec<_> = starts.iter().copied().collect();
    for start in starts {
        visited[start] = true;
    }
    let mut expansions = 0_u32;

    while let Some(state) = frontier.pop() {
        expansions = expansions.saturating_add(1);
        if stop_at_finish && finishes.contains(&state) {
            return Ok((GridReachabilityOutcome::Reachable { expansions }, visited));
        }
        let position = request.position(state)?;
        for (direction, (dx, dy)) in DIRECTIONS.iter().copied().enumerate() {
            let Some(x) = position.x.checked_add_signed(dx) else {
                continue;
            };
            let Some(y) = position.y.checked_add_signed(dy) else {
                continue;
            };
            if x >= request.width || y >= request.height {
                continue;
            }
            let next = position.layer * plane + y * request.width + x;
            if visited[next]
                || request.blocked_planar_transitions[state * DIRECTIONS.len() + direction]
                || request.blocked[next]
            {
                continue;
            }
            if dx != 0 && dy != 0 {
                let horizontal = position.layer * plane + position.y * request.width + x;
                let vertical = position.layer * plane + y * request.width + position.x;
                if request.blocked[horizontal] || request.blocked[vertical] {
                    continue;
                }
            }
            visited[next] = true;
            frontier.push(next);
        }

        if request.allow_vias {
            let cell = state % plane;
            for layer in 0..request.layers {
                if layer == position.layer {
                    continue;
                }
                let next = layer * plane + cell;
                if !visited[next]
                    && !request.blocked[next]
                    && request.via_enter_costs[next] != u32::MAX
                {
                    visited[next] = true;
                    frontier.push(next);
                }
            }
        }
    }

    let outcome = if finishes.iter().any(|&state| visited[state]) {
        GridReachabilityOutcome::Reachable { expansions }
    } else {
        GridReachabilityOutcome::Unreachable { expansions }
    };
    Ok((outcome, visited))
}

fn materialize_frontier(
    request: &GridRouteRequest,
    frontier: &BTreeSet<usize>,
) -> Result<Vec<GridPosition>, String> {
    frontier
        .iter()
        .copied()
        .map(|state| request.position(state))
        .collect()
}

struct SearchFrontier<'a> {
    request: &'a GridRouteRequest,
    lower_bounds: Option<&'a [u64]>,
    scores: Vec<u32>,
    parents: Vec<u32>,
    open: BinaryHeap<Reverse<(u32, u32, usize)>>,
}

impl<'a> SearchFrontier<'a> {
    fn new(
        request: &'a GridRouteRequest,
        search_states: usize,
        starts: &BTreeSet<usize>,
        lower_bounds: Option<&'a [u64]>,
    ) -> Self {
        let mut scores = vec![u32::MAX; search_states];
        let mut open = BinaryHeap::new();
        for &start in starts {
            if lower_bounds.is_some_and(|bounds| bounds[start] == u64::MAX) {
                continue;
            }
            let start_search = search_state(start, 0);
            scores[start_search] = 0;
            let estimate = lower_bounds.map_or_else(
                || heuristic(request, start, None),
                |bounds| bounds[start].min(u64::from(u32::MAX)) as u32,
            );
            open.push(Reverse((estimate, 0, start_search)));
        }
        Self {
            request,
            lower_bounds,
            scores,
            parents: vec![u32::MAX; search_states],
            open,
        }
    }

    fn relax(&mut self, current: usize, next: usize, direction: usize, candidate: u32) {
        if self
            .lower_bounds
            .is_some_and(|bounds| bounds[next] == u64::MAX)
        {
            return;
        }
        let next_search = search_state(next, direction);
        if candidate >= self.scores[next_search] {
            return;
        }
        self.scores[next_search] = candidate;
        self.parents[next_search] = current as u32;
        self.open.push(Reverse((
            candidate.saturating_add(self.lower_bounds.map_or_else(
                || heuristic(self.request, next, Some(direction)),
                |bounds| bounds[next].min(u64::from(u32::MAX)) as u32,
            )),
            candidate,
            next_search,
        )));
    }
}

fn heuristic(request: &GridRouteRequest, state: usize, incoming: Option<usize>) -> u32 {
    let position = request.position(state).expect("validated grid state");
    std::iter::once(request.finish)
        .chain(request.alternate_finishes.iter().copied())
        .filter(|finish| {
            !request.blocked[request.state_index(*finish).expect("validated terminal")]
        })
        .map(|finish| endpoint_heuristic(request, position, finish, incoming))
        .min()
        .unwrap_or(0)
}

fn endpoint_heuristic(
    request: &GridRouteRequest,
    position: GridPosition,
    finish: GridPosition,
    incoming: Option<usize>,
) -> u32 {
    let dx = position.x.abs_diff(finish.x) as u32;
    let dy = position.y.abs_diff(finish.y) as u32;
    let diagonal = dx.min(dy);
    let straight = dx.max(dy) - diagonal;
    let cheapest_diagonal = request
        .diagonal_cost
        .min(request.straight_cost.saturating_mul(2));
    let mut estimate = diagonal
        .saturating_mul(cheapest_diagonal)
        .saturating_add(straight.saturating_mul(request.straight_cost));
    if position.layer != finish.layer {
        estimate = estimate.saturating_add(request.via_cost);
    } else if let Some(incoming) = incoming {
        estimate = estimate.saturating_add(
            minimum_remaining_bends(position, finish, incoming).saturating_mul(request.bend_cost),
        );
    }
    estimate
}

fn minimum_remaining_bends(position: GridPosition, finish: GridPosition, incoming: usize) -> u32 {
    let dx = finish.x as isize - position.x as isize;
    let dy = finish.y as isize - position.y as isize;
    if dx == 0 && dy == 0 {
        return 0;
    }
    let step_x = dx.signum();
    let step_y = dy.signum();
    let incoming = DIRECTIONS.get(incoming).copied();
    if dx == 0 || dy == 0 || dx.abs() == dy.abs() {
        return u32::from(incoming != Some((step_x, step_y)));
    }
    // A non-collinear goal needs at least two future movement directions,
    // hence at least one bend. Claiming more would not be admissible for all
    // diagonal/straight cost ratios and incoming headings.
    1
}

fn search_state(grid_state: usize, incoming_direction: usize) -> usize {
    grid_state * INCOMING_DIRECTION_COUNT + incoming_direction
}

fn grid_state(search_state: usize) -> usize {
    search_state / INCOMING_DIRECTION_COUNT
}

fn incoming_direction(search_state: usize) -> usize {
    search_state % INCOMING_DIRECTION_COUNT
}

fn reconstruct(parents: &[u32], finish: usize) -> Vec<usize> {
    let mut result = Vec::new();
    let mut current = finish;
    loop {
        result.push(grid_state(current));
        let parent = parents[current];
        if parent == u32::MAX {
            break;
        }
        current = parent as usize;
    }
    result.reverse();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(width: usize, height: usize, layers: usize) -> GridRouteRequest {
        let states = width * height * layers;
        GridRouteRequest {
            width,
            height,
            layers,
            start: GridPosition {
                x: 0,
                y: height / 2,
                layer: 0,
            },
            finish: GridPosition {
                x: width - 1,
                y: height / 2,
                layer: 0,
            },
            max_expansions: 10_000,
            alternate_starts: Vec::new(),
            alternate_finishes: Vec::new(),
            straight_cost: 1_000,
            diagonal_cost: 1_414,
            bend_cost: 180,
            via_cost: 6_000,
            allow_vias: true,
            blocked: vec![false; states],
            blocked_planar_transitions: vec![false; states * DIRECTIONS.len()],
            trace_enter_costs: vec![0; states],
            via_enter_costs: vec![0; states],
        }
    }

    #[test]
    fn routes_a_straight_path_deterministically() {
        let request = request(5, 3, 2);
        let mut router = DutAStar;
        let outcome = router.route(&request).unwrap();
        let GridRouteOutcome::Found {
            path,
            cost,
            expansions: _,
        } = outcome
        else {
            panic!("expected a route")
        };
        assert_eq!(cost, 4_000);
        assert_eq!(path.first(), Some(&request.start));
        assert_eq!(path.last(), Some(&request.finish));
    }

    #[test]
    fn changes_layers_around_a_front_layer_wall() {
        let mut request = request(3, 1, 2);
        request.blocked[1] = true;
        let mut router = DutAStar;
        let GridRouteOutcome::Found { path, .. } = router.route(&request).unwrap() else {
            panic!("expected a route")
        };
        assert!(path.iter().any(|position| position.layer == 1));
    }

    #[test]
    fn can_use_a_third_layer() {
        let mut request = request(3, 1, 3);
        request.blocked[1] = true;
        request.blocked[4] = true;
        request.via_enter_costs[3] = u32::MAX;
        let mut router = DutAStar;
        let GridRouteOutcome::Found { path, .. } = router.route(&request).unwrap() else {
            panic!("expected a route")
        };
        assert!(path.iter().any(|position| position.layer == 2));
    }

    #[test]
    fn reports_the_expansion_budget() {
        let mut request = request(5, 3, 2);
        request.max_expansions = 1;
        let mut router = DutAStar;
        assert!(matches!(
            router.route(&request).unwrap(),
            GridRouteOutcome::BudgetExhausted { expansions: 2, .. }
        ));
    }

    #[test]
    fn reachability_rejects_a_two_layer_wall_without_via_entries() {
        let mut request = request(3, 1, 2);
        request.blocked[1] = true;
        request.via_enter_costs[3] = u32::MAX;
        request.via_enter_costs[4] = u32::MAX;
        request.via_enter_costs[5] = u32::MAX;
        let mut checker = GridFloodFill;
        assert_eq!(
            checker.check(&request).unwrap(),
            GridReachabilityOutcome::Unreachable { expansions: 1 }
        );
    }

    #[test]
    fn reachability_obeys_blocked_transitions_and_diagonal_corner_rules() {
        let mut request = request(2, 2, 1);
        request.allow_vias = false;
        request.start = GridPosition {
            x: 0,
            y: 0,
            layer: 0,
        };
        request.finish = GridPosition {
            x: 1,
            y: 1,
            layer: 0,
        };
        request.blocked[1] = true;
        request.blocked[2] = true;
        let mut checker = GridFloodFill;
        assert!(matches!(
            checker.check(&request).unwrap(),
            GridReachabilityOutcome::Unreachable { .. }
        ));

        request.blocked[1] = false;
        request
            .block_planar_transition(
                request.start,
                GridPosition {
                    x: 1,
                    y: 0,
                    layer: 0,
                },
            )
            .unwrap();
        assert!(matches!(
            checker.check(&request).unwrap(),
            GridReachabilityOutcome::Unreachable { .. }
        ));
    }

    #[test]
    fn equivalent_endpoints_choose_the_available_layer_without_vias() {
        let mut r = request(5, 3, 2);
        r.allow_vias = false;
        r.via_enter_costs.fill(u32::MAX);
        r.alternate_starts = vec![GridPosition {
            layer: 1,
            ..r.start
        }];
        r.alternate_finishes = vec![GridPosition {
            layer: 1,
            ..r.finish
        }];
        r.blocked[..15].fill(true);
        let GridRouteOutcome::Found { path, cost, .. } = DutAStar.route(&r).unwrap() else {
            panic!("bottom layer should connect")
        };
        assert!(path.iter().all(|p| p.layer == 1));
        assert_eq!(cost, 4 * r.straight_cost);
        assert!(matches!(
            GridFloodFill.check(&r).unwrap(),
            GridReachabilityOutcome::Reachable { .. }
        ));
        r.max_expansions = 1;
        assert!(matches!(
            DutAStar.route(&r).unwrap(),
            GridRouteOutcome::BudgetExhausted { expansions: 2, .. }
        ));
        r.alternate_finishes[0].layer = 2;
        assert!(r.validate().is_err());
    }

    #[test]
    fn equivalent_endpoint_search_matches_the_best_independent_endpoint_pair() {
        for allow_vias in [false, true] {
            for mask in 0_u32..(1 << 12) {
                let mut r = request(3, 2, 2);
                r.allow_vias = allow_vias;
                r.alternate_starts = vec![GridPosition {
                    layer: 1,
                    ..r.start
                }];
                r.alternate_finishes = vec![GridPosition {
                    layer: 1,
                    ..r.finish
                }];
                for state in 0..12 {
                    r.blocked[state] = mask & (1 << state) != 0;
                }
                if r.validate().is_err() {
                    continue;
                }
                let mut best = None;
                for start in [r.start, r.alternate_starts[0]] {
                    for finish in [r.finish, r.alternate_finishes[0]] {
                        let mut single = r.clone();
                        single.start = start;
                        single.finish = finish;
                        single.alternate_starts.clear();
                        single.alternate_finishes.clear();
                        if let Ok(GridRouteOutcome::Found { cost, .. }) = DutAStar.route(&single) {
                            best = Some(best.map_or(cost, |old: u32| old.min(cost)));
                        }
                    }
                }
                let actual = match DutAStar.route(&r).unwrap() {
                    GridRouteOutcome::Found { cost, path, .. } => {
                        assert!([r.start, r.alternate_starts[0]].contains(path.first().unwrap()));
                        assert!([r.finish, r.alternate_finishes[0]].contains(path.last().unwrap()));
                        Some(cost)
                    }
                    GridRouteOutcome::NoPath { .. } => None,
                    other => panic!("unexpected bound: {other:?}"),
                };
                assert_eq!(actual, best, "mask={mask:012b}, vias={allow_vias}");
                assert_eq!(
                    best.is_some(),
                    matches!(
                        GridFloodFill.check(&r).unwrap(),
                        GridReachabilityOutcome::Reachable { .. }
                    )
                );
            }
        }
    }

    #[test]
    fn reachability_matches_astar_for_every_small_obstacle_mask() {
        for mask in 0_u32..(1 << 6) {
            let mut request = request(3, 2, 1);
            request.allow_vias = false;
            request.start = GridPosition {
                x: 0,
                y: 0,
                layer: 0,
            };
            request.finish = GridPosition {
                x: 2,
                y: 1,
                layer: 0,
            };
            for state in 0..6 {
                request.blocked[state] = mask & (1 << state) != 0;
            }
            let start = request.state_index(request.start).unwrap();
            let finish = request.state_index(request.finish).unwrap();
            request.blocked[start] = false;
            request.blocked[finish] = false;

            let reachable = matches!(
                GridFloodFill.check(&request).unwrap(),
                GridReachabilityOutcome::Reachable { .. }
            );
            let routed = matches!(
                DutAStar.route(&request).unwrap(),
                GridRouteOutcome::Found { .. }
            );
            assert_eq!(reachable, routed, "obstacle mask {mask:06b}");
            assert_eq!(reachable, reachable_states(&request).unwrap()[finish]);
        }
    }

    #[test]
    fn reachability_map_continues_beyond_the_first_finish() {
        let mut r = request(8, 2, 2);
        r.start = GridPosition {
            x: 0,
            y: 0,
            layer: 0,
        };
        r.finish = GridPosition {
            x: 1,
            y: 0,
            layer: 0,
        };
        assert!(reachable_states(&r).unwrap().iter().all(|&v| v));
        r.allow_vias = false;
        let states = reachable_states(&r).unwrap();
        assert!(states[..16].iter().all(|&v| v));
        assert!(states[16..].iter().all(|&v| !v));
    }

    #[test]
    fn dynamic_trace_cost_selects_a_different_path() {
        let mut request = request(5, 3, 1);
        request.allow_vias = false;
        request.trace_enter_costs[7] = 10_000;
        let mut router = DutAStar;
        let GridRouteOutcome::Found { path, cost, .. } = router.route(&request).unwrap() else {
            panic!("expected a route")
        };
        assert!(!path.contains(&GridPosition {
            x: 2,
            y: 1,
            layer: 0
        }));
        assert!(cost < 10_000);
    }

    #[test]
    fn bend_lower_bound_is_zero_only_when_heading_can_reach_the_goal() {
        let position = GridPosition {
            x: 2,
            y: 2,
            layer: 0,
        };
        let east = DIRECTIONS
            .iter()
            .position(|direction| *direction == (1, 0))
            .unwrap();
        let north_east = DIRECTIONS
            .iter()
            .position(|direction| *direction == (1, 1))
            .unwrap();

        assert_eq!(
            minimum_remaining_bends(
                position,
                GridPosition {
                    x: 5,
                    y: 2,
                    layer: 0,
                },
                east,
            ),
            0
        );
        assert_eq!(
            minimum_remaining_bends(
                position,
                GridPosition {
                    x: 5,
                    y: 2,
                    layer: 0,
                },
                north_east,
            ),
            1
        );
        assert_eq!(
            minimum_remaining_bends(
                position,
                GridPosition {
                    x: 7,
                    y: 5,
                    layer: 0,
                },
                east,
            ),
            1
        );
    }
}
