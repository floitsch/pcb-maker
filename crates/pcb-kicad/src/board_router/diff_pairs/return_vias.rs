// Copyright (C) 2026 Toit contributors.

//! Return vias of the reference net (GND) beside the kept links' vias.

use super::*;
use pcb_router as core;

/// Return vias: the ring of a return via at most this far from the ring
/// of the pair's via it stands beside (millimetres), searched outward in
/// `RETURN_STEP`s from touching, at `RETURN_ANGLES` from the direction
/// away from the via's partner (degrees).
pub(super) const RETURN_GAP: f64 = 0.5;
pub(super) const RETURN_STEP: f64 = 0.05;
pub(super) const RETURN_ANGLES: [f64; 9] =
    [0.0, 30.0, -30.0, 60.0, -60.0, 90.0, -90.0, 120.0, -120.0];

/// Whether the kept links' vias get return vias (`place_return_vias`):
/// the settings' `diff_pair_return_vias`, else
/// `PCB_ROUTER_DIFF_PAIRS_RETURN_VIAS` (`0` or `1`), else on.
pub(super) fn return_vias_on(config: &KiCadBoardRouterConfig) -> bool {
    config.diff_pair_return_vias.unwrap_or_else(|| {
        std::env::var("PCB_ROUTER_DIFF_PAIRS_RETURN_VIAS").map_or(true, |value| value != "0")
    })
}

/// The return vias' net: the settings' `diff_pair_return_net` (a net of
/// the board by name), else the net named `GND` (past a sheet path), else
/// of the nets whose name ends in `GND` the one with the most pads; only a
/// net with a pour (`Board::planes`), which the vias join.
pub(super) fn return_net(
    board: &core::Board,
    config: &KiCadBoardRouterConfig,
) -> Option<core::NetId> {
    let poured = |net: usize| board.planes.iter().any(|plane| plane.net as usize == net);
    let short = |name: &str| name.rsplit('/').next().unwrap_or(name).to_ascii_uppercase();
    if let Some(name) = &config.diff_pair_return_net {
        let wanted = crate::kicad_nets::normalize_net(name);
        return board
            .nets
            .iter()
            .position(|net| crate::kicad_nets::normalize_net(&net.name) == wanted)
            .filter(|&net| poured(net))
            .map(|net| net as core::NetId);
    }
    if let Some(net) =
        (0..board.nets.len()).find(|&net| short(&board.nets[net].name) == "GND" && poured(net))
    {
        return Some(net as core::NetId);
    }
    (0..board.nets.len())
        .filter(|&net| short(&board.nets[net].name).ends_with("GND") && poured(net))
        .max_by_key(|&net| (board.nets[net].terminals.len(), std::cmp::Reverse(net)))
        .map(|net| net as core::NetId)
}

