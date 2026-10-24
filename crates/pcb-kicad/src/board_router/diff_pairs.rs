// Copyright (C) 2026 Toit contributors.

//! Differential pairs (`diff_pairs`, docs/router.md "Differential
//! pairs").
//!
//! Nets KiCad pairs by name (`diff_pair_of`: `D+`/`D-`, `TX_P`/`TX_N`,
//! `USB_DP`/`USB_DN`) route coupled. A pair's pads sit side by side on each
//! part it joins (a *pair point*); the pair points are linked by a minimum
//! spanning tree (pair points of one part are joined by ordinary routing),
//! and each link is a net of the route itself (`plan_pairs`), negotiated
//! with every other net: its track is the pair's envelope (two tracks and
//! their gap: `2 w + g`), its via two vias side by side, from a *launch*
//! clear of one pair point's pads to one of the other's, which straight
//! stubs join to the pads. The pair's own nets lose the connections the
//! links make (their pads at a link's ends are one terminal). When the
//! route is written (`finish_pairs`), each link's centreline is offset by
//! `(w + g) / 2` to either side into the pair's two tracks, its vias into a
//! via of each net, and the stubs are added: what holds KiCad's clearances
//! (`verify`, against everything routed) is the pair's copper. What a link
//! left open (not routed, refused, a stub refused, the stubs of a crossed
//! end) is routed for the pair's nets with everything else kept (the
//! *fallback*).

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
    /// Links between pair points (of different parts), and those coupled.
    pub links: usize,
    pub links_coupled: usize,
    /// Coupled links whose pads' order crosses the run's at one end (the
    /// stubs cross there: the fallback routes them).
    pub crossed: usize,
    /// The coupled runs' length (millimetres, along the positive net).
    pub coupled_mm: f64,
    /// The coupled pairs' length beyond their coupled runs, after routing:
    /// per pair the longer net's track length less its coupled length
    /// (KiCad's uncoupled length), summed.
    pub uncoupled_mm: f64,
    /// The fallback router's work (seconds of an idle machine), charged
    /// to the ladder.
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
/// The launch's search: steps outward from between the pads, along
/// directions turned from the pads' normal by these angles (degrees).
const LAUNCH_STEP: f64 = 0.05;
const LAUNCH_REACH: f64 = 4.0;
const LAUNCH_ANGLES: [f64; 13] = [
    0.0, 15.0, -15.0, 30.0, -30.0, 45.0, -45.0, 60.0, -60.0, 75.0, -75.0, 90.0, -90.0,
];
/// What a launch turned a right angle from the pads' normal costs, in
/// millimetres of stub.
const ANGLE_COST: f64 = 1.0;
/// Room beyond the clearance around a launch point, so that lattice nodes
/// next to it are legal.
const LAUNCH_MARGIN: f64 = 0.15;
/// Launches closer than this make no coupled run: the pads' stubs are the
/// whole connection.
const MINIMUM_RUN: f64 = 1.0;
/// The nodes a coupled search starts from: within this of the launch.
const LAUNCH_CONTACT: f64 = 0.15;
/// The fallback router's share of the ladder's budget, at least
/// `MINIMUM_WORK` seconds.
const WORK_SHARE: f64 = 0.05;
const MINIMUM_WORK: f64 = 1.0;
/// Other nets' copper within this of the pairs' copper is checked with it.
const CHECK_REACH: f64 = 2.0;
/// Room between the two stubs' ends beyond the router's clearance.
const ENDS_MARGIN: f64 = 0.01;
/// At a layer change the pair's two vias move apart along the bisector of
/// the runs' normals by up to 1 / this (a right-angle turn).
const VIA_TURN: f64 = 0.7;
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
/// A direction turned by `degrees`.
fn turned(a: Point, degrees: f64) -> Point {
    let (sin, cos) = degrees.to_radians().sin_cos();
    [a[0] * cos - a[1] * sin, a[0] * sin + a[1] * cos]
}

fn debug() -> bool {
    std::env::var_os("PCB_ROUTER_DIFF_PAIRS_DEBUG").is_some()
}

