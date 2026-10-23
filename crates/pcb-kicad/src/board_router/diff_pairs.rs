// Copyright (C) 2026 Toit contributors.

//! Differential pairs, phase 1 (`diff_pairs`, docs/router.md
//! "Differential pairs").
//!
//! Nets KiCad pairs by name (`diff_pair_of`: `D+`/`D-`, `TX_P`/`TX_N`,
//! `USB_DP`/`USB_DN`) route as one coupled run before the other nets. A
//! pair's pads sit side by side on each part it joins (a *pair point*);
//! the pair points are linked by a minimum spanning tree, and each link is
//! one net of a router of its own (the *pairs router*) whose track is the
//! pair's envelope (two tracks and their gap: `2 w + g`) and whose via is
//! two vias side by side, from a launch point just clear of one pair
//! point's pads to one of the other's. The centreline found is offset by
//! `(w + g) / 2` to either side into the pair's two tracks and its vias
//! into a via of each net; what holds KiCad's clearances (`verify`) is
//! written into the board as tracks of the two nets, which the route then
//! keeps as it keeps a designer's (`kept`): the pads' stubs to the coupled
//! run's ends are ordinary routing (uncoupled; where the pads' order
//! crosses the run's, the stubs cross, with a via), and a link the pairs
//! router does not finish or whose copper is refused leaves its
//! connection to ordinary routing too (the pair *falls back*).

use super::*;
use pcb_router as core;

/// What the differential pairs came to (`board-router.json`'s
/// `diff_pairs`, the verdict's).
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct KiCadDiffPairs {
    /// Pairs found by name (both nets with pads on the board).
    pub found: usize,
    /// Pairs whose every link is coupled.
    pub coupled: usize,
    /// Pairs with some links coupled, not all.
    pub partly_coupled: usize,
    /// Pairs with no link coupled: routed as two ordinary nets.
    pub fallen_back: usize,
    /// Links between pair points, and those coupled.
    pub links: usize,
    pub links_coupled: usize,
    /// Coupled links whose pads' order crosses the run's at one end (the
    /// stubs cross there; no pad swapping in phase 1).
    pub crossed: usize,
    /// The coupled runs' length (millimetres, along the positive net).
    pub coupled_mm: f64,
    /// The coupled pairs' length beyond their coupled runs, after routing:
    /// per pair the longer net's track length less its coupled length
    /// (KiCad's uncoupled length), summed.
    pub uncoupled_mm: f64,
    /// The pairs router's work (seconds of an idle machine), charged to
    /// the ladder.
    pub work_seconds: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pairs: Vec<KiCadDiffPair>,
}

/// One differential pair (`KiCadDiffPairs::pairs`).
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct KiCadDiffPair {
    pub positive: String,
    pub negative: String,
    pub width_mm: f64,
    pub gap_mm: f64,
    /// Where width and gap came from (a custom rule, the net class).
    pub source: String,
    pub links: usize,
    pub links_coupled: usize,
    pub coupled_mm: f64,
    /// After routing: the longer net's length less the coupled length.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uncoupled_mm: Option<f64>,
    /// Why links fell back, when some did.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub why: Vec<String>,
}

/// The largest distance between the centres of a pair's two pads on one
/// part that makes them a pair point.
const PAIR_POINT_SPAN: f64 = 3.0;
/// The launch point's search: steps outward from between the pads.
const LAUNCH_STEP: f64 = 0.05;
const LAUNCH_REACH: f64 = 4.0;
/// Room beyond the clearance around a launch point, so that lattice nodes
/// next to it are legal.
const LAUNCH_MARGIN: f64 = 0.15;
/// The stubs tried: whole, then this share of their length from the run.
const STUB_FRACTIONS: [f64; 4] = [1.0, 0.6, 0.35, 0.15];
/// The nodes a coupled search starts from: within this of the launch.
const LAUNCH_CONTACT: f64 = 0.15;
/// The pairs router's share of the ladder's budget, at least
/// `MINIMUM_WORK` seconds.
const WORK_SHARE: f64 = 0.15;
const MINIMUM_WORK: f64 = 2.0;
const EPSILON: f64 = 1.0e-9;

type Point = [f64; 2];
/// A point of a route (nanometres) on a layer: `link_path`'s graph node.
type PathNode = ((i64, i64), usize);

fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1]]
}
fn add(a: Point, b: Point) -> Point {
    [a[0] + b[0], a[1] + b[1]]
}
fn scale(a: Point, factor: f64) -> Point {
    [a[0] * factor, a[1] * factor]
}
fn dot(a: Point, b: Point) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
fn cross(a: Point, b: Point) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
fn norm(a: Point) -> f64 {
    dot(a, a).sqrt()
}
fn unit(a: Point) -> Point {
    let length = norm(a);
    if length < EPSILON {
        [0.0, 0.0]
    } else {
        scale(a, 1.0 / length)
    }
}
/// The left normal of a direction.
fn left(a: Point) -> Point {
    [-a[1], a[0]]
}

/// A pair found on the board: its nets and its geometry.
struct Pair {
    positive: core::NetId,
    negative: core::NetId,
    /// The pair's track width and gap on the layers it may use.
    width: f64,
    gap: f64,
    /// Layers where the rules give that width and gap.
    layers: core::LayerMask,
    source: String,
    /// The clearance of the two nets to others (the larger class's).
    clearance: f64,
    /// The smallest distance KiCad lets the pair's two nets come
    /// (the gap, or the classes' clearance where larger: never less than
    /// the project lets them).
    via_diameter: f64,
    via_drill: f64,
    /// The board's hole clearance and hole to hole.
    hole_clearance: f64,
    hole_to_hole: f64,
    /// Pair points: (positive terminal, negative terminal).
    points: Vec<(usize, usize)>,
    /// Links: indices into `points`.
    links: Vec<(usize, usize)>,
}

