// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! The global route as the detailed router's capacitated plan
//! (`pcb_router::capacitated`, the router's `capacitated_corridors`;
//! docs/router.md, "Capacitated corridors (2026-10-24)"): the router reads
//! each net's corridor from it, lowers the capacity of a boundary its
//! lattice search could not pass (never below the tracks routed across
//! it) and has the nets through that boundary planned again, against the
//! others' usage at the prices the negotiation ended with.

use crate::GlobalConfig;
use crate::negotiate::{GlobalRoute, Graph, NetRoute, Plan, plan, pour_owned};
use pcb_router::Board;
use pcb_router::capacitated::{CapacitatedPlan, PlanFrame};

/// The work model of the global route (docs/global-route.md, "Phase 2"):
/// units per shape-weighted distance test of the capacities and per state
/// the searches pop, at 150e6 units a second.
pub const WORK_PER_CAPACITY_TEST: u64 = 5;
pub const WORK_PER_POP: u64 = 30;

/// A global route the router learns on.
#[derive(Clone)]
pub struct CorridorPlan {
    route: GlobalRoute,
    config: GlobalConfig,
    /// Per board net its plan (`None`: nothing to route on the graph).
    plans: Vec<Option<Plan>>,
    /// Per board net its index in `route.nets`.
    index: Vec<Option<usize>>,
    /// Work units so far (the build, the re-plans).
    work: u64,
}

impl CorridorPlan {
    /// The full global route of `board` with `config`.
    pub fn new(board: &Board, config: &GlobalConfig) -> Self {
        let route = crate::route(board, config);
        let owned = pour_owned(board, &route.capacity);
        let plans = (0..board.nets.len())
            .map(|net| {
                plan(board, &route.frame, &route.capacity, config, &owned, net)
                    .map(|(_, plan)| plan)
            })
            .collect();
        let mut index = vec![None; board.nets.len()];
        for (slot, net) in route.nets.iter().enumerate() {
            index[net.net] = Some(slot);
        }
        Self {
            work: WORK_PER_CAPACITY_TEST * route.capacity.work + WORK_PER_POP * route.pops,
            route,
            config: config.clone(),
            plans,
            index,
        }
    }

    /// The global route as it stands.
    pub fn route(&self) -> &GlobalRoute {
        &self.route
    }

    fn net_route(&self, net: usize) -> Option<&NetRoute> {
        self.index
            .get(net)
            .copied()
            .flatten()
            .map(|slot| &self.route.nets[slot])
    }

    fn split(&self, boundary: u32) -> (usize, usize) {
        let per = self.route.frame.layers * self.route.frame.tiles();
        (boundary as usize / per, boundary as usize % per)
    }
}

/// Registers `CorridorPlan` (the default global configuration) as the
/// router's capacitated planner.
pub fn register_corridor_planner() {
    pcb_router::capacitated::register_planner(|board| {
        Box::new(CorridorPlan::new(board, &GlobalConfig::default()))
    });
}

impl CapacitatedPlan for CorridorPlan {
    fn boxed_clone(&self) -> Box<dyn CapacitatedPlan> {
        Box::new(self.clone())
    }

    fn frame(&self) -> PlanFrame {
        let frame = &self.route.frame;
        PlanFrame {
            origin: frame.origin,
            tile_mm: frame.tile_mm,
            tiles_x: frame.tiles_x,
            tiles_y: frame.tiles_y,
            layers: frame.layers,
        }
    }

    fn planned(&self, net: usize) -> bool {
        self.plans.get(net).is_some_and(Option::is_some) && self.net_route(net).is_some()
    }

    fn routed(&self, net: usize) -> bool {
        self.net_route(net).is_some_and(|route| route.routed)
    }

    fn states(&self, net: usize) -> &[u32] {
        self.net_route(net).map_or(&[], |route| &route.states)
    }

    fn crossings(&self, net: usize) -> &[u32] {
        self.net_route(net).map_or(&[], |route| &route.crossings)
    }

    fn vias(&self, net: usize) -> &[u32] {
        self.net_route(net).map_or(&[], |route| &route.vias)
    }

    fn demand(&self, net: usize) -> f32 {
        self.plans
            .get(net)
            .and_then(Option::as_ref)
            .map_or(1.0, |plan| plan.demand)
    }

    fn capacity(&self, boundary: u32) -> f32 {
        let (direction, index) = self.split(boundary);
        self.route.capacity.edges[direction][index]
    }

