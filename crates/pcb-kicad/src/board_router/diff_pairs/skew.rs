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
    let nets = [pair.positive, pair.negative];
    let lengths = nets.map(|net| net_length(restored, result, net, copper_out, heights));
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
    let skew = Skew {
        result,
        pairs,
        index,
        kept_runs,
    };
    for polarity in 0..2 {
        let need = goal - lengths[polarity];
        let skewed = (lengths[0] - lengths[1]).abs() > target && lengths[polarity] < longer;
        let short = goal > longer && need > 0.0;
        if need < 0.005 || !(skewed || short) {
            continue;
        }
        let shape = MeanderShape::of(need, cap, spacing, pair);
        let tracks = skew.meander_tracks(copper_out, polarity, &shape);
        skew.place_meander(restored, copper_out, entry, polarity, (&shape, tracks));
    }
}

/// A net's length from its copper: its route's tracks and vias (at the
/// stackup's `heights`), the kept links' copper of it, and its meanders.
fn net_length(
    restored: &core::Board,
    result: &core::RoutingResult,
    net: core::NetId,
    copper_out: &PairCopper,
    heights: &[f64],
) -> f64 {
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
}

/// A meander that adds `need`: `count` bumps of `height` (at most the
/// cap, as few as hold the length, as high as needed), `spacing` apart,
/// `span` long, at least `margin` from a track's ends.
struct MeanderShape {
    count: usize,
    height: f64,
    spacing: f64,
    span: f64,
    margin: f64,
}

impl MeanderShape {
    /// The meander that adds `need` with bumps of at most `cap`, `spacing`
    /// apart, on a track of `pair`.
    fn of(need: f64, cap: f64, spacing: f64, pair: &Pair) -> Self {
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
        MeanderShape {
            count,
            height,
            spacing,
            span,
            margin,
        }
    }
}

/// A pair's skew matching (`match_skew`): the route, the pairs, the pair's
/// index and the kept links' runs.
struct Skew<'a> {
    result: &'a core::RoutingResult,
    pairs: &'a [Pair],
    index: usize,
    kept_runs: &'a [KeptRun],
}

impl Skew<'_> {
    /// The straight tracks of the pair's coupled runs of `polarity` long
    /// enough for the meander, the longest first: (item, position in the
    /// item, length).
    fn meander_tracks(
        &self,
        copper_out: &PairCopper,
        polarity: usize,
        shape: &MeanderShape,
    ) -> Vec<(usize, usize, f64)> {
        let mut tracks: Vec<(usize, usize, f64)> = Vec::new();
        for (_, items, runs, _) in self.kept_runs.iter().filter(|run| run.0 == self.index) {
            let item = &copper_out.items[items[polarity]];
            for position in runs[polarity].clone() {
                let Some(&(_, a, b)) = item.4.get(position) else {
                    continue;
                };
                let length = core::geometry::distance(a, b);
                if length >= shape.span + 2.0 * shape.margin {
                    tracks.push((items[polarity], position, length));
                }
            }
        }
        tracks.sort_by(|x, y| y.2.total_cmp(&x.2).then(x.0.cmp(&y.0)).then(x.1.cmp(&y.1)));
        tracks
    }

    /// The meander on the first of `tracks` where it keeps every clearance
    /// (`refused_groups`): per track the meander nearest its start, its
    /// middle, its end. The track splits around it (in its item and as
    /// kept copper of `restored`); it goes into `copper_out`'s patterns and
    /// the pair's `entry`.
    fn place_meander(
        &self,
        restored: &mut core::Board,
        copper_out: &mut PairCopper,
        entry: &mut KiCadDiffPair,
        polarity: usize,
        (shape, tracks): (&MeanderShape, Vec<(usize, usize, f64)>),
    ) {
        let pair = &self.pairs[self.index];
        let sign_of = |item: usize| {
            self.kept_runs
                .iter()
                .find(|run| run.1[polarity] == item)
                .map_or(1.0, |run| if polarity == 0 { run.3 } else { -run.3 })
        };
        let (count, height, spacing) = (shape.count, shape.height, shape.spacing);
        let (span, margin) = (shape.span, shape.margin);
        for (item, position, length) in tracks {
            let (layer, a, b) = copper_out.items[item].4[position];
            let outward = scale(left(unit(sub(b, a))), sign_of(item));
            for from in [margin, (length - span) / 2.0, length - margin - span] {
                let points = bumps((a, b), outward, from, count, height, spacing);
                let mut group = Group {
                    pair: self.index,
                    segments: Default::default(),
                    vias: Default::default(),
                    run: Default::default(),
                    moved: false,
                };
                group.segments[polarity] = points
                    .windows(2)
                    .map(|pair_of_points| (layer, pair_of_points[0], pair_of_points[1]))
                    .collect();
                let refused = refused_groups(restored, &self.result.routes, self.pairs, &[group]);
                if refused[0].is_some() {
                    continue;
                }
                // The track splits around the meander.
                let (first, last) = (points[0], points[points.len() - 1]);
                let item_copper = &mut copper_out.items[item].4;
                item_copper.splice(position..=position, [(layer, a, first), (layer, last, b)]);
                let net = [pair.positive, pair.negative][polarity];
                split_kept_track(
                    restored,
                    net,
                    (layer, a, b),
                    (first, last),
                    &points,
                    pair.width,
                );
                let added = polyline_length(&points) - core::geometry::distance(first, last);
                copper_out.patterns.push(Meander {
                    net: restored.nets[net as usize].name.clone(),
                    width: pair.width,
                    layer,
                    points,
                    gap: pair.gap,
                    amplitude: height,
                    spacing,
                    added,
                });
                entry.meander_mm += added;
                return;
            }
        }
    }
}

/// The kept track of `net` from `a` to `b` on `layer` replaced on
/// `restored` by its pieces around a meander (`points`, from `first` to
/// `last`), each kept copper of `width`.
fn split_kept_track(
    restored: &mut core::Board,
    net: core::NetId,
    (layer, a, b): (usize, Point, Point),
    (first, last): (Point, Point),
    points: &[Point],
    width: f64,
) {
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
        restored.obstacles.push(net_copper(
            core::Shape::Capsule {
                start,
                end,
                radius: width / 2.0,
            },
            1 << layer,
            core::ObstacleKind::Copper,
            net,
            "existing track".into(),
        ));
    }
}