/// A pair found on the board: its nets and its geometry.
#[derive(Clone)]
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
    via_diameter: f64,
    via_drill: f64,
    /// The board's hole clearance and hole to hole.
    hole_clearance: f64,
    hole_to_hole: f64,
    /// Pair points: (positive terminal, negative terminal).
    points: Vec<(usize, usize)>,
    /// Links between pair points of different parts: indices into
    /// `points`.
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
    /// How far from the run's centreline the pads' stubs end: the offset,
    /// or as far apart as the router keeps the two nets (its clearance, not
    /// the pair's gap, between their stubs), so that both ends have legal
    /// nodes; a jog joins them to the run.
    fn ends_offset(&self) -> f64 {
        self.offset()
            .max((self.width + self.clearance) / 2.0 + ENDS_MARGIN)
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
#[derive(Clone, Default)]
struct LinkCopper {
    /// Per polarity (0 positive, 1 negative): segments (layer, start, end)
    /// and vias.
    segments: [Vec<(usize, Point, Point)>; 2],
    vias: [Vec<Point>; 2],
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
        // The classes' clearance, raised to every custom pair clearance
        // that may hold for the pair's copper (ducktop2's 0.18 mm between
        // vias and tracks over 0.15 classes): the coupled run and the vias'
        // spacing keep it; the check of the copper holds the classes.
        let names = [n.name.as_str(), net.name.as_str()];
        let clearance = config
            .net_rules
            .iter()
            .flat_map(|rules| rules.clearances.iter())
            .filter(|rule| {
                let holds = |side: &Option<Vec<String>>| {
                    side.as_ref()
                        .is_none_or(|nets| nets.iter().any(|known| names.contains(&known.as_str())))
                };
                holds(&rule.first) || holds(&rule.second)
            })
            .map(|rule| rule.clearance)
            .fold(class_p.clearance.max(class_n.clearance), f64::max);
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
                    && !straddles(board, ta, tb, [positive, negative])
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
        // A minimum spanning tree over the pair points (Prim), where pair
        // points of one part are joined first (at no cost) and by ordinary
        // routing (a USB-C receptacle's two rows): only links between
        // parts are coupled.
        let middle = |point: &(usize, usize)| {
            scale(
                add(p.terminals[point.0].anchor, n.terminals[point.1].anchor),
                0.5,
            )
        };
        let part = |point: &(usize, usize)| reference(&p.terminals[point.0].label);
        let mut links = Vec::new();
        if points.len() >= 2 {
            let mut inside = vec![false; points.len()];
            inside[0] = true;
            for _ in 1..points.len() {
                let mut best: Option<(f64, usize, usize)> = None;
                for (i, a) in points.iter().enumerate().filter(|(i, _)| inside[*i]) {
                    for (j, b) in points.iter().enumerate().filter(|(j, _)| !inside[*j]) {
                        let length = if part(a) == part(b) {
                            0.0
                        } else {
                            core::geometry::distance(middle(a), middle(b))
                        };
                        if best.is_none_or(|(known, _, _)| length < known) {
                            best = Some((length, i, j));
                        }
                    }
                }
                let (_, i, j) = best.expect("a point outside");
                inside[j] = true;
                if part(&points[i]) != part(&points[j]) {
                    links.push((i, j));
                }
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

/// Whether another net's copper lies between two pads (a pin between
/// them: they are not side by side).
fn straddles(
    board: &core::Board,
    a: &core::Terminal,
    b: &core::Terminal,
    nets: [core::NetId; 2],
) -> bool {
    let bounds = core::geometry::Aabb {
        minimum: [a.anchor[0].min(b.anchor[0]), a.anchor[1].min(b.anchor[1])],
        maximum: [a.anchor[0].max(b.anchor[0]), a.anchor[1].max(b.anchor[1])],
    };
    board.obstacles.iter().any(|obstacle| {
        obstacle.kind == core::ObstacleKind::Copper
            && obstacle.layers & a.layers & b.layers != 0
            && obstacle.net.is_none_or(|net| !nets.contains(&net))
            && obstacle.shape.aabb().intersects(bounds)
            && obstacle.shape.distance_to_segment(a.anchor, b.anchor) < EPSILON
    })
}

/// How far copper of `clearance` keeps from `obstacle`.
fn keep_from(board: &core::Board, obstacle: &core::Obstacle, clearance: f64) -> f64 {
    match obstacle.kind {
        core::ObstacleKind::Copper => board.copper_clearance(
            &core::RuleClass {
                trace_width: 0.0,
                clearance,
                via_diameter: 0.0,
                via_drill: 0.0,
                layers: 0,
            },
            obstacle,
        ),
        core::ObstacleKind::Keepout => obstacle.clearance,
        core::ObstacleKind::Hole => board.hole_clearance.max(obstacle.clearance),
    }
}

/// The obstacles that block tracks within `reach` of `at`.
fn obstacles_near(board: &core::Board, at: Point, reach: f64) -> Vec<usize> {
    board
        .obstacles
        .iter()
        .enumerate()
        .filter(|(_, obstacle)| {
            let bounds = obstacle.shape.aabb();
            obstacle.blocks_tracks
                && at[0] >= bounds.minimum[0] - reach
                && at[0] <= bounds.maximum[0] + reach
                && at[1] >= bounds.minimum[1] - reach
                && at[1] <= bounds.maximum[1] + reach
        })
        .map(|(index, _)| index)
        .collect()
}

/// The layers where a disc of `radius` (plus each obstacle's clearance)
/// at `at` is clear of the obstacles `near` and of the board edge.
fn clear_layers(
    board: &core::Board,
    near: &[usize],
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
    let mut free = mask;
    for &index in near {
        let obstacle = &board.obstacles[index];
        if obstacle.layers & free == 0 {
            continue;
        }
        if obstacle.shape.distance_to_point(at) < radius + keep_from(board, obstacle, clearance) {
            free &= !obstacle.layers;
        }
    }
    free
}

/// A launch: where a link's coupled run starts at a pair point, and where
/// its net starts in the route.
#[derive(Clone, Copy, Debug)]
struct Launch {
    /// Where the link's net starts: the end of the run's straight lead.
    at: Point,
    /// Where the run starts: the pads' stubs (ordinary routing of the
    /// pair's nets) end beside it, an offset to either side.
    start: Point,
    /// Where the link's net's copper begins on the lead: its track clear
    /// of the stubs' ends' copper, which reaches `ends` past the start.
    reserved: Point,
    ends: Point,
    /// The run's direction away from the pads.
    outward: Point,
    /// The layer (a bit) the run starts on.
    layers: core::LayerMask,
    /// The side of the run the positive pad is on (1 left, -1 right).
    side: f64,
}

/// The straight lead of a run from its start to its launch, at least, in
/// offsets (`Pair::offset`): a turn of the search at the launch leaves the
/// offset tracks room up to the miter limit (`offset_line`), away from the
/// pads' stubs.
const LEAD: f64 = 2.5;

/// Where the part of a terminal (`REF.pad`) sits: the middle of its pads'
/// copper, when it has more than the two given.
fn part_middle(board: &core::Board, label: &str) -> Option<Point> {
    let reference = label.rsplit_once('.').map(|(part, _)| part)?;
    let prefix = format!("{reference}.");
    let (mut sum, mut count) = ([0.0, 0.0], 0usize);
    for obstacle in &board.obstacles {
        if obstacle.kind == core::ObstacleKind::Copper
            && obstacle
                .label
                .strip_prefix(&prefix)
                .is_some_and(|pad| !pad.contains(' '))
        {
            let bounds = obstacle.shape.aabb();
            sum = add(sum, scale(add(bounds.minimum, bounds.maximum), 0.5));
            count += 1;
        }
    }
    (count > 2).then(|| scale(sum, 1.0 / count as f64))
}

/// Whether a track of the pair's width from `at` to `to` on `layer`
/// keeps its clearance from the obstacles `near`, the pair's own copper
/// the pair's gap (either net may end there: where the pads' order crosses
/// the run's, the stubs swap sides).
fn end_clear(
    board: &core::Board,
    near: &[usize],
    pair: &Pair,
    (at, to): (Point, Point),
    layer: core::LayerMask,
) -> bool {
    near.iter().all(|&index| {
        let obstacle = &board.obstacles[index];
        if obstacle.layers & layer == 0 {
            return true;
        }
        let mut keep = keep_from(board, obstacle, pair.clearance);
        if obstacle.net == Some(pair.positive) || obstacle.net == Some(pair.negative) {
            keep = keep.min(pair.gap);
        }
        obstacle.shape.distance_to_segment(at, to) >= pair.width / 2.0 + keep
    })
}

/// A launch for a link at the pair point of pads `p` and `n` toward
/// `toward`: from between the pads along the pads' normal away from their
/// part (toward the link's other end where the part does not say), or
/// turned from it by up to a right angle, the first run start where the
/// stubs' ends beside it, the run's straight lead and the envelope `class`
/// at the lead's end are clear; of those the one closest to the pads (a
/// turn costs `ANGLE_COST` a right angle). The stubs themselves are the
/// route's: the pair's nets' ordinary routing from the pads to the ends.
fn launch(
    board: &core::Board,
    pair: &Pair,
    class: &core::RuleClass,
    p: &core::Terminal,
    n: &core::Terminal,
    toward: Point,
) -> Option<Launch> {
    let middle = scale(add(p.anchor, n.anchor), 0.5);
    let mut normal = left(unit(sub(n.anchor, p.anchor)));
    let away = part_middle(board, &p.label)
        .map(|part| dot(normal, sub(middle, part)))
        .filter(|along| along.abs() > 0.1);
    if away.unwrap_or_else(|| dot(normal, sub(toward, middle))) < 0.0 {
        normal = scale(normal, -1.0);
    }
    // One layer: the stubs' ends and the run start on it.
    let layers = p.layers & n.layers & pair.layers;
    if layers == 0 {
        return None;
    }
    let layers = layers.isolate_lowest_one();
    let half = class.trace_width / 2.0;
    let d = pair.offset();
    // The stubs' ends' copper reaches a track width along the lead; the
    // link's copper keeps its clearance from it; the lead is at least
    // `LEAD` offsets long.
    let ends_reach = pair.width;
    let apart = pair.width / 2.0 + class.clearance + half;
    let reserve = ends_reach + (apart * apart - d * d).max(0.0).sqrt() + LAUNCH_STEP;
    let lead = (LEAD * d).max(reserve + 2.0 * LAUNCH_STEP);
    let near = obstacles_near(
        board,
        middle,
        LAUNCH_REACH
            + lead
            + half
            + LAUNCH_MARGIN
            + class.clearance.max(board.hole_clearance)
            + 2.0,
    );
    let steps = (LAUNCH_REACH / LAUNCH_STEP) as usize;
    let mut best: Option<(f64, Launch)> = None;
    for angle in LAUNCH_ANGLES {
        let outward = turned(normal, angle);
        let side = if dot(left(outward), sub(p.anchor, n.anchor)) >= 0.0 {
            1.0
        } else {
            -1.0
        };
        let found = (0..=steps).find_map(|step| {
            let start = add(middle, scale(outward, step as f64 * LAUNCH_STEP));
            let at = add(start, scale(outward, lead));
            let reserved = add(start, scale(outward, reserve));
            // The envelope at the launch, with room for lattice nodes, and
            // along the reserved lead.
            if clear_layers(
                board,
                &near,
                at,
                half + LAUNCH_MARGIN,
                class.clearance,
                layers,
            ) == 0
            {
                return None;
            }
            let samples = ((lead - reserve) / LAUNCH_STEP).ceil() as usize;
            if (0..samples).any(|sample| {
                let point = add(reserved, scale(outward, sample as f64 * LAUNCH_STEP));
                clear_layers(board, &near, point, half, class.clearance, layers) == 0
            }) {
                return None;
            }
            let e = pair.ends_offset();
            let ends = [
                add(start, scale(left(outward), side * e)),
                add(start, scale(left(outward), -side * e)),
            ];
            let lead_of = |end: Point| (end, add(end, scale(outward, reserve)));
            (end_clear(board, &near, pair, lead_of(ends[0]), layers)
                && end_clear(board, &near, pair, lead_of(ends[1]), layers))
            .then_some(Launch {
                at,
                start,
                reserved,
                ends: scale(outward, ends_reach),
                outward,
                layers,
                side,
            })
        });
        if let Some(found) = found {
            let cost =
                core::geometry::distance(middle, found.start) + ANGLE_COST * angle.abs() / 90.0;
            if best.is_none_or(|(known, _)| cost < known - 1.0e-9) {
                best = Some((cost, found));
            }
        }
    }
    if debug() {
        eprintln!(
            "diff pair launch {} / {}: {:?}",
            p.label,
            n.label,
            best.map(|(_, launch)| launch)
        );
    }
    best.map(|(_, launch)| launch)
}

/// A link routed as a net of its own (`PairPlan::links`).
#[derive(Clone, Debug)]
struct LinkPlan {
    net: core::NetId,
    pair: usize,
    link: usize,
    /// Per end: the launch (where the link's net starts) and where the run
    /// starts.
    launches: [Point; 2],
    starts: [Point; 2],
    /// Per end and polarity: where the stubs end, and the pads they start
    /// from.
    ends: [[Point; 2]; 2],
    pads: [[Point; 2]; 2],
    /// The side of the run the positive net takes (1 left, -1 right),
    /// along the run from the first end.
    side: f64,
    /// Whether the pads' order at the second end crosses the run's (the
    /// stubs cross there).
    crossed: bool,
}

impl LinkPlan {
    /// The copper of the stub end of `polarity` at `end`: from where the
    /// stub ends a track width along the lead (`plan_pairs`).
    fn stub_end(&self, end: usize, polarity: usize, width: f64) -> (Point, Point) {
        let at = self.ends[end][polarity];
        let along = unit(sub(self.launches[end], self.starts[end]));
        (at, add(at, scale(along, width)))
    }
}

/// A board whose pairs' links are nets of the route (`plan_pairs`), as
/// its `Board::attachment` keeps it for `finish_pairs`.
struct PairPlan {
    /// The board's nets, obstacles and classes before the links' (which
    /// follow).
    nets: usize,
    obstacles: usize,
    classes: usize,
    /// The pairs' nets and the board's merged pads and terminal copper as
    /// lowered, before the links' ends were added and merged.
    originals: Vec<(core::NetId, core::Net)>,
    merged_pads: Vec<core::MergedPad>,
    terminal_copper: Vec<core::TerminalCopper>,
    pairs: Vec<Pair>,
    entries: Vec<KiCadDiffPair>,
    links: Vec<LinkPlan>,
}

/// Merges terminals of `net` into their hosts (`host_of[t]`, the smallest
/// terminal of its group): they become merged pads of the host, which a
/// route may end on (`Board::terminal_copper`), as the board's own copper
/// merges the pads it joins (`keep_existing_copper`).
fn merge_terminals(board: &mut core::Board, net: core::NetId, host_of: &[usize]) {
    let terminals = board.nets[net as usize].terminals.len();
    let mut renumbered = vec![usize::MAX; terminals];
    let mut next = 0;
    for terminal in 0..terminals {
        if host_of[terminal] == terminal {
            renumbered[terminal] = next;
            next += 1;
        }
    }
    if next == terminals {
        return;
    }
    let mut merged = Vec::new();
    for terminal in 0..terminals {
        if host_of[terminal] != terminal {
            let description = &board.nets[net as usize].terminals[terminal];
            merged.push(core::MergedPad {
                net,
                host: renumbered[host_of[terminal]],
                pad: description.pad,
                anchor: description.anchor,
                layers: board.obstacles[description.pad].layers,
                padstack: false,
            });
        }
    }
    for existing in board
        .merged_pads
        .iter_mut()
        .filter(|existing| existing.net == net && existing.host < terminals)
    {
        existing.padstack &= host_of[existing.host] == existing.host;
        existing.host = renumbered[host_of[existing.host]];
    }
    for pad in &merged {
        board.terminal_copper.push(core::TerminalCopper {
            net,
            terminal: pad.host,
            obstacle: pad.pad,
        });
    }
    let mut index = 0;
    board.nets[net as usize].terminals.retain(|_| {
        index += 1;
        host_of[index - 1] == index - 1
    });
    board.merged_pads.extend(merged);
}

fn root(parent: &mut [usize], mut item: usize) -> usize {
    while parent[item] != item {
        parent[item] = parent[parent[item]];
        item = parent[item];
    }
    item
}

/// Makes each link between a pair's pair points a net of the route:
/// launches at both ends (`launch`), the run's lead as its copper (other
/// nets keep clear of it), a class for the pairs of one layer set (the
/// widest envelope and via of them, kept to the layers); the pair's nets
/// get a terminal beside each end of the run (their stubs from the pads
/// end there: ordinary routing), the two ends of a link one terminal. The
/// plan goes into `board.attachment` for `finish_pairs`.
pub(crate) fn plan_pairs(board: &mut core::Board, config: &KiCadBoardRouterConfig) {
    let pairs = find_pairs(board, config);
    let mut entries: Vec<KiCadDiffPair> = pairs
        .iter()
        .map(|pair| KiCadDiffPair {
            positive: board.nets[pair.positive as usize].name.clone(),
            negative: board.nets[pair.negative as usize].name.clone(),
            width_mm: pair.width,
            gap_mm: pair.gap,
            source: pair.source.clone(),
            links: pair.links.len(),
            why: if pair.links.is_empty() {
                vec!["fewer than two parts with the pair's pads side by side".into()]
            } else {
                Vec::new()
            },
            ..KiCadDiffPair::default()
        })
        .collect();
    let mut plan = PairPlan {
        nets: board.nets.len(),
        obstacles: board.obstacles.len(),
        classes: board.classes.len(),
        originals: Vec::new(),
        merged_pads: board.merged_pads.clone(),
        terminal_copper: board.terminal_copper.clone(),
        pairs: Vec::new(),
        entries: Vec::new(),
        links: Vec::new(),
    };
    // A class per layer set: the widest envelope, clearance and via of its
    // pairs (each class builds static maps of the whole board).
    let mut class_of: BTreeMap<core::LayerMask, core::ClassId> = BTreeMap::new();
    for (index, pair) in pairs.iter().enumerate() {
        if pair.links.is_empty() {
            continue;
        }
        if pair.layers == 0 {
            entries[index]
                .why
                .push("no layer holds the pair's width and gap".into());
            continue;
        }
        // Kept copper on the pair's nets (a board routed in part) keeps its
        // terminals as they are.
        if board
            .terminal_copper
            .iter()
            .any(|extra| extra.net == pair.positive || extra.net == pair.negative)
        {
            entries[index]
                .why
                .push("the pair's nets have kept copper".into());
            continue;
        }
        if class_of.contains_key(&pair.layers) {
            continue;
        }
        let members: Vec<&Pair> = pairs
            .iter()
            .filter(|other| other.layers == pair.layers && !other.links.is_empty())
            .collect();
        let widest = |measure: &dyn Fn(&Pair) -> f64| {
            members.iter().map(|pair| measure(pair)).fold(0.0, f64::max)
        };
        board.classes.push(core::RuleClass {
            trace_width: widest(&|pair| pair.envelope()),
            clearance: widest(&|pair| pair.clearance),
            // Two vias side by side, as far apart as at a right-angle turn
            // (`link_copper`).
            via_diameter: widest(&|pair| 2.0 * pair.via_offset() / VIA_TURN + pair.via_diameter),
            via_drill: widest(&|pair| 2.0 * pair.via_offset() / VIA_TURN + pair.via_drill),
            layers: if pair.layers == board.all_layers() {
                0
            } else {
                pair.layers
            },
        });
        class_of.insert(pair.layers, board.classes.len() - 1);
    }
    // Per pair net: its terminals joined by the links (union-find), the
    // stubs' ends added.
    let mut parents: BTreeMap<core::NetId, Vec<usize>> = BTreeMap::new();
    for (index, pair) in pairs.iter().enumerate() {
        let Some(&class) = class_of.get(&pair.layers) else {
            continue;
        };
        if !entries[index].why.is_empty() {
            continue;
        }
        let envelope = board.classes[class];
        for (link, &(a, b)) in pair.links.iter().enumerate() {
            let (point_a, point_b) = (pair.points[a], pair.points[b]);
            let terminal = |net: core::NetId, terminal: usize| {
                board.nets[net as usize].terminals[terminal].clone()
            };
            let ends = [
                (
                    terminal(pair.positive, point_a.0),
                    terminal(pair.negative, point_a.1),
                ),
                (
                    terminal(pair.positive, point_b.0),
                    terminal(pair.negative, point_b.1),
                ),
            ];
            let middle = |end: usize| scale(add(ends[end].0.anchor, ends[end].1.anchor), 0.5);
            let launches = [
                launch(board, pair, &envelope, &ends[0].0, &ends[0].1, middle(1)),
                launch(board, pair, &envelope, &ends[1].0, &ends[1].1, middle(0)),
            ];
            let [Some(launch_a), Some(launch_b)] = launches else {
                entries[index]
                    .why
                    .push(format!("link {}: no launch clear of the pads", link + 1));
                continue;
            };
            if core::geometry::distance(launch_a.at, launch_b.at) < MINIMUM_RUN {
                entries[index].why.push(format!(
                    "link {}: the launches are too close for a coupled run",
                    link + 1
                ));
                continue;
            }
            let id = board.nets.len() as core::NetId;
            // Along the run from the first end the positive net keeps the
            // side its pad has there; at the second end the run arrives
            // against that launch's outward direction.
            let side = launch_a.side;
            let crossed = launch_b.side != side;
            let mut terminals = Vec::new();
            let mut stub_ends = [[[0.0; 2]; 2]; 2];
            for (end, found) in [launch_a, launch_b].into_iter().enumerate() {
                let label = format!("pair launch {}{}", link + 1, ["a", "b"][end]);
                // The lead past the stubs' ends: the terminal's copper; the
                // search starts close to the launch.
                terminals.push(core::Terminal {
                    anchor: found.at,
                    layers: found.layers,
                    pad: board.obstacles.len(),
                    contact: Some(core::Shape::Circle {
                        center: found.at,
                        radius: LAUNCH_CONTACT,
                    }),
                    label: label.clone(),
                });
                board.obstacles.push(core::Obstacle {
                    shape: core::Shape::Capsule {
                        start: found.reserved,
                        end: found.at,
                        radius: envelope.trace_width / 2.0,
                    },
                    layers: found.layers,
                    kind: core::ObstacleKind::Copper,
                    net: Some(id),
                    clearance: 0.0,
                    clearance_override: None,
                    blocks_tracks: true,
                    blocks_vias: true,
                    label,
                });
                // The stubs' ends: a terminal of each of the pair's nets,
                // its copper the run's track up to the link's (which the
                // other nets keep clear of).
                let across = if end == 0 {
                    left(found.outward)
                } else {
                    scale(left(found.outward), -1.0)
                };
                let reach = found.ends;
                for (polarity, (net, sign)) in [(pair.positive, side), (pair.negative, -side)]
                    .into_iter()
                    .enumerate()
                {
                    let at = add(found.start, scale(across, sign * pair.ends_offset()));
                    stub_ends[end][polarity] = at;
                    let terminals = &mut board.nets[net as usize].terminals;
                    terminals.push(core::Terminal {
                        anchor: at,
                        layers: found.layers,
                        pad: board.obstacles.len(),
                        contact: Some(core::Shape::Circle {
                            center: at,
                            radius: LAUNCH_CONTACT,
                        }),
                        label: format!("pair run {}{}", link + 1, ["a", "b"][end]),
                    });
                    board.obstacles.push(core::Obstacle {
                        shape: core::Shape::Capsule {
                            start: at,
                            end: add(at, reach),
                            radius: pair.width / 2.0,
                        },
                        layers: found.layers,
                        kind: core::ObstacleKind::Copper,
                        net: Some(net),
                        clearance: 0.0,
                        clearance_override: None,
                        blocks_tracks: true,
                        blocks_vias: true,
                        label: format!("pair run {}{}", link + 1, ["a", "b"][end]),
                    });
                }
            }
            board.nets.push(core::Net {
                name: format!(
                    "{} / {} link {}",
                    board.nets[pair.positive as usize].name,
                    board.nets[pair.negative as usize].name,
                    link + 1
                ),
                class,
                terminals,
            });
            for net in [pair.positive, pair.negative] {
                let count = board.nets[net as usize].terminals.len();
                let parent = parents.entry(net).or_default();
                parent.extend(parent.len()..count);
                // The run's two ends, the last two pairs of terminals.
                let (x, y) = (root(parent, count - 2), root(parent, count - 1));
                if x != y {
                    parent[x.max(y)] = x.min(y);
                }
            }
            plan.links.push(LinkPlan {
                net: id,
                pair: index,
                link,
                launches: [launch_a.at, launch_b.at],
                starts: [launch_a.start, launch_b.start],
                ends: stub_ends,
                pads: [
                    [ends[0].0.anchor, ends[0].1.anchor],
                    [ends[1].0.anchor, ends[1].1.anchor],
                ],
                side,
                crossed,
            });
        }
    }
    for (&net, parent) in &mut parents {
        let mut original = board.nets[net as usize].clone();
        original
            .terminals
            .retain(|terminal| !terminal.label.starts_with("pair run "));
        plan.originals.push((net, original));
        let host_of: Vec<usize> = (0..parent.len()).map(|t| root(parent, t)).collect();
        merge_terminals(board, net, &host_of);
    }
    if debug() {
        eprintln!(
            "diff pairs: {} found, {} links as nets of the route",
            pairs.len(),
            plan.links.len()
        );
    }
    plan.pairs = pairs;
    plan.entries = entries;
    board.attachment = core::Attachment(Some(std::sync::Arc::new(plan)));
}

/// A link's path from its net's route: (point, layer) from the launch at
/// `start` to the one at `end`, through its segments and vias.
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
/// run and a via of each net side by side at each layer change; the
/// positive net on `side` of the run (1 left, -1 right).
fn link_copper(path: &[(Point, usize)], pair: &Pair, side: f64) -> Option<LinkCopper> {
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
    directions.iter().flatten().next()?;
    let mut copper = LinkCopper::default();
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
                let half_turn_cosine = dot(unit(bisector), left(into)).abs().max(VIA_TURN);
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
    }
    (!copper.segments[0].is_empty() && !copper.segments[1].is_empty()).then_some(copper)
}

/// The pair copper of a written board (`finish_pairs`), to write as
/// tracks and vias of the pairs' nets (`write_pair_copper`).
pub(crate) struct PairCopper {
    /// (net name, width, via diameter, via drill, segments, vias).
    #[allow(clippy::type_complexity)]
    items: Vec<(
        String,
        f64,
        f64,
        f64,
        Vec<(usize, Point, Point)>,
        Vec<Point>,
    )>,
}

/// Writes the pairs' copper into `pcb` as tracks and vias of their nets.
pub(crate) fn write_pair_copper(
    pcb: &mut Expr,
    copper: &PairCopper,
    layer_names: &[String],
) -> Result<(), String> {
    let Expr::List(items) = pcb else {
        return Err("PCB root is not a list".into());
    };
    for (name, width, size, drill, segments, vias) in &copper.items {
        for &(layer, start, end) in segments {
            items.push(supplemental_segment_expr(&SupplementalSegment {
                connection: name.clone(),
                start: nanometres(start),
                end: nanometres(end),
                width: *width,
                layer: layer_names[layer].clone(),
            }));
        }
        for &at in vias {
            items.push(supplemental_via_expr(&SupplementalVia {
                connection: name.clone(),
                at: nanometres(at),
                size: *size,
                drill: *drill,
                layers: ["F.Cu".into(), "B.Cu".into()],
            }));
        }
    }
    Ok(())
}

/// A routed board with its pairs' links as nets (`plan_pairs`) turned back
/// into the board as lowered: each link's route offset into the pair's
/// tracks and vias from one run start to the other (where the pair's nets'
/// stubs end) and checked (`refused_groups`, against everything routed);
/// what holds is kept copper of the returned board (`existing track`,
/// `existing via`) and in the returned `PairCopper`; the links' nets and
/// the stubs' end terminals are gone from the board and the result; what
/// the links left open is routed for the pairs' nets with everything else
/// kept (the fallback, its work in the report). `None`: the board has no
/// plan.
pub(crate) fn finish_pairs(
    board: &core::Board,
    result: &mut core::RoutingResult,
    config: &KiCadBoardRouterConfig,
) -> Option<(core::Board, PairCopper, KiCadDiffPairs)> {
    let plan = board
        .attachment
        .0
        .as_ref()
        .and_then(|any| any.clone().downcast::<PairPlan>().ok())?;
    let mut entries = plan.entries.clone();
    let mut report = KiCadDiffPairs {
        found: plan.pairs.len(),
        ..KiCadDiffPairs::default()
    };
    // The board as lowered.
    let mut restored = board.clone();
    restored.attachment = core::Attachment::default();
    restored.nets.truncate(plan.nets);
    restored.obstacles.truncate(plan.obstacles);
    restored.classes.truncate(plan.classes);
    for (net, original) in &plan.originals {
        restored.nets[*net as usize] = original.clone();
    }
    restored.merged_pads = plan.merged_pads.clone();
    restored.terminal_copper = plan.terminal_copper.clone();
    let link_routes: Vec<core::NetRoute> =
        result.routes.split_off(plan.nets.min(result.routes.len()));
    let link_status: Vec<core::NetStatus> =
        result.status.split_off(plan.nets.min(result.status.len()));
    // The links' nets in the router's diagnostics are their pairs'.
    let pair_net = |net: core::NetId| {
        if (net as usize) < plan.nets {
            net
        } else {
            plan.links
                .iter()
                .find(|link| link.net == net)
                .map_or(0, |link| plan.pairs[link.pair].positive)
        }
    };
    for dead in &mut result.diagnostics.dead_pads {
        dead.net = pair_net(dead.net);
    }
    for net in &mut result.diagnostics.conflicted {
        *net = pair_net(*net);
    }
    result.diagnostics.conflicted.sort_unstable();
    result.diagnostics.conflicted.dedup();
    // The links' copper.
    let mut candidates: Vec<(usize, usize, LinkCopper)> = Vec::new();
    let mut candidate_links: Vec<&LinkPlan> = Vec::new();
    let mut open_pairs: BTreeSet<usize> = BTreeSet::new();
    for plan_link in &plan.links {
        let (index, link) = (plan_link.pair, plan_link.link);
        let pair = &plan.pairs[index];
        let offset = (plan_link.net as usize) - plan.nets;
        if link_status.get(offset) != Some(&core::NetStatus::Routed) {
            entries[index]
                .why
                .push(format!("link {}: the coupled run was not routed", link + 1));
            open_pairs.insert(index);
            continue;
        }
        let [at_a, at_b] = plan_link.launches;
        let [start_a, start_b] = plan_link.starts;
        // The run from its start (the stubs' ends beside it) to the
        // route's first node past the launch (the route's own stub to the
        // launch goes), and the same at its end.
        let path = link_path(&link_routes[offset], at_a, at_b).map(|mut path| {
            if path.len() >= 3
                && path[0].1 == path[1].1
                && core::geometry::distance(path[0].0, at_a) < 1.0e-6
            {
                path.remove(0);
            }
            let count = path.len();
            if count >= 3
                && path[count - 1].1 == path[count - 2].1
                && core::geometry::distance(path[count - 1].0, at_b) < 1.0e-6
            {
                path.pop();
            }
            // Half the lead straight along the launch's direction, so that
            // the run's tracks start beside the stubs' ends.
            let (first, last) = (path[0], path[path.len() - 1]);
            let half = |start: Point, at: Point| scale(add(start, at), 0.5);
            path.insert(0, (half(start_a, at_a), first.1));
            path.insert(0, (start_a, first.1));
            path.push((half(start_b, at_b), last.1));
            path.push((start_b, last.1));
            path
        });
        if debug() {
            eprintln!("diff pair link {} path {:?}", plan_link.net, path);
        }
        let copper = path.as_ref().and_then(|path| {
            link_copper(path, pair, plan_link.side).map(|mut copper| {
                copper.crossed = plan_link.crossed;
                // The jogs from the stubs' ends to the run's tracks.
                for (polarity, segments) in copper.segments.iter_mut().enumerate() {
                    let [from, to] = [plan_link.ends[0][polarity], plan_link.ends[1][polarity]];
                    if let Some(&(layer, tip, _)) = segments.first()
                        && core::geometry::distance(tip, from) > 1.0e-6
                    {
                        segments.insert(0, (layer, from, tip));
                    }
                    if let Some(&(layer, _, tip)) = segments.last()
                        && core::geometry::distance(tip, to) > 1.0e-6
                    {
                        segments.push((layer, tip, to));
                    }
                }
                copper
            })
        });
        match copper {
            Some(copper) => {
                candidates.push((index, link, copper));
                candidate_links.push(plan_link);
            }
            None => {
                entries[index].why.push(format!(
                    "link {}: the coupled route has no clean path",
                    link + 1
                ));
                open_pairs.insert(index);
            }
        }
    }
    let groups: Vec<Group> = candidates
        .iter()
        .map(|(index, _, copper)| Group {
            pair: *index,
            segments: copper.segments.clone(),
            vias: copper.vias.clone(),
        })
        .collect();
    let refused = refused_groups(&restored, &result.routes, &plan.pairs, &groups);
    let mut copper_out = PairCopper { items: Vec::new() };
    let mut kept_links: Vec<&LinkPlan> = Vec::new();
    for (position, (index, link, copper)) in candidates.into_iter().enumerate() {
        if let Some(why) = &refused[position] {
            entries[index].why.push(format!("link {}: {why}", link + 1));
            open_pairs.insert(index);
            continue;
        }
        kept_links.push(candidate_links[position]);
        let pair = &plan.pairs[index];
        for (polarity, net) in [(0, pair.positive), (1, pair.negative)] {
            for &(layer, start, end) in &copper.segments[polarity] {
                restored.obstacles.push(core::Obstacle {
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
                restored.obstacles.push(core::Obstacle {
                    shape: core::Shape::Circle {
                        center: at,
                        radius: pair.via_diameter / 2.0,
                    },
                    layers: restored.all_layers(),
                    kind: core::ObstacleKind::Copper,
                    net: Some(net),
                    clearance: 0.0,
                    clearance_override: None,
                    blocks_tracks: true,
                    blocks_vias: true,
                    label: "existing via".into(),
                });
            }
            copper_out.items.push((
                restored.nets[net as usize].name.clone(),
                pair.width,
                pair.via_diameter,
                pair.via_drill,
                copper.segments[polarity].clone(),
                copper.vias[polarity].clone(),
            ));
        }
        entries[index].links_coupled += 1;
        entries[index].coupled_mm += copper.coupled;
        report.crossed += copper.crossed as usize;
    }
    // The stubs of the links not kept go (the fallback routes the pads
    // anew).
    let kept_nets: BTreeSet<core::NetId> = kept_links.iter().map(|link| link.net).collect();
    for plan_link in plan
        .links
        .iter()
        .filter(|link| !kept_nets.contains(&link.net))
    {
        let pair = &plan.pairs[plan_link.pair];
        for end in 0..2 {
            for (polarity, net) in [(0, pair.positive), (1, pair.negative)] {
                let route = &mut result.routes[net as usize];
                if let Some((_, stub)) = stub_path(
                    route,
                    plan_link.pads[end][polarity],
                    plan_link.stub_end(end, polarity, pair.width),
                    pair.width,
                ) {
                    let mut index = 0;
                    route.segments.retain(|_| {
                        index += 1;
                        !stub.contains(&(index - 1))
                    });
                }
            }
        }
    }
    // The kept links' stubs straight where that holds (the router's run
    // parallel out of the pads, which KiCad takes for a coupled piece at
    // the pads' pitch).
    for plan_link in kept_links {
        straighten_stubs(&restored, result, &plan.pairs, plan_link);
    }
    // The fallback: the pairs' nets with a connection a link left open,
    // routed with everything else kept.
    if !open_pairs.is_empty() {
        report.work_seconds = route_fallback(&restored, result, &plan.pairs, &open_pairs, config);
    }
    // A pair's net whose pads the links joined all is routed, not trivial.
    for (net, original) in &plan.originals {
        let net = *net as usize;
        if original.terminals.len() >= 2 && result.status[net] == core::NetStatus::Trivial {
            result.status[net] = core::NetStatus::Routed;
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
    if debug() {
        eprintln!(
            "diff pairs written: {} of {} links coupled, fallback {:.2} s of work for {} pairs",
            report.links_coupled,
            report.links,
            report.work_seconds,
            open_pairs.len()
        );
    }
    Some((restored, copper_out, report))
}

/// The segments of a route's stub from `pad` to `stub_end` (its track
/// touches the end's copper, a track of `width` along the lead: the
/// route may end anywhere on it, `Board::terminal_copper`): one layer, no
/// branch, no via; and the layer.
fn stub_path(
    route: &core::NetRoute,
    pad: Point,
    (stub_end, lead_end): (Point, Point),
    width: f64,
) -> Option<(usize, Vec<usize>)> {
    let key = |point: Point| {
        (
            (point[0] * 1.0e6).round() as i64,
            (point[1] * 1.0e6).round() as i64,
        )
    };
    let path = link_path(route, pad, stub_end)?;
    if debug() {
        eprintln!("diff pair stub path {path:?}");
    }
    let (first, last) = (path[0], path[path.len() - 1]);
    let reach = (width
        + route
            .segments
            .iter()
            .map(|segment| segment.width)
            .fold(0.0, f64::max))
        / 2.0;
    if core::geometry::distance(first.0, pad) > 1.0e-6
        || core::geometry::point_segment_distance(last.0, stub_end, lead_end) > reach + 1.0e-6
        || path.iter().any(|(_, layer)| *layer != first.1)
        || path.len() < 2
    {
        return None;
    }
    let mut degree: BTreeMap<(i64, i64), usize> = BTreeMap::new();
    for segment in route
        .segments
        .iter()
        .filter(|segment| segment.layer == first.1)
    {
        *degree.entry(key(segment.start)).or_default() += 1;
        *degree.entry(key(segment.end)).or_default() += 1;
    }
    let vias: BTreeSet<(i64, i64)> = route.vias.iter().map(|via| key(via.at)).collect();
    let ends_free = degree.get(&key(last.0)) == Some(&1) && !vias.contains(&key(last.0));
    if !ends_free
        || path[1..path.len() - 1]
            .iter()
            .any(|(point, _)| degree.get(&key(*point)) != Some(&2) || vias.contains(&key(*point)))
    {
        return None;
    }
    let on_path: BTreeSet<((i64, i64), (i64, i64))> = path
        .windows(2)
        .flat_map(|step| {
            let (a, b) = (key(step[0].0), key(step[1].0));
            [(a, b), (b, a)]
        })
        .collect();
    let stub: Vec<usize> = route
        .segments
        .iter()
        .enumerate()
        .filter(|(_, segment)| {
            segment.layer == first.1 && on_path.contains(&(key(segment.start), key(segment.end)))
        })
        .map(|(index, _)| index)
        .collect();
    (stub.len() + 1 == path.len()).then_some((first.1, stub))
}

/// Replaces each stub of a kept link (the pair's net's route from a pad to
/// its stub's end, one layer, no branch) by a straight track where that
/// keeps every clearance (`refused_groups`).
fn straighten_stubs(
    board: &core::Board,
    result: &mut core::RoutingResult,
    pairs: &[Pair],
    plan_link: &LinkPlan,
) {
    let pair = &pairs[plan_link.pair];
    for end in 0..2 {
        for (polarity, net) in [(0, pair.positive), (1, pair.negative)] {
            let (pad, stub_end) = (plan_link.pads[end][polarity], plan_link.ends[end][polarity]);
            let Some((layer, stub)) = stub_path(
                &result.routes[net as usize],
                pad,
                plan_link.stub_end(end, polarity, pair.width),
                pair.width,
            ) else {
                continue;
            };
            let route = &result.routes[net as usize];
            let first = (pad, layer);
            // The routed stub's narrowest width, at most the pair's (the
            // stubs' ends are placed for it).
            let width = stub
                .iter()
                .map(|&index| route.segments[index].width)
                .fold(pair.width, f64::min);
            // The stub out of the routes while the straight one is checked.
            let segments = &mut result.routes[net as usize].segments;
            let mut removed = Vec::new();
            let mut index = 0;
            segments.retain(|segment| {
                index += 1;
                let keep = !stub.contains(&(index - 1));
                if !keep {
                    removed.push(segment.clone());
                }
                keep
            });
            let mut straight: [Vec<(usize, Point, Point)>; 2] = Default::default();
            straight[polarity].push((first.1, pad, stub_end));
            let refused = refused_groups_at(
                board,
                &result.routes,
                pairs,
                &[Group {
                    pair: plan_link.pair,
                    segments: straight,
                    vias: Default::default(),
                }],
                width,
            );
            if debug() {
                eprintln!(
                    "diff pair stub {pad:?} -> {stub_end:?} of {}: {:?}",
                    board.nets[net as usize].name, refused[0]
                );
            }
            let segments = &mut result.routes[net as usize].segments;
            if refused[0].is_none() {
                segments.push(core::Segment {
                    layer: first.1,
                    start: pad,
                    end: stub_end,
                    width,
                });
            } else {
                segments.extend(removed);
            }
        }
    }
}

/// Routes the connections of the pairs `open` (indices into `pairs`) that
/// are still open on `board` (the board as lowered, the pairs' copper kept
/// on it) with every route of `result` kept, and adds what it routes to
/// `result` (the pairs' nets' status from it). Returns its work.
fn route_fallback(
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
    let mut fallback = board.clone();
    for (net, route) in result.routes.iter().enumerate() {
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
                blocks_tracks: true,
                blocks_vias: true,
                label: "existing via hole".into(),
            });
        }
    }
    keep_existing_copper(&mut fallback, true);
    // Only the pairs' nets route (with their pours, if they have any).
    for (index, net) in fallback.nets.iter_mut().enumerate() {
        if !nets.contains(&(index as core::NetId)) {
            net.terminals.clear();
        }
    }
    fallback.planes.retain(|plane| nets.contains(&plane.net));
    let mut router_config = core_config(config);
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
    }
    routed.expansions as f64 / core::router::expansions_per_second()
        + routed.build_work
        + routed.pour_work
}

/// Copper checked together (`refused_groups`): a link's run and vias.
#[derive(Clone)]
struct Group {
    pair: usize,
    segments: [Vec<(usize, Point, Point)>; 2],
    vias: [Vec<Point>; 2],
}

/// Per group: why its copper is refused (`None`: kept). The copper of
/// every group is checked at once against the board (`verify`: its
/// obstacles, the edge, each other) and the routes near it; a finding
/// between the two nets of one pair counts only below their gap (KiCad
/// lets them come that close: the class's diff pair rule, or the gap is no
/// less than the clearance).
fn refused_groups(
    board: &core::Board,
    routed: &[core::NetRoute],
    pairs: &[Pair],
    groups: &[Group],
) -> Vec<Option<String>> {
    refused_groups_at(board, routed, pairs, groups, 0.0)
}

/// `refused_groups` with the groups' tracks `width` wide (0: their pairs'
/// width).
fn refused_groups_at(
    board: &core::Board,
    routed: &[core::NetRoute],
    pairs: &[Pair],
    groups: &[Group],
    width: f64,
) -> Vec<Option<String>> {
    if groups.is_empty() {
        return Vec::new();
    }
    // The routes near the groups' copper.
    let mut bounds = core::geometry::Aabb {
        minimum: [f64::INFINITY; 2],
        maximum: [f64::NEG_INFINITY; 2],
    };
    let mut grow = |point: Point| {
        bounds.minimum = [
            bounds.minimum[0].min(point[0]),
            bounds.minimum[1].min(point[1]),
        ];
        bounds.maximum = [
            bounds.maximum[0].max(point[0]),
            bounds.maximum[1].max(point[1]),
        ];
    };
    for group in groups {
        for polarity in 0..2 {
            for &(_, start, end) in &group.segments[polarity] {
                grow(start);
                grow(end);
            }
            for &at in &group.vias[polarity] {
                grow(at);
            }
        }
    }
    let bounds = bounds.inflated(CHECK_REACH);
    let near = |a: Point, b: Point| {
        core::geometry::Aabb {
            minimum: [a[0].min(b[0]), a[1].min(b[1])],
            maximum: [a[0].max(b[0]), a[1].max(b[1])],
        }
        .intersects(bounds)
    };
    let mut routes = vec![core::NetRoute::default(); board.nets.len()];
    for (net, route) in routed.iter().enumerate().take(board.nets.len()) {
        routes[net].segments = route
            .segments
            .iter()
            .filter(|segment| near(segment.start, segment.end))
            .cloned()
            .collect();
        routes[net].vias = route
            .vias
            .iter()
            .filter(|via| near(via.at, via.at))
            .cloned()
            .collect();
    }
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
                    width: if width > 0.0 { width } else { pair.width },
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
    // A finding at a via names its net and where it is: the group of the
    // nearest of the groups' vias of the net, if one is that close.
    let via_owner = |net: core::NetId, at: Point| -> Option<usize> {
        vias_of.get(&net).and_then(|vias| {
            vias.iter()
                .min_by(|a, b| {
                    core::geometry::distance(a.0, at).total_cmp(&core::geometry::distance(b.0, at))
                })
                .filter(|(via, _)| core::geometry::distance(*via, at) < 1.0)
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
        let other = violation
            .other_segment
            .and_then(|key| owner_of_segment.get(&key).copied());
        if own.is_none() && other.is_none() {
            // Between routes the router made: not the pairs'.
            continue;
        }
        let other_net = violation.other_segment.map(|(net, _)| net).or_else(|| {
            board
                .nets
                .iter()
                .position(|net| violation.other == format!("via of {}", net.name))
                .map(|net| net as core::NetId)
        });
        if let (Some(other_net), Some(&(mate, gap))) = (other_net, partner.get(&violation.net))
            && other_net == mate
            && !violation.other.contains("hole")
            && violation.actual + 1.0e-6 >= gap.min(violation.required)
        {
            continue;
        }
        let why = format!(
            "its copper is {:.3} mm from {} (needs {:.3})",
            violation.actual.max(0.0),
            violation.other,
            violation.required
        );
        if debug() {
            eprintln!(
                "diff pair copper refused at {:?} layer {} (net {}, segment {:?}): {why}",
                violation.at,
                violation.layer,
                board.nets[violation.net as usize].name,
                violation.segment
            );
        }
        // A finding between two groups refuses both.
        for position in own.into_iter().chain(other) {
            refused[position].get_or_insert_with(|| why.clone());
        }
    }
    refused
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

/// The settings a route with differential pairs uses: a pair's nets at the
/// pair's width where a custom rule sets it (KiCad checks it on all their
/// tracks: the stubs, a pair that falls back; the net class's diff pair
/// width only the coupled runs take), and the project's diff pair rules
/// the pairs take reported `applied` (custom rules on `inDiffPair()` and
/// the `diff_pair_gap` and `diff_pair_uncoupled` constraints).
pub(crate) fn with_pairs(config: &KiCadBoardRouterConfig) -> KiCadBoardRouterConfig {
    let mut copy = config.clone();
    let mut used: BTreeSet<String> = BTreeSet::new();
    for (net, rule) in &config.diff_pair_rules {
        let first = (0..rule.layers.len())
            .find(|layer| !rule.disallowed.get(*layer).copied().unwrap_or(false));
        if let (Some(rules), Some((width, _)), true) = (
            copy.connection_rules.get_mut(net),
            first.and_then(|layer| rule.layers.get(layer)),
            first.is_some_and(|layer| rule.width_ruled.get(layer).copied().unwrap_or(false)),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A USB pair from J1 (two rows: two pair points of one part) to U2,
    /// past U1, whose D+ and D- pads have a GND pin between them.
    const BOARD: &str = r#"(kicad_pcb (version 20241229) (generator "pcbnew")
  (general (thickness 1.6)) (paper "A4")
  (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
  (gr_rect (start 0 0) (end 40 20) (stroke (width 0.05) (type default)) (layer "Edge.Cuts"))
  (footprint "test" (layer "F.Cu") (at 3 10)
    (property "Reference" "J1" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd rect (at 0 0) (size 1 0.3) (layers "F.Cu") (net "/D+"))
    (pad "2" smd rect (at 0 0.5) (size 1 0.3) (layers "F.Cu") (net "/D-"))
    (pad "3" smd rect (at 0 2) (size 1 0.3) (layers "F.Cu") (net "/D+"))
    (pad "4" smd rect (at 0 2.5) (size 1 0.3) (layers "F.Cu") (net "/D-")))
  (footprint "test" (layer "F.Cu") (at 18 10)
    (property "Reference" "U1" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd rect (at 0 0) (size 1 0.3) (layers "F.Cu") (net "/D+"))
    (pad "2" smd rect (at 0 0.5) (size 1 0.3) (layers "F.Cu") (net "/GND"))
    (pad "3" smd rect (at 0 1) (size 1 0.3) (layers "F.Cu") (net "/D-")))
  (footprint "test" (layer "F.Cu") (at 34 10)
    (property "Reference" "U2" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd rect (at 0 0) (size 1 0.3) (layers "F.Cu") (net "/D+"))
    (pad "2" smd rect (at 0 0.5) (size 1 0.3) (layers "F.Cu") (net "/D-")))
  (footprint "test" (layer "F.Cu") (at 18 4)
    (property "Reference" "J5" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "/GND"))))
"#;

    fn lowered() -> (core::Board, KiCadBoardRouterConfig) {
        let pcb = parse(BOARD).unwrap();
        let config = KiCadBoardRouterConfig {
            default_rules: Some(KiCadConnectionRoutingRules {
                trace_width_mm: 0.2,
                clearance_mm: 0.2,
                via_size_mm: 0.6,
                via_drill_mm: 0.3,
            }),
            ..KiCadBoardRouterConfig::default()
        };
        let board = lower(&pcb, &config, true).unwrap().board;
        (board, config)
    }

    fn terminals(board: &core::Board, name: &str) -> Vec<String> {
        let net = board
            .nets
            .iter()
            .find(|net| net.name.trim_start_matches('/') == name)
            .unwrap();
        net.terminals
            .iter()
            .map(|terminal| terminal.label.clone())
            .collect()
    }

    #[test]
    fn pair_points_are_pads_side_by_side_and_links_join_parts() {
        let (board, config) = lowered();
        let pairs = find_pairs(&board, &config);
        assert_eq!(pairs.len(), 1);
        let pair = &pairs[0];
        let p = &board.nets[pair.positive as usize];
        let labels: Vec<String> = pair
            .points
            .iter()
            .map(|&(a, _)| p.terminals[a].label.clone())
            .collect();
        // U1's pads have its GND pin between them: no pair point there.
        assert_eq!(labels, ["J1.1", "J1.3", "U2.1"]);
        // J1's two rows are joined by ordinary routing: one link, J1 to U2.
        assert_eq!(pair.links.len(), 1);
        let (a, b) = pair.links[0];
        let ends = [labels[a].as_str(), labels[b].as_str()];
        assert!(ends.contains(&"U2.1"), "{ends:?}");
    }

    #[test]
    fn a_link_is_a_net_with_launches_and_the_pairs_nets_end_beside_its_run() {
        let (mut board, config) = lowered();
        let (nets, obstacles) = (board.nets.len(), board.obstacles.len());
        plan_pairs(&mut board, &config);
        // One link: a net of its own at the envelope, kept to no layer.
        assert_eq!(board.nets.len(), nets + 1);
        let link = &board.nets[nets];
        let class = board.classes[link.class];
        assert!((class.trace_width - 0.6).abs() < 1.0e-9, "{class:?}");
        assert_eq!(class.layers, 0);
        assert_eq!(link.terminals.len(), 2);
        // The pair's nets: their four pads, and the run's two ends as one
        // terminal (the second a merged pad the route may end on).
        for name in ["D+", "D-"] {
            let labels = terminals(&board, name);
            assert_eq!(
                labels
                    .iter()
                    .filter(|label| label.starts_with("pair run"))
                    .count(),
                1,
                "{labels:?}"
            );
            assert_eq!(labels.len(), 5, "{labels:?}");
        }
        assert!(board.obstacles.len() > obstacles);
        // The launches lie outside the pads, the run's ends beside them.
        let plan = board
            .attachment
            .0
            .clone()
            .unwrap()
            .downcast::<PairPlan>()
            .unwrap();
        let link_plan = &plan.links[0];
        for end in 0..2 {
            let [p, n] = link_plan.ends[end];
            let apart = core::geometry::distance(p, n);
            assert!(apart >= 0.2 + 0.2 - 1.0e-9, "the ends {apart} apart");
            let pads = link_plan.pads[end];
            for (end_point, pad) in [(p, pads[0]), (n, pads[1])] {
                assert!(core::geometry::distance(end_point, pad) > 0.5);
            }
        }
    }
}