impl Pair {
    /// Half the distance between the two tracks' centrelines.
    fn offset(&self) -> f64 {
        (self.width + self.gap) / 2.0
    }
    fn envelope(&self) -> f64 {
        2.0 * self.width + self.gap
    }
    /// Half the distance between the two vias' centres: the vias keep the
    /// clearance from each other and from the other net's track.
    fn via_offset(&self) -> f64 {
        let d = self.offset();
        let keep = self.clearance.max(self.gap);
        // Via to via, a via to the other net's track, a hole to the other
        // net's via and track, hole to hole.
        d.max((self.via_diameter + keep) / 2.0)
            .max(keep + (self.via_diameter + self.width) / 2.0 - d)
            .max((self.via_diameter / 2.0 + self.via_drill / 2.0 + self.hole_clearance) / 2.0)
            .max(self.hole_clearance + (self.via_drill + self.width) / 2.0 - d)
            .max((self.via_drill + self.hole_to_hole) / 2.0)
    }
}

/// One link's copper, offset from its centreline.
#[derive(Default)]
struct LinkCopper {
    /// Per polarity (0 positive, 1 negative): segments (layer, start, end)
    /// and vias.
    segments: [Vec<(usize, Point, Point)>; 2],
    vias: [Vec<Point>; 2],
    /// Straight stubs from the pads to the run's ends, per polarity
    /// (layer, pad, end): at both ends where the pads' order is the run's,
    /// at the start only where it crosses (`crossed`).
    stubs: [Vec<(usize, Point, Point)>; 2],
    coupled: f64,
    crossed: bool,
}

/// The pairs of a lowered board, by name, with their rules.
fn find_pairs(board: &core::Board, config: &KiCadBoardRouterConfig) -> Vec<Pair> {
    let ids: BTreeMap<&str, core::NetId> = board
        .nets
        .iter()
        .enumerate()
        .map(|(index, net)| (net.name.as_str(), index as core::NetId))
        .collect();
    let mut pairs = Vec::new();
    for (index, net) in board.nets.iter().enumerate() {
        let Some((partner, _, true)) = crate::project_rules::diff_pair_of(&net.name) else {
            continue;
        };
        let Some(&negative) = ids.get(partner.as_str()) else {
            continue;
        };
        let positive = index as core::NetId;
        let (p, n) = (&board.nets[index], &board.nets[negative as usize]);
        if p.terminals.is_empty() || n.terminals.is_empty() {
            continue;
        }
        let (class_p, class_n) = (board.classes[p.class], board.classes[n.class]);
        let clearance = class_p.clearance.max(class_n.clearance);
        let (mut width, mut gap, mut layers, mut source) = (
            class_p.trace_width,
            clearance,
            board.all_layers(),
            "net class width and clearance".to_string(),
        );
        if let Some(rule) = config.diff_pair_rules.get(&net.name) {
            // The first layer the rules name a pair on sets the values; the
            // layers with the same ones are the pair's.
            let first = (0..rule.layers.len())
                .find(|layer| !rule.disallowed.get(*layer).copied().unwrap_or(false));
            if let Some(&(w, g)) = first.and_then(|layer| rule.layers.get(layer)) {
                width = w;
                gap = g;
                source = rule.sources[first.unwrap_or(0)].clone();
                layers = rule
                    .layers
                    .iter()
                    .enumerate()
                    .filter(|(layer, (w2, g2))| {
                        (w2 - w).abs() < 1.0e-6
                            && (g2 - g).abs() < 1.0e-6
                            && !rule.disallowed.get(*layer).copied().unwrap_or(false)
                    })
                    .fold(0, |mask, (layer, _)| mask | (1 << layer))
                    & board.all_layers();
            }
        }
        // Pair points: a positive and a negative pad of one part, nearest
        // first.
        let reference = |label: &str| {
            label
                .rsplit_once('.')
                .map_or(label, |(part, _)| part)
                .to_string()
        };
        let mut candidates: Vec<(f64, usize, usize)> = Vec::new();
        for (a, ta) in p.terminals.iter().enumerate() {
            for (b, tb) in n.terminals.iter().enumerate() {
                let span = core::geometry::distance(ta.anchor, tb.anchor);
                if span <= PAIR_POINT_SPAN
                    && span > EPSILON
                    && reference(&ta.label) == reference(&tb.label)
                    && ta.layers & tb.layers != 0
                {
                    candidates.push((span, a, b));
                }
            }
        }
        candidates.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)).then(x.2.cmp(&y.2)));
        let (mut used_p, mut used_n) = (BTreeSet::new(), BTreeSet::new());
        let mut points = Vec::new();
        for (_, a, b) in candidates {
            if !used_p.contains(&a) && !used_n.contains(&b) {
                used_p.insert(a);
                used_n.insert(b);
                points.push((a, b));
            }
        }
        points.sort();
        // A minimum spanning tree over the pair points (Prim).
        let middle = |point: &(usize, usize)| {
            scale(
                add(p.terminals[point.0].anchor, n.terminals[point.1].anchor),
                0.5,
            )
        };
        let mut links = Vec::new();
        if points.len() >= 2 {
            let mut inside = vec![false; points.len()];
            inside[0] = true;
            for _ in 1..points.len() {
                let mut best: Option<(f64, usize, usize)> = None;
                for (i, a) in points.iter().enumerate().filter(|(i, _)| inside[*i]) {
                    for (j, b) in points.iter().enumerate().filter(|(j, _)| !inside[*j]) {
                        let length = core::geometry::distance(middle(a), middle(b));
                        if best.is_none_or(|(known, _, _)| length < known) {
                            best = Some((length, i, j));
                        }
                    }
                }
                let (_, i, j) = best.expect("a point outside");
                inside[j] = true;
                links.push((i, j));
            }
        }
        pairs.push(Pair {
            positive,
            negative,
            width,
            gap,
            layers,
            source,
            clearance,
            via_diameter: class_p.via_diameter.max(class_n.via_diameter),
            via_drill: class_p.via_drill.max(class_n.via_drill),
            hole_clearance: board.hole_clearance,
            hole_to_hole: board.hole_to_hole,
            points,
            links,
        });
    }
    pairs
}