    fn lower(&mut self, boundary: u32, floor: f32) -> bool {
        let (direction, index) = self.split(boundary);
        let capacity = &mut self.route.capacity.edges[direction][index];
        let lowered = (*capacity - 1.0).max(floor).max(0.0);
        if lowered < *capacity - 1.0e-4 {
            *capacity = lowered;
            true
        } else {
            false
        }
    }

    fn lower_via(&mut self, tile: u32, floor: f32) -> bool {
        let capacity = &mut self.route.capacity.vias[tile as usize];
        let lowered = (*capacity - 1.0).max(floor).max(0.0);
        if lowered < *capacity - 1.0e-4 {
            *capacity = lowered;
            true
        } else {
            false
        }
    }

    fn nets_through(&self, boundaries: &[u32], via_tiles: &[u32]) -> Vec<usize> {
        let mut nets: Vec<usize> = self
            .route
            .nets
            .iter()
            .filter(|route| {
                route.crossings.iter().any(|edge| boundaries.contains(edge))
                    || route.vias.iter().any(|tile| via_tiles.contains(tile))
            })
            .map(|route| route.net)
            .collect();
        nets.sort_unstable();
        nets.dedup();
        nets
    }

    fn replan(&mut self, nets: &[usize]) {
        let GlobalRoute {
            frame,
            capacity,
            usage,
            via_usage,
            nets: routes,
            pops,
            history,
            via_history,
            present,
            ..
        } = &mut self.route;
        let mut graph = Graph::new(frame, capacity, &self.config);
        graph.usage = std::mem::take(usage);
        graph.history = std::mem::take(history);
        graph.via_usage = std::mem::take(via_usage);
        graph.via_history = std::mem::take(via_history);
        // Every net's old route leaves first.
        for net in nets {
            let Some(plan) = self.plans.get(*net).and_then(Option::as_ref) else {
                continue;
            };
            if let Some(slot) = self.index[*net] {
                let old = std::mem::take(&mut routes[slot]);
                graph.take(&old, plan.demand, -1.0);
                routes[slot] = NetRoute {
                    net: *net,
                    ..NetRoute::default()
                };
            }
        }
        for net in nets {
            let Some(plan) = self.plans.get(*net).and_then(Option::as_ref) else {
                continue;
            };
            let route = graph.route(plan, *present);
            graph.take(&route, plan.demand, 1.0);
            match self.index[*net] {
                Some(slot) => routes[slot] = route,
                None => {
                    self.index[*net] = Some(routes.len());
                    routes.push(route);
                }
            }
        }
        *pops += graph.pops;
        self.work += WORK_PER_POP * graph.pops;
        *usage = std::mem::take(&mut graph.usage);
        *history = std::mem::take(&mut graph.history);
        *via_usage = std::mem::take(&mut graph.via_usage);
        *via_history = std::mem::take(&mut graph.via_history);
    }