/// Beside each via of a kept link (`kept_runs`' copper), a via of the
/// `reference` net (GND) where one fits: its ring within `RETURN_GAP` of
/// the pair via's ring, first straight out from the via's partner, then
/// turned by up to 120°; inside a pour of the reference net (on some
/// layer, outside the pours above it there), and keeping every clearance
/// the router checks (`verify` with the project's rules: other nets'
/// copper and holes, the edge, the other return vias). The vias are the
/// pair's size and join the routes of the reference net. Each pair's
/// `pair_vias` and `return_vias` count them.
#[allow(clippy::too_many_arguments)]
pub(super) fn place_return_vias(
    board: &core::Board,
    result: &mut core::RoutingResult,
    pairs: &[Pair],
    kept_runs: &[KeptRun],
    copper: &PairCopper,
    entries: &mut [KiCadDiffPair],
    reference: core::NetId,
    config: &KiCadBoardRouterConfig,
) {
    let split = config
        .net_rules
        .as_deref()
        .map(|rules| core::rules::ClassRules::split(board, rules));
    let (checked, rules) = match &split {
        Some((board, rules)) => (board, Some(rules)),
        None => (board, None),
    };
    let reference_name = &board.nets[reference as usize].name;
    // Every kept link's vias (their holes: hole to hole and hole clearance,
    // which `verify` does not check against the board's via obstacles).
    let pair_holes: Vec<(Point, f64, f64)> = kept_runs
        .iter()
        .flat_map(|&(index, items, ..)| {
            let pair = &pairs[index];
            items.into_iter().flat_map(move |item| {
                copper.items[item]
                    .5
                    .iter()
                    .map(move |&at| (at, pair.via_diameter, pair.via_drill))
            })
        })
        .collect();
    let mut placed: Vec<core::Via> = Vec::new();
    for &(index, items, ..) in kept_runs {
        let pair = &pairs[index];
        let vias = [&copper.items[items[0]].5, &copper.items[items[1]].5];
        entries[index].pair_vias += vias[0].len() + vias[1].len();
        for polarity in 0..2 {
            for &at in vias[polarity] {
                let partner = vias[1 - polarity].iter().copied().min_by(|a, b| {
                    core::geometry::distance(*a, at).total_cmp(&core::geometry::distance(*b, at))
                });
                let away = partner
                    .map(|partner| unit(sub(at, partner)))
                    .filter(|direction| norm(*direction) > 0.5)
                    .unwrap_or([1.0, 0.0]);
                // The routes and obstacles near this via, for its candidates.
                let window = CHECK_REACH + pair.via_diameter + RETURN_GAP;
                let close =
                    |a: Point, b: Point| core::geometry::point_segment_distance(at, a, b) <= window;
                let local: Vec<core::NetRoute> = result
                    .routes
                    .iter()
                    .take(board.nets.len())
                    .map(|route| core::NetRoute {
                        segments: route
                            .segments
                            .iter()
                            .filter(|segment| close(segment.start, segment.end))
                            .cloned()
                            .collect(),
                        vias: route
                            .vias
                            .iter()
                            .filter(|other| close(other.at, other.at))
                            .cloned()
                            .collect(),
                    })
                    .chain(std::iter::repeat_with(core::NetRoute::default))
                    .take(board.nets.len())
                    .collect();
                let mut local = local;
                local[reference as usize].vias.extend(
                    placed
                        .iter()
                        .filter(|other| close(other.at, other.at))
                        .cloned(),
                );
                let near = obstacles_near(board, at, window);
                let touching = pair.via_diameter;
                let steps = (RETURN_GAP / RETURN_STEP).round() as usize;
                let found = (1..=steps).find_map(|step| {
                    let distance = touching + step as f64 * RETURN_STEP;
                    RETURN_ANGLES.iter().find_map(|&angle| {
                        let candidate = add(at, scale(turned(away, angle), distance));
                        let via = core::Via {
                            at: candidate,
                            diameter: pair.via_diameter,
                            drill: pair.via_drill,
                            span: None,
                        };
                        (joins_pour(board, &local, &near, reference, &via)
                            && return_via_clear(
                                checked,
                                rules,
                                &local,
                                &near,
                                reference,
                                reference_name,
                                &placed,
                                &pair_holes,
                                &via,
                            ))
                        .then_some(via)
                    })
                });
                if let Some(via) = found {
                    placed.push(via);
                    entries[index].return_vias += 1;
                }
            }
        }
    }
    if debug() {
        eprintln!(
            "diff pairs: {} return vias of {reference_name}",
            placed.len()
        );
    }
    result.routes[reference as usize].vias.extend(placed);
}

/// Whether a via of `reference` joins one of its pours: on a layer where a
/// pour of the net holds it (inside its outline, outside the pours above
/// it), the fill can reach the via's ring. Points around the ring, a fill's
/// half minimum width out, are tested: one counts where it lies in the pour
/// and every other net's copper on the layer (the board's and the routes')
/// keeps the larger of the pour's and its own class's clearance and that
/// half width from it (holes the hole clearance, the net's own pads that
/// join by a thermal relief the pour's thermal gap: m2_ground_hole's 6 mm
/// GND pad on oculink_m2 left a return via in its gap); six of sixteen must, on
/// two layers (KiCad calls a via joined on one layer dangling). KiCad joins
/// a via to the fill it touches (no thermal relief on a solid zone's
/// vias).
pub(super) fn joins_pour(
    board: &core::Board,
    local: &[core::NetRoute],
    near: &[usize],
    reference: core::NetId,
    via: &core::Via,
) -> bool {
    // The clearance a pour keeps from another net's copper: the larger of
    // its own and that net's class's (KiCad's fill takes the larger).
    let clearance_of = |net: usize| {
        board
            .nets
            .get(net)
            .map_or(0.0, |net| board.classes[net.class].clearance)
    };
    // The net's pads that join the pour by a thermal relief: its fill
    // keeps the thermal gap from them (a solid pad it touches).
    let mut own_pads: Vec<usize> = board.nets[reference as usize]
        .terminals
        .iter()
        .map(|terminal| terminal.pad)
        .chain(
            board
                .merged_pads
                .iter()
                .filter(|merged| merged.net == reference)
                .map(|merged| merged.pad),
        )
        .filter(|pad| board.solid_pad_obstacles.binary_search(pad).is_err())
        .collect();
    own_pads.sort_unstable();
    own_pads.dedup();
    let reaches = |plane: &core::Plane| {
        if plane.net != reference
            || !core::geometry::point_in_polygon(via.at, &plane.polygon)
            || plane
                .excluded
                .iter()
                .chain(&plane.above)
                .any(|area| core::geometry::point_in_polygon(via.at, area))
        {
            return false;
        }
        let class = board.classes[plane.class];
        let half = class.trace_width / 2.0;
        let keep = class.clearance + half;
        let layer = plane.layer;
        let fillable = |point: Point| {
            core::geometry::point_in_polygon(point, &plane.polygon)
                && near.iter().all(|&index| {
                    let obstacle = &board.obstacles[index];
                    let theirs = match obstacle.kind {
                        core::ObstacleKind::Hole => board.hole_clearance,
                        _ => obstacle.net.map_or(0.0, |net| clearance_of(net as usize)),
                    };
                    if own_pads.binary_search(&index).is_ok() {
                        // The fill's thermal gap around the net's own pads.
                        return obstacle.layers & (1 << layer) == 0
                            || obstacle.shape.distance_to_point(point) >= plane.thermal_gap + half;
                    }
                    (obstacle.net == Some(reference) && obstacle.kind != core::ObstacleKind::Hole)
                        || obstacle.layers & (1 << layer) == 0
                        || obstacle.shape.distance_to_point(point)
                            >= keep.max(obstacle.clearance.max(theirs) + half)
                })
                && local.iter().enumerate().all(|(net, route)| {
                    let keep = keep.max(clearance_of(net) + half);
                    net == reference as usize
                        || (route.segments.iter().all(|segment| {
                            segment.layer != layer
                                || core::geometry::point_segment_distance(
                                    point,
                                    segment.start,
                                    segment.end,
                                ) - segment.width / 2.0
                                    >= keep
                        }) && route.vias.iter().all(|other| {
                            core::geometry::distance(point, other.at) - other.diameter / 2.0 >= keep
                        }))
                })
        };
        (0..16)
            .filter(|&step| {
                let angle = step as f64 * 22.5;
                fillable(add(
                    via.at,
                    scale(turned([1.0, 0.0], angle), via.diameter / 2.0 + half),
                ))
            })
            .count()
            >= 6
    };
    // KiCad calls a via joined on one layer only dangling: two layers.
    let mut layers: Vec<usize> = board
        .planes
        .iter()
        .filter(|plane| reaches(plane))
        .map(|plane| plane.layer)
        .collect();
    layers.sort_unstable();
    layers.dedup();
    layers.len() >= 2
}