/// The layers where a disc of `radius` (plus each obstacle's clearance)
/// at `at` is clear of every obstacle and of the board edge.
fn clear_layers(
    board: &core::Board,
    at: Point,
    radius: f64,
    clearance: f64,
    mask: core::LayerMask,
) -> core::LayerMask {
    if !core::geometry::point_in_polygon(at, &board.outline)
        || core::geometry::polygon_edges(&board.outline)
            .map(|(a, b)| core::geometry::point_segment_distance(at, a, b))
            .fold(f64::INFINITY, f64::min)
            < radius + board.edge_clearance
    {
        return 0;
    }
    let reach = radius + clearance.max(board.hole_clearance) + 2.0;
    let mut free = mask;
    for obstacle in &board.obstacles {
        if obstacle.layers & free == 0 || !obstacle.blocks_tracks {
            continue;
        }
        let bounds = obstacle.shape.aabb();
        if at[0] < bounds.minimum[0] - reach
            || at[0] > bounds.maximum[0] + reach
            || at[1] < bounds.minimum[1] - reach
            || at[1] > bounds.maximum[1] + reach
        {
            continue;
        }
        let keep = match obstacle.kind {
            core::ObstacleKind::Copper => board.copper_clearance(
                &core::RuleClass {
                    trace_width: 0.0,
                    clearance,
                    via_diameter: 0.0,
                    via_drill: 0.0,
                },
                obstacle,
            ),
            core::ObstacleKind::Keepout => obstacle.clearance,
            core::ObstacleKind::Hole => board.hole_clearance.max(obstacle.clearance),
        };
        if obstacle.shape.distance_to_point(at) < radius + keep {
            free &= !obstacle.layers;
        }
    }
    free
}

/// A launch point for a link at pair point `point` toward `toward`: the
/// first point outward from between the two pads where the pair's
/// envelope is clear, and the layers it is clear on.
fn launch(
    board: &core::Board,
    pads: &core::Board,
    pair: &Pair,
    class: &core::RuleClass,
    point: (usize, usize),
    toward: Point,
) -> Option<(Point, core::LayerMask, Point)> {
    let p = &pads.nets[pair.positive as usize].terminals[point.0];
    let n = &pads.nets[pair.negative as usize].terminals[point.1];
    let middle = scale(add(p.anchor, n.anchor), 0.5);
    let mut outward = left(unit(sub(n.anchor, p.anchor)));
    if dot(outward, sub(toward, middle)) < 0.0 {
        outward = scale(outward, -1.0);
    }
    let layers = p.layers & n.layers & pair.layers;
    if layers == 0 {
        return None;
    }
    let steps = (LAUNCH_REACH / LAUNCH_STEP) as usize;
    (0..=steps).find_map(|step| {
        let at = add(middle, scale(outward, step as f64 * LAUNCH_STEP));
        let free = clear_layers(
            board,
            at,
            (class.trace_width + class.clearance) / 2.0 + LAUNCH_MARGIN,
            class.clearance,
            layers,
        );
        (free != 0).then_some((at, free, outward))
    })
}

/// A link's path from the pairs router's route: (point, layer) from the
/// launch at `start` to the one at `end`, through its segments and vias.
fn link_path(route: &core::NetRoute, start: Point, end: Point) -> Option<Vec<(Point, usize)>> {
    let key = |point: Point| {
        (
            (point[0] * 1.0e6).round() as i64,
            (point[1] * 1.0e6).round() as i64,
        )
    };
    let mut points: BTreeMap<(i64, i64), Point> = BTreeMap::new();
    let mut edges: BTreeMap<PathNode, Vec<PathNode>> = BTreeMap::new();
    let mut layers_at: BTreeMap<(i64, i64), BTreeSet<usize>> = BTreeMap::new();
    for segment in &route.segments {
        let (a, b) = (key(segment.start), key(segment.end));
        points.insert(a, segment.start);
        points.insert(b, segment.end);
        edges
            .entry((a, segment.layer))
            .or_default()
            .push((b, segment.layer));
        edges
            .entry((b, segment.layer))
            .or_default()
            .push((a, segment.layer));
        layers_at.entry(a).or_default().insert(segment.layer);
        layers_at.entry(b).or_default().insert(segment.layer);
    }
    for via in &route.vias {
        let at = key(via.at);
        let Some(layers) = layers_at.get(&at).cloned() else {
            continue;
        };
        for &from in &layers {
            for &to in &layers {
                if from != to {
                    edges.entry((at, from)).or_default().push((at, to));
                }
            }
        }
    }
    let nearest = |target: Point| {
        edges
            .keys()
            .min_by(|a, b| {
                core::geometry::distance(points[&a.0], target)
                    .total_cmp(&core::geometry::distance(points[&b.0], target))
            })
            .copied()
    };
    let (from, to) = (nearest(start)?, nearest(end)?);
    if core::geometry::distance(points[&from.0], start) > 0.5
        || core::geometry::distance(points[&to.0], end) > 0.5
    {
        return None;
    }
    // Breadth first over the tree.
    let mut previous: BTreeMap<PathNode, PathNode> = BTreeMap::new();
    let mut queue = std::collections::VecDeque::from([from]);
    previous.insert(from, from);
    while let Some(node) = queue.pop_front() {
        if node == to {
            break;
        }
        for next in edges.get(&node).into_iter().flatten() {
            if !previous.contains_key(next) {
                previous.insert(*next, node);
                queue.push_back(*next);
            }
        }
    }
    previous.get(&to)?;
    let mut path = vec![to];
    while *path.last()? != from {
        path.push(previous[path.last()?]);
    }
    path.reverse();
    Some(
        path.into_iter()
            .map(|(point, layer)| (points[&point], layer))
            .collect(),
    )
}

/// A polyline's points without repeats and without points in a straight
/// line between their neighbours.
fn simplified(points: &[Point]) -> Vec<Point> {
    let mut out: Vec<Point> = Vec::new();
    for &point in points {
        if out
            .last()
            .is_some_and(|last| core::geometry::distance(*last, point) < 1.0e-6)
        {
            continue;
        }
        if out.len() >= 2 {
            let (a, b) = (out[out.len() - 2], out[out.len() - 1]);
            if cross(sub(b, a), sub(point, b)).abs() < 1.0e-9 && dot(sub(b, a), sub(point, b)) > 0.0
            {
                out.pop();
            }
        }
        out.push(point);
    }
    out
}

