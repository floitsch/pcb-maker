// Copyright (C) 2026 Toit contributors.

//! The fallback: what the links left open, routed for the pairs' nets
//! with everything else kept, and its work budget.

use super::*;
use pcb_router as core;

/// The fallback router's share of the ladder's budget, at least
/// `MINIMUM_WORK` seconds.
pub(super) const WORK_SHARE: f64 = 0.05;
pub(super) const MINIMUM_WORK: f64 = 1.0;

/// Routes the connections of the pairs `open` (indices into `pairs`) that
/// are still open on `board` (the board as lowered, the pairs' copper kept
/// on it) with every route of `result` kept, and adds what it routes to
/// `result` (the pairs' nets' status from it). Returns its work.
pub(super) fn route_fallback(
    board: &core::Board,
    result: &mut core::RoutingResult,
    pairs: &[Pair],
    open: &BTreeSet<usize>,
    config: &KiCadBoardRouterConfig,
) -> f64 {
    let nets: BTreeSet<core::NetId> = open
        .iter()
        .flat_map(|&index| [pairs[index].positive, pairs[index].negative])
        .collect();
    let mut fallback = with_routes(board, &result.routes);
    keep_existing_copper(&mut fallback, true);
    // Only the pairs' nets route (with their pours, if they have any).
    for (index, net) in fallback.nets.iter_mut().enumerate() {
        if !nets.contains(&(index as core::NetId)) {
            net.terminals.clear();
        }
    }
    let mut router_config = core_config(config);
    keep_planes_off(&mut fallback, &router_config, &nets);
    router_config.verbose = false;
    router_config.deadline_work = Some(work_budget(config.ladder_budget_seconds.unwrap_or(1200.0)));
    router_config.run_work = None;
    let routed = core::route(&fallback, &router_config);
    for &net in &nets {
        let (net, from) = (net as usize, &routed.routes[net as usize]);
        result.routes[net]
            .segments
            .extend(from.segments.iter().cloned());
        result.routes[net].vias.extend(from.vias.iter().cloned());
        result.status[net] = routed.status[net];
        if debug() {
            eprintln!(
                "diff pair fallback: {} {:?}",
                board.nets[net].name, routed.status[net]
            );
        }
    }
    routed.expansions as f64 / core::router::expansions_per_second()
        + routed.build_work
        + routed.pour_work
}

/// The other nets' planes go from a board routed for `nets` alone (their
/// nets do not route there), but an exclusive plane still keeps the
/// pairs' tracks off its layer, as the attempt kept every other net's
/// (vias pass through it).
pub(super) fn keep_planes_off(
    board: &mut core::Board,
    router_config: &core::Config,
    nets: &BTreeSet<core::NetId>,
) {
    if router_config.exclusive_planes {
        let keepouts: Vec<core::Obstacle> = board
            .planes
            .iter()
            .filter(|plane| plane.exclusive && !nets.contains(&plane.net))
            .map(|plane| core::Obstacle {
                shape: core::Shape::Polygon {
                    points: plane.polygon.clone(),
                },
                layers: 1 << plane.layer,
                kind: core::ObstacleKind::Keepout,
                net: None,
                clearance: 0.0,
                clearance_override: None,
                plated: false,
                blocks_tracks: true,
                blocks_vias: false,
                label: "exclusive plane".into(),
            })
            .collect();
        board.obstacles.extend(keepouts);
    }
    board.planes.retain(|plane| nets.contains(&plane.net));
}

/// `board` with `routes` as its copper (existing tracks, vias and their
/// holes, of their nets).
pub(super) fn with_routes(board: &core::Board, routes: &[core::NetRoute]) -> core::Board {
    let mut fallback = board.clone();
    for (net, route) in routes.iter().enumerate() {
        for segment in &route.segments {
            fallback.obstacles.push(core::Obstacle {
                shape: core::Shape::Capsule {
                    start: segment.start,
                    end: segment.end,
                    radius: segment.width / 2.0,
                },
                layers: 1 << segment.layer,
                kind: core::ObstacleKind::Copper,
                net: Some(net as core::NetId),
                clearance: 0.0,
                clearance_override: None,
                plated: false,
                blocks_tracks: true,
                blocks_vias: true,
                label: "existing track".into(),
            });
        }
        for via in &route.vias {
            fallback.obstacles.push(core::Obstacle {
                shape: core::Shape::Circle {
                    center: via.at,
                    radius: via.diameter / 2.0,
                },
                layers: fallback.all_layers(),
                kind: core::ObstacleKind::Copper,
                net: Some(net as core::NetId),
                clearance: 0.0,
                clearance_override: None,
                plated: false,
                blocks_tracks: true,
                blocks_vias: true,
                label: "existing via".into(),
            });
            // Its drill keeps the hole clearance from other nets' copper.
            fallback.obstacles.push(core::Obstacle {
                shape: core::Shape::Circle {
                    center: via.at,
                    radius: via.drill / 2.0,
                },
                layers: fallback.all_layers(),
                kind: core::ObstacleKind::Hole,
                net: Some(net as core::NetId),
                clearance: 0.0,
                clearance_override: None,
                plated: false,
                blocks_tracks: true,
                blocks_vias: true,
                label: "existing via hole".into(),
            });
        }
    }
    fallback
}

/// The fallback router's work deadline for a ladder of `budget` seconds.
pub(crate) fn work_budget(budget: f64) -> f64 {
    // `PCB_ROUTER_DIFF_PAIRS_WORK` (seconds): an experiment's override.
    if let Some(seconds) = std::env::var("PCB_ROUTER_DIFF_PAIRS_WORK")
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
    {
        return seconds;
    }
    (WORK_SHARE * budget).max(MINIMUM_WORK)
}

/// The fallback's work on a layout's own board with `left` seconds of the
/// layout's work left (`work_budget` of it, as a ladder of that budget).
pub fn pairs_fallback_work(left: f64) -> f64 {
    work_budget(left)
}

/// Whether that is the fallback's minimum rather than its share: the
/// layout's budget was spent before its board was written.
pub fn pairs_fallback_short(left: f64) -> bool {
    WORK_SHARE * left < MINIMUM_WORK
}