/// Whether a return via of `reference` at `via` keeps every clearance the
/// router checks (`verify`, with the project's rules where given) from the
/// routes near it (`local`, the obstacles `near`), the board's obstacles,
/// the kept links' vias (`pair_holes`) and the return vias `placed`.
#[allow(clippy::too_many_arguments)]
pub(super) fn return_via_clear(
    board: &core::Board,
    rules: Option<&core::rules::ClassRules>,
    local: &[core::NetRoute],
    near: &[usize],
    reference: core::NetId,
    reference_name: &str,
    placed: &[core::Via],
    pair_holes: &[(Point, f64, f64)],
    via: &core::Via,
) -> bool {
    let mut routes = local.to_vec();
    routes[reference as usize] = core::NetRoute::default();
    // Only the new via of the reference net: a finding of that net, or
    // naming its via, is the new via's.
    routes[reference as usize].vias.push(via.clone());
    // What `verify` leaves out: hole to hole from every hole (the
    // reference net's own too), the other return vias and the reference
    // net's vias, and the reference net's own copper (no via in its pads).
    let holes_apart = |at: Point, drill: f64| {
        core::geometry::distance(at, via.at) - (drill + via.drill) / 2.0 >= board.hole_to_hole
    };
    if !placed
        .iter()
        .chain(&local[reference as usize].vias)
        .all(|other| {
            holes_apart(other.at, other.drill)
                && core::geometry::distance(other.at, via.at)
                    >= (other.diameter + via.diameter) / 2.0
        })
    {
        return false;
    }
    // The kept links' vias: hole to hole, and each one's drill the hole
    // clearance from the other's ring.
    if !pair_holes.iter().all(|&(at, diameter, drill)| {
        let distance = core::geometry::distance(at, via.at);
        holes_apart(at, drill)
            && distance - drill / 2.0 - via.diameter / 2.0 >= board.hole_clearance
            && distance - via.drill / 2.0 - diameter / 2.0 >= board.hole_clearance
    }) {
        return false;
    }
    for &index in near {
        let obstacle = &board.obstacles[index];
        let distance = obstacle.shape.distance_to_point(via.at);
        match obstacle.kind {
            core::ObstacleKind::Hole if distance - via.drill / 2.0 < board.hole_to_hole => {
                return false;
            }
            core::ObstacleKind::Copper
                if obstacle.net == Some(reference) && distance < via.diameter / 2.0 =>
            {
                return false;
            }
            _ => {}
        }
    }
    let own = format!("via of {reference_name}");
    let hole = format!("via hole of {reference_name}");
    !core::verify::verify_with_rules(board, &routes, rules)
        .iter()
        .any(|violation| {
            violation.net == reference
                || violation.other == own
                || violation.other == hole
                || violation.other.starts_with(&format!("{own} "))
        })
}