/// One side of a centreline polyline, `offset` to its left (`side` 1) or
/// right (-1): mitred on the inside of a bend where both segments are long
/// enough, bevelled on the outside (it stays inside the envelope).
fn offset_line(points: &[Point], side: f64, offset: f64) -> Vec<Point> {
    let mut out = Vec::new();
    let count = points.len();
    if count < 2 {
        return out;
    }
    let normal = |i: usize| scale(left(unit(sub(points[i + 1], points[i]))), side);
    out.push(add(points[0], scale(normal(0), offset)));
    for i in 1..count - 1 {
        let (n1, n2) = (normal(i - 1), normal(i));
        let (u1, u2) = (
            unit(sub(points[i], points[i - 1])),
            unit(sub(points[i + 1], points[i])),
        );
        let inner = cross(u1, u2) * side > 0.0;
        let denominator = 1.0 + dot(n1, n2);
        let miter = scale(add(n1, n2), offset / denominator.max(EPSILON));
        // How far along each segment the miter point lies from the corner.
        let along = norm(sub(miter, scale(n1, offset)));
        if inner
            && denominator > 0.3
            && along <= core::geometry::distance(points[i], points[i - 1])
            && along <= core::geometry::distance(points[i + 1], points[i])
        {
            out.push(add(points[i], miter));
        } else {
            out.push(add(points[i], scale(n1, offset)));
            out.push(add(points[i], scale(n2, offset)));
        }
    }
    out.push(add(points[count - 1], scale(normal(count - 2), offset)));
    out
}

/// The copper of a link from its path: the two offset tracks per layer
/// run and a via of each net side by side at each layer change. The
/// positive net takes the side its pad is on at the start.
fn link_copper(
    path: &[(Point, usize)],
    pair: &Pair,
    start_pads: [Point; 2],
    end_pads: [Point; 2],
) -> Option<LinkCopper> {
    let (start_pad, end_pad) = (start_pads[0], end_pads[0]);
    // Runs on one layer, split at the layer changes (the vias).
    let mut runs: Vec<(usize, Vec<Point>)> = Vec::new();
    for &(point, layer) in path {
        match runs.last_mut() {
            Some((known, points)) if *known == layer => points.push(point),
            _ => runs.push((layer, vec![point])),
        }
    }
    let runs: Vec<(usize, Vec<Point>)> = runs
        .into_iter()
        .map(|(layer, points)| (layer, simplified(&points)))
        .collect();
    let directions: Vec<Option<(Point, Point)>> = runs
        .iter()
        .map(|(_, points)| {
            (points.len() >= 2).then(|| {
                (
                    unit(sub(points[1], points[0])),
                    unit(sub(points[points.len() - 1], points[points.len() - 2])),
                )
            })
        })
        .collect();
    let first = directions.iter().flatten().next()?.0;
    let last = directions.iter().rev().flatten().next()?.1;
    let start = path.first()?.0;
    let end = path.last()?.0;
    // P's side of the run: where its pad is beside N's.
    let side = if dot(left(first), sub(start_pad, start_pads[1])) >= 0.0 {
        1.0
    } else {
        -1.0
    };
    let end_side = if dot(left(last), sub(end_pad, end_pads[1])) >= 0.0 {
        1.0
    } else {
        -1.0
    };
    if std::env::var_os("PCB_ROUTER_DIFF_PAIRS_DEBUG").is_some() {
        eprintln!(
            "sides: start {start:?} first {first:?} pads {start_pads:?} side {side}; end {end:?} last {last:?} pads {end_pads:?} side {end_side}"
        );
    }
    let mut copper = LinkCopper {
        crossed: side != end_side,
        ..LinkCopper::default()
    };
    let d = pair.offset();
    let dv = pair.via_offset();
    for (polarity, sign) in [(0, side), (1, -side)] {
        let mut previous_end: Option<(Point, usize)> = None;
        let mut incoming: Option<Point> = None;
        for (index, (layer, points)) in runs.iter().enumerate() {
            let line = offset_line(points, sign, d);
            if index > 0 {
                // The via between the previous run and this one.
                let at = points[0];
                let outgoing = directions[index..]
                    .iter()
                    .flatten()
                    .next()
                    .map(|(first, _)| *first)
                    .or(incoming)?;
                let into = incoming.unwrap_or(outgoing);
                let mut bisector = add(left(into), left(outgoing));
                if norm(bisector) < 0.2 {
                    bisector = left(into);
                }
                // Turned at the via, the vias move out along the bisector
                // so that across either run they are as far apart as
                // straight (up to a right angle).
                let half_turn_cosine = dot(unit(bisector), left(into)).abs().max(0.7);
                let via = add(at, scale(unit(bisector), sign * dv / half_turn_cosine));
                copper.vias[polarity].push(via);
                if let Some((from, from_layer)) = previous_end
                    && core::geometry::distance(from, via) > 1.0e-6
                {
                    copper.segments[polarity].push((from_layer, from, via));
                }
                if let Some(&to) = line.first()
                    && core::geometry::distance(to, via) > 1.0e-6
                {
                    copper.segments[polarity].push((*layer, via, to));
                }
                if line.is_empty() {
                    previous_end = Some((via, *layer));
                }
            }
            for pair_of_points in line.windows(2) {
                let length = core::geometry::distance(pair_of_points[0], pair_of_points[1]);
                if length > 1.0e-6 {
                    copper.segments[polarity].push((*layer, pair_of_points[0], pair_of_points[1]));
                    if polarity == 0 {
                        copper.coupled += length;
                    }
                }
            }
            if let Some(&last_point) = line.last() {
                previous_end = Some((last_point, *layer));
            }
            if let Some((_, last_direction)) = directions[index] {
                incoming = Some(last_direction);
            }
        }
        // The stubs: from the first and last points of the offset runs.
        let first_run = runs.iter().position(|(_, points)| points.len() >= 2)?;
        let last_run = runs.iter().rposition(|(_, points)| points.len() >= 2)?;
        let begin = *offset_line(&runs[first_run].1, sign, d).first()?;
        let finish = *offset_line(&runs[last_run].1, sign, d).last()?;
        copper.stubs[polarity].push((runs[first_run].0, start_pads[polarity], begin));
        if !copper.crossed {
            copper.stubs[polarity].push((runs[last_run].0, end_pads[polarity], finish));
        }
    }
    (!copper.segments[0].is_empty() && !copper.segments[1].is_empty()).then_some(copper)
}

