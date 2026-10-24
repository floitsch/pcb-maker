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
//! stubs join to the pads: the link's own where they keep every clearance
//! (`stubs_clear`; the pads are then of the run's terminal), else the
//! router's (the pair's nets route to the run's ends). The pair's own nets lose the connections the
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
    /// Whether the pairs routed coupled (`diff_pairs`); off, the counts
    /// below are 0 (no pair was planned).
    #[serde(default = "coupled_by_default")]
    pub on: bool,
    /// Why: "explicit" (the settings' `diff_pairs` or
    /// `PCB_ROUTER_DIFF_PAIRS`), "rules" (the project's custom rules ask
    /// for pairs: route mode's default by need), "off" (nothing asks).
    #[serde(default)]
    pub default: String,
    /// The setting, or the rule that asks and the net it holds for, or why
    /// nothing asks.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
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
    /// router routes the stubs there, crossing with a via).
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

/// Reports from before the switch's default by need were made with the
/// switch on.
fn coupled_by_default() -> bool {
    true
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
    // Per part (its reference): its pads' centres.
    let mut parts: BTreeMap<String, Vec<Point>> = BTreeMap::new();
    for obstacle in &board.obstacles {
        if obstacle.kind == core::ObstacleKind::Copper
            && let Some((part, pad)) = obstacle.label.rsplit_once('.')
            && !pad.contains(' ')
            && !part.contains(' ')
        {
            let bounds = obstacle.shape.aabb();
            parts
                .entry(part.to_string())
                .or_default()
                .push(scale(add(bounds.minimum, bounds.maximum), 0.5));
        }
    }
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
        // Or a pad of each of two two-pad parts side by side (the pair's
        // series or AC-coupling capacitors, resistors, ESD diodes: each
        // part's other pad beside the other's), after those of one part:
        // the pair is coupled up to the parts and on from them.
        let mut candidates: Vec<(bool, f64, usize, usize)> = Vec::new();
        for (a, ta) in p.terminals.iter().enumerate() {
            for (b, tb) in n.terminals.iter().enumerate() {
                let span = core::geometry::distance(ta.anchor, tb.anchor);
                let (part_a, part_b) = (reference(&ta.label), reference(&tb.label));
                let series = part_a != part_b
                    && series_parts(&parts, &part_a, &part_b, ta.anchor, tb.anchor);
                if span <= PAIR_POINT_SPAN
                    && span > EPSILON
                    && (part_a == part_b || series)
                    && ta.layers & tb.layers != 0
                    && !straddles(board, ta, tb, [positive, negative])
                {
                    candidates.push((series, span, a, b));
                }
            }
        }
        candidates.sort_by(|x, y| {
            x.0.cmp(&y.0)
                .then(x.1.total_cmp(&y.1))
                .then(x.2.cmp(&y.2))
                .then(x.3.cmp(&y.3))
        });
        let (mut used_p, mut used_n) = (BTreeSet::new(), BTreeSet::new());
        let mut points = Vec::new();
        for (_, _, a, b) in candidates {
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
        let part = |point: &(usize, usize)| {
            let (part_p, part_n) = (
                reference(&p.terminals[point.0].label),
                reference(&n.terminals[point.1].label),
            );
            if part_p == part_n {
                part_p
            } else {
                format!("{part_p}+{part_n}")
            }
        };
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

/// Whether parts `a` and `b` (references) are two-pad parts side by side:
/// their pads at `at_a` and `at_b`, and their other pads as close
/// (`PAIR_POINT_SPAN`: a pair's series capacitors, not two parts that
/// merely touch at one end).
fn series_parts(
    parts: &BTreeMap<String, Vec<Point>>,
    a: &str,
    b: &str,
    at_a: Point,
    at_b: Point,
) -> bool {
    let other = |pads: Option<&Vec<Point>>, at: Point| -> Option<Point> {
        let pads = pads?;
        (pads.len() == 2).then_some(())?;
        let far = if core::geometry::distance(pads[0], at) > core::geometry::distance(pads[1], at)
        {
            pads[0]
        } else {
            pads[1]
        };
        // The pad given is one of the two.
        (pads
            .iter()
            .map(|pad| core::geometry::distance(*pad, at))
            .fold(f64::INFINITY, f64::min)
            < 0.05)
            .then_some(far)
    };
    match (other(parts.get(a), at_a), other(parts.get(b), at_b)) {
        (Some(far_a), Some(far_b)) => core::geometry::distance(far_a, far_b) <= PAIR_POINT_SPAN,
        _ => false,
    }
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
    /// Whether straight stubs from the pads to the stubs' ends keep every
    /// clearance (`stubs_clear`): the link carries them, the router does
    /// not route them.
    fixed: bool,
    /// Where those stubs start on the pads (positive, negative).
    stubs: [Point; 2],
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

/// Where a straight stub of `width` from the pad of `terminal` to `end`
/// starts: where the line from the pad's anchor to the end leaves the pad,
/// less half the width (its end within the pad), so that only what lies
/// beyond the pad is the stub's (a card edge's finger reaches under the
/// edge's mask opening; the stub leaves its inner end).
fn stub_start(board: &core::Board, terminal: &core::Terminal, end: Point, width: f64) -> Point {
    let shape = &board.obstacles[terminal.pad].shape;
    let (anchor, length) = (terminal.anchor, core::geometry::distance(terminal.anchor, end));
    if length < EPSILON || !shape.contains(anchor) {
        return anchor;
    }
    let direction = unit(sub(end, anchor));
    // The exit, to 0.01 mm.
    let (mut inside, mut outside) = (0.0, length);
    if shape.contains(end) {
        return anchor;
    }
    while outside - inside > 0.01 {
        let middle = (inside + outside) / 2.0;
        if shape.contains(add(anchor, scale(direction, middle))) {
            inside = middle;
        } else {
            outside = middle;
        }
    }
    add(anchor, scale(direction, (inside - width / 2.0).max(0.0)))
}

/// Whether straight stubs of the pair's width from the pads (`pads`:
/// positive, negative) to their stubs' ends (`ends`) on `layer` keep their
/// clearance from the obstacles `near` (another net's copper, keepouts,
/// holes; the pair's other net its gap), from the board's edge and from
/// each other: then the link carries them (`LinkPlan::fixed`).
fn stubs_clear(
    board: &core::Board,
    near: &[usize],
    pair: &Pair,
    pads: [Point; 2],
    ends: [Point; 2],
    layer: core::LayerMask,
) -> bool {
    let half = pair.width / 2.0;
    if core::geometry::segment_segment_distance(pads[0], ends[0], pads[1], ends[1])
        < pair.width + pair.gap.min(pair.clearance) - EPSILON
    {
        return false;
    }
    let nets = [pair.positive, pair.negative];
    (0..2).all(|polarity| {
        let (pad, end) = (pads[polarity], ends[polarity]);
        let (own, other) = (nets[polarity], nets[1 - polarity]);
        core::geometry::polygon_edges(&board.outline).all(|(a, b)| {
            core::geometry::segment_segment_distance(pad, end, a, b)
                >= half + board.edge_clearance
        }) && near.iter().all(|&index| {
            let obstacle = &board.obstacles[index];
            if obstacle.layers & layer == 0 || obstacle.net == Some(own) {
                return true;
            }
            // A pad inside a mask opening leaves it by its stub (the
            // router's `stub_is_clear`, `verify`).
            if obstacle.kind == core::ObstacleKind::Keepout
                && obstacle.label.starts_with("solder mask opening")
                && obstacle.shape.contains(pad)
            {
                return true;
            }
            let mut keep = keep_from(board, obstacle, pair.clearance);
            if obstacle.net == Some(other) {
                keep = keep.min(pair.gap);
            }
            obstacle.shape.distance_to_segment(pad, end) >= half + keep - EPSILON
        })
    })
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
    want_fixed: bool,
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
    // The best launch, and the best whose stubs the link carries
    // (`stubs_clear`), which wins where there is one.
    let mut best: Option<(f64, Launch)> = None;
    let mut best_fixed: Option<(f64, Launch)> = None;
    for angle in LAUNCH_ANGLES {
        let outward = turned(normal, angle);
        let side = if dot(left(outward), sub(p.anchor, n.anchor)) >= 0.0 {
            1.0
        } else {
            -1.0
        };
        let clear_at = |step: usize| {
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
            if !(end_clear(board, &near, pair, lead_of(ends[0]), layers)
                && end_clear(board, &near, pair, lead_of(ends[1]), layers))
            {
                return None;
            }
            // Straight stubs the link carries, and the envelope clear along
            // the whole lead (the link's copper from the run's start: no
            // stub of the router's needs to reach the ends).
            let stubs = [
                stub_start(board, p, ends[0], pair.width),
                stub_start(board, n, ends[1], pair.width),
            ];
            let fixed = want_fixed
                && stubs_clear(board, &near, pair, stubs, ends, layers)
                && (0..=(reserve / LAUNCH_STEP).ceil() as usize).all(|sample| {
                    let point = add(start, scale(outward, sample as f64 * LAUNCH_STEP));
                    clear_layers(board, &near, point, half, class.clearance, layers) != 0
                });
            Some(Launch {
                at,
                start,
                reserved,
                ends: scale(outward, ends_reach),
                outward,
                layers,
                side,
                fixed,
                stubs,
            })
        };
        let cost_of = |found: &Launch| {
            core::geometry::distance(middle, found.start) + ANGLE_COST * angle.abs() / 90.0
        };
        // The first clear start along the direction, then the first whose
        // stubs the link carries.
        let mut step = 0;
        while step <= steps {
            let Some(found) = clear_at(step) else {
                step += 1;
                continue;
            };
            let cost = cost_of(&found);
            if best.is_none_or(|(known, _)| cost < known - 1.0e-9) {
                best = Some((cost, found));
            }
            if !want_fixed || best_fixed.is_some_and(|(known, _)| known <= cost) {
                break;
            }
            if found.fixed {
                best_fixed = Some((cost, found));
                break;
            }
            step += 1;
        }
    }
    let best = best_fixed.or(best);
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
    /// Whether the pads' order at one end crosses the run's (the stubs
    /// cross there, routed by the router).
    crossed: bool,
    /// Per end: whether the link carries the stubs there, straight from
    /// the pads to the stubs' ends (`stubs_clear`; the pads are of the
    /// run's terminal), or the router routes them.
    fixed: [bool; 2],
    /// Per end and polarity: where those stubs start on the pads.
    stubs: [[Point; 2]; 2],
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
            let launch_at = |end: usize, want_fixed: bool| {
                launch(
                    board,
                    pair,
                    &envelope,
                    &ends[end].0,
                    &ends[end].1,
                    middle(1 - end),
                    want_fixed,
                )
            };
            // Launches whose stubs the link carries where there are such,
            // else (or where those leave no run) the closest.
            let mut launches = [launch_at(0, true), launch_at(1, true)];
            let too_close = |launches: &[Option<Launch>; 2]| match launches {
                [Some(a), Some(b)] => core::geometry::distance(a.at, b.at) < MINIMUM_RUN,
                _ => false,
            };
            if too_close(&launches) && launches.iter().flatten().any(|found| found.fixed) {
                launches = [launch_at(0, false), launch_at(1, false)];
            }
            let [Some(mut launch_a), Some(mut launch_b)] = launches else {
                entries[index]
                    .why
                    .push(format!("link {}: no launch clear of the pads", link + 1));
                continue;
            };
            if too_close(&launches) {
                entries[index].why.push(format!(
                    "link {}: the launches are too close for a coupled run",
                    link + 1
                ));
                continue;
            }
            let id = board.nets.len() as core::NetId;
            // Along the run from the first end the positive net keeps the
            // side its pad has there; at the second end the run arrives
            // against that launch's outward direction, so the pads' order
            // matches the run's where the positive pad is on the other side
            // of that direction. Where it is not (crossed), the stubs cross
            // at one end, which the router routes (with a via): the second
            // end, unless only the second end's stubs are straight.
            let crossed = launch_b.side == launch_a.side;
            let side = if crossed && !launch_a.fixed && launch_b.fixed {
                -launch_b.side
            } else {
                launch_a.side
            };
            if crossed {
                // The crossed end's stubs are routed: its closest launch.
                if side == launch_a.side {
                    launch_b = launch_at(1, false).unwrap_or(launch_b);
                    launch_b.fixed = false;
                } else {
                    launch_a = launch_at(0, false).unwrap_or(launch_a);
                    launch_a.fixed = false;
                }
                if core::geometry::distance(launch_a.at, launch_b.at) < MINIMUM_RUN {
                    entries[index].why.push(format!(
                        "link {}: the launches are too close for a coupled run",
                        link + 1
                    ));
                    continue;
                }
            }
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
                // Where the link carries the stubs its copper begins at the
                // run's start, so that no other net comes between the run's
                // two tracks' beginnings and their clearance. It is the
                // terminal's pad ("pair launch": the router does not neck the
                // link down near it, `Router::prepare_net`).
                board.obstacles.push(core::Obstacle {
                    shape: core::Shape::Capsule {
                        start: if found.fixed {
                            found.start
                        } else {
                            found.reserved
                        },
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
                // Straight stubs the link carries: copper of the pair's nets
                // (other nets keep clear of it); the pads are of one
                // terminal with the run's ends.
                if found.fixed {
                    for (polarity, net) in [pair.positive, pair.negative].into_iter().enumerate() {
                        board.obstacles.push(core::Obstacle {
                            shape: core::Shape::Capsule {
                                start: found.stubs[polarity],
                                end: stub_ends[end][polarity],
                                radius: pair.width / 2.0,
                            },
                            layers: found.layers,
                            kind: core::ObstacleKind::Copper,
                            net: Some(net),
                            clearance: 0.0,
                            clearance_override: None,
                            blocks_tracks: true,
                            blocks_vias: true,
                            label: format!("pair stub {}{}", link + 1, ["a", "b"][end]),
                        });
                    }
                }
            }
            let fixed = [launch_a.fixed, launch_b.fixed];
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
            for (polarity, net) in [pair.positive, pair.negative].into_iter().enumerate() {
                let count = board.nets[net as usize].terminals.len();
                let parent = parents.entry(net).or_default();
                parent.extend(parent.len()..count);
                // The run's two ends, the last two pairs of terminals, and
                // the pads whose stubs the link carries.
                let pad_of = |point: (usize, usize)| [point.0, point.1][polarity];
                let mut joined = vec![count - 2, count - 1];
                for (end, point) in [point_a, point_b].into_iter().enumerate() {
                    if fixed[end] {
                        joined.push(pad_of(point));
                    }
                }
                for item in &joined[1..] {
                    let (x, y) = (root(parent, joined[0]), root(parent, *item));
                    if x != y {
                        parent[x.max(y)] = x.min(y);
                    }
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
                fixed,
                stubs: [launch_a.stubs, launch_b.stubs],
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
            if debug() {
                eprintln!(
                    "diff pair link {} ({}) not routed: {:?}, launches {:?}",
                    plan_link.net,
                    board.nets[plan_link.net as usize].name,
                    link_status.get(offset),
                    plan_link.launches
                );
            }
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
                    // The stubs the link carries, from the pads.
                    if plan_link.fixed[0]
                        && let Some(&(layer, _, _)) = segments.first()
                    {
                        segments.insert(0, (layer, plan_link.stubs[0][polarity], from));
                    }
                    if plan_link.fixed[1]
                        && let Some(&(layer, _, _)) = segments.last()
                    {
                        segments.push((layer, to, plan_link.stubs[1][polarity]));
                    }
                }
                copper
            })
        });
        // Each pad's stub must reach its end of the run (else the run is an
        // island and the pad open): the link falls back.
        let reach = |net: core::NetId| {
            (pair.width
                + result.routes[net as usize]
                    .segments
                    .iter()
                    .map(|segment| segment.width)
                    .fold(0.0, f64::max))
                / 2.0
        };
        let stubs_joined = (0..2).filter(|&end| !plan_link.fixed[end]).all(|end| {
            [(0, pair.positive), (1, pair.negative)]
                .into_iter()
                .all(|(polarity, net)| {
                    stub_joined(
                        &restored,
                        &result.routes[net as usize],
                        net,
                        plan_link.pads[end][polarity],
                        plan_link.stub_end(end, polarity, pair.width),
                        reach(net),
                    )
                })
        });
        if copper.is_some() && !stubs_joined {
            entries[index].why.push(format!(
                "link {}: a pad's stub does not reach the run",
                link + 1
            ));
            open_pairs.insert(index);
            continue;
        }
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
        for end in (0..2).filter(|&end| !plan_link.fixed[end]) {
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
        join_stub_ends(result, &plan.pairs, plan_link);
    }
    // The fallback: the pairs' nets with a connection a link left open,
    // routed with everything else kept.
    if !open_pairs.is_empty() {
        report.work_seconds = route_fallback(&restored, result, &plan.pairs, &open_pairs, config);
        // What is left of the failed links' stubs that joins nothing.
        for plan_link in plan
            .links
            .iter()
            .filter(|link| !kept_nets.contains(&link.net))
        {
            let pair = &plan.pairs[plan_link.pair];
            for end in (0..2).filter(|&end| !plan_link.fixed[end]) {
                for (polarity, net) in [(0, pair.positive), (1, pair.negative)] {
                    let route = &mut result.routes[net as usize];
                    let reach = (pair.width
                        + route
                            .segments
                            .iter()
                            .map(|segment| segment.width)
                            .fold(0.0, f64::max))
                        / 2.0;
                    prune_stub(
                        &restored,
                        route,
                        net,
                        plan_link.stub_end(end, polarity, pair.width),
                        reach,
                    );
                }
            }
        }
        // Then any track or via of a fallen-back pair's nets that joins
        // nothing at one end (a stub's piece the prune from its end did not
        // reach: one that ended on the other end's copper, a pad exit left
        // when the stub went; KiCad's track_dangling and via_dangling).
        for &index in &open_pairs {
            let pair = &plan.pairs[index];
            for net in [pair.positive, pair.negative] {
                prune_dangling(&restored, &mut result.routes[net as usize], net);
            }
        }
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

/// Whether `net`'s route joins the pad at `pad` (its copper on the board)
/// to a stub's end (its copper `lead`, `reach` around it): the copper the
/// route's tracks and vias overlap, followed from the pad (as the router
/// joins copper). A link whose stubs do not reach both its ends would
/// leave its run an island (the router may end a pad's route on the other
/// end of the link, merged into one terminal, or leave it open).
fn stub_joined(
    board: &core::Board,
    route: &core::NetRoute,
    net: core::NetId,
    pad: Point,
    lead: (Point, Point),
    reach: f64,
) -> bool {
    let pads: Vec<&core::Obstacle> = board
        .obstacles
        .iter()
        .filter(|obstacle| {
            obstacle.net == Some(net)
                && obstacle.kind == core::ObstacleKind::Copper
                && obstacle.shape.distance_to_point(pad) <= 1.0e-6
        })
        .collect();
    let segments = &route.segments;
    let count = segments.len();
    // Items: the segments, then the vias.
    let touches_pad = |item: usize| {
        if item < count {
            let segment = &segments[item];
            pads.iter().any(|obstacle| {
                obstacle.layers & (1 << segment.layer) != 0
                    && [segment.start, segment.end].into_iter().any(|end| {
                        obstacle.shape.distance_to_point(end) <= segment.width / 2.0 + 1.0e-6
                    })
            })
        } else {
            let via = &route.vias[item - count];
            pads.iter().any(|obstacle| {
                obstacle.shape.distance_to_point(via.at) <= via.diameter / 2.0 + 1.0e-6
            })
        }
    };
    let joined = |a: usize, b: usize| match (a < count, b < count) {
        (true, true) => {
            let (x, y) = (&segments[a], &segments[b]);
            let keep = (x.width + y.width) / 2.0 + 1.0e-6;
            x.layer == y.layer
                && ([x.start, x.end]
                    .into_iter()
                    .any(|end| core::geometry::point_segment_distance(end, y.start, y.end) <= keep)
                    || [y.start, y.end].into_iter().any(|end| {
                        core::geometry::point_segment_distance(end, x.start, x.end) <= keep
                    }))
        }
        (true, false) | (false, true) => {
            let (segment, via) = if a < count {
                (&segments[a], &route.vias[b - count])
            } else {
                (&segments[b], &route.vias[a - count])
            };
            core::geometry::point_segment_distance(via.at, segment.start, segment.end)
                <= (segment.width + via.diameter) / 2.0 + 1.0e-6
        }
        (false, false) => false,
    };
    let at_end = |item: usize| {
        item < count
            && [segments[item].start, segments[item].end]
                .into_iter()
                .any(|end| {
                    core::geometry::point_segment_distance(end, lead.0, lead.1) <= reach + 1.0e-6
                })
    };
    let total = count + route.vias.len();
    let mut seen = vec![false; total];
    let mut queue: Vec<usize> = (0..total).filter(|&item| touches_pad(item)).collect();
    for &item in &queue {
        seen[item] = true;
    }
    while let Some(item) = queue.pop() {
        if at_end(item) {
            return true;
        }
        for (other, known) in seen.iter_mut().enumerate() {
            if !*known && joined(item, other) {
                *known = true;
                queue.push(other);
            }
        }
    }
    if debug() {
        let nearest = segments
            .iter()
            .flat_map(|segment| [segment.start, segment.end])
            .map(|end| core::geometry::point_segment_distance(end, lead.0, lead.1))
            .fold(f64::INFINITY, f64::min);
        eprintln!(
            "diff pair stub of {} from {pad:?} does not reach {lead:?} (pads {}, from the pad {} of {} items, nearest track end {nearest:.3} mm, reach {reach:.3})",
            board.nets[net as usize].name,
            pads.len(),
            seen.iter().filter(|seen| **seen).count(),
            total
        );
    }
    false
}

/// A kept link's stub that ends elsewhere on its end's copper (the router
/// may end it anywhere there) gets a track to the end itself, where the
/// run's jog starts: else the jog's end touches only the stub's copper,
/// not its end, and KiCad finds it dangling.
fn join_stub_ends(result: &mut core::RoutingResult, pairs: &[Pair], plan_link: &LinkPlan) {
    let pair = &pairs[plan_link.pair];
    for end in (0..2).filter(|&end| !plan_link.fixed[end]) {
        for (polarity, net) in [(0, pair.positive), (1, pair.negative)] {
            let (at, lead_end) = plan_link.stub_end(end, polarity, pair.width);
            let route = &mut result.routes[net as usize];
            let reach = (pair.width
                + route
                    .segments
                    .iter()
                    .map(|segment| segment.width)
                    .fold(0.0, f64::max))
                / 2.0;
            let ends: Vec<(f64, Point, usize, f64)> = route
                .segments
                .iter()
                .flat_map(|segment| {
                    [segment.start, segment.end].map(|point| (point, segment.layer, segment.width))
                })
                .filter(|(point, _, _)| {
                    core::geometry::point_segment_distance(*point, at, lead_end) <= reach + 1.0e-6
                })
                .map(|(point, layer, width)| {
                    (core::geometry::distance(point, at), point, layer, width)
                })
                .collect();
            if ends.iter().any(|(distance, ..)| *distance < 1.0e-6) {
                continue;
            }
            if let Some(&(_, point, layer, width)) = ends.iter().min_by(|a, b| a.0.total_cmp(&b.0))
            {
                route.segments.push(core::Segment {
                    layer,
                    start: point,
                    end: at,
                    width: width.min(pair.width),
                });
            }
        }
    }
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
    for end in (0..2).filter(|&end| !plan_link.fixed[end]) {
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

/// Removes what is left of a failed link's stub on `net`'s route: from
/// the stub's end (its copper `lead`, `reach` around it) along the tracks
/// whose end there touches nothing else of the net (no other track, via,
/// pad or kept copper), up to the first junction (KiCad's
/// `track_dangling`, an error in some projects).
fn prune_stub(
    board: &core::Board,
    route: &mut core::NetRoute,
    net: core::NetId,
    lead: (Point, Point),
    reach: f64,
) {
    let own: Vec<&core::Obstacle> = board
        .obstacles
        .iter()
        .filter(|obstacle| obstacle.net == Some(net) && obstacle.kind == core::ObstacleKind::Copper)
        .collect();
    // Whether a track end of `width` at `point` overlaps other copper of
    // the net (as the router joins copper: `joins_copper`).
    // The tracks that meet the track's other end do not count (KiCad finds
    // a short track's end dangling though its copper overlaps the next).
    let touches = |route: &core::NetRoute, index: usize, point: Point, layer: usize| {
        let half = route
            .segments
            .get(index)
            .map_or(0.0, |segment| segment.width / 2.0);
        let far = route.segments.get(index).map(|segment| {
            if core::geometry::distance(segment.start, point) < 1.0e-6 {
                segment.end
            } else {
                segment.start
            }
        });
        let meets_far = |segment: &core::Segment| {
            far.is_some_and(|far| {
                core::geometry::distance(segment.start, far) < 1.0e-6
                    || core::geometry::distance(segment.end, far) < 1.0e-6
            })
        };
        route.segments.iter().enumerate().any(|(other, segment)| {
            other != index
                && !meets_far(segment)
                && segment.layer == layer
                && core::geometry::point_segment_distance(point, segment.start, segment.end)
                    <= segment.width / 2.0 + half + 1.0e-6
        }) || route.vias.iter().any(|via| {
            core::geometry::distance(via.at, point) <= via.diameter / 2.0 + half + 1.0e-6
        }) || own.iter().any(|obstacle| {
            obstacle.layers & (1 << layer) != 0
                && obstacle.shape.distance_to_point(point) <= half + 1.0e-6
        })
    };
    // The dangling end at the stub's end.
    let mut at: Option<(Point, usize)> =
        route
            .segments
            .iter()
            .enumerate()
            .find_map(|(index, segment)| {
                [segment.start, segment.end]
                    .into_iter()
                    .find(|&end| {
                        core::geometry::point_segment_distance(end, lead.0, lead.1)
                            <= reach + 1.0e-6
                            && !touches(route, index, end, segment.layer)
                    })
                    .map(|end| (end, segment.layer))
            });
    while let Some((point, layer)) = at.take() {
        let Some(index) = route.segments.iter().position(|segment| {
            segment.layer == layer
                && (core::geometry::distance(segment.start, point) < 1.0e-6
                    || core::geometry::distance(segment.end, point) < 1.0e-6)
        }) else {
            break;
        };
        if touches(route, index, point, layer) {
            break;
        }
        let segment = route.segments.remove(index);
        let other = if core::geometry::distance(segment.start, point) < 1.0e-6 {
            segment.end
        } else {
            segment.start
        };
        let alone = !route.segments.iter().any(|candidate| {
            candidate.layer == layer
                && core::geometry::point_segment_distance(other, candidate.start, candidate.end)
                    <= (candidate.width + segment.width) / 2.0 + 1.0e-6
        });
        if alone {
            // A via there that joins nothing else of the net but one track
            // on another layer goes too, and the chain goes on along that
            // track (a stub that changed layers: KiCad's via_dangling).
            let Some(via) = route
                .vias
                .iter()
                .position(|via| core::geometry::distance(via.at, other) < 1.0e-6)
            else {
                continue;
            };
            let (at_via, diameter) = (route.vias[via].at, route.vias[via].diameter);
            if own
                .iter()
                .any(|obstacle| obstacle.shape.distance_to_point(at_via) <= diameter / 2.0 + 1.0e-6)
                || route.vias.iter().enumerate().any(|(index, other_via)| {
                    index != via
                        && core::geometry::distance(other_via.at, at_via)
                            <= (other_via.diameter + diameter) / 2.0 + 1.0e-6
                })
            {
                continue;
            }
            let meeting: Vec<&core::Segment> = route
                .segments
                .iter()
                .filter(|candidate| {
                    core::geometry::point_segment_distance(at_via, candidate.start, candidate.end)
                        <= (candidate.width + diameter) / 2.0 + 1.0e-6
                })
                .collect();
            match meeting[..] {
                [] => {
                    route.vias.remove(via);
                }
                [next]
                    if core::geometry::distance(next.start, at_via) < 1.0e-6
                        || core::geometry::distance(next.end, at_via) < 1.0e-6 =>
                {
                    let next_layer = next.layer;
                    route.vias.remove(via);
                    at = Some((at_via, next_layer));
                }
                _ => {}
            }
            continue;
        }
        // The next track of the chain, if `other` is its end and touches
        // nothing but it.
        let next = route.segments.iter().enumerate().filter(|(_, candidate)| {
            candidate.layer == layer
                && core::geometry::point_segment_distance(other, candidate.start, candidate.end)
                    <= (candidate.width + segment.width) / 2.0 + 1.0e-6
        });
        let next: Vec<(usize, &core::Segment)> = next.collect();
        if let [(index, candidate)] = next[..]
            && (core::geometry::distance(candidate.start, other) < 1.0e-6
                || core::geometry::distance(candidate.end, other) < 1.0e-6)
            && !touches(route, index, other, layer)
        {
            at = Some((other, layer));
        }
    }
}

/// The pieces of copper `route` and `own` (a net's pads and kept copper)
/// make, as the router joins copper (overlap), without the segment
/// `without`.
fn pieces(route: &core::NetRoute, own: &[&core::Obstacle], without: Option<usize>) -> usize {
    let (o, n) = (own.len(), route.segments.len());
    let total = o + n + route.vias.len();
    let mut parent: Vec<usize> = (0..total).collect();
    let joined = |a: usize, b: usize| -> bool {
        let segment = |index: usize| &route.segments[index - o];
        let via = |index: usize| &route.vias[index - o - n];
        match (a < o, a >= o && a < o + n, b < o, b >= o && b < o + n) {
            (true, _, true, _) => false,
            (true, _, _, true) | (_, true, true, _) => {
                let (obstacle, track) = if a < o {
                    (own[a], segment(b))
                } else {
                    (own[b], segment(a))
                };
                obstacle.layers & (1 << track.layer) != 0
                    && obstacle.shape.distance_to_segment(track.start, track.end)
                        <= track.width / 2.0 + 1.0e-6
            }
            (true, _, _, false) | (_, false, true, _) => {
                let (obstacle, hole) = if a < o {
                    (own[a], via(b))
                } else {
                    (own[b], via(a))
                };
                obstacle.shape.distance_to_point(hole.at) <= hole.diameter / 2.0 + 1.0e-6
            }
            (_, true, _, true) => {
                let (x, y) = (segment(a), segment(b));
                x.layer == y.layer
                    && core::geometry::segment_segment_distance(x.start, x.end, y.start, y.end)
                        <= (x.width + y.width) / 2.0 + 1.0e-6
            }
            (_, true, _, false) | (_, false, _, true) => {
                let (track, hole) = if a < o + n {
                    (segment(a), via(b))
                } else {
                    (segment(b), via(a))
                };
                core::geometry::point_segment_distance(hole.at, track.start, track.end)
                    <= (track.width + hole.diameter) / 2.0 + 1.0e-6
            }
            (_, false, _, false) => false,
        }
    };
    let skip = without.map(|index| o + index);
    for a in 0..total {
        for b in a + 1..total {
            if Some(a) != skip && Some(b) != skip && joined(a, b) {
                let (x, y) = (root(&mut parent, a), root(&mut parent, b));
                if x != y {
                    parent[x.max(y)] = x.min(y);
                }
            }
        }
    }
    (0..total)
        .filter(|&item| Some(item) != skip && root(&mut parent, item) == item)
        .count()
}

/// Removes `net`'s tracks with an end that joins nothing else of the net
/// (no track, via, pad or kept copper its copper overlaps there, but the
/// tracks meeting its other end), and its vias that join no copper (a via
/// with copper on one layer stays: a zone, a designer's teardrop, may hold
/// it), until none is left; also a track no longer than its
/// width whose end lies in no other copper (KiCad finds such an end
/// dangling though it overlaps the next track) where removing it splits no
/// piece of the net's copper.
fn prune_dangling(board: &core::Board, route: &mut core::NetRoute, net: core::NetId) {
    let own: Vec<&core::Obstacle> = board
        .obstacles
        .iter()
        .filter(|obstacle| obstacle.net == Some(net) && obstacle.kind == core::ObstacleKind::Copper)
        .collect();
    let near = |a: Point, b: Point| core::geometry::distance(a, b) < 1.0e-6;
    // Whether the end `point` of segment `index` (its other end `far`) is
    // held: by overlap, or (`inside`) by lying in the other track's copper.
    let held = |route: &core::NetRoute, index: usize, point: Point, far: Point, inside: bool| {
        let segment = &route.segments[index];
        let half = segment.width / 2.0;
        route.segments.iter().enumerate().any(|(other, candidate)| {
            other != index
                && candidate.layer == segment.layer
                && !(near(candidate.start, far) || near(candidate.end, far))
                && core::geometry::point_segment_distance(point, candidate.start, candidate.end)
                    <= candidate.width / 2.0 + if inside { 0.0 } else { half } + 1.0e-6
        }) || route.vias.iter().any(|via| {
            core::geometry::distance(via.at, point) <= via.diameter / 2.0 + half + 1.0e-6
        }) || own.iter().any(|obstacle| {
            obstacle.layers & (1 << segment.layer) != 0
                && obstacle.shape.distance_to_point(point) <= half + 1.0e-6
        })
    };
    let ends =
        |segment: &core::Segment| [(segment.start, segment.end), (segment.end, segment.start)];
    loop {
        let dangling = (0..route.segments.len()).find(|&index| {
            ends(&route.segments[index])
                .into_iter()
                .any(|(point, far)| !held(route, index, point, far, false))
        });
        if let Some(index) = dangling {
            route.segments.remove(index);
            continue;
        }
        let nub = (0..route.segments.len()).find(|&index| {
            let segment = &route.segments[index];
            core::geometry::distance(segment.start, segment.end) <= segment.width
                && ends(segment)
                    .into_iter()
                    .any(|(point, far)| !held(route, index, point, far, true))
                && pieces(route, &own, Some(index)) <= pieces(route, &own, None)
        });
        if let Some(index) = nub {
            route.segments.remove(index);
            continue;
        }
        let lonely = route.vias.iter().position(|via| {
            let mut layers: core::LayerMask = 0;
            for segment in &route.segments {
                if core::geometry::point_segment_distance(via.at, segment.start, segment.end)
                    <= (segment.width + via.diameter) / 2.0 + 1.0e-6
                {
                    layers |= 1 << segment.layer;
                }
            }
            for obstacle in &own {
                if obstacle.shape.distance_to_point(via.at) <= via.diameter / 2.0 + 1.0e-6 {
                    layers |= obstacle.layers;
                }
            }
            layers == 0
        });
        match lonely {
            Some(index) => {
                route.vias.remove(index);
            }
            None => break,
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
    let mut router_config = core_config(config);
    // The other nets' planes go (their nets do not route here), but an
    // exclusive plane still keeps the pairs' tracks off its layer, as the
    // attempt kept every other net's (vias pass through it).
    if router_config.exclusive_planes {
        for plane in fallback
            .planes
            .iter()
            .filter(|plane| plane.exclusive && !nets.contains(&plane.net))
        {
            fallback.obstacles.push(core::Obstacle {
                shape: core::Shape::Polygon {
                    points: plane.polygon.clone(),
                },
                layers: 1 << plane.layer,
                kind: core::ObstacleKind::Keepout,
                net: None,
                clearance: 0.0,
                clearance_override: None,
                blocks_tracks: true,
                blocks_vias: false,
                label: "exclusive plane".into(),
            });
        }
    }
    fallback.planes.retain(|plane| nets.contains(&plane.net));
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

    /// Two two-pad parts side by side on the pair (its AC-coupling
    /// capacitors) are a pair point; two whose other pads are far apart
    /// are not.
    #[test]
    fn series_parts_side_by_side_are_a_pair_point() {
        let caps = BOARD.replace(
            r#"  (footprint "test" (layer "F.Cu") (at 18 4)"#,
            r#"  (footprint "test" (layer "F.Cu") (at 26 10)
    (property "Reference" "C1" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd rect (at -0.5 0) (size 0.5 0.5) (layers "F.Cu") (net "/D+"))
    (pad "2" smd rect (at 0.5 0) (size 0.5 0.5) (layers "F.Cu") (net "/X+")))
  (footprint "test" (layer "F.Cu") (at 26 11)
    (property "Reference" "C2" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd rect (at -0.5 0) (size 0.5 0.5) (layers "F.Cu") (net "/D-"))
    (pad "2" smd rect (at 0.5 0) (size 0.5 0.5) (layers "F.Cu") (net "/X-")))
  (footprint "test" (layer "F.Cu") (at 10 5)
    (property "Reference" "C3" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd rect (at -0.5 0) (size 0.5 0.5) (layers "F.Cu") (net "/D+"))
    (pad "2" smd rect (at 0.5 0) (size 0.5 0.5) (layers "F.Cu") (net "/Y")))
  (footprint "test" (layer "F.Cu") (at 9.5 6)
    (property "Reference" "C4" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd rect (at 0 0) (size 0.5 0.5) (layers "F.Cu") (net "/D-"))
    (pad "2" smd rect (at 0 3.5) (size 0.5 0.5) (layers "F.Cu") (net "/Z")))
  (footprint "test" (layer "F.Cu") (at 18 4)"#,
        );
        let pcb = parse(&caps).unwrap();
        let (_, config) = lowered();
        let board = lower(&pcb, &config, true).unwrap().board;
        let pairs = find_pairs(&board, &config);
        let pair = pairs
            .iter()
            .find(|pair| board.nets[pair.positive as usize].name.ends_with("D+"))
            .unwrap();
        let (p, n) = (
            &board.nets[pair.positive as usize],
            &board.nets[pair.negative as usize],
        );
        let labels: Vec<(String, String)> = pair
            .points
            .iter()
            .map(|&(a, b)| (p.terminals[a].label.clone(), n.terminals[b].label.clone()))
            .collect();
        assert!(
            labels.contains(&("C1.1".to_string(), "C2.1".to_string())),
            "{labels:?}"
        );
        // C3's and C4's other pads are 4 mm apart.
        assert!(
            !labels.iter().any(|(a, _)| a == "C3.1"),
            "{labels:?}"
        );
        // J1's rows and U2 as before, and the capacitors: three links.
        assert_eq!(labels.len(), 4, "{labels:?}");
        assert_eq!(pair.links.len(), 2);
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
        // The pair's nets: the link carries straight stubs at both ends
        // (the pads have room), so the link's two pads and the run's two
        // ends are one terminal (the link's first pad, the others merged
        // pads a route may end on): of four pads three terminals.
        for name in ["D+", "D-"] {
            let labels = terminals(&board, name);
            assert!(
                !labels.iter().any(|label| label.starts_with("pair run")),
                "{labels:?}"
            );
            assert_eq!(labels.len(), 3, "{labels:?}");
            assert!(!labels.contains(&"U2.1".to_string()), "{labels:?}");
        }
        assert!(board.obstacles.len() > obstacles);
        assert_eq!(
            board.obstacles[obstacles..]
                .iter()
                .filter(|obstacle| obstacle.label.starts_with("pair stub"))
                .count(),
            4
        );
        // The launches lie outside the pads, the run's ends beside them.
        let plan = board
            .attachment
            .0
            .clone()
            .unwrap()
            .downcast::<PairPlan>()
            .unwrap();
        let link_plan = &plan.links[0];
        assert_eq!(link_plan.fixed, [true, true]);
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

    /// A rule disallowing tracks on In1.Cu for the pair's nets (ducktop2's
    /// ground layers), and one bounding their width on In2.Cu below the
    /// width they route at: the pair's nets (their stubs, the fallback) and
    /// the link keep off both, as the coupled run does; other nets do not.
    #[test]
    fn a_pairs_nets_keep_off_the_layers_a_rule_disallows() {
        let four = BOARD.replace(
            r#"(layers (0 "F.Cu" signal) (31 "B.Cu" signal)"#,
            r#"(layers (0 "F.Cu" signal) (4 "In1.Cu" signal) (6 "In2.Cu" signal) (31 "B.Cu" signal)"#,
        );
        let pcb = parse(&four).unwrap();
        let mut config = KiCadBoardRouterConfig {
            default_rules: Some(KiCadConnectionRoutingRules {
                trace_width_mm: 0.2,
                clearance_mm: 0.2,
                via_size_mm: 0.6,
                via_drill_mm: 0.3,
            }),
            ..KiCadBoardRouterConfig::default()
        };
        let names: Vec<String> = lower(&pcb, &config, true)
            .unwrap()
            .board
            .nets
            .iter()
            .map(|net| net.name.clone())
            .filter(|name| name.ends_with("D+") || name.ends_with("D-"))
            .collect();
        assert_eq!(names.len(), 2, "{names:?}");
        for name in &names {
            config.diff_pair_rules.insert(
                name.clone(),
                crate::board_router::KiCadDiffPairRule {
                    layers: vec![(0.2, 0.2), (0.2, 0.2), (0.1, 0.2), (0.2, 0.2)],
                    disallowed: vec![false, true, false, false],
                    width_ruled: vec![false, false, true, false],
                    // In2.Cu: at most 0.1 mm (the nets route at 0.2).
                    width_bounds: vec![
                        (0.0, f64::INFINITY),
                        (0.0, f64::INFINITY),
                        (0.0, 0.1),
                        (0.0, f64::INFINITY),
                    ],
                    sources: vec!["rule 'pairs'".into(); 4],
                    ..Default::default()
                },
            );
        }
        let class_layers = |board: &core::Board, name: &str| {
            let net = board.nets.iter().find(|net| net.name == name).unwrap();
            board.classes[net.class].layers
        };
        // Off: the pair's nets go anywhere, as before.
        config.diff_pairs = Some(false);
        let board = lower(&pcb, &config, true).unwrap().board;
        assert_eq!(class_layers(&board, &names[0]), 0);
        // On: F.Cu and B.Cu (In1.Cu is bit 1, In2.Cu bit 2); GND anywhere.
        config.diff_pairs = Some(true);
        let mut board = lower(&pcb, &config, true).unwrap().board;
        assert_eq!(board.layer_count, 4);
        for name in &names {
            assert_eq!(class_layers(&board, name), 0b1001, "{name}");
        }
        let gnd = board
            .nets
            .iter()
            .position(|net| net.name.ends_with("GND"))
            .unwrap();
        assert_eq!(board.classes[board.nets[gnd].class].layers, 0);
        let nets = board.nets.len();
        plan_pairs(&mut board, &config);
        assert_eq!(board.nets.len(), nets + 1);
        let link = board.classes[board.nets[nets].class].layers;
        assert!(link != 0 && link & 0b110 == 0, "{link:b}");
        // Routed: no track of the pair's nets or the link on In1.Cu or
        // In2.Cu.
        let routed = core::route(&board, &core_config(&config));
        let pair_nets: Vec<usize> = board
            .nets
            .iter()
            .enumerate()
            .filter(|(index, net)| *index >= nets || names.contains(&net.name))
            .map(|(index, _)| index)
            .collect();
        for net in pair_nets {
            assert!(
                routed.routes[net]
                    .segments
                    .iter()
                    .all(|segment| segment.layer == 0 || segment.layer == 3),
                "{} has a track on an inner layer",
                board.nets[net].name
            );
        }
    }

    /// A fallen-back pair's leftovers: what joins nothing at one end goes
    /// (a track to nowhere, a via on no copper), what joins the two pads
    /// stays.
    #[test]
    fn what_joins_nothing_is_pruned() {
        let pad = |x: f64| core::Obstacle {
            shape: core::Shape::Circle {
                center: [x, 0.0],
                radius: 0.3,
            },
            layers: 0b11,
            kind: core::ObstacleKind::Copper,
            net: Some(0),
            clearance: 0.0,
            clearance_override: None,
            blocks_tracks: true,
            blocks_vias: true,
            label: "pad".into(),
        };
        let mut board = lowered().0;
        board.obstacles = vec![pad(0.0), pad(10.0)];
        let segment = |layer: usize, start: Point, end: Point| core::Segment {
            layer,
            start,
            end,
            width: 0.2,
        };
        let mut route = core::NetRoute {
            segments: vec![
                segment(0, [0.0, 0.0], [5.0, 0.0]),
                segment(0, [5.0, 0.0], [10.0, 0.0]),
                // A spur off the middle, to a via, on to nothing on B.Cu.
                segment(0, [5.0, 0.0], [5.0, 2.0]),
                segment(1, [5.0, 2.0], [7.0, 2.0]),
                // A short piece at the pad's edge, its far end loose.
                segment(0, [10.0, 0.0], [10.0, 0.6]),
            ],
            vias: vec![
                core::Via {
                    at: [5.0, 2.0],
                    diameter: 0.6,
                    drill: 0.3,
                },
                // A via that joins nothing.
                core::Via {
                    at: [8.0, 5.0],
                    diameter: 0.6,
                    drill: 0.3,
                },
            ],
        };
        prune_dangling(&board, &mut route, 0);
        // The B.Cu track, the lone via and the piece at the pad go; a via
        // with copper on one layer stays (a zone or a teardrop may hold it),
        // and so the spur to it.
        assert_eq!(route.segments.len(), 3, "{:?}", route.segments);
        assert_eq!(route.vias.len(), 1);
        assert!(route.segments.iter().all(|segment| segment.layer == 0));
        // A short piece whose end only overlaps the next track (KiCad: that
        // end dangles) stays where it joins two tracks, goes where it is a
        // nub off a track.
        let mut route = core::NetRoute {
            segments: vec![
                segment(0, [0.0, 0.0], [5.0, 0.0]),
                segment(0, [5.0, 0.0], [5.15, 0.0]),
                segment(0, [5.3, 0.0], [10.0, 0.0]),
                segment(0, [2.0, 0.0], [2.0, 0.15]),
            ],
            vias: Vec::new(),
        };
        prune_dangling(&board, &mut route, 0);
        assert_eq!(route.segments.len(), 3, "{:?}", route.segments);
        assert!(route.segments.iter().all(|segment| segment.end[1] == 0.0));
        assert!(
            route
                .segments
                .iter()
                .all(|segment| segment.layer == 0 && segment.start[1] == 0.0)
        );
    }
}
