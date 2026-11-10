// Copyright (C) 2026 Toit contributors.

//! The pairs' skew: a meander on the shorter net's coupled run.

use super::*;
use pcb_router as core;

/// Whether the pairs' lengths are matched (`match_skew`):
/// `PCB_ROUTER_DIFF_PAIRS_SKEW=0` turns it off (an experiment's switch).
pub(super) fn skew_matching() -> bool {
    std::env::var("PCB_ROUTER_DIFF_PAIRS_SKEW").map_or(true, |value| value != "0")
}

/// Matches a pair's lengths (KiCad's `skew` and `length`): the two nets'
/// lengths from their copper (the routes and the kept links', vias at the
/// stackup's `heights`); where they are further apart than the target (the
/// rules' `skew` maximum, else the pair's gap), or a net is shorter than
/// the rules' `length` minimum, a meander on a straight track of the
/// shorter net's coupled run adds what it lacks: bumps away from its
/// partner, the first placement whose copper keeps every clearance
/// (`refused_groups`), else none. KiCad's gap check skips the meander's
/// tracks (a tuning pattern, `write_pair_copper`).
#[allow(clippy::too_many_arguments)]
pub(super) fn match_skew(
    restored: &mut core::Board,
    result: &core::RoutingResult,
    pairs: &[Pair],
    index: usize,
    kept_runs: &[KeptRun],
    copper_out: &mut PairCopper,
    entry: &mut KiCadDiffPair,
    heights: &[f64],
) {
    let pair = &pairs[index];
    let target = pair.max_skew.unwrap_or(pair.gap);
    entry.skew_target_mm = target;
    let length_of = |net: core::NetId, copper_out: &PairCopper| {
        let route = &result.routes[net as usize];
        let mut tracks: Vec<(usize, Point, Point)> = route
            .segments
            .iter()
            .map(|segment| (segment.layer, segment.start, segment.end))
            .collect();
        let mut vias: Vec<Point> = route.vias.iter().map(|via| via.at).collect();
        let name = &restored.nets[net as usize].name;
        for item in copper_out.items.iter().filter(|item| &item.0 == name) {
            tracks.extend(item.4.iter().copied());
            vias.extend(item.5.iter().copied());
        }
        // A meander's track is in its pattern, not its item.
        let meanders: f64 = copper_out
            .patterns
            .iter()
            .filter(|pattern| &pattern.net == name)
            .map(|pattern| polyline_length(&pattern.points))
            .sum();
        copper_length(&tracks, &vias, heights) + meanders
    };
    let nets = [pair.positive, pair.negative];
    let lengths = nets.map(|net| length_of(net, copper_out));
    let longer = lengths[0].max(lengths[1]);
    let mut goal = longer;
    if let Some((min, opt, _)) = pair.length
        && longer < min
    {
        goal = opt.unwrap_or(min).max(min);
    }
    let w = pair.width;
    let spacing = (w + pair.clearance).max(2.0 * w);
    let cap = (3.0 * w).max(2.0 * pair.gap);
    for polarity in 0..2 {
        let need = goal - lengths[polarity];
        let skewed = (lengths[0] - lengths[1]).abs() > target && lengths[polarity] < longer;
        let short = goal > longer && need > 0.0;
        if need < 0.005 || !(skewed || short) {
            continue;
        }
        // Bumps of at most `cap`: as few as hold the length, as high as
        // needed (bisected).
        let added = |height: f64| {
            polyline_length(&bumps(
                ([0.0, 0.0], [1.0, 0.0]),
                [0.0, 1.0],
                0.0,
                1,
                height,
                spacing,
            )) - spacing
        };
        let count = (need / added(cap)).ceil().max(1.0) as usize;
        let (mut low, mut high) = (0.0, cap);
        for _ in 0..40 {
            let middle = (low + high) / 2.0;
            if count as f64 * added(middle) < need {
                low = middle;
            } else {
                high = middle;
            }
        }
        let height = high;
        let span = (2 * count - 1) as f64 * spacing;
        let margin = spacing.max(2.0 * pair.offset());
        // The straight tracks of the run long enough, the longest first;
        // per track the meander nearest its start, its middle, its end.
        let mut tracks: Vec<(usize, usize, f64)> = Vec::new();
        for (_, items, runs, _) in kept_runs.iter().filter(|run| run.0 == index) {
            let item = &copper_out.items[items[polarity]];
            for position in runs[polarity].clone() {
                let Some(&(_, a, b)) = item.4.get(position) else {
                    continue;
                };
                let length = core::geometry::distance(a, b);
                if length >= span + 2.0 * margin {
                    tracks.push((items[polarity], position, length));
                }
            }
        }
        tracks.sort_by(|x, y| y.2.total_cmp(&x.2).then(x.0.cmp(&y.0)).then(x.1.cmp(&y.1)));
        let sign_of = |item: usize| {
            kept_runs
                .iter()
                .find(|run| run.1[polarity] == item)
                .map_or(1.0, |run| if polarity == 0 { run.3 } else { -run.3 })
        };
        'tracks: for (item, position, length) in tracks {
            let (layer, a, b) = copper_out.items[item].4[position];
            let outward = scale(left(unit(sub(b, a))), sign_of(item));
            for from in [margin, (length - span) / 2.0, length - margin - span] {
                let points = bumps((a, b), outward, from, count, height, spacing);
                let mut group = Group {
                    pair: index,
                    segments: Default::default(),
                    vias: Default::default(),
                    run: Default::default(),
                    moved: false,
                };
                group.segments[polarity] = points
                    .windows(2)
                    .map(|pair_of_points| (layer, pair_of_points[0], pair_of_points[1]))
                    .collect();
                let refused = refused_groups(restored, &result.routes, pairs, &[group]);
                if refused[0].is_some() {
                    continue;
                }
                // The track splits around the meander.
                let (first, last) = (points[0], points[points.len() - 1]);
                let item_copper = &mut copper_out.items[item].4;
                item_copper.splice(position..=position, [(layer, a, first), (layer, last, b)]);
                let net = nets[polarity];
                let pieces: Vec<(Point, Point)> = std::iter::once((a, first))
                    .chain(points.windows(2).map(|w| (w[0], w[1])))
                    .chain(std::iter::once((last, b)))
                    .collect();
                if let Some(obstacle) = restored.obstacles.iter().position(|obstacle| {
                    obstacle.net == Some(net)
                        && obstacle.layers == 1 << layer
                        && matches!(obstacle.shape, core::Shape::Capsule { start, end, .. }
                            if core::geometry::distance(start, a) < 1.0e-9
                                && core::geometry::distance(end, b) < 1.0e-9)
                }) {
                    restored.obstacles.remove(obstacle);
                }
                for (start, end) in pieces {
                    restored.obstacles.push(core::Obstacle {
                        shape: core::Shape::Capsule {
                            start,
                            end,
                            radius: w / 2.0,
                        },
                        layers: 1 << layer,
                        kind: core::ObstacleKind::Copper,
                        net: Some(net),
                        clearance: 0.0,
                        clearance_override: None,
                        plated: false,
                        blocks_tracks: true,
                        blocks_vias: true,
                        label: "existing track".into(),
                    });
                }
                let added = polyline_length(&points) - core::geometry::distance(first, last);
                copper_out.patterns.push(Meander {
                    net: restored.nets[net as usize].name.clone(),
                    width: w,
                    layer,
                    points,
                    gap: pair.gap,
                    amplitude: height,
                    spacing,
                    added,
                });
                entry.meander_mm += added;
                break 'tracks;
            }
        }
    }
}