/// Routes the board's differential pairs as coupled runs and writes what
/// holds into `pcb` as tracks and vias of the pairs' nets (the route keeps
/// them); `work_budget` is the pairs router's work deadline. Returns the
/// report (`uncoupled_mm` is measured after the route: `measure_uncoupled`).
pub(crate) fn route_diff_pairs(
    pcb: &mut Expr,
    config: &KiCadBoardRouterConfig,
    work_budget: f64,
) -> Result<KiCadDiffPairs, String> {
    let board = lower(pcb, config, true)?.board;
    let layer_names = LayerTable::from_pcb(pcb)?.names;
    let pairs = find_pairs(&board, config);
    let mut report = KiCadDiffPairs {
        found: pairs.len(),
        ..KiCadDiffPairs::default()
    };
    let mut entries: Vec<KiCadDiffPair> = pairs
        .iter()
        .map(|pair| KiCadDiffPair {
            positive: board.nets[pair.positive as usize].name.clone(),
            negative: board.nets[pair.negative as usize].name.clone(),
            width_mm: pair.width,
            gap_mm: pair.gap,
            source: pair.source.clone(),
            links: pair.links.len(),
            why: if pair.points.len() < 2 {
                vec!["fewer than two parts with the pair's pads side by side".into()]
            } else {
                Vec::new()
            },
            ..KiCadDiffPair::default()
        })
        .collect();
    // The kept copper so far, per pair and link.
    let mut kept: Vec<(usize, LinkCopper)> = Vec::new();
    // Pairs grouped by their layers: one pairs router each, each seeing
    // the copper the groups before kept.
    let mut groups: BTreeMap<core::LayerMask, Vec<usize>> = BTreeMap::new();
    for (index, pair) in pairs.iter().enumerate() {
        if !pair.links.is_empty() && pair.layers != 0 {
            groups.entry(pair.layers).or_default().push(index);
        } else if pair.layers == 0 {
            entries[index]
                .why
                .push("no layer holds the pair's width and gap".into());
        }
    }
    // One run where the pairs share layers (each run builds the board's
    // static maps again): on the layers all of them may use.
    let common = groups
        .keys()
        .fold(core::LayerMask::MAX, |mask, known| mask & known);
    if groups.len() > 1 && common != 0 {
        let members: Vec<usize> = groups.into_values().flatten().collect();
        groups = BTreeMap::from([(common, members)]);
    }
    let group_count = groups.len().max(1);
    let mut base = board.clone();
    for (mask, members) in groups {
        // The board's nets route nothing here: one class, the largest
        // clearance of all (each class builds static maps of the whole
        // board; ducktop2-center's took 40 s of work), its pads at least
        // as far from the pairs as any class asks.
        let mut pairs_board = base.clone();
        pairs_board.classes = vec![core::RuleClass {
            trace_width: board
                .classes
                .iter()
                .map(|class| class.trace_width)
                .fold(f64::INFINITY, f64::min),
            clearance: board
                .classes
                .iter()
                .map(|class| class.clearance)
                .fold(0.0, f64::max),
            via_diameter: board
                .classes
                .iter()
                .map(|class| class.via_diameter)
                .fold(0.0, f64::max),
            via_drill: board
                .classes
                .iter()
                .map(|class| class.via_drill)
                .fold(0.0, f64::max),
        }];
        for net in &mut pairs_board.nets {
            net.terminals.clear();
            net.class = 0;
        }
        pairs_board.planes.clear();
        pairs_board.merged_pads.clear();
        pairs_board.terminal_copper.clear();
        // Layers the pairs may not use.
        let others = pairs_board.all_layers() & !mask;
        if others != 0 {
            let bounds = pairs_board
                .outline
                .iter()
                .fold(
                    core::geometry::Aabb {
                        minimum: [f64::INFINITY; 2],
                        maximum: [f64::NEG_INFINITY; 2],
                    },
                    |b, p| core::geometry::Aabb {
                        minimum: [b.minimum[0].min(p[0]), b.minimum[1].min(p[1])],
                        maximum: [b.maximum[0].max(p[0]), b.maximum[1].max(p[1])],
                    },
                )
                .inflated(1.0);
            pairs_board.obstacles.push(core::Obstacle {
                shape: core::Shape::Polygon {
                    points: vec![
                        bounds.minimum,
                        [bounds.maximum[0], bounds.minimum[1]],
                        bounds.maximum,
                        [bounds.minimum[0], bounds.maximum[1]],
                    ],
                },
                layers: others,
                kind: core::ObstacleKind::Keepout,
                net: None,
                clearance: 0.0,
                clearance_override: None,
                blocks_tracks: true,
                blocks_vias: false,
                label: "diff pair layer".into(),
            });
        }
        // One net per link, from launch to launch.
        // (link net, pair, link, launch at start, launch at end)
        #[allow(clippy::type_complexity)]
        let mut link_nets: Vec<(
            core::NetId,
            usize,
            usize,
            (Point, Point),
            (Point, Point),
        )> = Vec::new();
        // The group's pairs share one class, the widest envelope and via
        // of them (each class builds static maps of the whole board:
        // ducktop2-center's four pair classes took 17 s of work).
        let widest = |measure: &dyn Fn(&Pair) -> f64| {
            members
                .iter()
                .map(|&index| measure(&pairs[index]))
                .fold(0.0, f64::max)
        };
        let group_class = core::RuleClass {
            trace_width: widest(&|pair| pair.envelope()),
            clearance: widest(&|pair| pair.clearance),
            via_diameter: widest(&|pair| 2.0 * pair.via_offset() + pair.via_diameter),
            via_drill: widest(&|pair| 2.0 * pair.via_offset() + pair.via_drill),
        };
        pairs_board.classes.push(group_class);
        let class = 1;
        for &index in &members {
            let pair = &pairs[index];
            let p = &board.nets[pair.positive as usize];
            let n = &board.nets[pair.negative as usize];
            let middle = |point: (usize, usize)| {
                scale(
                    add(p.terminals[point.0].anchor, n.terminals[point.1].anchor),
                    0.5,
                )
            };
            for (link, &(a, b)) in pair.links.iter().enumerate() {
                let (point_a, point_b) = (pair.points[a], pair.points[b]);
                let (Some((at_a, layers_a, out_a)), Some((at_b, layers_b, out_b))) = (
                    launch(
                        &pairs_board,
                        &board,
                        pair,
                        &group_class,
                        point_a,
                        middle(point_b),
                    ),
                    launch(
                        &pairs_board,
                        &board,
                        pair,
                        &group_class,
                        point_b,
                        middle(point_a),
                    ),
                ) else {
                    entries[index]
                        .why
                        .push(format!("link {}: no launch clear of the pads", link + 1));
                    continue;
                };
                let id = pairs_board.nets.len() as core::NetId;
                let mut terminals = Vec::new();
                for (at, layers, end, from) in [
                    (at_a, layers_a, "a", middle(point_a)),
                    (at_b, layers_b, "b", middle(point_b)),
                ] {
                    terminals.push(core::Terminal {
                        anchor: at,
                        layers,
                        pad: pairs_board.obstacles.len(),
                        // The run starts at a node close to the launch (the
                        // obstacle is wider, so that no neck applies).
                        contact: Some(core::Shape::Circle {
                            center: at,
                            radius: LAUNCH_CONTACT,
                        }),
                        label: format!("pair launch {}{end}", link + 1),
                    });
                    // The launch and the stubs' corridor from the pads:
                    // other pairs keep out of it.
                    pairs_board.obstacles.push(core::Obstacle {
                        shape: core::Shape::Capsule {
                            start: from,
                            end: at,
                            radius: (group_class.trace_width + group_class.clearance) / 2.0,
                        },
                        layers,
                        kind: core::ObstacleKind::Copper,
                        net: Some(id),
                        clearance: 0.0,
                        clearance_override: None,
                        blocks_tracks: true,
                        blocks_vias: true,
                        label: format!("pair launch {}{end}", link + 1),
                    });
                }
                pairs_board.nets.push(core::Net {
                    name: format!("{} / {} link {}", p.name, n.name, link + 1),
                    class,
                    terminals,
                });
                link_nets.push((id, index, link, (at_a, out_a), (at_b, out_b)));
            }
        }
        if link_nets.is_empty() {
            continue;
        }
        let mut router_config = core_config(config);
        router_config.verbose = false;
        router_config.deadline_work = Some(work_budget / group_count as f64);
        router_config.run_work = None;
        // The rules beyond the classes name the board's nets, which route
        // nothing here (the check of the copper holds the classes).
        router_config.net_rules = None;
        let result = core::route(&pairs_board, &router_config);
        report.work_seconds += result.expansions as f64 / core::router::expansions_per_second()
            + result.build_work
            + result.pour_work;
        if std::env::var_os("PCB_ROUTER_DIFF_PAIRS_DEBUG").is_some() {
            eprintln!(
                "diff pairs router: {} links, search {:.1} s, build {:.1} s, pour {:.1} s of work",
                link_nets.len(),
                result.expansions as f64 / core::router::expansions_per_second(),
                result.build_work,
                result.pour_work
            );
        }
        // The links' copper, checked against the board and each other.
        let mut candidates: Vec<(usize, usize, LinkCopper)> = Vec::new();
        for &(id, index, link, (at_a, out_a), (at_b, out_b)) in &link_nets {
            let pair = &pairs[index];
            if result.status[id as usize] != core::NetStatus::Routed {
                entries[index].why.push(format!(
                    "link {}: the coupled search found no path ({})",
                    link + 1,
                    match result.status[id as usize] {
                        core::NetStatus::Unreachable => "a launch has no legal node",
                        _ => "no path between the launches",
                    }
                ));
                continue;
            }
            let (a, b) = pair.links[link];
            let pads = |point: (usize, usize)| {
                [
                    board.nets[pair.positive as usize].terminals[point.0].anchor,
                    board.nets[pair.negative as usize].terminals[point.1].anchor,
                ]
            };
            let (pad_a, pad_b) = (pads(pair.points[a]), pads(pair.points[b]));
            // Behind each end a point toward the pads, so that the run
            // leaves the pads straight out (the stubs then meet it square,
            // whatever way the search turned at the launch).
            // The run starts at the launch itself (centred on the pads),
            // whatever lattice node the search left from.
            let path = link_path(&result.routes[id as usize], at_a, at_b).map(|mut path| {
                let back = pair.offset();
                // The router's stub from the node to the launch (at any
                // angle) goes; the back point, on the launch's axis, joins
                // the node directly.
                let off_lattice = |a: Point, b: Point| {
                    let angle = (b[1] - a[1]).atan2(b[0] - a[0]);
                    let step = std::f64::consts::FRAC_PI_4;
                    ((angle / step).round() * step - angle).abs() > 1.0e-3
                };
                if path.len() >= 3 && path[0].1 == path[1].1 && off_lattice(path[0].0, path[1].0) {
                    path.remove(0);
                }
                let count = path.len();
                if count >= 3
                    && path[count - 1].1 == path[count - 2].1
                    && off_lattice(path[count - 2].0, path[count - 1].0)
                {
                    path.pop();
                }
                let (first, last) = (path[0], path[path.len() - 1]);
                path.insert(0, (sub(first.0, scale(out_a, back)), first.1));
                path.push((sub(last.0, scale(out_b, back)), last.1));
                path
            });
            if std::env::var_os("PCB_ROUTER_DIFF_PAIRS_DEBUG").is_some() {
                eprintln!(
                    "diff pair link {}: route {:?}\npath {:?}",
                    pairs_board.nets[id as usize].name, result.routes[id as usize], path
                );
            }
            let copper = path.and_then(|path| link_copper(&path, pair, pad_a, pad_b));
            if std::env::var_os("PCB_ROUTER_DIFF_PAIRS_DEBUG").is_some()
                && let Some(copper) = &copper
            {
                eprintln!(
                    "copper P {:?} {:?}\ncopper N {:?} {:?}",
                    copper.segments[0], copper.vias[0], copper.segments[1], copper.vias[1]
                );
            }
            match copper {
                Some(copper) => candidates.push((index, link, copper)),
                None => entries[index].why.push(format!(
                    "link {}: the coupled route has no clean path",
                    link + 1
                )),
            }
        }
        // First the coupled runs and their vias, then the stubs of the runs
        // kept (a stub refused leaves that end to the ordinary routing).
        let groups: Vec<Group> = candidates
            .iter()
            .map(|(index, _, copper)| Group {
                pair: *index,
                segments: copper.segments.clone(),
                vias: copper.vias.clone(),
                fixed: false,
            })
            .collect();
        let refused = refused_groups(&base, &pairs, &groups);
        // The stubs, in rounds: whole, then shorter from the run's end
        // toward the pad (a stub cut short leaves the ordinary routing a
        // start clear of the other net, where the run's own end may have
        // no legal lattice node).
        let mut kept_stubs: Vec<[Vec<(usize, Point, Point)>; 2]> =
            vec![Default::default(); candidates.len()];
        // (candidate, polarity, stub) still to place.
        let mut open: Vec<(usize, usize, (usize, Point, Point))> = Vec::new();
        for (position, (_, _, copper)) in candidates.iter().enumerate() {
            if refused[position].is_none() {
                for polarity in 0..2 {
                    for &stub in &copper.stubs[polarity] {
                        open.push((position, polarity, stub));
                    }
                }
            }
        }
        for fraction in STUB_FRACTIONS {
            if open.is_empty() {
                break;
            }
            let mut stub_groups: Vec<Group> = Vec::new();
            for (position, (index, _, copper)) in candidates.iter().enumerate() {
                if refused[position].is_none() {
                    let mut segments = copper.segments.clone();
                    for polarity in 0..2 {
                        segments[polarity].extend(kept_stubs[position][polarity].iter().copied());
                    }
                    stub_groups.push(Group {
                        pair: *index,
                        segments,
                        vias: copper.vias.clone(),
                        fixed: true,
                    });
                }
            }
            let first_open = stub_groups.len();
            for &(position, polarity, (layer, pad, end)) in &open {
                let mut segments: [Vec<(usize, Point, Point)>; 2] = Default::default();
                let from = add(end, scale(sub(pad, end), fraction));
                segments[polarity].push((layer, from, end));
                stub_groups.push(Group {
                    pair: candidates[position].0,
                    segments,
                    vias: Default::default(),
                    fixed: false,
                });
            }
            let stub_refused = refused_groups(&base, &pairs, &stub_groups);
            let mut still = Vec::new();
            for (offset, &(position, polarity, stub)) in open.iter().enumerate() {
                let group = &stub_groups[first_open + offset];
                match &stub_refused[first_open + offset] {
                    None => kept_stubs[position][polarity]
                        .extend(group.segments[polarity].iter().copied()),
                    Some(why) => {
                        if std::env::var_os("PCB_ROUTER_DIFF_PAIRS_DEBUG").is_some() {
                            eprintln!(
                                "diff pair stub {:?} ({fraction}) refused: {why}",
                                group.segments
                            );
                        }
                        still.push((position, polarity, stub));
                    }
                }
            }
            open = still;
        }
        for (position, (index, link, mut copper)) in candidates.into_iter().enumerate() {
            if let Some(why) = &refused[position] {
                entries[index].why.push(format!("link {}: {why}", link + 1));
                continue;
            }
            for (polarity, stubs) in kept_stubs[position].iter_mut().enumerate() {
                copper.segments[polarity].extend(std::mem::take(stubs));
            }
            // The kept copper is fixed copper of the next group's board.
            let pair = &pairs[index];
            for (polarity, net) in [(0, pair.positive), (1, pair.negative)] {
                for &(layer, start, end) in &copper.segments[polarity] {
                    base.obstacles.push(core::Obstacle {
                        shape: core::Shape::Capsule {
                            start,
                            end,
                            radius: pair.width / 2.0,
                        },
                        layers: 1 << layer,
                        kind: core::ObstacleKind::Copper,
                        net: Some(net),
                        clearance: 0.0,
                        clearance_override: None,
                        blocks_tracks: true,
                        blocks_vias: true,
                        label: "existing track".into(),
                    });
                }
                for &at in &copper.vias[polarity] {
                    base.obstacles.push(core::Obstacle {
                        shape: core::Shape::Circle {
                            center: at,
                            radius: pair.via_diameter / 2.0,
                        },
                        layers: base.all_layers(),
                        kind: core::ObstacleKind::Copper,
                        net: Some(net),
                        clearance: 0.0,
                        clearance_override: None,
                        blocks_tracks: true,
                        blocks_vias: true,
                        label: "existing via".into(),
                    });
                }
            }
            entries[index].links_coupled += 1;
            entries[index].coupled_mm += copper.coupled;
            report.crossed += copper.crossed as usize;
            kept.push((index, copper));
        }
    }
    // Into the board.
    let Expr::List(items) = pcb else {
        return Err("PCB root is not a list".into());
    };
    for (index, copper) in &kept {
        let pair = &pairs[*index];
        for (polarity, net) in [(0, pair.positive), (1, pair.negative)] {
            let name = &board.nets[net as usize].name;
            for &(layer, start, end) in &copper.segments[polarity] {
                items.push(supplemental_segment_expr(&SupplementalSegment {
                    connection: name.clone(),
                    start: nanometres(start),
                    end: nanometres(end),
                    width: pair.width,
                    layer: layer_names[layer].clone(),
                }));
            }
            for &at in &copper.vias[polarity] {
                items.push(supplemental_via_expr(&SupplementalVia {
                    connection: name.clone(),
                    at: nanometres(at),
                    size: pair.via_diameter,
                    drill: pair.via_drill,
                    layers: ["F.Cu".into(), "B.Cu".into()],
                }));
            }
        }
    }
    for entry in &entries {
        report.links += entry.links;
        report.links_coupled += entry.links_coupled;
        report.coupled_mm += entry.coupled_mm;
        if entry.links > 0 && entry.links_coupled == entry.links {
            report.coupled += 1;
        } else if entry.links_coupled > 0 {
            report.partly_coupled += 1;
        } else {
            report.fallen_back += 1;
        }
    }
    report.pairs = entries;
    Ok(report)
}