    fn work_units(&self) -> u64 {
        self.work
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capacity::tests::{empty_board, wall};

    #[test]
    fn a_lowered_boundary_sends_the_net_around() {
        // A one-layer board 9 x 6 mm, a wall across x = 4.5 with two gaps:
        // the net takes the straight one; once the plan learns that its
        // boundary holds nothing, the net goes through the other.
        let mut board = empty_board(9.0, 6.0, 1);
        board.nets[0].terminals[0].anchor = [0.5, 0.75];
        board.nets[0].terminals[1].anchor = [8.5, 0.75];
        board.obstacles.push(wall([4.5, 3.0], [0.2, 1.5], 1));
        board.obstacles.push(wall([4.5, 5.6], [0.2, 0.4], 1));
        let config = GlobalConfig::default();
        let mut plan = CorridorPlan::new(&board, &config);
        assert!(plan.planned(0) && plan.routed(0));
        let first: Vec<u32> = plan.crossings(0).to_vec();
        // The boundary between tiles 2 and 3 of the first row.
        let frame = plan.frame();
        let tiles = frame.tiles() as u32;
        let gap = first
            .iter()
            .copied()
            .find(|boundary| *boundary < tiles && boundary % frame.tiles_x as u32 == 2)
            .expect("the route crosses x = 4.5 in the first row");
        while plan.lower(gap, 0.0) {}
        assert_eq!(plan.capacity(gap), 0.0);
        assert!(!plan.lower(gap, 0.0));
        assert_eq!(plan.nets_through(&[gap], &[]), vec![0]);
        let before = plan.work_units();
        plan.replan(&[0]);
        assert!(plan.work_units() > before);
        assert!(plan.routed(0));
        assert!(!plan.crossings(0).contains(&gap));
        assert!(plan.nets_through(&[gap], &[]).is_empty());
    }

    #[test]
    fn a_floor_keeps_the_capacity() {
        let board = empty_board(6.0, 2.0, 1);
        let mut plan = CorridorPlan::new(&board, &GlobalConfig::default());
        let boundary = plan.crossings(0)[0];
        let capacity = plan.capacity(boundary);
        assert!(capacity >= 1.0);
        assert!(!plan.lower(boundary, capacity));
        assert!(plan.lower(boundary, capacity - 0.5));
        assert_eq!(plan.capacity(boundary), capacity - 0.5);
    }

    /// A board whose wall hides inside a tile: the global route runs
    /// straight along the first row, the lattice cannot pass the wall
    /// there, and only a gap three rows up leads through.
    fn hidden_wall() -> Board {
        use pcb_router::{Net, Obstacle, ObstacleKind, RuleClass, Shape, Terminal};
        let pads = [[0.75, 0.75], [11.25, 0.75]];
        let mut obstacles: Vec<Obstacle> = pads
            .iter()
            .enumerate()
            .map(|(index, at)| Obstacle {
                shape: Shape::Circle {
                    center: *at,
                    radius: 0.3,
                },
                layers: 1,
                kind: ObstacleKind::Copper,
                net: Some(0),
                clearance: 0.0,
                clearance_override: None,
                blocks_tracks: true,
                blocks_vias: true,
                label: format!("A.{index}"),
            })
            .collect();
        obstacles.push(Obstacle {
            label: "wall".into(),
            kind: ObstacleKind::Keepout,
            net: None,
            ..wall([5.2, 2.25], [0.2, 2.25], 1)
        });
        let terminals = pads
            .iter()
            .enumerate()
            .map(|(index, at)| Terminal {
                anchor: *at,
                layers: 1,
                pad: index,
                contact: None,
                label: format!("A.{index}"),
            })
            .collect();
        Board {
            layer_count: 1,
            outline: vec![[0.0, 0.0], [12.0, 0.0], [12.0, 6.0], [0.0, 6.0]],
            edge_clearance: 0.1,
            hole_clearance: 0.25,
            hole_to_hole: 0.25,
            classes: vec![RuleClass {
                trace_width: 0.25,
                clearance: 0.2,
                via_diameter: 0.6,
                via_drill: 0.3,
                layers: 0,
            }],
            neck_width: 0.25,
            obstacles,
            nets: vec![Net {
                name: "A".into(),
                class: 0,
                terminals,
            }],
            planes: Vec::new(),
            solid_pads: Vec::new(),
            isolated_pads: Vec::new(),
            solid_pad_obstacles: Vec::new(),
            isolated_pad_obstacles: Vec::new(),
            pad_spokes: Default::default(),
            merged_pads: Vec::new(),
            no_via_pads: Vec::new(),
            fill_passes: Vec::new(),
            pin_row_gaps: Vec::new(),
            terminal_copper: Vec::new(),
            attachment: Default::default(),
        }
    }

    #[test]
    fn the_router_learns_the_hidden_wall_and_routes_through_the_gap() {
        use pcb_router::capacitated::{Learn, corridor_totals};
        register_corridor_planner();
        let board = hidden_wall();
        // The global route alone goes straight along the first row.
        let plan = CorridorPlan::new(&board, &GlobalConfig::default());
        let frame = plan.frame();
        assert!(
            plan.states(0)
                .iter()
                .all(|state| (*state as usize % frame.tiles()) < frame.tiles_x)
        );
        let before = corridor_totals();
        let config = pcb_router::Config {
            pitches: vec![0.1],
            capacitated_corridors: true,
            ..pcb_router::Config::default()
        };
        let result = pcb_router::route(&board, &config);
        let after = corridor_totals();
        assert_eq!(result.status[0], pcb_router::NetStatus::Routed);
        let delta = |field: Learn| after.get(field) - before.get(field);
        assert!(delta(Learn::PlansBuilt) >= 1);
        assert!(delta(Learn::Misses) >= 1, "{after:?}");
        assert!(delta(Learn::Reductions) >= 1, "{after:?}");
        assert!(delta(Learn::Replans) >= 1, "{after:?}");
        // The path found goes round the wall's top end.
        let top = result.routes[0]
            .segments
            .iter()
            .flat_map(|segment| [segment.start[1], segment.end[1]])
            .fold(0.0f64, f64::max);
        assert!(top > 4.5, "{top}");
    }
}