/// Copper checked together (`refused_groups`): a link's run and vias, or
/// one stub; `fixed` copper is checked against, never refused.
#[derive(Clone)]
struct Group {
    pair: usize,
    segments: [Vec<(usize, Point, Point)>; 2],
    vias: [Vec<Point>; 2],
    fixed: bool,
}

/// Per group: why its copper is refused (`None`: kept). The copper of
/// every group is checked at once against the board (`verify`: its
/// obstacles, the edge, each other); a finding between the two nets of one
/// pair counts only below their gap (KiCad lets them come that close: the
/// class's diff pair rule, or the gap is no less than the clearance).
fn refused_groups(board: &core::Board, pairs: &[Pair], groups: &[Group]) -> Vec<Option<String>> {
    let mut routes = vec![core::NetRoute::default(); board.nets.len()];
    let mut owner_of_segment: BTreeMap<(core::NetId, usize), usize> = BTreeMap::new();
    let mut vias_of: BTreeMap<core::NetId, Vec<(Point, usize)>> = BTreeMap::new();
    for (position, group) in groups.iter().enumerate() {
        let pair = &pairs[group.pair];
        for (polarity, net) in [(0, pair.positive), (1, pair.negative)] {
            let route = &mut routes[net as usize];
            for &(layer, start, end) in &group.segments[polarity] {
                owner_of_segment.insert((net, route.segments.len()), position);
                route.segments.push(core::Segment {
                    layer,
                    start,
                    end,
                    width: pair.width,
                });
            }
            for &at in &group.vias[polarity] {
                vias_of.entry(net).or_default().push((at, position));
                route.vias.push(core::Via {
                    at,
                    diameter: pair.via_diameter,
                    drill: pair.via_drill,
                });
            }
        }
    }
    let partner: BTreeMap<core::NetId, (core::NetId, f64)> = pairs
        .iter()
        .flat_map(|pair| {
            [
                (pair.positive, (pair.negative, pair.gap)),
                (pair.negative, (pair.positive, pair.gap)),
            ]
        })
        .collect();
    let via_owner = |net: core::NetId, at: Point| -> Option<usize> {
        vias_of.get(&net).and_then(|vias| {
            vias.iter()
                .min_by(|a, b| {
                    core::geometry::distance(a.0, at).total_cmp(&core::geometry::distance(b.0, at))
                })
                .map(|(_, position)| *position)
        })
    };
    let mut refused: Vec<Option<String>> = vec![None; groups.len()];
    for violation in core::verify(board, &routes) {
        // Copper at exactly its clearance, but for rounding.
        if violation.actual + 1.0e-6 >= violation.required {
            continue;
        }
        let own = match violation.segment {
            Some(segment) => owner_of_segment.get(&(violation.net, segment)).copied(),
            None => via_owner(violation.net, violation.at),
        };
        let Some(own) = own else {
            continue;
        };
        let other_net = violation.other_segment.map(|(net, _)| net).or_else(|| {
            board
                .nets
                .iter()
                .position(|net| violation.other == format!("via of {}", net.name))
                .map(|net| net as core::NetId)
        });
        if let (Some(other), Some(&(mate, gap))) = (other_net, partner.get(&violation.net))
            && other == mate
            && !violation.other.contains("hole")
            && violation.actual + 1.0e-6 >= gap.min(violation.required)
        {
            continue;
        }
        let other = violation
            .other_segment
            .and_then(|key| owner_of_segment.get(&key).copied());
        let why = format!(
            "its copper is {:.3} mm from {} (needs {:.3})",
            violation.actual.max(0.0),
            violation.other,
            violation.required
        );
        // A finding between two groups refuses the one not fixed (both
        // when neither is).
        let blame: Vec<usize> = match other {
            Some(other) if groups[own].fixed => vec![other],
            Some(other) if groups[other].fixed => vec![own],
            Some(other) => vec![own, other],
            None => vec![own],
        };
        for position in blame {
            if !groups[position].fixed {
                refused[position].get_or_insert_with(|| why.clone());
            }
        }
    }
    refused
}

/// The pairs router's work deadline for a ladder of `budget` seconds.
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

/// The settings a route with differential pairs uses: a pair's nets at the
/// pair's width (the ordinary routing of its stubs and of a pair that
/// falls back too), and the project's diff pair rules the pairs take
/// reported `applied` (custom rules on `inDiffPair()` and the
/// `diff_pair_gap` and `diff_pair_uncoupled` constraints).
pub(crate) fn with_pairs(config: &KiCadBoardRouterConfig) -> KiCadBoardRouterConfig {
    let mut copy = config.clone();
    let mut used: BTreeSet<String> = BTreeSet::new();
    for (net, rule) in &config.diff_pair_rules {
        let first = (0..rule.layers.len())
            .find(|layer| !rule.disallowed.get(*layer).copied().unwrap_or(false));
        if let (Some(rules), Some((width, _))) = (
            copy.connection_rules.get_mut(net),
            first.and_then(|layer| rule.layers.get(layer)),
        ) {
            rules.trace_width_mm = *width;
        }
        used.extend(rule.rules.iter().cloned());
    }
    if let Some(report) = copy.project_rules.as_mut() {
        for entry in &mut report.custom_rules {
            if entry.status == "unmodelled" && used.contains(&entry.rule) {
                entry.status = "applied".into();
                entry.detail = "differential pairs (diff_pairs): the pairs' width and gap".into();
                let prefix = format!("{} ('{}'", entry.constraint, entry.rule);
                report.unmodelled.retain(|line| !line.starts_with(&prefix));
            }
        }
    }
    copy
}

/// After the route: each coupled pair's uncoupled length (the longer net's
/// tracks less the coupled run), from the routed board.
pub(crate) fn measure_uncoupled(report: &mut KiCadDiffPairs, pcb: &Expr) {
    let mut lengths: BTreeMap<String, f64> = BTreeMap::new();
    for item in pcb.children() {
        if item.head() != Some("segment") {
            continue;
        }
        let (Ok(start), Ok(end)) = (form_xy(item, "start"), form_xy(item, "end")) else {
            continue;
        };
        if let Some(net) = node_net(item) {
            *lengths
                .entry(crate::kicad_nets::normalize_net(&unescape(net)).to_string())
                .or_default() += core::geometry::distance(start, end);
        }
    }
    report.uncoupled_mm = 0.0;
    for pair in &mut report.pairs {
        if pair.links_coupled == 0 {
            continue;
        }
        let longer = lengths
            .get(&pair.positive)
            .copied()
            .unwrap_or(0.0)
            .max(lengths.get(&pair.negative).copied().unwrap_or(0.0));
        let uncoupled = (longer - pair.coupled_mm).max(0.0);
        pair.uncoupled_mm = Some(uncoupled);
        report.uncoupled_mm += uncoupled;
    }
}
