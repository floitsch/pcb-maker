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

use super::meander::{
    Tuned, Tuning, bumps, copper_heights, copper_length, polyline_length, tuning_pattern_expr,
};
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
    /// Of those, the links whose crossed end has a crossover in its lead:
    /// one net dives under the other with a via on each side of it, so the
    /// stubs do not cross (docs/router.md, "Polarity").
    #[serde(default)]
    pub crossovers: usize,
    /// The coupled runs' length (millimetres, along the positive net).
    pub coupled_mm: f64,
    /// The coupled pairs' length beyond their coupled runs, after routing:
    /// per pair the longer net's track length less its coupled length
    /// (KiCad's uncoupled length), summed.
    pub uncoupled_mm: f64,
    /// The links whose run was not routed, by cause (`not_routed_cause`:
    /// "a launch has no legal node", "no path in the negotiation", "the
    /// hard repair found no room", "no hard repair: the attempt was judged
    /// hopeless", "the hard repair's budget ran out before it", "lost
    /// after the repair").
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub not_routed: BTreeMap<String, usize>,
    /// Of those, the links the negotiation gave up (`core::GiveUp`;
    /// docs/router.md, "Giving up on a link"): their pairs' nets routed
    /// uncoupled from then on, in the same attempt. Their cause in
    /// `not_routed` is "given up in the negotiation: no room" or "...: no
    /// path".
    #[serde(default)]
    pub given_up: usize,
    /// Stubs routed after the attempt for links whose copper holds but
    /// whose pads' stubs did not reach the run (`repair_stubs`; before
    /// 2026-11-05 such a link fell back: "a pad's stub does not reach the
    /// run"). Their work is in `work_seconds`.
    #[serde(default, skip_serializing_if = "is_no_count")]
    pub stubs_routed: usize,
    /// The fallback router's work (seconds of an idle machine), charged
    /// to the ladder.
    pub work_seconds: f64,
    /// The largest skew of a pair with a coupled link (its nets' lengths
    /// apart after the route, from the written copper: tracks, and vias at
    /// the stackup's heights; docs/router.md, "Skew and length").
    #[serde(default)]
    pub skew_mm: f64,
    /// Meanders added to match the pairs' lengths (`KiCadPairSkew`).
    #[serde(default)]
    pub meanders: usize,
    /// The kept links' vias (both nets' at each layer change) and the
    /// return vias of the reference net placed beside them
    /// (`place_return_vias`; docs/router.md, "Return vias").
    #[serde(default)]
    pub pair_vias: usize,
    #[serde(default)]
    pub return_vias: usize,
    /// The reference net of the return vias, where they are on.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub return_net: String,
    /// Per pair with a coupled link: its lengths and skew (kept in the
    /// verdict, unlike `pairs`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skews: Vec<KiCadPairSkew>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pairs: Vec<KiCadDiffPair>,
}

/// One coupled pair's lengths (`KiCadDiffPairs::skews`).
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct KiCadPairSkew {
    pub positive: String,
    pub negative: String,
    /// The two nets' lengths (positive, negative), millimetres.
    pub lengths_mm: [f64; 2],
    /// The longer net's length less the coupled run's (KiCad's uncoupled
    /// length: the stubs and the router's ways to the pads).
    #[serde(default)]
    pub uncoupled_mm: f64,
    pub skew_mm: f64,
    /// The skew the pair is matched to: the custom rules' `skew` maximum,
    /// else the pair's gap.
    pub target_mm: f64,
    /// What the meander added to the shorter net (0: none).
    pub meander_mm: f64,
    /// Its kept links' vias and the return vias beside them.
    #[serde(default)]
    pub pair_vias: usize,
    #[serde(default)]
    pub return_vias: usize,
}

fn is_no_count(count: &usize) -> bool {
    *count == 0
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
    /// After routing: the two nets' lengths apart (`KiCadPairSkew`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skew_mm: Option<f64>,
    /// The skew it is matched to and what the meander added.
    #[serde(default)]
    pub skew_target_mm: f64,
    #[serde(default)]
    pub meander_mm: f64,
    /// Its kept links' vias and the return vias beside them
    /// (`place_return_vias`).
    #[serde(default)]
    pub pair_vias: usize,
    #[serde(default)]
    pub return_vias: usize,
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

/// `PCB_ROUTER_PAIRS_WHY`: per link that falls back for its stubs or its
/// copper, the geometry on stderr (`pairs why: ...`): the pads, the stubs'
/// ends, the run's start and direction, what the pad's copper reaches;
/// the finding that refuses copper and what of the pair's copper it is on.
fn pairs_why() -> bool {
    std::env::var_os("PCB_ROUTER_PAIRS_WHY").is_some()
}

/// Whether the pairs' lengths are matched (`match_skew`):
/// `PCB_ROUTER_DIFF_PAIRS_SKEW=0` turns it off (an experiment's switch).
fn skew_matching() -> bool {
    std::env::var("PCB_ROUTER_DIFF_PAIRS_SKEW").map_or(true, |value| value != "0")
}

/// Whether crossed ends get a crossover in their lead (`crossover`, where
/// the link carries the stubs to it), the links' paths lose their hairpins
/// (`despiked`) and the kept links' vias keep their holes' clearance in the
/// fallback: a test's override, else `CROSSOVERS_BY_DEFAULT`.
fn crossovers() -> bool {
    CROSSOVERS
        .with(|cell| cell.get())
        .unwrap_or(CROSSOVERS_BY_DEFAULT)
}

/// The crossovers' default (`crossovers`): on since 2026-11-05 (pairs6),
/// a crossover only where the link carries the stubs to it. Measured
/// (docs/router.md, "A pair's stubs in the plan"): the crossed ends whose
/// stubs the router could not cross (no room for a via between the pads
/// and the run's start) were the most of the open stubs, and with the
/// crossovers coupled links and pairs rise on the measured boards. Off
/// before (2026-10-27, "The safe crossover": a crossover whose stubs the
/// router routes walled in a pad).
const CROSSOVERS_BY_DEFAULT: bool = true;

thread_local! {
    /// A test's override of `crossovers` (this thread only).
    static CROSSOVERS: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
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
    /// Pair points: (positive terminal, negative terminal), and what each
    /// is (`PointKind`).
    points: Vec<(usize, usize)>,
    kinds: Vec<PointKind>,
    /// Links between pair points of different parts: indices into
    /// `points`.
    links: Vec<(usize, usize)>,
    /// The custom rules' `skew` maximum and `length` (min, opt, max) for
    /// the pair's nets, if any.
    max_skew: Option<f64>,
    length: Option<(f64, Option<f64>, f64)>,
}

/// What makes a pair point (`Pair::points`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PointKind {
    /// A positive and a negative pad side by side: of one part, nothing
    /// between them, or of two two-pad parts side by side.
    Side,
    /// A positive and a negative pad of one part with another net's pin
    /// between them (an HDMI connector's or an ESD part's ground pin): a
    /// pair point only for a pair that has no link otherwise. The launch
    /// keeps the pin clear; the router routes the stubs round it.
    Pin,
}

/// Whether pairs without a link get pair points across a pin
/// (`PointKind::Pin`): a test's override (to see a board as before), else
/// on.
fn extended_points() -> bool {
    EXTENDED.with(|cell| cell.get()).unwrap_or(true)
}

thread_local! {
    /// A test's override of `extended_points` (this thread only).
    static EXTENDED: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

/// A minimum spanning tree over a pair's pair points (Prim), where pair
/// points of one part are joined first (at no cost) and by ordinary
/// routing (a USB-C receptacle's two rows): only links between parts are
/// coupled.
fn spanning_links(p: &core::Net, n: &core::Net, points: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let reference = |label: &str| {
        label
            .rsplit_once('.')
            .map_or(label, |(part, _)| part)
            .to_string()
    };
    let middle = |point: &(usize, usize)| {
        scale(
            add(p.terminals[point.0].anchor, n.terminals[point.1].anchor),
            0.5,
        )
    };
    let part = |index: usize| {
        let point = &points[index];
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
                    let length = if part(i) == part(j) {
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
            if part(i) != part(j) {
                links.push((i, j));
            }
        }
    }
    links
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
    /// Crossovers in its leads (`Crossover`).
    crossovers: usize,
    /// Per polarity: the coupled run's tracks among `segments` (the rest
    /// are stubs, jogs and crossovers), each along the run from the first
    /// end, its partner on its right where the sign is 1 (left -1).
    run: [std::ops::Range<usize>; 2],
    /// Whether it gives way to the links on their routes (where their
    /// copper meets, it alone is refused): its centreline is not the
    /// router's route as it is (`centrelines`: moved where the router did
    /// not check the room), or its stubs are still to route
    /// (`repair_stubs`).
    moved: bool,
}

/// The name of a pair's coupled link (a net of its own on the lowered
/// board), the `number`-th of the pair of `positive` and `negative`.
pub(crate) fn link_name(positive: &str, negative: &str, number: usize) -> String {
    format!("{positive} / {negative} link {number}")
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
    scanned(board.obstacles.len());
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
        let (max_skew, length) = config
            .diff_pair_rules
            .get(&net.name)
            .map_or((None, None), |rule| (rule.max_skew, rule.length));
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
        let mut kinds = vec![PointKind::Side; points.len()];
        let mut links = spanning_links(p, n, &points);
        // A pair with no link (fewer than two parts with its pads side by
        // side): its pads of one part with another net's pin between them
        // are a pair point too (docs/router.md, "A pair point across a
        // pin").
        if links.is_empty() && extended_points() {
            let mut pins: Vec<(f64, usize, usize)> = Vec::new();
            for (a, ta) in p.terminals.iter().enumerate() {
                for (b, tb) in n.terminals.iter().enumerate() {
                    let span = core::geometry::distance(ta.anchor, tb.anchor);
                    if span <= PAIR_POINT_SPAN
                        && span > EPSILON
                        && reference(&ta.label) == reference(&tb.label)
                        && ta.layers & tb.layers != 0
                        && !used_p.contains(&a)
                        && !used_n.contains(&b)
                        && straddles(board, ta, tb, [positive, negative])
                    {
                        pins.push((span, a, b));
                    }
                }
            }
            pins.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)).then(x.2.cmp(&y.2)));
            for (_, a, b) in pins {
                if !used_p.contains(&a) && !used_n.contains(&b) {
                    used_p.insert(a);
                    used_n.insert(b);
                    points.push((a, b));
                    kinds.push(PointKind::Pin);
                }
            }
            links = spanning_links(p, n, &points);
        }
        if links.is_empty() && pairs_why() {
            side_why(board, &parts, [positive, negative], &points);
        }
        if pairs_why() && kinds.iter().any(|kind| *kind != PointKind::Side) {
            let described: Vec<String> = points
                .iter()
                .zip(&kinds)
                .map(|(&(a, b), kind)| {
                    format!(
                        "{kind:?} {} {} {:.2} mm",
                        p.terminals[a].label,
                        n.terminals[b].label,
                        core::geometry::distance(p.terminals[a].anchor, n.terminals[b].anchor)
                    )
                })
                .collect();
            eprintln!(
                "pairs why: points {}: {} links; {}",
                p.name,
                links.len(),
                described.join(", ")
            );
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
            kinds,
            links,
            max_skew,
            length,
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
        let far = if core::geometry::distance(pads[0], at) > core::geometry::distance(pads[1], at) {
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

/// `PCB_ROUTER_PAIRS_WHY`: for a pair without a link ("fewer than two
/// parts with the pair's pads side by side") its pads, pair points, the
/// pads of one part with another net's copper between them (and whose),
/// those of one part further apart, and per positive pad the nearest
/// negative pad of another part, as one JSON line (`pairs why: side`).
fn side_why(
    board: &core::Board,
    parts: &BTreeMap<String, Vec<Point>>,
    nets: [core::NetId; 2],
    points: &[(usize, usize)],
) {
    let (p, n) = (&board.nets[nets[0] as usize], &board.nets[nets[1] as usize]);
    let reference = |label: &str| {
        label
            .rsplit_once('.')
            .map_or(label, |(part, _)| part)
            .to_string()
    };
    let pads_of = |label: &str| parts.get(&reference(label)).map_or(0, Vec::len);
    let mut straddled = Vec::new();
    let mut far = Vec::new();
    let mut loose = Vec::new();
    for ta in &p.terminals {
        let mut nearest: Option<(f64, &core::Terminal)> = None;
        for tb in &n.terminals {
            let span = core::geometry::distance(ta.anchor, tb.anchor);
            if reference(&ta.label) == reference(&tb.label) {
                if span <= PAIR_POINT_SPAN {
                    let between: Vec<String> = board
                        .obstacles
                        .iter()
                        .filter(|obstacle| {
                            obstacle.kind == core::ObstacleKind::Copper
                                && obstacle.layers & ta.layers & tb.layers != 0
                                && obstacle.net.is_none_or(|net| !nets.contains(&net))
                                && obstacle.shape.distance_to_segment(ta.anchor, tb.anchor)
                                    < EPSILON
                        })
                        .map(|obstacle| {
                            let net = obstacle.net.map_or("-".to_string(), |net| {
                                board.nets[net as usize].name.clone()
                            });
                            format!("{} ({net})", obstacle.label)
                        })
                        .collect();
                    if !between.is_empty() {
                        straddled.push(serde_json::json!([ta.label, tb.label, span, between]));
                    }
                } else if span <= 3.0 * PAIR_POINT_SPAN {
                    far.push(serde_json::json!([ta.label, tb.label, span]));
                }
            } else if nearest.is_none_or(|(known, _)| span < known) {
                nearest = Some((span, tb));
            }
        }
        if let Some((span, tb)) = nearest {
            loose.push(serde_json::json!([
                ta.label,
                tb.label,
                span,
                pads_of(&ta.label),
                pads_of(&tb.label)
            ]));
        }
    }
    let line = serde_json::json!({
        "positive": p.name,
        "negative": n.name,
        "p": p.terminals.iter().map(|t| format!("{}/{}", t.label, pads_of(&t.label))).collect::<Vec<_>>(),
        "n": n.terminals.iter().map(|t| format!("{}/{}", t.label, pads_of(&t.label))).collect::<Vec<_>>(),
        "points": points.iter().map(|&(a, b)| [p.terminals[a].label.clone(), n.terminals[b].label.clone()]).collect::<Vec<_>>(),
        "straddled": straddled,
        "far": far,
        "loose": loose,
    });
    eprintln!("pairs why: side {line}");
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
    scanned(board.obstacles.len());
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
    scanned(board.obstacles.len());
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
    // The outline's edges twice (inside, the edge's distance).
    tested(2 * board.outline.len());
    if !core::geometry::point_in_polygon(at, &board.outline)
        || core::geometry::polygon_edges(&board.outline)
            .map(|(a, b)| core::geometry::point_segment_distance(at, a, b))
            .fold(f64::INFINITY, f64::min)
            < radius + board.edge_clearance
    {
        return 0;
    }
    let mut free = mask;
    tested(near.len());
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

/// What a launch's run start has in front of it (`launch`): a run whose
/// stubs go straight to it, a crossover in its lead (`crossover`), or stubs
/// that cross on their way to it, which the router routes with a via
/// (a crossed end without a crossover): the start then keeps room for
/// that via beyond the pads (`CROSSING_ROOM`).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Lead {
    Straight,
    Crossover,
    Crossing,
}

/// A launch: where a link's coupled run starts at a pair point, and where
/// its net starts in the route.
#[derive(Clone, Debug)]
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
    /// Where the stubs end (positive, negative), on the pads' sides.
    stub_ends: [Point; 2],
    /// A crossover in the lead (`crossover`; asked for at a crossed end):
    /// the run past it has the nets on the other sides.
    crossover: Option<Crossover>,
    /// With a crossover: how far its copper keeps beyond the clearance from
    /// the neighbouring pairs' launches (`neighbour_room`; infinite where
    /// none is near, and without a crossover).
    room: f64,
}

/// Where a neighbouring pair's link end launches (`launch` on the board
/// before any link's copper): its straight stubs from the pads to the
/// stubs' ends and its run's lead. A crossover keeps its clearance from
/// them (`neighbour_room`): else it pushes that pair's stubs out of their
/// gap, or its launch away from the pads (oculink_port's TX pairs beside
/// the RX crossovers, 2026-10-25).
#[derive(Clone, Debug)]
struct Neighbour {
    pair: usize,
    layers: core::LayerMask,
    /// Capsules (start, end, radius, clearance): the two stubs and the
    /// leads from their ends (the pair's width and clearance) and the lead
    /// (the link's envelope with the launch's margin for lattice nodes, the
    /// link class's clearance), as `launch` checks them.
    copper: [(Point, Point, f64, f64); 5],
}

/// How far the copper of a crossover (`found`) and the run's lead past it
/// (`lead`: a capsule on `layers`, the link's envelope) keep beyond their
/// clearance from the launches of the `neighbours` (the least margin;
/// infinite where none is near; below zero where they reach into one).
fn neighbour_room(
    found: &Crossover,
    lead: (Point, Point, f64),
    layers: core::LayerMask,
    pair: &Pair,
    neighbours: &[&Neighbour],
) -> f64 {
    let mut room = f64::INFINITY;
    for neighbour in neighbours {
        for &(start, end, radius, clearance) in &neighbour.copper {
            let keep = pair.clearance.max(clearance);
            if neighbour.layers & layers != 0 {
                let distance = core::geometry::segment_segment_distance(lead.0, lead.1, start, end);
                room = room.min(distance - lead.2 - radius - keep);
            }
            for &(layer, from, to) in found.segments.iter().flatten() {
                if neighbour.layers & (1 << layer) == 0 {
                    continue;
                }
                let distance = core::geometry::segment_segment_distance(from, to, start, end);
                room = room.min(distance - pair.width / 2.0 - radius - keep);
            }
            for &via in found.vias.iter().flatten() {
                let distance = core::geometry::point_segment_distance(via, start, end);
                let need = (pair.via_diameter / 2.0 + keep)
                    .max(pair.via_drill / 2.0 + pair.hole_clearance);
                room = room.min(distance - radius - need);
            }
        }
    }
    room
}

/// The straight lead of a run from its start to its launch, at least, in
/// offsets (`Pair::offset`): a turn of the search at the launch leaves the
/// offset tracks room up to the miter limit (`offset_line`), away from the
/// pads' stubs.
const LEAD: f64 = 2.5;

/// The room a crossover leaves the neighbouring pairs' launches beyond its
/// clearance, wanted: this many of its pair's gap plus clearance
/// (`neighbour_room`); a crossover with less costs up to `ROOM_COST` (mm,
/// more than the launch search's reach: the room comes first).
const ROOM: f64 = 1.0;
const ROOM_COST: f64 = 5.0;

/// Where the part of a terminal (`REF.pad`) sits: the middle of its pads'
/// copper, when it has more than the two given. For a pair point of two
/// parts (`series_parts`: the pads `p` and `n` of two two-pad parts side
/// by side), the middle of both parts' pads: the launch leaves the parts on
/// the pads' side, not through the bodies towards their other pads (a
/// pair row's source side launched toward a far end beside the row, across
/// the parts and past the other pair's pads: right_io's C150 and C151).
fn part_middle(board: &core::Board, label: &str, other: &str) -> Option<Point> {
    let reference = |label: &str| label.rsplit_once('.').map(|(part, _)| part.to_string());
    let mut prefixes = vec![format!("{}.", reference(label)?)];
    if let Some(second) = reference(other)
        && format!("{second}.") != prefixes[0]
    {
        prefixes.push(format!("{second}."));
    }
    let (mut sum, mut count) = ([0.0, 0.0], 0usize);
    scanned(board.obstacles.len());
    for obstacle in &board.obstacles {
        if obstacle.kind == core::ObstacleKind::Copper
            && prefixes.iter().any(|prefix| {
                obstacle
                    .label
                    .strip_prefix(prefix.as_str())
                    .is_some_and(|pad| !pad.contains(' '))
            })
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
    let (anchor, length) = (
        terminal.anchor,
        core::geometry::distance(terminal.anchor, end),
    );
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
    tested(2 * (near.len() + board.outline.len()));
    (0..2).all(|polarity| {
        let (pad, end) = (pads[polarity], ends[polarity]);
        let (own, other) = (nets[polarity], nets[1 - polarity]);
        core::geometry::polygon_edges(&board.outline).all(|(a, b)| {
            core::geometry::segment_segment_distance(pad, end, a, b) >= half + board.edge_clearance
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

/// Where the stubs cross on their way to the run's start (a crossed end
/// without a crossover, `Lead::Crossing`): how far beyond the pads' stubs'
/// starts, along the run, the start lies at least, in vias with their
/// clearance on both sides and a track: room for the via the router
/// crosses with (pairs6: crossed ends launched 0.65 mm from the pads, no
/// path for their stubs in the negotiation).
const CROSSING_ROOM: f64 = 1.0;

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
    tested(near.len());
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

/// A crossover in a launch's lead (`crossover`): where the pads' order at
/// a link end is against the run's (a *crossed* end), one net (`straight`)
/// runs straight on from its stub's end while the other dives under it, a
/// via beside the run on each side of the straight track, and comes up on
/// its far side; the run starts past it, its centreline beside the
/// straight track. KiCad's gap check sees the straight track and the
/// diving net's tracks beside it a gap apart, as in the run.
#[derive(Clone, Debug)]
struct Crossover {
    /// The run's start past the crossover (on its centreline).
    run_start: Point,
    /// Per polarity: the tracks (layer index, from, to) from the stub's end
    /// to the run's start, in order, and the vias.
    segments: [Vec<(usize, Point, Point)>; 2],
    vias: [Vec<Point>; 2],
}

/// A crossover at a run start `start` going `outward` on `layer` (one
/// bit), the positive pad on `side` of it (1 left): the stubs' ends at
/// `ends` (positive, negative, on the pads' sides), the run past it with
/// the nets on the other sides. Either net may run straight; the first
/// whose copper (the tracks, the vias on every layer, the diving net's
/// track on another of the pair's layers) keeps every clearance from the
/// obstacles `near` and the edge wins. `None`: no layer or no room.
fn crossover(
    board: &core::Board,
    near: &[usize],
    pair: &Pair,
    (start, outward, side): (Point, Point, f64),
    (ends, e): ([Point; 2], f64),
    layer: core::LayerMask,
) -> Option<Crossover> {
    let index = layer.trailing_zeros() as usize;
    let mut others: Vec<usize> = (0..board.layer_count)
        .filter(|&other| other != index && pair.layers & (1 << other) != 0)
        .collect();
    others.sort_by_key(|&other| (other as i64 - index as i64).abs());
    if others.is_empty() {
        return None;
    }
    let v = left(outward);
    let at = |x: f64, y: f64| add(start, add(scale(outward, x), scale(v, y)));
    let (d, w) = (pair.offset(), pair.width);
    let radius = pair.via_diameter / 2.0;
    // A via of the diving net keeps the clearance from the straight track,
    // its hole the hole clearance.
    let apart = (radius + pair.clearance).max(pair.via_drill / 2.0 + pair.hole_clearance)
        + w / 2.0
        + ENDS_MARGIN;
    let round = |value: f64| (value.max(0.0) / 0.01).ceil() * 0.01;
    let (out_near, out_far) = (round(apart - 2.0 * e), round(apart - 2.0 * d));
    for straight in [0usize, 1] {
        let dive = 1 - straight;
        // The straight net's side (in `v`) at the pads.
        let sigma = if straight == 0 { side } else { -side };
        let jog = LAUNCH_STEP;
        let first = at(jog + out_near, -sigma * (e + out_near));
        let second = at(jog + out_near, sigma * (e + 2.0 * d + out_far));
        let back = at(jog + out_near + out_far, sigma * (e + 2.0 * d));
        let end_x = jog + out_near + out_far + LAUNCH_STEP;
        let run_start = at(end_x, sigma * (e + d));
        if core::geometry::distance(first, second) < pair.via_drill + board.hole_to_hole {
            continue;
        }
        let mut segments: [Vec<(usize, Point, Point)>; 2] = Default::default();
        segments[straight].push((index, ends[straight], at(end_x, sigma * e)));
        let corner = at(jog, -sigma * e);
        let diving = &mut segments[dive];
        diving.push((index, ends[dive], corner));
        if out_near > 0.0 {
            diving.push((index, corner, first));
        }
        let mut on_layer = None;
        for &other in &others {
            if end_clear(board, near, pair, (first, second), 1 << other) {
                on_layer = Some(other);
                break;
            }
        }
        let Some(other) = on_layer else {
            continue;
        };
        diving.push((other, first, second));
        if out_far > 0.0 {
            diving.push((index, second, back));
        }
        diving.push((index, back, at(end_x, sigma * (e + 2.0 * d))));
        let vias = [first, second];
        let tracks_clear = segments.iter().flatten().all(|&(layer, from, to)| {
            core::geometry::distance(from, to) < EPSILON
                || end_clear(board, near, pair, (from, to), 1 << layer)
        });
        // The vias' copper keeps the clearance, their holes the hole
        // clearance from other copper and hole to hole from other holes.
        let vias_clear = vias.iter().all(|&via| {
            clear_layers(board, near, via, radius, pair.clearance, board.all_layers())
                == board.all_layers()
                && near.iter().all(|&index| {
                    let obstacle = &board.obstacles[index];
                    let distance = obstacle.shape.distance_to_point(via);
                    match obstacle.kind {
                        core::ObstacleKind::Hole => {
                            distance >= pair.via_drill / 2.0 + board.hole_to_hole
                        }
                        core::ObstacleKind::Copper
                            if obstacle
                                .net
                                .is_none_or(|net| net != pair.positive && net != pair.negative) =>
                        {
                            distance >= pair.via_drill / 2.0 + board.hole_clearance
                        }
                        _ => true,
                    }
                })
        });
        if tracks_clear && vias_clear {
            let mut all_vias: [Vec<Point>; 2] = Default::default();
            all_vias[dive] = vias.to_vec();
            return Some(Crossover {
                run_start,
                segments,
                vias: all_vias,
            });
        }
    }
    None
}

/// `grid_reaches`' grid pitch (half the router's lattice).
const REACH_PITCH: f64 = 0.05;
/// Round a narrow pad the router necks its tracks down this far (its
/// `neck_reach`, the default).
const REACH_NECK: f64 = 1.5;
/// The room round the pad (and the goal) `grid_reaches` looks in.
const REACH_MARGIN: f64 = 1.5;

/// Whether a track of the pair's width can go from the pad of `terminal`
/// (of `polarity`) to `to` on `layers`, past the obstacles `near` and the
/// `extra` shapes (with their clearance; a name each for the diagnostics):
/// `grid_reaches` (`PCB_ROUTER_PAIRS_REACH`'s `grid` verdict).
fn stub_reaches(
    board: &core::Board,
    near: &[usize],
    pair: &Pair,
    (terminal, polarity): (&core::Terminal, usize),
    to: Point,
    layers: core::LayerMask,
    extra: &[(core::Shape, f64, &str)],
) -> bool {
    let nets = [pair.positive, pair.negative];
    grid_reaches(
        board,
        near,
        pair,
        (terminal.pad, terminal.anchor),
        (Some(nets[polarity]), Some(nets[1 - polarity])),
        Some(to),
        layers,
        extra,
    )
}

/// At a pair point across a pin (`PointKind::Pin`): the first pin between
/// the pads (another net's copper on the line between their centres) that
/// gets no way out (`grid_reaches` to its window's border) past straight
/// stubs from the pads to their ends and the run's lead at `found`: a
/// ground pin between a fine-pitch part's D+ and D- whose only way out lay
/// between them (MSign's UQFN USB muxes).
fn walled_pin(
    board: &core::Board,
    pair: &Pair,
    class: &core::RuleClass,
    (p, n): (&core::Terminal, &core::Terminal),
    found: &Launch,
) -> Option<String> {
    let nets = [pair.positive, pair.negative];
    let half = pair.width / 2.0;
    let stubs = [
        core::Shape::Capsule {
            start: p.anchor,
            end: found.stub_ends[0],
            radius: half,
        },
        core::Shape::Capsule {
            start: n.anchor,
            end: found.stub_ends[1],
            radius: half,
        },
        core::Shape::Capsule {
            start: found.reserved,
            end: found.at,
            radius: class.trace_width / 2.0,
        },
    ];
    let extra = [
        (stubs[0].clone(), pair.clearance, "the positive stub"),
        (stubs[1].clone(), pair.clearance, "the negative stub"),
        (
            stubs[2].clone(),
            class.clearance.max(pair.clearance),
            "the run's lead",
        ),
    ];
    scanned(board.obstacles.len());
    board
        .obstacles
        .iter()
        .enumerate()
        .filter(|(_, obstacle)| {
            obstacle.kind == core::ObstacleKind::Copper
                && obstacle.layers & p.layers & n.layers & found.layers != 0
                && obstacle.net.is_none_or(|net| !nets.contains(&net))
                && obstacle.shape.distance_to_segment(p.anchor, n.anchor) < EPSILON
        })
        .find(|(index, obstacle)| {
            let bounds = obstacle.shape.aabb();
            let centre = scale(add(bounds.minimum, bounds.maximum), 0.5);
            let near = obstacles_near(board, centre, REACH_MARGIN + 1.0);
            !grid_reaches(
                board,
                &near,
                pair,
                (*index, centre),
                (obstacle.net, None),
                None,
                found.layers,
                &extra,
            )
        })
        .map(|(_, obstacle)| obstacle.label.clone())
}

/// A flood fill on a grid of `REACH_PITCH`: a track of the pair's width from the
/// pad `pad` (an obstacle, its anchor `from`) of net `own` (`other` kept
/// at the pair's gap) to `goal`, or where there is none, out of the window
/// round the pad (`REACH_MARGIN`): a pin that does not get out is walled in.
#[allow(clippy::too_many_arguments)]
fn grid_reaches(
    board: &core::Board,
    near: &[usize],
    pair: &Pair,
    (pad_index, from): (usize, Point),
    (own, other): (Option<core::NetId>, Option<core::NetId>),
    goal: Option<Point>,
    layers: core::LayerMask,
    extra: &[(core::Shape, f64, &str)],
) -> bool {
    let to = goal.unwrap_or(from);
    let half = pair.width / 2.0;
    let low = [
        from[0].min(to[0]) - REACH_MARGIN,
        from[1].min(to[1]) - REACH_MARGIN,
    ];
    let high = [
        from[0].max(to[0]) + REACH_MARGIN,
        from[1].max(to[1]) + REACH_MARGIN,
    ];
    let columns = ((high[0] - low[0]) / REACH_PITCH).ceil() as usize + 1;
    let rows = ((high[1] - low[1]) / REACH_PITCH).ceil() as usize + 1;
    let cell = |column: usize, row: usize| {
        [
            low[0] + column as f64 * REACH_PITCH,
            low[1] + row as f64 * REACH_PITCH,
        ]
    };
    // Per cell: 0 free, else what blocks it (`sources`, from 1).
    let mut blocked = vec![0u32; columns * rows];
    let mut sources: Vec<String> = Vec::new();
    let mut tests = 0usize;
    // Round a pad narrower than the track and its clearance the router
    // routes at the neck width (its `neck_reach` of the pad's anchor).
    let pad_bounds = board.obstacles[pad_index].shape.aabb();
    let narrow = (pad_bounds.maximum[0] - pad_bounds.minimum[0])
        .min(pad_bounds.maximum[1] - pad_bounds.minimum[1])
        < pair.width + pair.clearance;
    let neck_half = board.neck_width.min(pair.width) / 2.0;
    let necked = |at: Point| {
        narrow && (at[0] - from[0]).abs() <= REACH_NECK && (at[1] - from[1]).abs() <= REACH_NECK
    };
    // Blocks the cells within `keep` and half the track's width of `shape`.
    let mut block = |shape: &core::Shape, keep: f64, blocked: &mut Vec<u32>, source: u32| {
        let reach = keep + half;
        let bounds = shape.aabb();
        if bounds.maximum[0] + reach < low[0]
            || bounds.minimum[0] - reach > high[0]
            || bounds.maximum[1] + reach < low[1]
            || bounds.minimum[1] - reach > high[1]
        {
            return;
        }
        let first = |value: f64, origin: f64| {
            ((value - reach - origin) / REACH_PITCH).floor().max(0.0) as usize
        };
        let last = |value: f64, origin: f64, count: usize| {
            (((value + reach - origin) / REACH_PITCH).ceil().max(0.0) as usize).min(count - 1)
        };
        for row in first(bounds.minimum[1], low[1])..=last(bounds.maximum[1], low[1], rows) {
            for column in
                first(bounds.minimum[0], low[0])..=last(bounds.maximum[0], low[0], columns)
            {
                let index = row * columns + column;
                if blocked[index] == 0 {
                    tests += 1;
                    let at = cell(column, row);
                    let width = if necked(at) { neck_half } else { half };
                    if shape.distance_to_point(at) < keep + width {
                        blocked[index] = source;
                    }
                }
            }
        }
    };
    for &index in near {
        let obstacle = &board.obstacles[index];
        if obstacle.layers & layers == 0
            || index == pad_index
            || (own.is_some() && obstacle.net == own)
        {
            continue;
        }
        // The pad's mask opening is left by its stub.
        if obstacle.kind == core::ObstacleKind::Keepout
            && obstacle.label.starts_with("solder mask opening")
            && obstacle.shape.contains(from)
        {
            continue;
        }
        let mut keep = keep_from(board, obstacle, pair.clearance);
        if other.is_some() && obstacle.net == other {
            keep = keep.min(pair.gap);
        }
        sources.push(obstacle.label.clone());
        block(&obstacle.shape, keep, &mut blocked, sources.len() as u32);
    }
    for (shape, keep, label) in extra {
        sources.push(label.to_string());
        block(shape, *keep, &mut blocked, sources.len() as u32);
    }
    // The board's edge.
    sources.push("the edge".into());
    let edge_source = sources.len() as u32;
    for (a, b) in core::geometry::polygon_edges(&board.outline) {
        let edge = core::Shape::Capsule {
            start: a,
            end: b,
            radius: 0.0,
        };
        block(&edge, board.edge_clearance, &mut blocked, edge_source);
    }
    tested(tests + board.outline.len());
    // A flood fill from the pad's free cells.
    let pad = &board.obstacles[pad_index].shape;
    let mut queue = std::collections::VecDeque::new();
    let mut seen = vec![false; columns * rows];
    for row in 0..rows {
        for column in 0..columns {
            let index = row * columns + column;
            // The pad's own cells, blocked or not: a track leaves a pad
            // from inside it (the router's pad exits).
            if pad.contains(cell(column, row)) {
                seen[index] = true;
                queue.push_back((column, row));
            }
        }
    }
    if queue.is_empty() {
        // A pad smaller than a cell: the cell at its anchor.
        let column = (((from[0] - low[0]) / REACH_PITCH).round() as usize).min(columns - 1);
        let row = (((from[1] - low[1]) / REACH_PITCH).round() as usize).min(rows - 1);
        seen[row * columns + column] = true;
        queue.push_back((column, row));
    }
    let seeds = queue.len();
    let mut closest = f64::INFINITY;
    let mut walls: BTreeSet<u32> = BTreeSet::new();
    while let Some((column, row)) = queue.pop_front() {
        let distance = core::geometry::distance(cell(column, row), to);
        let out =
            goal.is_none() && (column == 0 || row == 0 || column + 1 == columns || row + 1 == rows);
        if out || (goal.is_some() && distance <= LAUNCH_CONTACT) {
            return true;
        }
        closest = closest.min(distance);
        for (dc, dr) in [
            (1i64, 0i64),
            (-1, 0),
            (0, 1),
            (0, -1),
            (1, 1),
            (1, -1),
            (-1, 1),
            (-1, -1),
        ] {
            let (c, r) = (column as i64 + dc, row as i64 + dr);
            if c < 0 || r < 0 || c >= columns as i64 || r >= rows as i64 {
                continue;
            }
            let index = r as usize * columns + c as usize;
            if blocked[index] != 0 {
                walls.insert(blocked[index]);
                continue;
            }
            if !seen[index] {
                seen[index] = true;
                queue.push_back((c as usize, r as usize));
            }
        }
    }
    if debug() {
        eprintln!(
            "diff pair reach {}: {seeds} seeds, closest {closest:.3} of {:.3}; walls {:?}",
            board.obstacles[pad_index].label,
            core::geometry::distance(from, to),
            walls
                .iter()
                .map(|wall| sources[*wall as usize - 1].as_str())
                .collect::<Vec<_>>()
        );
    }
    false
}

/// A launch for a link at the pair point of pads `p` and `n` toward
/// `toward`: from between the pads along the pads' normal away from their
/// part (toward the link's other end where the part does not say), or
/// turned from it by up to a right angle, the first run start where the
/// stubs' ends beside it, the run's straight lead and the envelope `class`
/// at the lead's end are clear; of those the one closest to the pads (a
/// turn costs `ANGLE_COST` a right angle). The stubs themselves are the
/// route's: the pair's nets' ordinary routing from the pads to the ends.
/// With `cross`, a start with a crossover in its lead, which keeps its
/// clearance from the `neighbours`' launches (`neighbour_room`); where it
/// leaves them less than `ROOM` (the pair's gap plus its clearance) the
/// search goes on along the direction, and the room costs (`ROOM_COST`).
#[allow(clippy::too_many_arguments)]
fn launch(
    board: &core::Board,
    pair: &Pair,
    class: &core::RuleClass,
    p: &core::Terminal,
    n: &core::Terminal,
    toward: Point,
    want_fixed: bool,
    lead_kind: Lead,
    neighbours: &[Neighbour],
) -> Option<Launch> {
    let cross = lead_kind == Lead::Crossover;
    let middle = scale(add(p.anchor, n.anchor), 0.5);
    let mut normal = left(unit(sub(n.anchor, p.anchor)));
    let away = part_middle(board, &p.label, &n.label)
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
    let crossing_room = CROSSING_ROOM * (pair.via_diameter + 2.0 * pair.clearance + pair.width);
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
    // The neighbours' launches a crossover here may reach.
    let neighbours: Vec<&Neighbour> = neighbours
        .iter()
        .filter(|neighbour| {
            neighbour.copper.iter().any(|&(start, end, radius, _)| {
                core::geometry::point_segment_distance(middle, start, end)
                    <= LAUNCH_REACH + 2.0 * lead + radius + 2.0
            })
        })
        .collect();
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
        // A start with a crossover in its lead: the stubs' ends on the
        // pads' sides, the crossover's copper clear, the run's lead past it
        // (beside the straight net) and the envelope at its end clear.
        let crossed_with = |start: Point, e: f64| -> Option<Launch> {
            let ends = [
                add(start, scale(left(outward), side * e)),
                add(start, scale(left(outward), -side * e)),
            ];
            let found = crossover(
                board,
                &near,
                pair,
                (start, outward, side),
                (ends, e),
                layers,
            )?;
            let run_start = found.run_start;
            let lead = LEAD * d;
            let at = add(run_start, scale(outward, lead));
            // Past the crossover the run's centreline lies beside the
            // straight net, off the pads' middle: its lead keeps the room
            // too.
            let room = neighbour_room(&found, (run_start, at, half), layers, pair, &neighbours);
            if room < 0.0 {
                return None;
            }
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
            let samples = (lead / LAUNCH_STEP).ceil() as usize;
            if (0..samples).any(|sample| {
                let point = add(run_start, scale(outward, sample as f64 * LAUNCH_STEP));
                clear_layers(board, &near, point, half, class.clearance, layers) == 0
            }) {
                return None;
            }
            let stubs = [
                stub_start(board, p, ends[0], pair.width),
                stub_start(board, n, ends[1], pair.width),
            ];
            // The stubs keep their clearance from the other net's crossover
            // copper too.
            let apart_from_crossover = |polarity: usize| {
                let (pad, end) = (stubs[polarity], ends[polarity]);
                let other = 1 - polarity;
                found.segments[other]
                    .iter()
                    .filter(|(layer, ..)| 1 << layer == layers)
                    .all(|&(_, from, to)| {
                        core::geometry::segment_segment_distance(pad, end, from, to)
                            >= pair.width + pair.gap.min(pair.clearance) - EPSILON
                    })
                    && found.vias[other].iter().all(|&via| {
                        core::geometry::point_segment_distance(via, pad, end)
                            >= pair.width / 2.0
                                + (pair.via_diameter / 2.0 + pair.clearance)
                                    .max(pair.via_drill / 2.0 + pair.hole_clearance)
                    })
            };
            let fixed = want_fixed
                && stubs_clear(board, &near, pair, stubs, ends, layers)
                && apart_from_crossover(0)
                && apart_from_crossover(1);
            Some(Launch {
                at,
                start,
                reserved: run_start,
                ends: scale(outward, ends_reach),
                outward,
                layers,
                side,
                fixed,
                stubs,
                stub_ends: ends,
                crossover: Some(found),
                room,
            })
        };
        let crossed_at = |start: Point| -> Option<Launch> {
            // The stubs' ends an offset apart where the link carries the
            // stubs (KiCad then sees the tracks before the crossover a gap
            // apart), else as far apart as the router keeps the two nets.
            let (d, e) = (pair.offset(), pair.ends_offset());
            let offsets: &[f64] = if e > d + EPSILON && want_fixed {
                &[d, e]
            } else {
                &[e]
            };
            offsets.iter().find_map(|&offset| {
                let launch = crossed_with(start, offset)?;
                (launch.fixed || offset >= e).then_some(launch)
            })
        };
        let clear_at = |step: usize| {
            let start = add(middle, scale(outward, step as f64 * LAUNCH_STEP));
            if cross {
                return crossed_at(start);
            }
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
            // Stubs that cross keep room for the router's via between the
            // pads and the start.
            if lead_kind == Lead::Crossing
                && stubs
                    .iter()
                    .any(|&stub| dot(sub(start, stub), outward) < crossing_room)
            {
                return None;
            }
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
                stub_ends: ends,
                crossover: None,
                room: f64::INFINITY,
            })
        };
        let room_wanted = ROOM * (pair.gap + pair.clearance);
        let cost_of = |found: &Launch| {
            let crowded = if cross {
                ROOM_COST * (1.0 - found.room.min(room_wanted) / room_wanted)
            } else {
                0.0
            };
            core::geometry::distance(middle, found.start)
                + ANGLE_COST * angle.abs() / 90.0
                + crowded
        };
        // The first clear start along the direction, then the first whose
        // stubs the link carries; a crossover that leaves its neighbours
        // less than the room wanted looks further along.
        let mut step = 0;
        while step <= steps {
            let Some(found) = clear_at(step) else {
                step += 1;
                continue;
            };
            let cost = cost_of(&found);
            if best.as_ref().is_none_or(|(known, _)| cost < known - 1.0e-9) {
                best = Some((cost, found.clone()));
            }
            if cross && found.room < room_wanted {
                if want_fixed
                    && found.fixed
                    && best_fixed
                        .as_ref()
                        .is_none_or(|(known, _)| cost < known - 1.0e-9)
                {
                    best_fixed = Some((cost, found));
                }
                step += 1;
                continue;
            }
            if !want_fixed || best_fixed.as_ref().is_some_and(|(known, _)| *known <= cost) {
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
            best.as_ref().map(|(_, launch)| launch)
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
    /// Per end: the crossover in its lead (at a crossed end, where one
    /// fits): the run starts past it (`starts`), the stubs' ends are on the
    /// pads' sides.
    crossovers: [Option<Crossover>; 2],
    /// Per end and polarity: the terminal of the pair's net at the stub's
    /// end (`plan_pairs`), for the nets as they are with the link given up
    /// (`GiveUp::released`: without it).
    runs: [[core::Terminal; 2]; 2],
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
#[derive(Clone)]
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
    /// The plan's work (`plan_work_of` its obstacle tests), booked by the
    /// stage that lowered the board (`planned_work`).
    work_seconds: f64,
}

impl PairPlan {
    /// The net of `polarity` (0 positive, 1 negative) of pair `pair` as
    /// `plan_pairs` lowers it with only the links not in `released`: its
    /// terminals as lowered, each such link's stubs' ends added and joined
    /// with each other and with the pads whose stubs the link carries, then
    /// merged (`merge_net_terminals`). With nothing released it is the net
    /// as planned.
    fn net_with_links(
        &self,
        board: &core::Board,
        pair: usize,
        polarity: usize,
        released: &BTreeSet<core::NetId>,
    ) -> Option<core::NetRewrite> {
        let net = [self.pairs[pair].positive, self.pairs[pair].negative][polarity];
        let (_, original) = self.originals.iter().find(|(owner, _)| *owner == net)?;
        let mut description = original.clone();
        let mut merged_pads: Vec<core::MergedPad> = self
            .merged_pads
            .iter()
            .filter(|merged| merged.net == net)
            .cloned()
            .collect();
        let mut terminal_copper: Vec<core::TerminalCopper> = self
            .terminal_copper
            .iter()
            .filter(|extra| extra.net == net)
            .copied()
            .collect();
        let mut parent: Vec<usize> = (0..description.terminals.len()).collect();
        for link in self
            .links
            .iter()
            .filter(|link| link.pair == pair && !released.contains(&link.net))
        {
            description.terminals.push(link.runs[0][polarity].clone());
            description.terminals.push(link.runs[1][polarity].clone());
            let count = description.terminals.len();
            parent.extend(parent.len()..count);
            let (a, b) = self.pairs[pair].links[link.link];
            let points = [self.pairs[pair].points[a], self.pairs[pair].points[b]];
            let mut joined = vec![count - 2, count - 1];
            for (end, point) in points.into_iter().enumerate() {
                if link.fixed[end] {
                    joined.push([point.0, point.1][polarity]);
                }
            }
            for item in &joined[1..] {
                let (x, y) = (root(&mut parent, joined[0]), root(&mut parent, *item));
                if x != y {
                    parent[x.max(y)] = x.min(y);
                }
            }
        }
        let host_of: Vec<usize> = (0..parent.len()).map(|t| root(&mut parent, t)).collect();
        merge_net_terminals(
            net,
            &mut description,
            &mut merged_pads,
            &mut terminal_copper,
            &board.obstacles,
            &host_of,
        );
        Some(core::NetRewrite {
            net,
            terminals: description.terminals,
            merged_pads,
            terminal_copper,
        })
    }
}

/// The links may be given up in the negotiation (docs/router.md, "Giving
/// up on a link"): each is a net that stands in for the pair's two nets
/// between the stubs' ends; given up, the pair's nets go back to their pads
/// as lowered, joined by the links still standing.
impl core::GiveUp for PairPlan {
    fn optional(&self, net: core::NetId) -> bool {
        net as usize >= self.nets && self.links.iter().any(|link| link.net == net)
    }

    fn released(&self, board: &core::Board, released: &[core::NetId]) -> Vec<core::NetRewrite> {
        let released: BTreeSet<core::NetId> = released.iter().copied().collect();
        let pairs: BTreeSet<usize> = self
            .links
            .iter()
            .filter(|link| released.contains(&link.net))
            .map(|link| link.pair)
            .collect();
        let mut rewrites = Vec::new();
        for pair in pairs {
            for polarity in 0..2 {
                rewrites.extend(self.net_with_links(board, pair, polarity, &released));
            }
        }
        rewrites
    }

    fn standing(
        &self,
        board: &core::Board,
        net: core::NetId,
        released: &[core::NetId],
    ) -> Vec<core::NetRewrite> {
        let released: BTreeSet<core::NetId> = released
            .iter()
            .copied()
            .filter(|other| *other != net)
            .collect();
        let Some(link) = self.links.iter().find(|link| link.net == net) else {
            return Vec::new();
        };
        (0..2)
            .filter_map(|polarity| self.net_with_links(board, link.pair, polarity, &released))
            .collect()
    }
}

/// Whether the router may give up a pair's link in the negotiation
/// (`core::GiveUp`, docs/router.md "Giving up on a link"): a test's
/// override, else the settings' `diff_pair_give_up`, else
/// `PCB_ROUTER_DIFF_PAIRS_GIVE_UP` (`1` or `0`), else `GIVE_UP_BY_DEFAULT`.
fn give_up_links(config: &KiCadBoardRouterConfig) -> bool {
    GIVE_UP.with(|cell| cell.get()).unwrap_or_else(|| {
        config.diff_pair_give_up.unwrap_or_else(|| {
            std::env::var("PCB_ROUTER_DIFF_PAIRS_GIVE_UP")
                .map_or(GIVE_UP_BY_DEFAULT, |value| value == "1")
        })
    })
}

/// The links' give-up's default (`give_up_links`): off. Measured
/// 2026-11-05 (docs/router.md, "Giving up on a link"): on jetson-nano-
/// baseboard it brings the layout's unconnected from 23 and 44 to 10 and 16
/// with as many pairs coupled, but on ducktop2's left_io seed 101 it costs
/// 3 of 6 coupled pairs and 16 connections, beyond the evidence rule's one
/// pair.
const GIVE_UP_BY_DEFAULT: bool = false;

thread_local! {
    /// A test's override of `give_up_links` (this thread only).
    static GIVE_UP: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

thread_local! {
    /// The obstacles the pairs' code looked at on this thread: those of
    /// board-wide scans (a bounding box or a label each) and those of a
    /// launch's near lists (a distance each), `plan_pairs`' work model
    /// (`PLAN_SCAN_NANOS`, `PLAN_TEST_NANOS`).
    static PLAN_TESTS: std::cell::Cell<[u64; 2]> = const { std::cell::Cell::new([0, 0]) };
}

/// Counts `count` obstacles of a board-wide scan (`PLAN_TESTS`).
fn scanned(count: usize) {
    PLAN_TESTS.with(|tests| {
        let [scans, near] = tests.get();
        tests.set([scans + count as u64, near]);
    });
}

/// Counts `count` obstacles tested by distance (`PLAN_TESTS`).
fn tested(count: usize) {
    PLAN_TESTS.with(|tests| {
        let [scans, near] = tests.get();
        tests.set([scans, near + count as u64]);
    });
}

/// The obstacles scanned and tested on this thread so far (`PLAN_TESTS`).
fn tests_so_far() -> [u64; 2] {
    PLAN_TESTS.with(std::cell::Cell::get)
}

/// An obstacle of a board-wide scan and one tested by distance (or an
/// outline edge), in nanoseconds of the reference machine (the work model
/// of `plan_pairs`, as `LOWERING_EXPRESSION_NANOS` is the lowering's).
/// Measured 2026-11-04 (one plan, the time over the rate's scale): right_io
/// 0.15 M scanned and 6.8 M tested in 0.24 s of the reference, 35 ns a
/// test; left_io 0.52 M and 21.5 M in 0.67 s, 31 ns; jetson 7.9 M and 42.3
/// M in 0.90 s, about 20 ns (its tests leave sooner: more obstacles on
/// other layers). The model is off by -20, -10 and +35 % on them
/// (docs/router.md, "The layout's own routes").
pub(crate) const PLAN_SCAN_NANOS: f64 = 4.0;
pub(crate) const PLAN_TEST_NANOS: f64 = 28.0;

/// The modelled work of the obstacles scanned and tested since `before`
/// (`tests_so_far`), in seconds at this process's rate.
fn plan_work_since(before: [u64; 2]) -> f64 {
    let [scans, near] = tests_so_far();
    ((scans - before[0]) as f64 * PLAN_SCAN_NANOS + (near - before[1]) as f64 * PLAN_TEST_NANOS)
        / 1.0e9
        * core::router::reference_time_scale()
}

/// Merges terminals of `net` into their hosts (`host_of[t]`, the smallest
/// terminal of its group): they become merged pads of the host, which a
/// route may end on (`Board::terminal_copper`), as the board's own copper
/// merges the pads it joins (`keep_existing_copper`).
fn merge_terminals(board: &mut core::Board, net: core::NetId, host_of: &[usize]) {
    let mut description = std::mem::replace(
        &mut board.nets[net as usize],
        core::Net {
            name: String::new(),
            class: 0,
            terminals: Vec::new(),
        },
    );
    merge_net_terminals(
        net,
        &mut description,
        &mut board.merged_pads,
        &mut board.terminal_copper,
        &board.obstacles,
        host_of,
    );
    board.nets[net as usize] = description;
}

/// `merge_terminals` on a net of its own: `description` is the net
/// `net`, `merged_pads` and `terminal_copper` hold (at least) its entries.
fn merge_net_terminals(
    net: core::NetId,
    description: &mut core::Net,
    merged_pads: &mut Vec<core::MergedPad>,
    terminal_copper: &mut Vec<core::TerminalCopper>,
    obstacles: &[core::Obstacle],
    host_of: &[usize],
) {
    let terminals = description.terminals.len();
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
            let terminal_description = &description.terminals[terminal];
            merged.push(core::MergedPad {
                net,
                host: renumbered[host_of[terminal]],
                pad: terminal_description.pad,
                anchor: terminal_description.anchor,
                layers: obstacles[terminal_description.pad].layers,
                padstack: false,
            });
        }
    }
    for existing in merged_pads
        .iter_mut()
        .filter(|existing| existing.net == net && existing.host < terminals)
    {
        existing.padstack &= host_of[existing.host] == existing.host;
        existing.host = renumbered[host_of[existing.host]];
    }
    for pad in &merged {
        terminal_copper.push(core::TerminalCopper {
            net,
            terminal: pad.host,
            obstacle: pad.pad,
        });
    }
    let mut index = 0;
    description.terminals.retain(|_| {
        index += 1;
        host_of[index - 1] == index - 1
    });
    merged_pads.extend(merged);
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
    let tests_before = tests_so_far();
    let started = std::time::Instant::now();
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
        work_seconds: 0.0,
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
    // With crossovers: every link end's launch on the board as lowered (no
    // link's copper yet), the room a crossover leaves its neighbours
    // (`neighbour_room`).
    let mut launches_before: Vec<Neighbour> = Vec::new();
    if crossovers() {
        for (index, pair) in pairs.iter().enumerate() {
            let Some(&class) = class_of.get(&pair.layers) else {
                continue;
            };
            if !entries[index].why.is_empty() {
                continue;
            }
            let envelope = board.classes[class];
            for &(a, b) in &pair.links {
                let points = [pair.points[a], pair.points[b]];
                let pads = points.map(|(p, n)| {
                    (
                        &board.nets[pair.positive as usize].terminals[p],
                        &board.nets[pair.negative as usize].terminals[n],
                    )
                });
                for end in 0..2 {
                    let (p, n) = pads[end];
                    let (q, m) = pads[1 - end];
                    let toward = scale(add(q.anchor, m.anchor), 0.5);
                    let Some(found) = launch(
                        board,
                        pair,
                        &envelope,
                        p,
                        n,
                        toward,
                        true,
                        Lead::Straight,
                        &[],
                    ) else {
                        continue;
                    };
                    let half = pair.width / 2.0;
                    let along = sub(found.reserved, found.start);
                    let lead_of = |end: Point| (end, add(end, along), half, pair.clearance);
                    launches_before.push(Neighbour {
                        pair: index,
                        layers: found.layers,
                        copper: [
                            (found.stubs[0], found.stub_ends[0], half, pair.clearance),
                            (found.stubs[1], found.stub_ends[1], half, pair.clearance),
                            lead_of(found.stub_ends[0]),
                            lead_of(found.stub_ends[1]),
                            (
                                found.start,
                                found.at,
                                envelope.trace_width / 2.0 + LAUNCH_MARGIN,
                                envelope.clearance,
                            ),
                        ],
                    });
                }
            }
        }
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
        let neighbours: Vec<Neighbour> = launches_before
            .iter()
            .filter(|neighbour| neighbour.pair != index)
            .cloned()
            .collect();
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
            let launch_at = |end: usize, want_fixed: bool, lead_kind: Lead| {
                launch(
                    board,
                    pair,
                    &envelope,
                    &ends[end].0,
                    &ends[end].1,
                    middle(1 - end),
                    want_fixed,
                    lead_kind,
                    &neighbours,
                )
            };
            // Launches whose stubs the link carries where there are such,
            // else (or where those leave no run) the closest.
            let mut launches = [
                launch_at(0, true, Lead::Straight),
                launch_at(1, true, Lead::Straight),
            ];
            let too_close = |launches: &[Option<Launch>; 2]| match launches {
                [Some(a), Some(b)] => core::geometry::distance(a.at, b.at) < MINIMUM_RUN,
                _ => false,
            };
            if too_close(&launches) && launches.iter().flatten().any(|found| found.fixed) {
                launches = [
                    launch_at(0, false, Lead::Straight),
                    launch_at(1, false, Lead::Straight),
                ];
            }
            let close = too_close(&launches);
            let [Some(mut launch_a), Some(mut launch_b)] = launches else {
                entries[index]
                    .why
                    .push(format!("link {}: no launch clear of the pads", link + 1));
                continue;
            };
            if close {
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
            let mut side = if crossed && !launch_a.fixed && launch_b.fixed {
                -launch_b.side
            } else {
                launch_a.side
            };
            // At a crossed end a crossover in the lead (`crossover`), where
            // one fits: at the second end, else at the first; one whose
            // stubs the link carries first.
            let mut crossing = None;
            if crossed && crossovers() {
                let at_b = launch_at(1, true, Lead::Crossover);
                let at_a = launch_at(0, true, Lead::Crossover);
                let far = |found: &Launch, other: &Launch| {
                    core::geometry::distance(found.at, other.at) >= MINIMUM_RUN
                };
                // Only where the link carries the stubs to it: a crossover
                // whose stubs the router routes can wall in a pad with its
                // copper (a dead pad, the stub's net open; pairs6).
                let options = [
                    at_b.filter(|found| found.fixed && far(found, &launch_a))
                        .map(|found| (1, found)),
                    at_a.filter(|found| found.fixed && far(found, &launch_b))
                        .map(|found| (0, found)),
                ];
                let fixed_at = |end: usize, found: &Launch| {
                    found.fixed && [&launch_a, &launch_b][1 - end].fixed
                };
                // Both ends' stubs the link's first.
                crossing = options
                    .iter()
                    .flatten()
                    .find(|(end, found)| fixed_at(*end, found))
                    .or(options.iter().flatten().next())
                    .cloned();
            }
            if let Some((end, found)) = crossing {
                if end == 1 {
                    side = launch_a.side;
                    launch_b = found;
                } else {
                    side = -launch_a.side;
                    launch_a = found;
                }
            } else if crossed {
                // The crossed end's stubs are routed: its closest launch.
                if side == launch_a.side {
                    launch_b = launch_at(1, false, Lead::Crossing).unwrap_or(launch_b);
                    launch_b.fixed = false;
                } else {
                    launch_a = launch_at(0, false, Lead::Crossing).unwrap_or(launch_a);
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
            // At a pair point across a pin the pin must keep a way out past
            // the stubs and the run's lead (`walled_pin`).
            if let Some(pin) = [(&launch_a, 0usize), (&launch_b, 1)]
                .into_iter()
                .filter(|(_, end)| pair.kinds[[a, b][*end]] == PointKind::Pin)
                .find_map(|(found, end)| {
                    walled_pin(board, pair, &envelope, (&ends[end].0, &ends[end].1), found)
                })
            {
                if pairs_why() {
                    eprintln!(
                        "pairs why: walled {}: link {}, {pin}",
                        board.nets[pair.positive as usize].name,
                        link + 1
                    );
                }
                entries[index].why.push(format!(
                    "link {}: the pin between its pads ({pin}) would be walled in by its stubs",
                    link + 1
                ));
                continue;
            }
            let mut terminals = Vec::new();
            let mut stub_ends = [[[0.0; 2]; 2]; 2];
            let mut runs: [[Option<core::Terminal>; 2]; 2] = Default::default();
            for (end, found) in [&launch_a, &launch_b].into_iter().enumerate() {
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
                        start: if found.fixed && found.crossover.is_none() {
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
                    // At a crossover the stubs' ends are on the pads' sides.
                    let at = if found.crossover.is_some() {
                        found.stub_ends[polarity]
                    } else {
                        add(found.start, scale(across, sign * pair.ends_offset()))
                    };
                    stub_ends[end][polarity] = at;
                    let run = core::Terminal {
                        anchor: at,
                        layers: found.layers,
                        pad: board.obstacles.len(),
                        contact: Some(core::Shape::Circle {
                            center: at,
                            radius: LAUNCH_CONTACT,
                        }),
                        label: format!("pair run {}{}", link + 1, ["a", "b"][end]),
                    };
                    board.nets[net as usize].terminals.push(run.clone());
                    runs[end][polarity] = Some(run);
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
                // A crossover's copper: of the pair's nets (other nets keep
                // clear of it).
                if let Some(crossing) = &found.crossover {
                    for (polarity, net) in [pair.positive, pair.negative].into_iter().enumerate() {
                        for &(layer, from, to) in &crossing.segments[polarity] {
                            board.obstacles.push(core::Obstacle {
                                shape: core::Shape::Capsule {
                                    start: from,
                                    end: to,
                                    radius: pair.width / 2.0,
                                },
                                layers: 1 << layer,
                                kind: core::ObstacleKind::Copper,
                                net: Some(net),
                                clearance: 0.0,
                                clearance_override: None,
                                blocks_tracks: true,
                                blocks_vias: true,
                                label: format!("pair crossover {}{}", link + 1, ["a", "b"][end]),
                            });
                        }
                        for &via in &crossing.vias[polarity] {
                            for (kind, radius) in [
                                (core::ObstacleKind::Copper, pair.via_diameter / 2.0),
                                (core::ObstacleKind::Hole, pair.via_drill / 2.0),
                            ] {
                                board.obstacles.push(core::Obstacle {
                                    shape: core::Shape::Circle {
                                        center: via,
                                        radius,
                                    },
                                    layers: board.all_layers(),
                                    kind,
                                    net: Some(net),
                                    clearance: 0.0,
                                    clearance_override: None,
                                    blocks_tracks: true,
                                    blocks_vias: true,
                                    label: format!(
                                        "pair crossover {}{}",
                                        link + 1,
                                        ["a", "b"][end]
                                    ),
                                });
                            }
                        }
                    }
                }
            }
            let fixed = [launch_a.fixed, launch_b.fixed];
            if debug() {
                eprintln!(
                    "diff pair link plan {} / {} link {}: crossed {crossed}, fixed {fixed:?}, layers {:#x}, side {side}, launches {:?} {:?}, crossovers {:?} {:?}",
                    board.nets[pair.positive as usize].name,
                    board.nets[pair.negative as usize].name,
                    link + 1,
                    pair.layers,
                    (launch_a.start, launch_a.at, launch_a.side),
                    (launch_b.start, launch_b.at, launch_b.side),
                    launch_a.crossover,
                    launch_b.crossover
                );
            }
            board.nets.push(core::Net {
                name: link_name(
                    &board.nets[pair.positive as usize].name,
                    &board.nets[pair.negative as usize].name,
                    link + 1,
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
                // Past a crossover the run starts beyond it.
                starts: [&launch_a, &launch_b].map(|found| {
                    found
                        .crossover
                        .as_ref()
                        .map_or(found.start, |crossing| crossing.run_start)
                }),
                ends: stub_ends,
                pads: [
                    [ends[0].0.anchor, ends[0].1.anchor],
                    [ends[1].0.anchor, ends[1].1.anchor],
                ],
                side,
                crossed,
                fixed,
                stubs: [launch_a.stubs, launch_b.stubs],
                crossovers: [launch_a.crossover, launch_b.crossover],
                runs: runs.map(|end| end.map(|run| run.expect("a run terminal per end and net"))),
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
    plan.work_seconds = plan_work_since(tests_before);

    if debug() {
        let [scans, near] = tests_so_far();
        eprintln!(
            "diff pairs plan: {} obstacles scanned, {} tested, {:.4} s, {:.4} s of work modelled (rate scale {:.3})",
            scans - tests_before[0],
            near - tests_before[1],
            started.elapsed().as_secs_f64(),
            plan.work_seconds,
            core::router::reference_time_scale()
        );
    }
    if std::env::var_os("PCB_ROUTER_PAIRS_REACH").is_some() {
        plan_reach_check(board, &plan, config);
    }
    let plan = std::sync::Arc::new(plan);
    board.attachment = core::Attachment {
        data: Some(plan.clone()),
        give_up: give_up_links(config).then_some(plan),
    };
}

/// Whether a route with `config` plans the pairs (`plan_pairs`): the
/// router's `diff_pairs` on, and not the topological engine.
pub(crate) fn pairs_planned(config: &KiCadBoardRouterConfig) -> bool {
    config.diff_pairs_on() && config.engine.as_deref() != Some("topological")
}

/// The modelled work of the pairs' plan on `board` (`plan_pairs`), zero
/// where it has none.
pub(crate) fn planned_work(board: &core::Board) -> f64 {
    plan_of(board).map_or(0.0, |plan| plan.work_seconds)
}

/// The pairs' plan on `board` (`plan_pairs`), if it has one.
fn plan_of(board: &core::Board) -> Option<std::sync::Arc<PairPlan>> {
    board
        .attachment
        .data
        .as_ref()
        .and_then(|any| any.clone().downcast::<PairPlan>().ok())
}

/// A layout's lowering for a stage that routes the board: `lower`, and
/// where the router plans the pairs (`pairs_planned`) their plan, so that
/// every route of a layout (the first route, the placement race's probes,
/// the moves, the outline's sizing) negotiates the pairs' links as the
/// final ladder's attempts do. Returns the board and the plan's work
/// (`planned_work`), which the stage books.
pub(crate) fn lower_for_route(
    pcb: &Expr,
    config: &KiCadBoardRouterConfig,
    connect: bool,
) -> Result<(core::Board, f64), String> {
    let mut board = lower(pcb, config, connect)?.board;
    if !pairs_planned(config) {
        return Ok((board, 0.0));
    }
    plan_pairs(&mut board, config);
    let work = planned_work(&board);
    Ok((board, work))
}

/// Why a link's net was not routed, from the router's diagnostics: a
/// launch with no legal node of the link's class (`dead_pads`: the
/// envelope does not fit there, between pads or in another net's pour); no
/// path even through other nets' copper when the negotiation ended (the
/// link's envelope or via disc finds no way past the fixed copper, or its
/// searches gave up); in conflict then and the hard repair, which treats
/// every other net's copper as fixed, found no room (the negotiation
/// ejected it); or the hard repair did not search for it (the attempt the
/// ladder kept was judged hopeless, which runs no hard repair, or the
/// repair's budget or the attempt's work deadline ran out before it;
/// Diagnostics, `repair`). The verdict's `not_routed` counts them.
fn not_routed_cause(diagnostics: &core::Diagnostics, net: core::NetId) -> &'static str {
    if let Some(entry) = diagnostics.released.iter().find(|entry| entry.net == net) {
        return if entry.reason == core::router::NO_PATH {
            "given up in the negotiation: no path"
        } else {
            "given up in the negotiation: no room"
        };
    }
    if diagnostics.dead_pads.iter().any(|dead| dead.net == net) {
        return "a launch has no legal node";
    }
    match diagnostics.repair.iter().find(|entry| entry.net == net) {
        Some(entry) if entry.skipped == "hopeless" => {
            "no hard repair: the attempt was judged hopeless"
        }
        Some(entry) if !entry.skipped.is_empty() => "the hard repair's budget ran out before it",
        Some(entry) if !entry.complete && !entry.conflicted => "no path in the negotiation",
        Some(_) => "the hard repair found no room",
        None => "lost after the repair",
    }
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

/// A link's path without its hairpins, which the offset tracks cannot
/// follow apart: a point of one layer where the path turns back by more
/// than a right angle goes, and two turns the same way with less than
/// `short` between them become one at the lines' meeting point, where that
/// lies within `short` of both (the first and last two points, the run's
/// lead, stay).
fn despiked(path: &[(Point, usize)], short: f64) -> Vec<(Point, usize)> {
    let mut out = path.to_vec();
    let same_layer = |points: &[(Point, usize)]| points.windows(2).all(|w| w[0].1 == w[1].1);
    loop {
        let count = out.len();
        let spike = (2..count.saturating_sub(2)).find(|&i| {
            let (a, b, c) = (out[i - 1], out[i], out[i + 1]);
            same_layer(&out[i - 1..=i + 1])
                && core::geometry::distance(a.0, b.0) > 1.0e-6
                && core::geometry::distance(b.0, c.0) > 1.0e-6
                && dot(unit(sub(b.0, a.0)), unit(sub(c.0, b.0))) < -1.0e-3
        });
        if let Some(i) = spike {
            out.remove(i);
            continue;
        }
        // A short piece between two turns the same way: A B C D, B to C
        // short.
        let merged = (2..count.saturating_sub(3)).find_map(|i| {
            let (a, b, c, e) = (out[i - 1].0, out[i].0, out[i + 1].0, out[i + 2].0);
            if !same_layer(&out[i - 1..=i + 2]) || core::geometry::distance(b, c) >= short {
                return None;
            }
            let (u1, u2, u3) = (unit(sub(b, a)), unit(sub(c, b)), unit(sub(e, c)));
            let (t1, t2) = (cross(u1, u2), cross(u2, u3));
            if t1 * t2 <= 0.0 || t1.abs() < 1.0e-6 || t2.abs() < 1.0e-6 {
                return None;
            }
            // Where line A B meets line C D.
            let denominator = cross(u1, u3);
            if denominator.abs() < 1.0e-6 {
                return None;
            }
            let along = cross(sub(c, b), u3) / denominator;
            let meet = add(b, scale(u1, along));
            (along > 0.0
                && core::geometry::distance(meet, b) <= short
                && core::geometry::distance(meet, c) <= short
                && dot(sub(meet, a), u1) > 0.0
                && dot(sub(e, meet), u3) > 0.0)
                .then_some((i, meet))
        });
        match merged {
            Some((i, meet)) => {
                out[i].0 = meet;
                out.remove(i + 1);
            }
            None => return out,
        }
    }
}

/// The centrelines `finish_pairs` tries for a link's copper, in order: the
/// route as it is, without its hairpins and short jogs (`despiked`), with
/// its layer changes moved off its turns (`vias_on_straights`), both, then
/// with longer jogs straightened; no two alike. The first is the router's
/// own; the others were not checked by it (`LinkCopper::moved`).
fn centrelines(path: &[(Point, usize)], pair: &Pair) -> Vec<Vec<(Point, usize)>> {
    let shift = 2.0 * pair.via_offset() + pair.width;
    let moved = vias_on_straights(path, shift);
    let d = pair.offset();
    let mut out: Vec<Vec<(Point, usize)>> = Vec::new();
    for candidate in [
        path.to_vec(),
        despiked(path, 2.5 * d),
        moved.clone(),
        despiked(&moved, 2.5 * d),
        despiked(path, 5.0 * d),
        despiked(&moved, 5.0 * d),
        despiked(path, 10.0 * d),
    ] {
        if !out.contains(&candidate) {
            out.push(candidate);
        }
    }
    out
}

/// A link's centreline with each layer change in a turn moved `shift`
/// back along the segment before it (or on along the one after it, where
/// that is longer), so that the two vias stand on a straight piece and the
/// turn is on one layer: at a via in a turn the offset tracks of the two
/// layers meet the vias across (jetson's PCIE1_TX0: a via of one net 0.14
/// mm into the other net's track). Turns closer to the ends than the lead
/// (the first and last two points) and segments too short stay as they
/// are.
fn vias_on_straights(path: &[(Point, usize)], shift: f64) -> Vec<(Point, usize)> {
    let mut out: Vec<(Point, usize)> = Vec::with_capacity(path.len() + 4);
    let count = path.len();
    let mut index = 0;
    while index < count {
        let (point, layer) = path[index];
        let change = index >= 2
            && index + 3 < count
            && path[index + 1].1 != layer
            && core::geometry::distance(path[index + 1].0, point) < 1.0e-6;
        if !change {
            out.push(path[index]);
            index += 1;
            continue;
        }
        let (before, after) = (path[index - 1].0, path[index + 2].0);
        let next_layer = path[index + 1].1;
        let (back, on) = (
            core::geometry::distance(before, point),
            core::geometry::distance(point, after),
        );
        let (incoming, outgoing) = (unit(sub(point, before)), unit(sub(after, point)));
        let straight = back < EPSILON || on < EPSILON || dot(incoming, outgoing) > 0.999;
        if straight || back.max(on) < shift + EPSILON {
            out.push(path[index]);
            index += 1;
            continue;
        }
        if back >= on {
            let moved = sub(point, scale(incoming, shift));
            out.push((moved, layer));
            out.push((moved, next_layer));
            out.push((point, next_layer));
        } else {
            let moved = add(point, scale(outgoing, shift));
            out.push((point, layer));
            out.push((moved, layer));
            out.push((moved, next_layer));
        }
        // Past the via's second point.
        index += 2;
    }
    out
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

/// The spreads `finish_pairs` tries for a link's vias where those of
/// `Pair::via_offset` come too close to the other net (`own_vias_clear`).
const VIA_SPREADS: [f64; 3] = [1.5, 2.0, 3.0];

/// Whether each via of a link's copper keeps from the other net's tracks
/// and vias what the copper check asks between the pair's two nets (the
/// via's copper the gap, below the clearance; its hole the hole clearance;
/// hole to hole).
fn own_vias_clear(copper: &LinkCopper, pair: &Pair) -> bool {
    let near = pair.gap.min(pair.clearance) - EPSILON;
    let (radius, drill) = (pair.via_diameter / 2.0, pair.via_drill / 2.0);
    (0..2).all(|polarity| {
        copper.vias[polarity].iter().all(|&via| {
            copper.segments[1 - polarity].iter().all(|&(_, from, to)| {
                let distance =
                    core::geometry::point_segment_distance(via, from, to) - pair.width / 2.0;
                distance - radius >= near && distance - drill >= pair.hole_clearance - EPSILON
            }) && copper.vias[1 - polarity].iter().all(|&other| {
                let distance = core::geometry::distance(via, other);
                distance - 2.0 * radius >= near
                    && distance - radius - drill >= pair.hole_clearance - EPSILON
                    && distance - 2.0 * drill >= pair.hole_to_hole - EPSILON
            })
        })
    })
}

/// Whether a link's two nets keep from each other what the copper check
/// asks between them (`refused_groups`: the pair's gap where it is below
/// the clearance), tracks on one layer and vias (`own_vias_clear`).
fn own_copper_clear(copper: &LinkCopper, pair: &Pair) -> bool {
    let near = pair.width + pair.gap.min(pair.clearance) - EPSILON;
    own_vias_clear(copper, pair)
        && copper.segments[0].iter().all(|&(layer, a, b)| {
            copper.segments[1]
                .iter()
                .filter(|(other, ..)| *other == layer)
                .all(|&(_, c, d)| core::geometry::segment_segment_distance(a, b, c, d) >= near)
        })
}

/// The copper of a link from its path: the two offset tracks per layer
/// run and a via of each net side by side at each layer change; the
/// positive net on `side` of the run (1 left, -1 right), the vias
/// `spread` times as far from the centreline as `Pair::via_offset`.
fn link_copper(path: &[(Point, usize)], pair: &Pair, side: f64, spread: f64) -> Option<LinkCopper> {
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
    let dv = pair.via_offset() * spread;
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
    /// Meanders, written as KiCad tuning patterns (`generated`, type
    /// `tuning_pattern`: KiCad's gap check skips their tracks).
    patterns: Vec<Meander>,
}

impl PairCopper {
    /// The copper as routes (tracks and vias of no net in particular), for
    /// what keeps off copper (`place_copper_texts`).
    pub(crate) fn routes(&self) -> Vec<core::NetRoute> {
        let mut routes: Vec<core::NetRoute> = self
            .items
            .iter()
            .map(|(_, width, size, drill, segments, vias)| core::NetRoute {
                segments: segments
                    .iter()
                    .map(|&(layer, start, end)| core::Segment {
                        layer,
                        start,
                        end,
                        width: *width,
                    })
                    .collect(),
                vias: vias
                    .iter()
                    .map(|&at| core::Via {
                        at,
                        diameter: *size,
                        drill: *drill,
                        span: None,
                    })
                    .collect(),
            })
            .collect();
        routes.extend(self.patterns.iter().map(|meander| {
            core::NetRoute {
                segments: meander
                    .points
                    .windows(2)
                    .map(|pair| core::Segment {
                        layer: meander.layer,
                        start: pair[0],
                        end: pair[1],
                        width: meander.width,
                    })
                    .collect(),
                vias: Vec::new(),
            }
        }));
        routes
    }
}

/// A kept link's run (`match_skew`): its pair, its copper's items in
/// `PairCopper::items` (positive, negative), their run ranges
/// (`LinkCopper::run`) and the positive net's side.
type KeptRun = (usize, [usize; 2], [std::ops::Range<usize>; 2], f64);

/// A net's copper: tracks (layer, start, end) and vias.
type NetCopper = (Vec<(usize, Point, Point)>, Vec<Point>);

/// A meander on one net of a pair's coupled run (`match_skew`).
#[derive(Clone, Debug)]
struct Meander {
    net: String,
    width: f64,
    layer: usize,
    /// The meander's polyline, from where it leaves the run's track to
    /// where it rejoins it.
    points: Vec<Point>,
    gap: f64,
    amplitude: f64,
    spacing: f64,
    /// The length it adds.
    added: f64,
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
                kind: None,
            }));
        }
    }
    // The meanders: their tracks, grouped as KiCad's tuning patterns (a
    // diff pair skew pattern; KiCad's gap check skips a pattern's tracks).
    for meander in &copper.patterns {
        let layer = layer_names[meander.layer].clone();
        let mut members = Vec::new();
        for pair in meander.points.windows(2) {
            let segment = supplemental_segment_expr(&SupplementalSegment {
                connection: meander.net.clone(),
                start: nanometres(pair[0]),
                end: nanometres(pair[1]),
                width: meander.width,
                layer: layer.clone(),
            });
            if let Some(uuid) = segment
                .child("uuid")
                .and_then(|uuid| uuid.children().get(1))
                .and_then(Expr::atom)
            {
                members.push(Expr::Atom(quote(uuid)));
            }
            items.push(segment);
        }
        let tuned = Tuned {
            net: &meander.net,
            width: meander.width,
            points: &meander.points,
            amplitude: meander.amplitude,
            spacing: meander.spacing,
            added: meander.added,
            tuning: Tuning::PairSkew { gap: meander.gap },
        };
        items.push(tuning_pattern_expr(&tuned, &layer, members));
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
    heights: &[f64],
) -> Option<(core::Board, PairCopper, KiCadDiffPairs)> {
    let plan = plan_of(board)?;
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
    // `PCB_ROUTER_PAIRS_WHY`: the hard repair's record and the dead pads of
    // the pairs' nets, before the links' are named for them.
    let pair_repairs: BTreeMap<core::NetId, core::RepairEntry> = if pairs_why() {
        result
            .diagnostics
            .repair
            .iter()
            .filter(|entry| (entry.net as usize) < plan.nets)
            .map(|entry| (entry.net, entry.clone()))
            .collect()
    } else {
        BTreeMap::new()
    };
    let pair_dead: Vec<String> = if pairs_why() {
        result
            .diagnostics
            .dead_pads
            .iter()
            .map(|dead| format!("{}:{}", dead.net, dead.label))
            .collect()
    } else {
        Vec::new()
    };
    // Why each link not routed was not (`not_routed_cause`), before the
    // diagnostics name the pairs' nets for the links'.
    let causes: BTreeMap<core::NetId, &'static str> = plan
        .links
        .iter()
        .filter(|link| {
            link_status.get(link.net as usize - plan.nets) != Some(&core::NetStatus::Routed)
        })
        .map(|link| (link.net, not_routed_cause(&result.diagnostics, link.net)))
        .collect();
    // The links the negotiation gave up: their pairs' nets routed to the
    // pads without them (no stubs to remove).
    let given_up: BTreeSet<core::NetId> = result
        .diagnostics
        .released
        .iter()
        .map(|entry| entry.net)
        .collect();
    for dead in &mut result.diagnostics.dead_pads {
        dead.net = pair_net(dead.net);
    }
    for entry in &mut result.diagnostics.repair {
        entry.net = pair_net(entry.net);
    }
    for net in &mut result.diagnostics.conflicted {
        *net = pair_net(*net);
    }
    result.diagnostics.conflicted.sort_unstable();
    result.diagnostics.conflicted.dedup();
    // The links' copper.
    let mut candidates: Vec<(usize, usize, LinkCopper)> = Vec::new();
    let mut candidate_links: Vec<&LinkPlan> = Vec::new();
    let mut alternatives: Vec<Vec<LinkCopper>> = Vec::new();
    // Per candidate: the stubs (end, polarity) that do not reach the run.
    let mut lacking: Vec<Vec<(usize, usize)>> = Vec::new();
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
            let cause = causes.get(&plan_link.net).copied().unwrap_or("");
            *report.not_routed.entry(cause.to_string()).or_default() += 1;
            report.given_up += usize::from(given_up.contains(&plan_link.net));
            entries[index].why.push(format!(
                "link {}: the coupled run was not routed ({cause})",
                link + 1
            ));
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
        let build_at = |path: &Vec<(Point, usize)>, spread: f64| {
            link_copper(path, pair, plan_link.side, spread).map(|mut copper| {
                copper.crossed = plan_link.crossed;
                // The jogs from the stubs' ends to the run's tracks.
                for (polarity, segments) in copper.segments.iter_mut().enumerate() {
                    let [from, to] = [plan_link.ends[0][polarity], plan_link.ends[1][polarity]];
                    let length = segments.len();
                    let mut front = 0;
                    // A crossover's copper from the stubs' ends to the run,
                    // else a jog.
                    if let Some(crossing) = &plan_link.crossovers[0] {
                        segments.splice(0..0, crossing.segments[polarity].iter().copied());
                        front += crossing.segments[polarity].len();
                    } else if let Some(&(layer, tip, _)) = segments.first()
                        && core::geometry::distance(tip, from) > 1.0e-6
                    {
                        segments.insert(0, (layer, from, tip));
                        front += 1;
                    }
                    if let Some(crossing) = &plan_link.crossovers[1] {
                        segments.extend(
                            crossing.segments[polarity]
                                .iter()
                                .rev()
                                .map(|&(layer, from, to)| (layer, to, from)),
                        );
                    } else if let Some(&(layer, _, tip)) = segments.last()
                        && core::geometry::distance(tip, to) > 1.0e-6
                    {
                        segments.push((layer, tip, to));
                    }
                    // The stubs the link carries, from the pads.
                    if plan_link.fixed[0]
                        && let Some(&(layer, _, _)) = segments.first()
                    {
                        segments.insert(0, (layer, plan_link.stubs[0][polarity], from));
                        front += 1;
                    }
                    if plan_link.fixed[1]
                        && let Some(&(layer, _, _)) = segments.last()
                    {
                        segments.push((layer, to, plan_link.stubs[1][polarity]));
                    }
                    copper.run[polarity] = front..front + length;
                }
                for crossing in plan_link.crossovers.iter().flatten() {
                    for polarity in 0..2 {
                        copper.vias[polarity].extend(crossing.vias[polarity].iter().copied());
                    }
                    copper.crossovers += 1;
                }
                copper
            })
        };
        // Where a via of one net comes too close to the other net's copper
        // (a layer change in a sharp turn), the vias further out.
        let build = |path: &Vec<(Point, usize)>| {
            let first = build_at(path, 1.0);
            if first
                .as_ref()
                .is_none_or(|copper| own_vias_clear(copper, pair))
            {
                return first;
            }
            VIA_SPREADS
                .iter()
                .filter_map(|&spread| build_at(path, spread))
                .find(|copper| own_vias_clear(copper, pair))
                .or(first)
        };
        // The route as it is, without its hairpins (the route turning back
        // by more than a right angle, which the offset tracks cannot follow
        // apart) and short jogs (`despiked`), with its layer changes moved
        // off its turns (`vias_on_straights`: a via pair placed in a turn
        // lands on the partner's track of the other layer), both, then with
        // longer jogs straightened (`centrelines`): the route as it is is
        // the copper, the others whose two nets keep the pair's gap from
        // each other (`own_copper_clear`) the alternatives, in turn, where
        // that is refused.
        let centrelines = path
            .as_ref()
            .map(|path| centrelines(path, pair))
            .unwrap_or_default();
        // The route as it is with its vias where the router put them
        // first; every other copper (the vias spread out, `build`, or
        // another centreline) moved.
        let mut built: Vec<LinkCopper> = Vec::new();
        for (position, line) in centrelines.iter().enumerate() {
            if position == 0
                && let Some(copper) = build_at(line, 1.0)
            {
                built.push(copper);
            }
            if let Some(mut copper) = build(line) {
                copper.moved = true;
                if position > 0
                    || built.first().is_none_or(|first| {
                        first.segments != copper.segments || first.vias != copper.vias
                    })
                {
                    built.push(copper);
                }
            }
        }
        // The route as it is first, as before (where it holds, nothing
        // changes); then the moved ones whose nets keep their gap.
        let mut built = built.into_iter();
        let first = built.next();
        let (copper, alternative) = match first {
            Some(copper) if !copper.moved => (
                Some(copper),
                built
                    .filter(|copper| own_copper_clear(copper, pair))
                    .collect::<Vec<LinkCopper>>(),
            ),
            first => {
                let mut clean: Vec<LinkCopper> = first
                    .into_iter()
                    .chain(built)
                    .filter(|copper| own_copper_clear(copper, pair))
                    .collect();
                let copper = (!clean.is_empty()).then(|| clean.remove(0));
                (copper, clean)
            }
        };
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
        let missing: Vec<(usize, usize)> = (0..2)
            .filter(|&end| !plan_link.fixed[end])
            .flat_map(|end| [(end, 0), (end, 1)])
            .filter(|&(end, polarity)| {
                let net = [pair.positive, pair.negative][polarity];
                !stub_joined(
                    &restored,
                    &result.routes[net as usize],
                    net,
                    (
                        plan_link.pads[end][polarity],
                        plan_link.pads[1 - end][polarity],
                    ),
                    plan_link.stub_end(end, polarity, pair.width),
                    reach(net),
                )
            })
            .collect();
        let stubs_joined = missing.is_empty();
        if copper.is_some() && !stubs_joined && debug() {
            eprintln!(
                "diff pair link {} stub fails: crossed {}, fixed {:?}",
                board.nets[plan_link.net as usize].name, plan_link.crossed, plan_link.fixed
            );
        }
        if copper.is_some() && !stubs_joined && pairs_why() {
            for end in (0..2).filter(|&end| !plan_link.fixed[end]) {
                for (polarity, net) in [(0, pair.positive), (1, pair.negative)] {
                    let line = stub_why(
                        &restored,
                        result,
                        plan_link,
                        end,
                        polarity,
                        net,
                        (pair.width, reach(net)),
                    );
                    if line.is_some() && result.status[net as usize] != core::NetStatus::Routed {
                        let blocked = stub_blocked(
                            board,
                            (&result.routes, &link_routes),
                            plan.obstacles,
                            plan_link,
                            (end, polarity),
                            [pair.positive, pair.negative],
                            config,
                        );
                        let repair = pair_repairs.get(&net).map_or_else(
                            || "no repair entry".to_string(),
                            |entry| {
                                format!(
                                    "complete {}, conflicted {}, skipped '{}'",
                                    entry.complete, entry.conflicted, entry.skipped
                                )
                            },
                        );
                        let dead: Vec<&String> = pair_dead
                            .iter()
                            .filter(|dead| {
                                dead.starts_with(&format!("{net}:"))
                                    || dead.starts_with(&format!("{}:", plan_link.net))
                            })
                            .collect();
                        eprintln!(
                            "pairs why: open {} link {} end {} ({} end): {blocked}; repair: {repair}; dead pads {dead:?}",
                            board.nets[net as usize].name,
                            link + 1,
                            ["a", "b"][end],
                            if end_crossed(plan_link, end) {
                                "crossed"
                            } else {
                                "straight"
                            },
                        );
                    }
                    if let Some(line) = line {
                        eprintln!(
                            "pairs why: stub {} link {} end {} ({}): {line}",
                            board.nets[net as usize].name,
                            link + 1,
                            ["a", "b"][end],
                            if plan_link.crossed {
                                "crossed"
                            } else {
                                "straight"
                            },
                        );
                    }
                }
            }
        }
        // Its copper is checked with the others; the stubs it lacks are
        // routed after that (`repair_stubs`).
        match copper {
            Some(mut copper) => {
                // A link whose stubs are still to route gives way too.
                copper.moved |= !missing.is_empty();
                candidates.push((index, link, copper));
                alternatives.push(alternative);
                lacking.push(missing);
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
            run: copper.run.clone(),
            moved: copper.moved,
        })
        .collect();
    let mut refused = refused_groups(&restored, &result.routes, &plan.pairs, &groups);
    // A refused link with another clean copper (`centrelines`) is checked
    // again with it, beside every other link's, until one holds or none is
    // left.
    loop {
        let swapped: Vec<bool> = refused
            .iter()
            .zip(&alternatives)
            .map(|(why, other)| why.is_some() && !other.is_empty())
            .collect();
        if !swapped.contains(&true) {
            break;
        }
        let second: Vec<Group> = candidates
            .iter()
            .zip(&alternatives)
            .zip(&swapped)
            .map(|(((index, _, copper), other), &swap)| {
                let copper = if swap { &other[0] } else { copper };
                Group {
                    pair: *index,
                    segments: copper.segments.clone(),
                    vias: copper.vias.clone(),
                    run: copper.run.clone(),
                    moved: copper.moved,
                }
            })
            .collect();
        let again = refused_groups(&restored, &result.routes, &plan.pairs, &second);
        for position in 0..candidates.len() {
            if !swapped[position] {
                continue;
            }
            let option = alternatives[position].remove(0);
            if again[position].is_none() {
                refused[position] = None;
                candidates[position].2 = option;
            }
        }
    }
    // The stubs a link whose copper holds lacks: routed from the pad to the
    // stub's end with everything else kept (`repair_stubs`); where one does
    // not route, the link falls back as before.
    let mut repaired: BTreeMap<usize, Vec<RoutedStub>> = BTreeMap::new();
    if (0..candidates.len())
        .any(|position| refused[position].is_none() && !lacking[position].is_empty())
    {
        let (done, work) = repair_stubs(
            &restored,
            result,
            &plan.pairs,
            (&candidates, &candidate_links),
            &lacking,
            &refused,
            config,
        );
        report.work_seconds += work;
        for position in 0..candidates.len() {
            if refused[position].is_some() || lacking[position].is_empty() {
                continue;
            }
            match done.get(&position) {
                Some(stubs) => {
                    // A pair's net left open by the attempt: the fallback
                    // completes it around the link.
                    let pair = &plan.pairs[candidates[position].0];
                    if [pair.positive, pair.negative]
                        .iter()
                        .any(|&net| result.status[net as usize] != core::NetStatus::Routed)
                    {
                        open_pairs.insert(candidates[position].0);
                    }
                    repaired.insert(position, stubs.clone());
                }
                None => refused[position] = Some("a pad's stub does not reach the run".into()),
            }
        }
    }
    report.stubs_routed += repaired.values().map(Vec::len).sum::<usize>();
    let mut copper_out = PairCopper {
        items: Vec::new(),
        patterns: Vec::new(),
    };
    // Per kept link: its pair, its copper's items (positive, negative) and
    // run ranges, and the positive net's side.
    let mut kept_runs: Vec<KeptRun> = Vec::new();
    let mut kept_links: Vec<&LinkPlan> = Vec::new();
    for (position, (index, link, copper)) in candidates.into_iter().enumerate() {
        if let Some(why) = &refused[position] {
            entries[index].why.push(format!("link {}: {why}", link + 1));
            open_pairs.insert(index);
            continue;
        }
        kept_links.push(candidate_links[position]);
        kept_runs.push((
            index,
            [copper_out.items.len(), copper_out.items.len() + 1],
            copper.run.clone(),
            candidate_links[position].side,
        ));
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
                // Its drill keeps the hole clearance from other nets'
                // copper (the fallback routes beside it).
                if !crossovers() {
                    continue;
                }
                restored.obstacles.push(core::Obstacle {
                    shape: core::Shape::Circle {
                        center: at,
                        radius: pair.via_drill / 2.0,
                    },
                    layers: restored.all_layers(),
                    kind: core::ObstacleKind::Hole,
                    net: Some(net),
                    clearance: 0.0,
                    clearance_override: None,
                    blocks_tracks: true,
                    blocks_vias: true,
                    label: "existing via hole".into(),
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
        report.crossovers += copper.crossovers;
    }
    // The stubs of the links not kept go (the fallback routes the pads
    // anew).
    let kept_nets: BTreeSet<core::NetId> = kept_links.iter().map(|link| link.net).collect();
    for plan_link in plan
        .links
        .iter()
        .filter(|link| !kept_nets.contains(&link.net) && !given_up.contains(&link.net))
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
    // A repaired stub's pad keeps one way to the run: its old route
    // elsewhere (to the link's other end, through other pads) goes where
    // the stub makes it a loop (`cut_old_connection`).
    for stubs in repaired.values() {
        for (net, pad, new) in stubs {
            cut_old_connection(
                &restored,
                &mut result.routes[*net as usize],
                *net,
                *pad,
                new,
            );
        }
    }
    // The kept links' stubs straight where that holds (the router's run
    // parallel out of the pads, which KiCad takes for a coupled piece at
    // the pads' pitch).
    for plan_link in kept_links {
        straighten_stubs(&restored, result, &plan.pairs, plan_link);
        join_stub_ends(result, &plan.pairs, plan_link);
    }
    // A pair whose links not kept were all given up in the negotiation,
    // and whose nets the attempt routed without them, needs no fallback.
    open_pairs.retain(|&index| {
        let pair = &plan.pairs[index];
        let routed = [pair.positive, pair.negative]
            .iter()
            .all(|&net| result.status[net as usize] == core::NetStatus::Routed);
        !routed
            || plan.links.iter().any(|link| {
                link.pair == index
                    && !kept_nets.contains(&link.net)
                    && !given_up.contains(&link.net)
            })
    });
    // The fallback: the pairs' nets with a connection a link left open,
    // routed with everything else kept.
    if !open_pairs.is_empty() {
        report.work_seconds = route_fallback(&restored, result, &plan.pairs, &open_pairs, config);
        // What is left of the failed links' stubs that joins nothing.
        for plan_link in plan
            .links
            .iter()
            .filter(|link| !kept_nets.contains(&link.net) && !given_up.contains(&link.net))
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
    // The skew: per pair with a kept link, the shorter net's meander.
    for (index, entry) in entries.iter_mut().enumerate() {
        if skew_matching() && kept_runs.iter().any(|(owner, ..)| *owner == index) {
            match_skew(
                &mut restored,
                result,
                &plan.pairs,
                index,
                &kept_runs,
                &mut copper_out,
                entry,
                heights,
            );
            report.meanders += (entry.meander_mm > 0.0) as usize;
        }
    }
    // The return vias beside the kept links' vias.
    if return_vias_on(config)
        && let Some(reference) = return_net(&restored, config)
    {
        report.return_net = restored.nets[reference as usize].name.clone();
        place_return_vias(
            &restored,
            result,
            &plan.pairs,
            &kept_runs,
            &copper_out,
            &mut entries,
            reference,
            config,
        );
    } else {
        for &(index, items, ..) in &kept_runs {
            entries[index].pair_vias += items
                .iter()
                .map(|&item| copper_out.items[item].5.len())
                .sum::<usize>();
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
        report.pair_vias += entry.pair_vias;
        report.return_vias += entry.return_vias;
        if entry.links > 0 && entry.links_coupled == entry.links {
            report.coupled += 1;
        } else if entry.links_coupled > 0 {
            report.partly_coupled += 1;
        } else {
            report.fallen_back += 1;
        }
    }
    report.pairs = entries;
    if pairs_why() {
        let count = |text: &str| {
            report
                .pairs
                .iter()
                .flat_map(|pair| &pair.why)
                .filter(|why| why.contains(text))
                .count()
        };
        eprintln!(
            "pairs why: written: {} of {} links coupled, pairs {} / {} / {}, stubs routed {}, a pad's stub {}, copper {}",
            report.links_coupled,
            report.links,
            report.coupled,
            report.partly_coupled,
            report.fallen_back,
            report.stubs_routed,
            count("stub does not reach"),
            count("its copper is")
        );
    }
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

/// Return vias: the ring of a return via at most this far from the ring
/// of the pair's via it stands beside (millimetres), searched outward in
/// `RETURN_STEP`s from touching, at `RETURN_ANGLES` from the direction
/// away from the via's partner (degrees).
const RETURN_GAP: f64 = 0.5;
const RETURN_STEP: f64 = 0.05;
const RETURN_ANGLES: [f64; 9] = [0.0, 30.0, -30.0, 60.0, -60.0, 90.0, -90.0, 120.0, -120.0];

/// Whether the kept links' vias get return vias (`place_return_vias`):
/// the settings' `diff_pair_return_vias`, else
/// `PCB_ROUTER_DIFF_PAIRS_RETURN_VIAS` (`0` or `1`), else on.
fn return_vias_on(config: &KiCadBoardRouterConfig) -> bool {
    config.diff_pair_return_vias.unwrap_or_else(|| {
        std::env::var("PCB_ROUTER_DIFF_PAIRS_RETURN_VIAS").map_or(true, |value| value != "0")
    })
}

/// The return vias' net: the settings' `diff_pair_return_net` (a net of
/// the board by name), else the net named `GND` (past a sheet path), else
/// of the nets whose name ends in `GND` the one with the most pads; only a
/// net with a pour (`Board::planes`), which the vias join.
fn return_net(board: &core::Board, config: &KiCadBoardRouterConfig) -> Option<core::NetId> {
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
fn place_return_vias(
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
fn joins_pour(
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
fn return_via_clear(
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
fn match_skew(
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
/// joins copper), and through the net's other pads and kept copper on the
/// way (a stub that passes an ESD diode's pad or a series part's pad on
/// its way to the run: right_io's ETH_MDI pairs through U501 and U502),
/// but not through the pad at `avoid` (the link's other end: a route from
/// the pad to the other end's pad and on to this end's stub would make
/// the run a loop beside it). A link whose stubs do not reach both its
/// ends would leave its run an island (the router may end a pad's route
/// on the other end of the link, merged into one terminal, or leave it
/// open).
fn stub_joined(
    board: &core::Board,
    route: &core::NetRoute,
    net: core::NetId,
    (pad, avoid): (Point, Point),
    lead: (Point, Point),
    reach: f64,
) -> bool {
    let own: Vec<&core::Obstacle> = board
        .obstacles
        .iter()
        .filter(|obstacle| {
            obstacle.net == Some(net)
                && obstacle.kind == core::ObstacleKind::Copper
                && obstacle.shape.distance_to_point(avoid) > 1.0e-6
        })
        .collect();
    let starts: Vec<bool> = own
        .iter()
        .map(|obstacle| obstacle.shape.distance_to_point(pad) <= 1.0e-6)
        .collect();
    let segments = &route.segments;
    let (count, vias) = (segments.len(), route.vias.len());
    // Items: the segments, the vias, then the net's own copper.
    let touches_copper = |item: usize, obstacle: &core::Obstacle| {
        if item < count {
            let segment = &segments[item];
            obstacle.layers & (1 << segment.layer) != 0
                && [segment.start, segment.end].into_iter().any(|end| {
                    obstacle.shape.distance_to_point(end) <= segment.width / 2.0 + 1.0e-6
                })
        } else {
            let via = &route.vias[item - count];
            obstacle.shape.distance_to_point(via.at) <= via.diameter / 2.0 + 1.0e-6
        }
    };
    let joined = |a: usize, b: usize| {
        let (a, b) = (a.min(b), a.max(b));
        if b >= count + vias {
            return a < count + vias && touches_copper(a, own[b - count - vias]);
        }
        match (a < count, b < count) {
            (true, true) => {
                let (x, y) = (&segments[a], &segments[b]);
                let keep = (x.width + y.width) / 2.0 + 1.0e-6;
                x.layer == y.layer
                    && ([x.start, x.end].into_iter().any(|end| {
                        core::geometry::point_segment_distance(end, y.start, y.end) <= keep
                    }) || [y.start, y.end].into_iter().any(|end| {
                        core::geometry::point_segment_distance(end, x.start, x.end) <= keep
                    }))
            }
            (true, false) => {
                let (segment, via) = (&segments[a], &route.vias[b - count]);
                core::geometry::point_segment_distance(via.at, segment.start, segment.end)
                    <= (segment.width + via.diameter) / 2.0 + 1.0e-6
            }
            _ => false,
        }
    };
    let at_end = |item: usize| {
        item < count
            && [segments[item].start, segments[item].end]
                .into_iter()
                .any(|end| {
                    core::geometry::point_segment_distance(end, lead.0, lead.1) <= reach + 1.0e-6
                })
    };
    let total = count + vias + own.len();
    let mut seen = vec![false; total];
    let mut queue: Vec<usize> = (0..own.len())
        .filter(|&index| starts[index])
        .map(|index| count + vias + index)
        .collect();
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
            "diff pair stub of {} from {pad:?} does not reach {lead:?} (from the pad {} of {} items, nearest track end {nearest:.3} mm, reach {reach:.3})",
            board.nets[net as usize].name,
            seen.iter().filter(|seen| **seen).count(),
            total
        );
    }
    false
}

/// Whether the pads' order at `end` of a link crosses the stubs' ends'
/// (the stubs must cross there).
fn end_crossed(link: &LinkPlan, end: usize) -> bool {
    let outward = sub(link.launches[end], link.starts[end]);
    let pads = cross(outward, sub(link.pads[end][0], link.pads[end][1]));
    let ends = cross(outward, sub(link.ends[end][0], link.ends[end][1]));
    pads * ends < 0.0
}

/// What a diagnosis trial takes off the board (`trial_reaches`): by the
/// obstacle's index and the obstacle.
type Gone<'a> = dyn Fn(usize, &core::Obstacle) -> bool + 'a;

/// A trial for the diagnosis (`PCB_ROUTER_PAIRS_WHY`): whether `net`
/// routes from `terminal` to `target` alone on `board` (other nets with no
/// terminals, no pours), in a window around the two where it lies inside
/// the board, with the obstacles `gone` takes moved off the board.
fn trial_reaches(
    board: &core::Board,
    net: core::NetId,
    (terminal, target): (&core::Terminal, &core::Terminal),
    gone: &Gone<'_>,
    config: &KiCadBoardRouterConfig,
) -> bool {
    let mut trial = board.clone();
    trial.planes.clear();
    for other in &mut trial.nets {
        other.terminals.clear();
    }
    trial.merged_pads.retain(|merged| merged.net != net);
    trial.terminal_copper.retain(|extra| extra.net != net);
    trial.nets[net as usize].terminals = vec![terminal.clone(), target.clone()];
    trial.attachment = core::Attachment::default();
    let (pad, to) = (terminal.anchor, target.anchor);
    let margin = 3.0;
    let (low, high) = (
        [pad[0].min(to[0]) - margin, pad[1].min(to[1]) - margin],
        [pad[0].max(to[0]) + margin, pad[1].max(to[1]) + margin],
    );
    let window = vec![low, [high[0], low[1]], high, [low[0], high[1]]];
    if window
        .iter()
        .all(|&corner| core::geometry::point_in_polygon(corner, &board.outline))
    {
        trial.outline = window;
    }
    for (index, obstacle) in trial.obstacles.iter_mut().enumerate() {
        if obstacle.net != Some(net) && gone(index, obstacle) {
            obstacle.shape = core::Shape::Circle {
                center: [1.0e6, 1.0e6],
                radius: 0.01,
            };
        }
    }
    let mut router_config = core_config(config);
    router_config.verbose = false;
    router_config.deadline_work = Some(2.0);
    router_config.run_work = None;
    core::route(&trial, &router_config).status[net as usize] == core::NetStatus::Routed
}

/// `PCB_ROUTER_PAIRS_WHY`: what keeps the stub of `polarity` at `end` of
/// a link from its pad to its stub's end in the attempt's state: the pad
/// routed to the end alone on the planned board with the attempt's routes
/// (`routes`: the board's nets, then the links'), and again with what might
/// block it taken away in turn: the other nets' routes, the link's own
/// route, the partner's routes, the plan's copper (the leads, the stubs'
/// ends, the carried stubs). The first that frees it is named.
fn stub_blocked(
    board: &core::Board,
    (routes, link_routes): (&[core::NetRoute], &[core::NetRoute]),
    planned_from: usize,
    link: &LinkPlan,
    (end, polarity): (usize, usize),
    nets: [core::NetId; 2],
    config: &KiCadBoardRouterConfig,
) -> String {
    let (net, partner) = (nets[polarity], nets[1 - polarity]);
    let pad = link.pads[end][polarity];
    let Some(terminal) = board.nets[net as usize]
        .terminals
        .iter()
        .find(|terminal| core::geometry::distance(terminal.anchor, pad) < 1.0e-6)
        .cloned()
    else {
        return "the pad is no terminal".into();
    };
    let target = link.runs[end][polarity].clone();
    let mut all: Vec<core::NetRoute> = routes.to_vec();
    all.extend(link_routes.iter().cloned());
    let base_count = board.obstacles.len();
    let trial = with_routes(board, &all);
    let kind = |index: usize, obstacle: &core::Obstacle| -> usize {
        let route = index >= base_count;
        if route && obstacle.net == Some(link.net) {
            2
        } else if route && obstacle.net == Some(partner) {
            3
        } else if route {
            1
        } else if index >= planned_from {
            4
        } else {
            0
        }
    };
    let names = [
        "routes with the attempt's copper",
        "freed without the other nets' routes",
        "freed without the link's route",
        "freed without the partner's routes",
        "freed without the plan's copper",
    ];
    for (step, name) in names.iter().enumerate() {
        let gone = |index: usize, obstacle: &core::Obstacle| {
            let k = kind(index, obstacle);
            k != 0 && k <= step
        };
        if trial_reaches(&trial, net, (&terminal, &target), &gone, config) {
            return (*name).to_string();
        }
    }
    "not even on the bare board".into()
}

/// `PCB_ROUTER_PAIRS_REACH`: per stub the router is to route (a link end
/// whose stubs the link does not carry), whether its pad reaches its stub's
/// end on the planned board alone (no route yet: the pads, the keepouts and
/// the plan's copper), and where it does not, what frees it: the link's own
/// lead, its partner's copper of the link (the stub's end, a crossover),
/// the other links' plan copper; else nothing does.
fn plan_reach_check(board: &core::Board, plan: &PairPlan, config: &KiCadBoardRouterConfig) {
    for link in &plan.links {
        let pair = &plan.pairs[link.pair];
        let nets = [pair.positive, pair.negative];
        for end in (0..2).filter(|&end| !link.fixed[end]) {
            for polarity in 0..2 {
                let (net, partner) = (nets[polarity], nets[1 - polarity]);
                let pad = link.pads[end][polarity];
                let terminal = board.nets[net as usize]
                    .terminals
                    .iter()
                    .find(|terminal| core::geometry::distance(terminal.anchor, pad) < 1.0e-6)
                    .cloned();
                let Some(terminal) = terminal else {
                    continue;
                };
                let target = &link.runs[end][polarity];
                let planned = |index: usize| index >= plan.obstacles;
                let steps: [(&str, &Gone<'_>); 5] = [
                    ("reaches", &|_, _| false),
                    ("freed without the link's lead", &|index, obstacle| {
                        planned(index) && obstacle.net == Some(link.net)
                    }),
                    (
                        "freed without the partner's plan copper",
                        &|index, obstacle| {
                            planned(index)
                                && (obstacle.net == Some(link.net) || obstacle.net == Some(partner))
                        },
                    ),
                    ("freed without the other links' plan copper", &|index, _| {
                        planned(index)
                    }),
                    (
                        "freed without the board's other pads",
                        &|index, obstacle| planned(index) || obstacle.net != Some(partner),
                    ),
                ];
                let verdict = steps
                    .iter()
                    .find(|(_, gone)| trial_reaches(board, net, (&terminal, target), *gone, config))
                    .map_or("not even alone with its partner", |(name, _)| *name);
                // The plan's own check (`stub_reaches`) on the planned board.
                let near = obstacles_near(
                    board,
                    pad,
                    core::geometry::distance(pad, target.anchor) + REACH_MARGIN + 1.0,
                );
                let grid = stub_reaches(
                    board,
                    &near,
                    pair,
                    (&terminal, polarity),
                    target.anchor,
                    target.layers,
                    &[],
                );
                let start = link.starts[end];
                let outward = unit(sub(link.launches[end], start));
                // What lies near the straight line from the pad to the end.
                let mut near: Vec<String> = Vec::new();
                if !verdict.starts_with("reaches") {
                    let layer = target.layers;
                    for obstacle in &board.obstacles {
                        if obstacle.net == Some(net) || obstacle.layers & layer == 0 {
                            continue;
                        }
                        let gap = obstacle
                            .shape
                            .distance_to_segment(link.stubs[end][polarity], target.anchor)
                            - pair.width / 2.0;
                        if gap < pair.clearance + 0.05 {
                            near.push(format!(
                                "{} [{}] {:.3}",
                                obstacle.label,
                                obstacle
                                    .net
                                    .map_or("-".to_string(), |owner| if owner == partner {
                                        "partner".to_string()
                                    } else if owner == link.net {
                                        "link".to_string()
                                    } else {
                                        board
                                            .nets
                                            .get(owner as usize)
                                            .map_or("?".into(), |n| n.name.clone())
                                    }),
                                gap
                            ));
                        }
                    }
                }
                let local = |point: Point| {
                    let v = sub(point, start);
                    format!("({:.2}, {:.2})", dot(v, outward), cross(outward, v))
                };
                eprintln!(
                    "pairs reach: {} link {} end {} ({} end, {:.3} mm): {verdict}; grid {}; along/across the run from its start: pad {}, partner's pad {}, stub end {}, partner's end {}; near the straight stub: {near:?}",
                    board.nets[net as usize].name,
                    link.link + 1,
                    ["a", "b"][end],
                    if end_crossed(link, end) {
                        "crossed"
                    } else {
                        "straight"
                    },
                    core::geometry::distance(pad, target.anchor),
                    if grid { "reaches" } else { "does not" },
                    local(pad),
                    local(link.pads[end][1 - polarity]),
                    local(target.anchor),
                    local(link.ends[end][1 - polarity]),
                );
            }
        }
    }
}

/// `PCB_ROUTER_PAIRS_WHY`: why the stub of `net` (`polarity`) at `end` of
/// a link does not reach its end of the run (`None`: it does): the pad,
/// the stub's end, the run's start and direction, and what the copper the
/// pad's route belongs to reaches (the pads of the net joined through, the
/// link's other end), and what the stub's end is joined to.
fn stub_why(
    board: &core::Board,
    result: &core::RoutingResult,
    plan_link: &LinkPlan,
    end: usize,
    polarity: usize,
    net: core::NetId,
    (width, reach): (f64, f64),
) -> Option<String> {
    let route = &result.routes[net as usize];
    let pad = plan_link.pads[end][polarity];
    let lead = plan_link.stub_end(end, polarity, width);
    let other_lead = plan_link.stub_end(1 - end, polarity, width);
    if stub_joined(
        board,
        route,
        net,
        (pad, plan_link.pads[1 - end][polarity]),
        lead,
        reach,
    ) {
        return None;
    }
    let pads: Vec<&core::Obstacle> = board
        .obstacles
        .iter()
        .filter(|obstacle| obstacle.net == Some(net) && obstacle.kind == core::ObstacleKind::Copper)
        .collect();
    let (segments, vias) = (&route.segments, &route.vias);
    let (count, total) = (segments.len(), segments.len() + vias.len() + pads.len());
    let touches = |a: usize, b: usize| -> bool {
        let (a, b) = (a.min(b), a.max(b));
        let segment = |index: usize| &segments[index];
        if b < count {
            let (x, y) = (segment(a), segment(b));
            let keep = (x.width + y.width) / 2.0 + 1.0e-6;
            x.layer == y.layer
                && core::geometry::segment_segment_distance(x.start, x.end, y.start, y.end) <= keep
        } else if b < count + vias.len() {
            let via = &vias[b - count];
            if a < count {
                let x = segment(a);
                core::geometry::point_segment_distance(via.at, x.start, x.end)
                    <= (x.width + via.diameter) / 2.0 + 1.0e-6
            } else {
                core::geometry::distance(via.at, vias[a - count].at) < 1.0e-6
            }
        } else {
            let obstacle = pads[b - count - vias.len()];
            if a < count {
                let x = segment(a);
                obstacle.layers & (1 << x.layer) != 0
                    && obstacle.shape.distance_to_segment(x.start, x.end) <= x.width / 2.0 + 1.0e-6
            } else if a < count + vias.len() {
                let via = &vias[a - count];
                obstacle.shape.distance_to_point(via.at) <= via.diameter / 2.0 + 1.0e-6
            } else {
                false
            }
        }
    };
    let mut parent: Vec<usize> = (0..total).collect();
    for a in 0..total {
        for b in a + 1..total {
            if touches(a, b) {
                let (x, y) = (root(&mut parent, a), root(&mut parent, b));
                if x != y {
                    parent[x.max(y)] = x.min(y);
                }
            }
        }
    }
    let at = |item: usize, lead: (Point, Point)| {
        item < count
            && [segments[item].start, segments[item].end]
                .into_iter()
                .any(|point| {
                    core::geometry::point_segment_distance(point, lead.0, lead.1) <= reach + 1.0e-6
                })
    };
    let pad_items: Vec<usize> = (0..pads.len())
        .filter(|&index| pads[index].shape.distance_to_point(pad) <= 1.0e-6)
        .map(|index| count + vias.len() + index)
        .collect();
    let component = |item: usize, parent: &mut Vec<usize>| {
        let home = root(parent, item);
        (0..total)
            .filter(|&other| root(parent, other) == home)
            .collect::<Vec<usize>>()
    };
    let describe = |items: &[usize]| {
        let mut labels: Vec<String> = items
            .iter()
            .filter(|&&item| item >= count + vias.len())
            .map(|&item| pads[item - count - vias.len()].label.clone())
            .collect();
        labels.dedup();
        let tracks = items.iter().filter(|&&item| item < count).count();
        let via_count = items
            .iter()
            .filter(|&&item| item >= count && item < count + vias.len())
            .count();
        let own_end = items.iter().any(|&item| at(item, lead));
        let other_end = items.iter().any(|&item| at(item, other_lead));
        format!(
            "{tracks} tracks, {via_count} vias, pads [{}]{}{}",
            labels.join(" "),
            if own_end { ", its end" } else { "" },
            if other_end {
                ", the link's other end"
            } else {
                ""
            }
        )
    };
    let from_pad = pad_items
        .first()
        .map(|&item| component(item, &mut parent))
        .unwrap_or_default();
    let at_end: Vec<usize> = (0..count).filter(|&item| at(item, lead)).collect();
    let end_side = at_end
        .first()
        .map(|&item| describe(&component(item, &mut parent)))
        .unwrap_or_else(|| "no track".into());
    let nearest = segments
        .iter()
        .flat_map(|segment| [segment.start, segment.end])
        .map(|point| core::geometry::point_segment_distance(point, lead.0, lead.1))
        .fold(f64::INFINITY, f64::min);
    let start = plan_link.starts[end];
    let outward = unit(sub(plan_link.launches[end], start));
    let towards = sub(lead.0, pad);
    Some(format!(
        "status {:?}; pad ({:.3}, {:.3}) -> stub end ({:.3}, {:.3}) {:.3} mm, {:.3} along the run's direction ({:.3}, {:.3}) and {:.3} across; run start ({:.3}, {:.3}); the pad's copper: {}; the end's: {}; nearest track end {:.3} mm (reach {:.3})",
        result.status[net as usize],
        pad[0],
        pad[1],
        lead.0[0],
        lead.0[1],
        norm(towards),
        dot(towards, outward),
        outward[0],
        outward[1],
        cross(outward, towards),
        start[0],
        start[1],
        if pad_items.is_empty() {
            "no pad found".into()
        } else {
            describe(&from_pad)
        },
        end_side,
        nearest,
        reach
    ))
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
                    run: Default::default(),
                    moved: false,
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
fn keep_planes_off(
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
                blocks_tracks: true,
                blocks_vias: false,
                label: "exclusive plane".into(),
            })
            .collect();
        board.obstacles.extend(keepouts);
    }
    board.planes.retain(|plane| nets.contains(&plane.net));
}

/// A stub routed after the attempt (`repair_stubs`): its net, its pad and
/// its tracks.
type RoutedStub = (core::NetId, Point, Vec<core::Segment>);

/// The stubs a link lacks (`lacking`: per candidate, (end, polarity)),
/// for each candidate whose copper holds (`refused` none): each routed
/// from its pad to its stub's end on a board where every route and every
/// such candidate's copper is kept, the pair's net with those two
/// terminals only (its other terminals and its merged pads out: a route
/// from the pad that ends at the link's other end, or at another pad, is
/// what left the stub out). A net lacking two stubs routes them in two
/// rounds. Returns per candidate whose every stub routed and now reaches
/// its end (`stub_joined`): the stubs added (net, pad, segments), already
/// in `result`; the rest's added copper is taken out again. And the work.
fn repair_stubs(
    board: &core::Board,
    result: &mut core::RoutingResult,
    pairs: &[Pair],
    (candidates, links): (&[(usize, usize, LinkCopper)], &[&LinkPlan]),
    lacking: &[Vec<(usize, usize)>],
    refused: &[Option<String>],
    config: &KiCadBoardRouterConfig,
) -> (BTreeMap<usize, Vec<RoutedStub>>, f64) {
    let wanted: Vec<usize> = (0..candidates.len())
        .filter(|&position| refused[position].is_none() && !lacking[position].is_empty())
        .collect();
    let mut base = with_routes(board, &result.routes);
    // Every candidate's copper whose check held, as copper of its nets.
    for (position, (index, _, copper)) in candidates.iter().enumerate() {
        if refused[position].is_some() {
            continue;
        }
        let pair = &pairs[*index];
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
                for (kind, radius, label) in [
                    (
                        core::ObstacleKind::Copper,
                        pair.via_diameter / 2.0,
                        "existing via",
                    ),
                    (
                        core::ObstacleKind::Hole,
                        pair.via_drill / 2.0,
                        "existing via hole",
                    ),
                ] {
                    base.obstacles.push(core::Obstacle {
                        shape: core::Shape::Circle { center: at, radius },
                        layers: board.all_layers(),
                        kind,
                        net: Some(net),
                        clearance: 0.0,
                        clearance_override: None,
                        blocks_tracks: true,
                        blocks_vias: true,
                        label: label.into(),
                    });
                }
            }
        }
    }
    let mut todo: Vec<(usize, usize, usize)> = wanted
        .iter()
        .flat_map(|&position| {
            lacking[position]
                .iter()
                .map(move |&(end, polarity)| (position, end, polarity))
        })
        .collect();
    let mut failed: BTreeSet<usize> = BTreeSet::new();
    let mut added: BTreeMap<usize, Vec<(RoutedStub, Vec<core::Via>)>> = BTreeMap::new();
    let mut work = 0.0;
    let budget = work_budget(config.ladder_budget_seconds.unwrap_or(1200.0));
    let router_config = {
        let mut router_config = core_config(config);
        router_config.verbose = false;
        router_config.deadline_work = Some(budget);
        router_config.run_work = None;
        router_config
    };
    while !todo.is_empty() {
        // One stub per net a round.
        let mut round: BTreeMap<core::NetId, (usize, usize, usize)> = BTreeMap::new();
        let mut later = Vec::new();
        for item in todo {
            let (position, _, polarity) = item;
            if failed.contains(&position) {
                continue;
            }
            let pair = &pairs[candidates[position].0];
            let net = [pair.positive, pair.negative][polarity];
            if let std::collections::btree_map::Entry::Vacant(entry) = round.entry(net) {
                entry.insert(item);
            } else {
                later.push(item);
            }
        }
        todo = later;
        if round.is_empty() {
            break;
        }
        let mut sub = base.clone();
        for net in &mut sub.nets {
            net.terminals.clear();
        }
        let nets: BTreeSet<core::NetId> = round.keys().copied().collect();
        sub.merged_pads.retain(|merged| !nets.contains(&merged.net));
        sub.terminal_copper
            .retain(|extra| !nets.contains(&extra.net));
        for (&net, &(position, end, polarity)) in &round {
            let pair = &pairs[candidates[position].0];
            let link = links[position];
            let pad = link.pads[end][polarity];
            // The pad's terminal, or the pad merged into another (its own
            // terminal here).
            let terminal = board.nets[net as usize]
                .terminals
                .iter()
                .find(|terminal| core::geometry::distance(terminal.anchor, pad) < 1.0e-6)
                .cloned()
                .or_else(|| {
                    board
                        .merged_pads
                        .iter()
                        .filter(|merged| merged.net == net)
                        .find(|merged| {
                            board.obstacles[merged.pad].shape.distance_to_point(pad) <= 1.0e-6
                        })
                        .map(|merged| core::Terminal {
                            anchor: pad,
                            layers: board.obstacles[merged.pad].layers,
                            pad: merged.pad,
                            contact: None,
                            label: board.obstacles[merged.pad].label.clone(),
                        })
                });
            let Some(terminal) = terminal else {
                failed.insert(position);
                continue;
            };
            let (at, along) = link.stub_end(end, polarity, pair.width);
            let layers = link.runs[end][polarity].layers;
            sub.obstacles.push(core::Obstacle {
                shape: core::Shape::Capsule {
                    start: at,
                    end: along,
                    radius: pair.width / 2.0,
                },
                layers,
                kind: core::ObstacleKind::Copper,
                net: Some(net),
                clearance: 0.0,
                clearance_override: None,
                blocks_tracks: true,
                blocks_vias: true,
                label: "pair stub end".into(),
            });
            sub.nets[net as usize].terminals = vec![
                terminal,
                core::Terminal {
                    anchor: at,
                    layers,
                    pad: sub.obstacles.len() - 1,
                    contact: Some(core::Shape::Circle {
                        center: at,
                        radius: LAUNCH_CONTACT,
                    }),
                    label: "pair stub end".into(),
                },
            ];
        }
        // The other nets' pours stay as they are: a stub that cuts one
        // pays for it (`Config::pour_integrity`), as in the attempt (the
        // atmega32u4 board's GND pour on B.Cu split by a stub crossing
        // under its run, pairs5).
        let routed = core::route(&sub, &router_config);
        work += routed.expansions as f64 / core::router::expansions_per_second()
            + routed.build_work
            + routed.pour_work;
        for (&net, &(position, _, _)) in &round {
            if failed.contains(&position) {
                continue;
            }
            if routed.status[net as usize] != core::NetStatus::Routed {
                if debug() {
                    eprintln!(
                        "diff pair stub repair of {}: {:?}, terminals {:?}",
                        board.nets[net as usize].name,
                        routed.status[net as usize],
                        sub.nets[net as usize]
                            .terminals
                            .iter()
                            .map(|terminal| (terminal.anchor, terminal.layers, &terminal.label))
                            .collect::<Vec<_>>()
                    );
                }
                failed.insert(position);
                continue;
            }
            let (end, polarity) = (round[&net].1, round[&net].2);
            let from = &routed.routes[net as usize];
            added.entry(position).or_default().push((
                (
                    net,
                    links[position].pads[end][polarity],
                    from.segments.clone(),
                ),
                from.vias.clone(),
            ));
            // The next round keeps it.
            if todo.is_empty() {
                continue;
            }
            let mut kept = core::NetRoute::default();
            kept.segments.clone_from(&from.segments);
            kept.vias.clone_from(&from.vias);
            let mut routes = vec![core::NetRoute::default(); base.nets.len()];
            routes[net as usize] = kept;
            base = with_routes(&base, &routes);
        }
    }
    let mut done = BTreeMap::new();
    for position in wanted {
        let stubs = added.remove(&position).unwrap_or_default();
        let complete = !failed.contains(&position);
        for ((net, _, segments), vias) in &stubs {
            let route = &mut result.routes[*net as usize];
            route.segments.extend(segments.iter().cloned());
            route.vias.extend(vias.iter().cloned());
        }
        let pair = &pairs[candidates[position].0];
        let link = links[position];
        let joined = complete
            && (0..2).filter(|&end| !link.fixed[end]).all(|end| {
                [(0, pair.positive), (1, pair.negative)]
                    .into_iter()
                    .all(|(polarity, net)| {
                        let route = &result.routes[net as usize];
                        let reach = (pair.width
                            + route
                                .segments
                                .iter()
                                .map(|segment| segment.width)
                                .fold(0.0, f64::max))
                            / 2.0;
                        stub_joined(
                            board,
                            route,
                            net,
                            (link.pads[end][polarity], link.pads[1 - end][polarity]),
                            link.stub_end(end, polarity, pair.width),
                            reach,
                        )
                    })
            });
        if joined {
            done.insert(position, stubs.into_iter().map(|(stub, _)| stub).collect());
        } else {
            for ((net, _, segments), vias) in &stubs {
                let route = &mut result.routes[*net as usize];
                route.segments.retain(|segment| !segments.contains(segment));
                route.vias.retain(|via| !vias.contains(via));
            }
        }
        if debug() || pairs_why() {
            eprintln!(
                "pairs why: stubs of {} link {}: {}",
                board.nets[pair.positive as usize].name,
                link.link + 1,
                if joined {
                    "routed after the attempt"
                } else if complete {
                    "routed, still not reaching the run"
                } else {
                    "not routed after the attempt"
                }
            );
        }
    }
    (done, work)
}

/// After a stub repaired (`repair_stubs`) from the pad at `pad`: the pad's
/// old tracks (not the stub's, `new`) that now close a loop go, one at a
/// time, each only where the net's copper stays in as many pieces, and
/// then what joins nothing (`prune_dangling`): the pad keeps one way to
/// the run, not also its old route to the link's other end or through
/// other pads.
fn cut_old_connection(
    board: &core::Board,
    route: &mut core::NetRoute,
    net: core::NetId,
    pad: Point,
    new: &[core::Segment],
) {
    // The net's kept copper (the links' tracks and vias) as tracks and
    // vias, so that the pieces join along it; its pads and other copper as
    // they are.
    let mut kept = core::NetRoute::default();
    let mut own: Vec<&core::Obstacle> = Vec::new();
    for obstacle in board
        .obstacles
        .iter()
        .filter(|obstacle| obstacle.net == Some(net) && obstacle.kind == core::ObstacleKind::Copper)
    {
        match (&obstacle.shape, obstacle.label.as_str()) {
            (&core::Shape::Capsule { start, end, radius }, "existing track")
                if obstacle.layers.count_ones() == 1 =>
            {
                kept.segments.push(core::Segment {
                    layer: obstacle.layers.trailing_zeros() as usize,
                    start,
                    end,
                    width: 2.0 * radius,
                });
            }
            (&core::Shape::Circle { center, radius }, "existing via") => {
                kept.vias.push(core::Via {
                    at: center,
                    diameter: 2.0 * radius,
                    drill: 0.0,
                    span: None,
                });
            }
            _ => own.push(obstacle),
        }
    }
    let at_pad: Vec<&core::Obstacle> = own
        .iter()
        .copied()
        .filter(|obstacle| obstacle.shape.distance_to_point(pad) <= 1.0e-6)
        .collect();
    let joined_with_kept = |route: &core::NetRoute| {
        let mut all = route.clone();
        all.segments.extend(kept.segments.iter().cloned());
        all.vias.extend(kept.vias.iter().cloned());
        all
    };
    let mut cut = false;
    loop {
        let all = joined_with_kept(route);
        let before = pieces(&all, &own, None);
        let loop_at_pad = (0..route.segments.len()).find(|&index| {
            let segment = &route.segments[index];
            !new.contains(segment)
                && at_pad.iter().any(|obstacle| {
                    obstacle.layers & (1 << segment.layer) != 0
                        && [segment.start, segment.end].into_iter().any(|end| {
                            obstacle.shape.distance_to_point(end) <= segment.width / 2.0 + 1.0e-6
                        })
                })
                && pieces(&all, &own, Some(index)) <= before
        });
        match loop_at_pad {
            Some(index) => {
                route.segments.remove(index);
                cut = true;
            }
            None => break,
        }
    }
    if cut {
        prune_dangling(board, route, net);
    }
}

/// `board` with `routes` as its copper (existing tracks, vias and their
/// holes, of their nets).
fn with_routes(board: &core::Board, routes: &[core::NetRoute]) -> core::Board {
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
    fallback
}

/// Copper checked together (`refused_groups`): a link's run and vias.
#[derive(Clone)]
struct Group {
    pair: usize,
    segments: [Vec<(usize, Point, Point)>; 2],
    vias: [Vec<Point>; 2],
    /// Per polarity: the coupled run's tracks among `segments` (the rest
    /// are the ends' stubs, jogs and crossovers), for `PCB_ROUTER_PAIRS_WHY`.
    run: [std::ops::Range<usize>; 2],
    /// Whether its copper is not on the router's route (`LinkCopper::moved`).
    moved: bool,
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
    // Per routed segment of a group: its polarity and place in the group.
    let mut place_of_segment: BTreeMap<(core::NetId, usize), (usize, usize)> = BTreeMap::new();
    let mut vias_of: BTreeMap<core::NetId, Vec<(Point, usize)>> = BTreeMap::new();
    for (position, group) in groups.iter().enumerate() {
        let pair = &pairs[group.pair];
        for (polarity, net) in [(0, pair.positive), (1, pair.negative)] {
            let route = &mut routes[net as usize];
            for (local, &(layer, start, end)) in group.segments[polarity].iter().enumerate() {
                owner_of_segment.insert((net, route.segments.len()), position);
                place_of_segment.insert((net, route.segments.len()), (polarity, local));
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
                    span: None,
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
        // A group's via the finding is at, on the other side.
        let other_via = match (violation.other_segment, other_net) {
            (None, Some(net)) => via_owner(net, violation.at),
            _ => None,
        };
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
        // Only the links' copper (a straight stub's or a meander's check
        // has no run).
        if pairs_why()
            && own
                .or(other)
                .is_some_and(|position| !groups[position].run[0].is_empty())
        {
            // What of the pair's copper the finding is on: the run, an
            // end's stub, jog or crossover, a via; else the router's.
            let role = |net: core::NetId, segment: Option<usize>, owner: Option<usize>| {
                let Some(position) = owner else {
                    return format!("the router's copper of {}", board.nets[net as usize].name);
                };
                let group = &groups[position];
                let name = &board.nets[net as usize].name;
                match segment.and_then(|segment| place_of_segment.get(&(net, segment))) {
                    Some(&(polarity, local)) => {
                        let run = &group.run[polarity];
                        let (_, a, b) = group.segments[polarity][local];
                        let place = if run.is_empty() {
                            "a stub"
                        } else if local < run.start {
                            "end a's stub, jog or crossover"
                        } else if local < run.end {
                            "the run"
                        } else {
                            "end b's stub, jog or crossover"
                        };
                        format!(
                            "{name}'s {place} ({:.3}, {:.3}) - ({:.3}, {:.3})",
                            a[0], a[1], b[0], b[1]
                        )
                    }
                    None => format!("{name}'s via"),
                }
            };
            let other_owner = other;
            let other_text = match violation.other_segment {
                Some((net, segment)) => role(net, Some(segment), other_owner),
                None => violation.other.clone(),
            };
            let pair = own.or(other).map(|position| &pairs[groups[position].pair]);
            let partner_finding = pair.is_some_and(|pair| {
                let names =
                    [pair.positive, pair.negative].map(|net| &board.nets[net as usize].name);
                let own_pair = names.contains(&&board.nets[violation.net as usize].name);
                own_pair
                    && names
                        .iter()
                        .any(|name| violation.other.contains(name.as_str()))
            });
            eprintln!(
                "pairs why: refused {}: at ({:.3}, {:.3}) layer {}: {} is {:.3} mm from {} (needs {:.3}){}",
                pair.map_or(String::new(), |pair| board.nets[pair.positive as usize]
                    .name
                    .clone()),
                violation.at[0],
                violation.at[1],
                violation.layer,
                role(violation.net, violation.segment, own),
                violation.actual,
                other_text,
                violation.required,
                if partner_finding { " [partner]" } else { "" }
            );
        }
        // A finding between two groups refuses both, but where one of them
        // lies on the router's route and the other was moved off it
        // (`centrelines`), the moved one only.
        let culprits: Vec<usize> = match (own, other.or(other_via)) {
            (Some(a), Some(b)) if a != b && groups[a].moved != groups[b].moved => {
                vec![if groups[a].moved { a } else { b }]
            }
            _ => own.into_iter().chain(other).collect(),
        };
        for position in culprits {
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

/// The fallback's work on a layout's own board with `left` seconds of the
/// layout's work left (`work_budget` of it, as a ladder of that budget).
pub(crate) fn pairs_fallback_work(left: f64) -> f64 {
    work_budget(left)
}

/// Whether that is the fallback's minimum rather than its share: the
/// layout's budget was spent before its board was written.
pub(crate) fn pairs_fallback_short(left: f64) -> bool {
    WORK_SHARE * left < MINIMUM_WORK
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
            // The narrow-signal step keeps it (KiCad checks it).
            copy.ruled_widths.insert(net.clone());
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
    // Per net: its tracks (layer as an index into `names`) and vias, for
    // the skew.
    let by_name = copper_heights(pcb);
    let names: Vec<&String> = by_name.keys().collect();
    let heights: Vec<f64> = by_name.values().copied().collect();
    let mut copper: BTreeMap<String, NetCopper> = BTreeMap::new();
    for item in pcb.children() {
        let kind = item.head();
        if kind != Some("segment") && kind != Some("via") {
            continue;
        }
        let Some(net) = node_net(item) else {
            continue;
        };
        let net = crate::kicad_nets::normalize_net(&unescape(net)).to_string();
        if kind == Some("via") {
            if let Ok(at) = form_xy(item, "at") {
                copper.entry(net).or_default().1.push(at);
            }
            continue;
        }
        let (Ok(start), Ok(end)) = (form_xy(item, "start"), form_xy(item, "end")) else {
            continue;
        };
        *lengths.entry(net.clone()).or_default() += core::geometry::distance(start, end);
        let layer = form_atom(item, "layer", 1)
            .and_then(|layer| names.iter().position(|name| name.as_str() == layer))
            .unwrap_or(usize::MAX);
        copper.entry(net).or_default().0.push((layer, start, end));
    }
    // The skew: per pair with a coupled link its nets' lengths apart, the
    // vias at the stackup's heights.
    report.skew_mm = 0.0;
    report.skews.clear();
    for pair in &mut report.pairs {
        if pair.links_coupled == 0 {
            continue;
        }
        let length = |net: &str| {
            copper
                .get(net)
                .map_or(0.0, |(tracks, vias)| copper_length(tracks, vias, &heights))
        };
        let (p, n) = (length(&pair.positive), length(&pair.negative));
        let skew = (p - n).abs();
        pair.skew_mm = Some(skew);
        report.skew_mm = report.skew_mm.max(skew);
        report.skews.push(KiCadPairSkew {
            positive: pair.positive.clone(),
            negative: pair.negative.clone(),
            lengths_mm: [p, n],
            skew_mm: skew,
            target_mm: pair.skew_target_mm,
            meander_mm: pair.meander_mm,
            pair_vias: pair.pair_vias,
            return_vias: pair.return_vias,
            uncoupled_mm: 0.0,
        });
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
        if let Some(skew) = report
            .skews
            .iter_mut()
            .find(|skew| skew.positive == pair.positive && skew.negative == pair.negative)
        {
            skew.uncoupled_mm = uncoupled;
        }
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

    /// Two pairs whose pads lie side by side on one part only (U2): S
    /// across a ground pin on another (U1, an HDMI connector's or an ESD
    /// part's pins), T apart on two series parts (R1, R2, `apart` mm).
    fn one_point_board(apart: f64) -> String {
        format!(
            r#"(kicad_pcb (version 20241229) (generator "pcbnew")
  (general (thickness 1.6)) (paper "A4")
  (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
  (gr_rect (start 0 0) (end 40 24) (stroke (width 0.05) (type default)) (layer "Edge.Cuts"))
  (footprint "test" (layer "F.Cu") (at 18 10)
    (property "Reference" "U1" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd rect (at 0 0) (size 1 0.3) (layers "F.Cu") (net "/S+"))
    (pad "2" smd rect (at 0 0.5) (size 1 0.3) (layers "F.Cu") (net "/GND"))
    (pad "3" smd rect (at 0 1) (size 1 0.3) (layers "F.Cu") (net "/S-")))
  (footprint "test" (layer "F.Cu") (at 34 10)
    (property "Reference" "U2" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd rect (at 0 0) (size 1 0.3) (layers "F.Cu") (net "/S+"))
    (pad "2" smd rect (at 0 0.5) (size 1 0.3) (layers "F.Cu") (net "/S-"))
    (pad "3" smd rect (at 0 4) (size 1 0.3) (layers "F.Cu") (net "/T+"))
    (pad "4" smd rect (at 0 4.5) (size 1 0.3) (layers "F.Cu") (net "/T-")))
  (footprint "test" (layer "F.Cu") (at 10 14)
    (property "Reference" "R1" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd rect (at 0.5 0) (size 0.5 0.5) (layers "F.Cu") (net "/T+"))
    (pad "2" smd rect (at -0.5 0) (size 0.5 0.5) (layers "F.Cu") (net "/A")))
  (footprint "test" (layer "F.Cu") (at 10 {})
    (property "Reference" "R2" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd rect (at 0.5 0) (size 0.5 0.5) (layers "F.Cu") (net "/T-"))
    (pad "2" smd rect (at -0.5 0) (size 0.5 0.5) (layers "F.Cu") (net "/B")))
  (footprint "test" (layer "F.Cu") (at 3 22)
    (property "Reference" "J5" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "/GND"))))
"#,
            14.0 + apart
        )
    }

    fn one_point_pairs(
        apart: f64,
        extended: bool,
    ) -> (core::Board, KiCadBoardRouterConfig, Vec<Pair>) {
        let pcb = parse(&one_point_board(apart)).unwrap();
        let (_, config) = lowered();
        let board = lower(&pcb, &config, true).unwrap().board;
        EXTENDED.with(|cell| cell.set(Some(extended)));
        let pairs = find_pairs(&board, &config);
        EXTENDED.with(|cell| cell.set(None));
        (board, config, pairs)
    }

    fn pair_named<'a>(board: &core::Board, pairs: &'a [Pair], name: &str) -> &'a Pair {
        pairs
            .iter()
            .find(|pair| board.nets[pair.positive as usize].name.ends_with(name))
            .unwrap()
    }

    /// A pair with no link otherwise (one part with its pads side by side)
    /// takes a part's pads with a ground pin between them as a pair point:
    /// the run goes from U2 to U1.
    #[test]
    fn a_pair_without_a_link_takes_pads_across_a_pin_as_a_pair_point() {
        let (board, _, pairs) = one_point_pairs(4.0, false);
        assert!(pair_named(&board, &pairs, "S+").links.is_empty());
        let (board, _, pairs) = one_point_pairs(4.0, true);
        let pair = pair_named(&board, &pairs, "S+");
        assert_eq!(pair.kinds, [PointKind::Side, PointKind::Pin]);
        assert_eq!(pair.links.len(), 1);
        let p = &board.nets[pair.positive as usize];
        let labels: Vec<&str> = pair
            .points
            .iter()
            .map(|&(a, _)| p.terminals[a].label.as_str())
            .collect();
        assert_eq!(labels, ["U2.1", "U1.1"]);
        // A pair with a link keeps its points: the original board's U1 is
        // no pair point (`pair_points_are_pads_side_by_side_and_links_join_parts`).
        // T's other pads lie on two series parts 4 mm apart: still no link
        // (a split end there was built and measured, and left out).
        let pair = pair_named(&board, &pairs, "T+");
        assert_eq!(pair.kinds, [PointKind::Side]);
        assert!(pair.links.is_empty());
    }

    /// A pin between a pair point's pads must keep a way out past the
    /// stubs and the run's lead: U1's ground pin (1 mm long pads, room on
    /// its far side) does, U9's (a 0.4 mm pitch row with other nets' pins
    /// beside and behind it, MSign's USB muxes) does not, and that pair
    /// point makes no run.
    #[test]
    fn a_pin_point_whose_pin_would_be_walled_in_makes_no_run() {
        let (mut board, config, _) = one_point_pairs(4.0, true);
        EXTENDED.with(|cell| cell.set(Some(true)));
        plan_pairs(&mut board, &config);
        let plan = plan_of(&board).unwrap();
        let entry = plan
            .entries
            .iter()
            .find(|entry| entry.positive.ends_with("S+"))
            .unwrap();
        assert!(entry.why.is_empty(), "{entry:?}");
        assert_eq!(entry.links, 1);
        let walled = one_point_board(4.0).replace(
            r#"  (footprint "test" (layer "F.Cu") (at 18 10)
    (property "Reference" "U1" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd rect (at 0 0) (size 1 0.3) (layers "F.Cu") (net "/S+"))
    (pad "2" smd rect (at 0 0.5) (size 1 0.3) (layers "F.Cu") (net "/GND"))
    (pad "3" smd rect (at 0 1) (size 1 0.3) (layers "F.Cu") (net "/S-")))"#,
            r#"  (footprint "test" (layer "F.Cu") (at 18 10)
    (property "Reference" "U9" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd rect (at -0.4 0) (size 0.2 0.6) (layers "F.Cu") (net "/X1"))
    (pad "2" smd rect (at 0 0) (size 0.2 0.6) (layers "F.Cu") (net "/S+"))
    (pad "3" smd rect (at 0.4 0) (size 0.2 0.6) (layers "F.Cu") (net "/GND"))
    (pad "4" smd rect (at 0.8 0) (size 0.2 0.6) (layers "F.Cu") (net "/S-"))
    (pad "5" smd rect (at 1.2 0) (size 0.2 0.6) (layers "F.Cu") (net "/X2"))
    (pad "6" smd rect (at 0.4 -0.75) (size 1.8 0.6) (layers "F.Cu") (net "/X3")))"#,
        );
        assert_ne!(walled, one_point_board(4.0));
        let pcb = parse(&walled).unwrap();
        let mut board = lower(&pcb, &config, true).unwrap().board;
        let pair = pair_named(&board, &find_pairs(&board, &config), "S+").clone();
        assert_eq!(pair.kinds, [PointKind::Side, PointKind::Pin]);
        plan_pairs(&mut board, &config);
        EXTENDED.with(|cell| cell.set(None));
        let plan = plan_of(&board).unwrap();
        let entry = plan
            .entries
            .iter()
            .find(|entry| entry.positive.ends_with("S+"))
            .unwrap();
        assert!(
            entry
                .why
                .iter()
                .any(|why| why.contains("U9.3") && why.contains("walled in")),
            "{entry:?}"
        );
        assert!(
            !plan
                .links
                .iter()
                .any(|link| plan.pairs[link.pair].positive == pair.positive)
        );
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
        assert!(!labels.iter().any(|(a, _)| a == "C3.1"), "{labels:?}");
        // J1's rows and U2 as before, and the capacitors: three links.
        assert_eq!(labels.len(), 4, "{labels:?}");
        assert_eq!(pair.links.len(), 2);
    }

    /// A pair row's side launches out of the parts, away from their other
    /// pads, wherever the link's far end lies: J1 north-east of the
    /// capacitors' D side (the source of right_io's HDMI pairs beside their
    /// row), whose normal pointing "toward" it went north through the
    /// bodies past the X pair's pads.
    #[test]
    fn a_series_pair_point_launches_away_from_its_parts() {
        let board_text = r#"(kicad_pcb (version 20241229) (generator "pcbnew")
  (general (thickness 1.6)) (paper "A4")
  (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
  (gr_rect (start 0 0) (end 40 30) (stroke (width 0.05) (type default)) (layer "Edge.Cuts"))
  (footprint "test" (layer "F.Cu") (at 30 9.5)
    (property "Reference" "J1" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd rect (at 0 0) (size 0.3 1) (layers "F.Cu") (net "/D+"))
    (pad "2" smd rect (at 0.5 0) (size 0.3 1) (layers "F.Cu") (net "/D-")))
  (footprint "test" (layer "F.Cu") (at 20 20) (property "Reference" "C1" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd rect (at 0 0.48) (size 0.62 0.56) (layers "F.Cu") (net "/D+"))
    (pad "2" smd rect (at 0 -0.48) (size 0.62 0.56) (layers "F.Cu") (net "/X+")))
  (footprint "test" (layer "F.Cu") (at 20.93 20) (property "Reference" "C2" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd rect (at 0 0.48) (size 0.62 0.56) (layers "F.Cu") (net "/D-"))
    (pad "2" smd rect (at 0 -0.48) (size 0.62 0.56) (layers "F.Cu") (net "/X-")))
  (footprint "test" (layer "F.Cu") (at 10 10)
    (property "Reference" "J2" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd rect (at 0 0) (size 0.3 1) (layers "F.Cu") (net "/X+"))
    (pad "2" smd rect (at 0.5 0) (size 0.3 1) (layers "F.Cu") (net "/X-"))))
"#;
        let pcb = parse(board_text).unwrap();
        let (_, config) = lowered();
        let mut board = lower(&pcb, &config, true).unwrap().board;
        plan_pairs(&mut board, &config);
        let plan = board
            .attachment
            .data
            .clone()
            .unwrap()
            .downcast::<PairPlan>()
            .unwrap();
        // The D pair's link: its end at the capacitors' D pads (y 20.48)
        // launches south, out of the row; the X pair's (y 19.52) north.
        let mut seen = 0;
        for link in &plan.links {
            for end in 0..2 {
                let pads = link.pads[end];
                let middle = scale(add(pads[0], pads[1]), 0.5);
                if (middle[0] - 20.465).abs() > 0.01 {
                    continue;
                }
                let outward = sub(link.starts[end], middle);
                let south = middle[1] > 20.0;
                assert!(
                    if south {
                        outward[1] > 0.0
                    } else {
                        outward[1] < 0.0
                    },
                    "the end at {middle:?} launches along {outward:?}"
                );
                seen += 1;
            }
        }
        assert_eq!(seen, 2);
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
            .data
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

    /// The links may be given up (`core::GiveUp`, with the settings'
    /// `diff_pair_give_up`; off by default): with nothing released
    /// a pair's net is as planned, with its link released it is the net as
    /// lowered (its pads, no run ends, nothing merged), and a route of the
    /// planned board whose link finds no room couples nothing but leaves
    /// no connection of the pair open.
    #[test]
    fn a_link_given_up_gives_the_pairs_nets_their_pads_back() {
        use pcb_router::GiveUp;
        let (lowered_board, config) = lowered();
        // Off by default: the plan offers no link to give up.
        let mut board = lowered_board.clone();
        plan_pairs(&mut board, &config);
        assert!(board.attachment.give_up.is_none());
        let config = KiCadBoardRouterConfig {
            diff_pair_give_up: Some(true),
            ..config
        };
        let mut board = lowered_board.clone();
        plan_pairs(&mut board, &config);
        let plan = plan_of(&board).unwrap();
        assert!(board.attachment.give_up.is_some());
        let link = plan.links[0].net;
        assert!(plan.optional(link));
        assert!(!plan.optional(plan.pairs[0].positive));
        let labels = |terminals: &[core::Terminal]| -> Vec<String> {
            terminals
                .iter()
                .map(|terminal| terminal.label.clone())
                .collect()
        };
        for polarity in 0..2 {
            let net = [plan.pairs[0].positive, plan.pairs[0].negative][polarity];
            let planned = plan
                .net_with_links(&board, 0, polarity, &BTreeSet::new())
                .unwrap();
            assert_eq!(
                labels(&planned.terminals),
                labels(&board.nets[net as usize].terminals)
            );
            let of_net = |merged: &[core::MergedPad]| -> Vec<(usize, usize)> {
                merged
                    .iter()
                    .filter(|merged| merged.net == net)
                    .map(|merged| (merged.host, merged.pad))
                    .collect()
            };
            assert_eq!(of_net(&planned.merged_pads), of_net(&board.merged_pads));
            let copper = |extras: &[core::TerminalCopper]| -> Vec<core::TerminalCopper> {
                extras
                    .iter()
                    .filter(|extra| extra.net == net)
                    .copied()
                    .collect()
            };
            assert_eq!(
                copper(&planned.terminal_copper),
                copper(&board.terminal_copper)
            );
            assert!(!planned.merged_pads.is_empty());
        }
        // Standing again (`retry_released`): the nets as planned.
        let standing = plan.standing(&board, link, &[link]);
        assert_eq!(standing.len(), 2);
        for rewrite in &standing {
            assert_eq!(
                labels(&rewrite.terminals),
                labels(&board.nets[rewrite.net as usize].terminals)
            );
        }
        let rewrites = plan.released(&board, &[link]);
        assert_eq!(rewrites.len(), 2);
        for rewrite in &rewrites {
            assert_eq!(
                labels(&rewrite.terminals),
                labels(&lowered_board.nets[rewrite.net as usize].terminals)
            );
            assert!(rewrite.merged_pads.is_empty());
            assert!(rewrite.terminal_copper.is_empty());
        }
    }

    /// A layout's routed stages lower with `lower_for_route`: with the
    /// router's `diff_pairs` the board carries the pairs' plan (the link a
    /// net of the route) and the plan's work is booked by the stage; off
    /// (a layout's default) the board is `lower`'s alone, no work; the
    /// topological engine plans none.
    #[test]
    fn a_layouts_routed_stages_lower_with_the_pairs_plan_and_book_its_work() {
        let pcb = parse(BOARD).unwrap();
        let (plain, config) = lowered();
        let (board, work) = lower_for_route(&pcb, &config, true).unwrap();
        assert_eq!(board.nets.len(), plain.nets.len());
        assert!(plan_of(&board).is_none());
        assert_eq!(work, 0.0);
        let on = KiCadBoardRouterConfig {
            diff_pairs: Some(true),
            ..config.clone()
        };
        assert!(pairs_planned(&on));
        let (board, work) = lower_for_route(&pcb, &on, true).unwrap();
        assert_eq!(board.nets.len(), plain.nets.len() + 1);
        let plan = plan_of(&board).unwrap();
        assert_eq!(plan.links.len(), 1);
        assert!(work > 0.0, "{work}");
        assert_eq!(work, planned_work(&board));
        // The same board plans the same work (a model of its tests, not
        // the clock).
        let (_, again) = lower_for_route(&pcb, &on, true).unwrap();
        assert_eq!(again, work);
        let topological = KiCadBoardRouterConfig {
            engine: Some("topological".into()),
            ..on
        };
        assert!(!pairs_planned(&topological));
        assert_eq!(lower_for_route(&pcb, &topological, true).unwrap().1, 0.0);
    }

    /// A move that changes the pairs' plan (here a pair point gone: U2's
    /// pads apart, and U1's, so that no pair point across a pin takes its
    /// place)
    /// changes the nets the router holds: `Router::update`
    /// refuses the board with `UPDATE_NEEDS_SAME_NETS`, which a layout's
    /// moves take for a move not to make (not an error of the layout).
    #[test]
    fn a_move_that_changes_the_pairs_plan_is_refused_by_the_routers_update() {
        let (_, config) = lowered();
        let on = KiCadBoardRouterConfig {
            diff_pairs: Some(true),
            ..config
        };
        let (board, _) = lower_for_route(&parse(BOARD).unwrap(), &on, true).unwrap();
        let apart = BOARD
            .replace(
                r#"(pad "2" smd rect (at 0 0.5) (size 1 0.3) (layers "F.Cu") (net "/D-")))
  (footprint "test" (layer "F.Cu") (at 18 4)"#,
                r#"(pad "2" smd rect (at 0 5.5) (size 1 0.3) (layers "F.Cu") (net "/D-")))
  (footprint "test" (layer "F.Cu") (at 18 4)"#,
            )
            .replace(
                r#"(pad "3" smd rect (at 0 1) (size 1 0.3) (layers "F.Cu") (net "/D-")))"#,
                r#"(pad "3" smd rect (at 0 6) (size 1 0.3) (layers "F.Cu") (net "/D-")))"#,
            );
        assert_ne!(apart, BOARD);
        let (moved, _) = lower_for_route(&parse(&apart).unwrap(), &on, true).unwrap();
        assert_ne!(moved.nets.len(), board.nets.len());
        let mut router = core::router::Router::new(&board, &core_config(&on));
        assert_eq!(
            router.update(&moved).unwrap_err(),
            core::router::UPDATE_NEEDS_SAME_NETS
        );
        // The same plan updates.
        assert!(router.update(&board).is_ok());
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

    /// A pair whose two nets prefer the same layers by a layer-limited
    /// length or skew rule (`length_rule_layers`, trial 7's Sisu_SSE-9:
    /// RP_USB off In1.Cu and In4.Cu as the designer runs it): its coupled
    /// links prefer them too, under the name the lowering gives them;
    /// nets preferring different layers leave the links free.
    #[test]
    fn a_pairs_links_prefer_the_layers_both_its_nets_prefer() {
        let pcb = parse(BOARD).unwrap();
        let mut config = KiCadBoardRouterConfig {
            default_rules: Some(KiCadConnectionRoutingRules {
                trace_width_mm: 0.2,
                clearance_mm: 0.2,
                via_size_mm: 0.6,
                via_drill_mm: 0.3,
            }),
            diff_pairs: Some(true),
            ..KiCadBoardRouterConfig::default()
        };
        let mut board = lower(&pcb, &config, true).unwrap().board;
        let names: Vec<String> = board
            .nets
            .iter()
            .map(|net| net.name.clone())
            .filter(|name| name.ends_with("D+") || name.ends_with("D-"))
            .collect();
        assert_eq!(names.len(), 2, "{names:?}");
        let nets = board.nets.len();
        plan_pairs(&mut board, &config);
        assert_eq!(board.nets.len(), nets + 1);
        let link = board.nets[nets].name.clone();
        let prefer = |config: &mut KiCadBoardRouterConfig, masks: [core::LayerMask; 2]| {
            for (name, mask) in names.iter().zip(masks) {
                config.length_layer_preferences.insert(
                    name.clone(),
                    crate::board_router::KiCadLayerPreference {
                        mask,
                        ..Default::default()
                    },
                );
            }
        };
        let link_mask = |config: &KiCadBoardRouterConfig| {
            core_config(config)
                .layer_preferences
                .and_then(|masks| masks.get(&link).copied())
        };
        prefer(&mut config, [0b10, 0b10]);
        assert_eq!(link_mask(&config), Some(0b10));
        prefer(&mut config, [0b10, 0b01]);
        assert_eq!(link_mask(&config), None);
    }

    /// U2's pads in the other order (D- above D+): the link is crossed at
    /// one end, which gets a crossover in its lead (the negative net dives
    /// under the positive one or the other way round): the link couples,
    /// its stubs straight, no stub crossing another.
    #[test]
    fn a_crossed_end_crosses_over_in_its_lead() {
        let crossed = BOARD.replace(
            r#"(pad "1" smd rect (at 0 0) (size 1 0.3) (layers "F.Cu") (net "/D+"))
    (pad "2" smd rect (at 0 0.5) (size 1 0.3) (layers "F.Cu") (net "/D-")))
  (footprint "test" (layer "F.Cu") (at 18 4)"#,
            r#"(pad "1" smd rect (at 0 0) (size 1 0.3) (layers "F.Cu") (net "/D-"))
    (pad "2" smd rect (at 0 0.5) (size 1 0.3) (layers "F.Cu") (net "/D+")))
  (footprint "test" (layer "F.Cu") (at 18 4)"#,
        );
        assert_ne!(crossed, BOARD);
        let pcb = parse(&crossed).unwrap();
        let config = KiCadBoardRouterConfig {
            default_rules: Some(KiCadConnectionRoutingRules {
                trace_width_mm: 0.2,
                clearance_mm: 0.2,
                via_size_mm: 0.6,
                via_drill_mm: 0.3,
            }),
            ..KiCadBoardRouterConfig::default()
        };
        let plan_of = |board: &core::Board| {
            board
                .attachment
                .data
                .clone()
                .unwrap()
                .downcast::<PairPlan>()
                .unwrap()
        };
        // Off by default: the crossed end's stubs are the router's.
        CROSSOVERS.with(|cell| cell.set(Some(false)));
        let mut board = lower(&pcb, &config, true).unwrap().board;
        plan_pairs(&mut board, &config);
        assert!(
            plan_of(&board).links[0]
                .crossovers
                .iter()
                .all(Option::is_none)
        );
        CROSSOVERS.with(|cell| cell.set(Some(true)));
        let mut board = lower(&pcb, &config, true).unwrap().board;
        plan_pairs(&mut board, &config);
        let plan = board
            .attachment
            .data
            .clone()
            .unwrap()
            .downcast::<PairPlan>()
            .unwrap();
        let link_plan = &plan.links[0];
        assert!(link_plan.crossed);
        let crossing = link_plan
            .crossovers
            .iter()
            .flatten()
            .next()
            .expect("a crossover");
        // One net dives: two vias, a track on B.Cu.
        let vias: usize = crossing.vias.iter().map(Vec::len).sum();
        assert_eq!(vias, 2);
        assert!(
            crossing
                .segments
                .iter()
                .flatten()
                .any(|(layer, ..)| *layer == 1)
        );
        // At each end the stubs' ends keep the pads' order: the stubs do
        // not cross.
        for end in 0..2 {
            let [p, n] = link_plan.pads[end];
            let [end_p, end_n] = link_plan.ends[end];
            assert!(
                core::geometry::segment_segment_distance(p, end_p, n, end_n) > 0.2,
                "end {end}: stubs cross"
            );
        }
        let mut result = core::route(&board, &core_config(&config));
        let (_, copper, report) = finish_pairs(&board, &mut result, &config, &[0.0, 1.6]).unwrap();
        assert_eq!(report.links_coupled, 1, "{:?}", report.pairs);
        assert_eq!(report.crossovers, 1);
        // The diving net's two vias (1.6 mm each between F.Cu and B.Cu) and
        // its detour make the pair's lengths differ by more than its gap:
        // the shorter net gets a meander of what it lacks, its tracks a
        // tuning pattern.
        assert_eq!(report.meanders, 1, "{:?}", report.pairs);
        let entry = report.pairs.iter().find(|pair| pair.links > 0).unwrap();
        assert!(entry.meander_mm > 0.2, "{entry:?}");
        assert_eq!(copper.patterns.len(), 1);
        let meander = &copper.patterns[0];
        assert!((meander.added - entry.meander_mm).abs() < 1.0e-9);
        assert!(meander.amplitude <= 3.0 * 0.2 + 1.0e-9);
    }

    /// Without a crossover a crossed end's stubs cross on their way to the
    /// run, which the router does with a via: the end's start keeps room
    /// for it beyond the pads (`CROSSING_ROOM`), not the closest clear
    /// start (pairs6: left_io's J22 launched 0.65 mm from its pads, the
    /// stubs with no path in the negotiation).
    #[test]
    fn a_crossed_end_without_a_crossover_leaves_room_for_the_crossing() {
        let crossed = BOARD.replace(
            r#"(pad "1" smd rect (at 0 0) (size 1 0.3) (layers "F.Cu") (net "/D+"))
    (pad "2" smd rect (at 0 0.5) (size 1 0.3) (layers "F.Cu") (net "/D-")))
  (footprint "test" (layer "F.Cu") (at 18 4)"#,
            r#"(pad "1" smd rect (at 0 0) (size 1 0.3) (layers "F.Cu") (net "/D-"))
    (pad "2" smd rect (at 0 0.5) (size 1 0.3) (layers "F.Cu") (net "/D+")))
  (footprint "test" (layer "F.Cu") (at 18 4)"#,
        );
        let pcb = parse(&crossed).unwrap();
        let config = KiCadBoardRouterConfig {
            default_rules: Some(KiCadConnectionRoutingRules {
                trace_width_mm: 0.2,
                clearance_mm: 0.2,
                via_size_mm: 0.6,
                via_drill_mm: 0.3,
            }),
            ..KiCadBoardRouterConfig::default()
        };
        CROSSOVERS.with(|cell| cell.set(Some(false)));
        let mut board = lower(&pcb, &config, true).unwrap().board;
        plan_pairs(&mut board, &config);
        CROSSOVERS.with(|cell| cell.set(None));
        let plan = board
            .attachment
            .data
            .clone()
            .unwrap()
            .downcast::<PairPlan>()
            .unwrap();
        let link_plan = plan
            .links
            .iter()
            .find(|link| link.crossed)
            .expect("a crossed link");
        let pair = &plan.pairs[link_plan.pair];
        let end = (0..2)
            .find(|&end| end_crossed(link_plan, end))
            .expect("a crossed end");
        assert!(!link_plan.fixed[end]);
        let outward = unit(sub(link_plan.launches[end], link_plan.starts[end]));
        let room = CROSSING_ROOM * (pair.via_diameter + 2.0 * pair.clearance + pair.width);
        for stub in link_plan.stubs[end] {
            let along = dot(sub(link_plan.starts[end], stub), outward);
            assert!(along >= room - 1.0e-9, "{along} from a pad, {room} wanted");
        }
    }

    /// A crossover is planned only where the link carries the stubs from
    /// the pads to it: at U2, whose stubs a GND pad between the pads' exits
    /// keeps from going straight, the router's stubs would have to reach
    /// round the crossover's copper (pairs6: the ch32v003 board's D+ pad
    /// walled in, its net open), so U2's end gets none.
    #[test]
    fn a_crossover_is_planned_only_where_the_link_carries_its_stubs() {
        let crossed = BOARD.replace(
            r#"(pad "1" smd rect (at 0 0) (size 1 0.3) (layers "F.Cu") (net "/D+"))
    (pad "2" smd rect (at 0 0.5) (size 1 0.3) (layers "F.Cu") (net "/D-")))
  (footprint "test" (layer "F.Cu") (at 18 4)"#,
            r#"(pad "1" smd rect (at 0 0) (size 1 0.3) (layers "F.Cu") (net "/D-"))
    (pad "2" smd rect (at 0 0.5) (size 1 0.3) (layers "F.Cu") (net "/D+")))
  (footprint "test" (layer "F.Cu") (at 33.25 10.25)
    (property "Reference" "R9" (at 0 -2) (layer "F.SilkS"))
    (pad "1" smd rect (at 0 0) (size 0.15 0.15) (layers "F.Cu") (net "/GND")))
  (footprint "test" (layer "F.Cu") (at 18 4)"#,
        );
        assert_ne!(crossed, BOARD);
        let pcb = parse(&crossed).unwrap();
        let config = KiCadBoardRouterConfig {
            default_rules: Some(KiCadConnectionRoutingRules {
                trace_width_mm: 0.2,
                clearance_mm: 0.2,
                via_size_mm: 0.6,
                via_drill_mm: 0.3,
            }),
            ..KiCadBoardRouterConfig::default()
        };
        CROSSOVERS.with(|cell| cell.set(Some(true)));
        let mut board = lower(&pcb, &config, true).unwrap().board;
        plan_pairs(&mut board, &config);
        CROSSOVERS.with(|cell| cell.set(None));
        let plan = board
            .attachment
            .data
            .clone()
            .unwrap()
            .downcast::<PairPlan>()
            .unwrap();
        let link_plan = plan
            .links
            .iter()
            .find(|link| link.pads.iter().any(|pads| pads[0][0] > 33.0))
            .expect("a link to U2");
        assert!(link_plan.crossed);
        let u2 = (0..2)
            .find(|&end| link_plan.pads[end][0][0] > 33.0)
            .unwrap();
        assert!(!link_plan.fixed[u2], "U2's stubs are the router's");
        assert!(link_plan.crossovers[u2].is_none(), "a crossover at U2");
        for end in 0..2 {
            if link_plan.crossovers[end].is_some() {
                assert!(
                    link_plan.fixed[end],
                    "a crossover at end {end} without its stubs"
                );
            }
        }
    }

    /// A crossed pair whose link falls back (its run not routed, or its
    /// copper refused) leaves no crossover copper: no via or track of the
    /// crossover in the pairs' nets' routes or in the written pair copper,
    /// nothing of the pair's nets dangling, and the fallback joins the pads.
    #[test]
    fn a_crossed_pair_that_falls_back_leaves_no_crossover_copper() {
        let crossed = BOARD.replace(
            r#"(pad "1" smd rect (at 0 0) (size 1 0.3) (layers "F.Cu") (net "/D+"))
    (pad "2" smd rect (at 0 0.5) (size 1 0.3) (layers "F.Cu") (net "/D-")))
  (footprint "test" (layer "F.Cu") (at 18 4)"#,
            r#"(pad "1" smd rect (at 0 0) (size 1 0.3) (layers "F.Cu") (net "/D-"))
    (pad "2" smd rect (at 0 0.5) (size 1 0.3) (layers "F.Cu") (net "/D+")))
  (footprint "test" (layer "F.Cu") (at 18 4)"#,
        );
        let pcb = parse(&crossed).unwrap();
        let config = KiCadBoardRouterConfig {
            default_rules: Some(KiCadConnectionRoutingRules {
                trace_width_mm: 0.2,
                clearance_mm: 0.2,
                via_size_mm: 0.6,
                via_drill_mm: 0.3,
            }),
            ..KiCadBoardRouterConfig::default()
        };
        CROSSOVERS.with(|cell| cell.set(Some(true)));
        for how in ["not routed", "refused"] {
            let mut board = lower(&pcb, &config, true).unwrap().board;
            plan_pairs(&mut board, &config);
            let plan = board
                .attachment
                .data
                .clone()
                .unwrap()
                .downcast::<PairPlan>()
                .unwrap();
            let link_plan = plan.links[0].clone();
            let pair = plan.pairs[link_plan.pair].clone();
            let crossing = link_plan
                .crossovers
                .iter()
                .flatten()
                .next()
                .expect("a crossover")
                .clone();
            let mut result = core::route(&board, &core_config(&config));
            let link = link_plan.net as usize;
            assert_eq!(result.status[link], core::NetStatus::Routed);
            match how {
                "not routed" => {
                    result.routes[link] = core::NetRoute::default();
                    result.status[link] = core::NetStatus::Partial {
                        unconnected_terminals: 1,
                    };
                }
                _ => {
                    // Another net's track across the middle of the run, on
                    // both layers: the copper check refuses the link.
                    let segments = &result.routes[link].segments;
                    let longest = segments
                        .iter()
                        .max_by(|a, b| {
                            core::geometry::distance(a.start, a.end)
                                .total_cmp(&core::geometry::distance(b.start, b.end))
                        })
                        .unwrap()
                        .clone();
                    let middle = scale(add(longest.start, longest.end), 0.5);
                    let across = left(unit(sub(longest.end, longest.start)));
                    let ground = board
                        .nets
                        .iter()
                        .position(|net| net.name.trim_start_matches('/') == "GND")
                        .unwrap();
                    for layer in 0..2 {
                        result.routes[ground].segments.push(core::Segment {
                            layer,
                            start: add(middle, scale(across, -1.0)),
                            end: add(middle, scale(across, 1.0)),
                            width: 0.2,
                        });
                    }
                }
            }
            let (restored, copper, report) =
                finish_pairs(&board, &mut result, &config, &[0.0, 1.6]).unwrap();
            assert_eq!(report.links_coupled, 0, "{how}: {:?}", report.pairs);
            assert_eq!(report.crossovers, 0, "{how}");
            assert!(copper.items.is_empty(), "{how}: pair copper written");
            assert!(
                !restored
                    .obstacles
                    .iter()
                    .any(|obstacle| obstacle.label.starts_with("pair ")),
                "{how}: the plan's copper stayed on the board"
            );
            let same = |a: Point, b: Point| core::geometry::distance(a, b) < 1.0e-6;
            for net in [pair.positive, pair.negative] {
                let route = &result.routes[net as usize];
                for via in &route.vias {
                    assert!(
                        !crossing.vias.iter().flatten().any(|&at| same(at, via.at)),
                        "{how}: a crossover via stayed"
                    );
                }
                for segment in &route.segments {
                    assert!(
                        !crossing
                            .segments
                            .iter()
                            .flatten()
                            .any(|&(layer, from, to)| {
                                layer == segment.layer
                                    && ((same(from, segment.start) && same(to, segment.end))
                                        || (same(to, segment.start) && same(from, segment.end)))
                            }),
                        "{how}: a crossover track stayed"
                    );
                }
                // Nothing of the pair's nets joins nothing.
                let mut pruned = route.clone();
                prune_dangling(&restored, &mut pruned, net);
                assert_eq!(
                    (pruned.segments.len(), pruned.vias.len()),
                    (route.segments.len(), route.vias.len()),
                    "{how}: copper of {} dangles",
                    restored.nets[net as usize].name
                );
                assert_eq!(
                    result.status[net as usize],
                    core::NetStatus::Routed,
                    "{how}: the fallback left {} open",
                    restored.nets[net as usize].name
                );
            }
        }
        CROSSOVERS.with(|cell| cell.set(None));
    }

    /// The room a crossover leaves a neighbouring pair's launch: its
    /// tracks, vias and the run's lead past it against the neighbour's
    /// stubs and lead, each with its clearance; negative where it reaches
    /// in (`launch` then skips that crossover), infinite with no neighbour.
    #[test]
    fn a_crossover_keeps_room_beside_a_neighbours_launch() {
        let (board, config) = lowered();
        let pair = find_pairs(&board, &config)
            .into_iter()
            .find(|pair| !pair.links.is_empty())
            .unwrap();
        let (w, c) = (pair.width, pair.clearance);
        // A crossover along x at y = 0: a track on F.Cu, a via at (1, 0.4).
        let found = Crossover {
            run_start: [2.0, 0.0],
            segments: [vec![(0, [0.0, 0.0], [2.0, 0.0])], Vec::new()],
            vias: [Vec::new(), vec![[1.0, 0.4]]],
        };
        let lead = ([2.0, 0.0], [3.0, 0.0], w);
        let neighbour_at = |y: f64| Neighbour {
            pair: 1,
            layers: 1,
            copper: [([0.0, y], [3.0, y], w / 2.0, c); 5],
        };
        assert_eq!(neighbour_room(&found, lead, 1, &pair, &[]), f64::INFINITY);
        // Far: the via's side at y = 0.4 plus its radius and the clearance.
        let need = (pair.via_diameter / 2.0 + c).max(pair.via_drill / 2.0 + pair.hole_clearance);
        let far = neighbour_at(0.4 + need + w / 2.0 + 0.1);
        let room = neighbour_room(&found, lead, 1, &pair, &[&far]);
        assert!((room - 0.1).abs() < 1.0e-9, "{room}");
        // Close: the neighbour's stub inside the via's clearance.
        let close = neighbour_at(0.4 + need + w / 2.0 - 0.05);
        assert!(neighbour_room(&found, lead, 1, &pair, &[&close]) < 0.0);
        // On another layer the tracks and the lead do not count, the via
        // (on every layer) does.
        let other = Neighbour {
            layers: 2,
            ..neighbour_at(-(w + c + 0.01))
        };
        assert!(neighbour_room(&found, lead, 1, &pair, &[&other]) > 0.0);
    }

    /// A via of a coupled run gets a return via of GND beside it where GND
    /// has a pour there (on two layers), its ring within `RETURN_GAP` of the pair via's and
    /// clear of the pair's copper; without a pour of GND, none.
    #[test]
    fn a_pair_via_gets_a_return_via_of_ground_beside_it() {
        let crossed = BOARD.replace(
            r#"(pad "1" smd rect (at 0 0) (size 1 0.3) (layers "F.Cu") (net "/D+"))
    (pad "2" smd rect (at 0 0.5) (size 1 0.3) (layers "F.Cu") (net "/D-")))
  (footprint "test" (layer "F.Cu") (at 18 4)"#,
            r#"(pad "1" smd rect (at 0 0) (size 1 0.3) (layers "F.Cu") (net "/D-"))
    (pad "2" smd rect (at 0 0.5) (size 1 0.3) (layers "F.Cu") (net "/D+")))
  (footprint "test" (layer "F.Cu") (at 18 4)"#,
        );
        let poured = crossed.replace(
            "(gr_rect (start 0 0)",
            r#"(zone (net "/GND") (net_name "/GND") (layers "F.Cu" "B.Cu") (hatch edge 0.5) (connect_pads (clearance 0.2)) (min_thickness 0.2) (fill yes (thermal_gap 0.2) (thermal_bridge_width 0.2)) (polygon (pts (xy 0.5 0.5) (xy 39.5 0.5) (xy 39.5 19.5) (xy 0.5 19.5))))
  (gr_rect (start 0 0)"#,
        );
        assert_ne!(poured, crossed);
        let config = KiCadBoardRouterConfig {
            default_rules: Some(KiCadConnectionRoutingRules {
                trace_width_mm: 0.2,
                clearance_mm: 0.2,
                via_size_mm: 0.6,
                via_drill_mm: 0.3,
            }),
            ..KiCadBoardRouterConfig::default()
        };
        CROSSOVERS.with(|cell| cell.set(Some(true)));
        let routed = |text: &str| {
            let pcb = parse(text).unwrap();
            let mut board = lower(&pcb, &config, true).unwrap().board;
            plan_pairs(&mut board, &config);
            let mut result = core::route(&board, &core_config(&config));
            let (restored, _, report) =
                finish_pairs(&board, &mut result, &config, &[0.0, 1.6]).unwrap();
            (restored, result, report)
        };
        let (board, result, report) = routed(&poured);
        CROSSOVERS.with(|cell| cell.set(None));
        assert_eq!(report.links_coupled, 1, "{:?}", report.pairs);
        // The crossover's two vias of the diving net.
        assert_eq!(report.pair_vias, 2, "{report:?}");
        assert!(report.return_vias >= 1, "{report:?}");
        assert_eq!(report.return_net.trim_start_matches('/'), "GND");
        let ground = board
            .nets
            .iter()
            .position(|net| net.name == report.return_net)
            .unwrap();
        // They follow the router's own vias of GND (into its pour).
        let all = &result.routes[ground].vias;
        let returns = &all[all.len() - report.return_vias..];
        let pair_vias: Vec<Point> = board
            .obstacles
            .iter()
            .filter(|obstacle| obstacle.label == "existing via")
            .filter_map(|obstacle| match obstacle.shape {
                core::Shape::Circle { center, .. } => Some(center),
                _ => None,
            })
            .collect();
        for via in returns {
            let nearest = pair_vias
                .iter()
                .map(|at| core::geometry::distance(*at, via.at))
                .fold(f64::INFINITY, f64::min);
            assert!(nearest - 0.6 <= RETURN_GAP + 1.0e-9, "{nearest}");
            assert!(nearest - 0.6 >= 0.2 - 1.0e-9, "{nearest}");
        }
        assert!(core::verify(&board, &result.routes).is_empty());
        // No pour of GND: no return via.
        CROSSOVERS.with(|cell| cell.set(Some(true)));
        let (_, _, report) = routed(&crossed);
        CROSSOVERS.with(|cell| cell.set(None));
        assert_eq!(report.return_vias, 0);
        assert_eq!(report.pair_vias, 2);
    }

    /// The skew from the written copper: the tracks, and a via at the
    /// stackup's height between the layers its tracks join.
    #[test]
    fn the_skew_counts_tracks_and_vias_at_the_stackups_height() {
        let pcb = parse(
            r#"(kicad_pcb (version 20241229) (generator "pcbnew")
  (general (thickness 1.6))
  (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
  (setup (stackup
    (layer "F.Cu" (type "copper") (thickness 0.035))
    (layer "dielectric 1" (type "core") (thickness 1.5))
    (layer "B.Cu" (type "copper") (thickness 0.035))))
  (segment (start 0 0) (end 10 0) (width 0.2) (layer "F.Cu") (net "/D+"))
  (via (at 10 0) (size 0.6) (drill 0.3) (layers "F.Cu" "B.Cu") (net "/D+"))
  (segment (start 10 0) (end 15 0) (width 0.2) (layer "B.Cu") (net "/D+"))
  (segment (start 0 0.4) (end 15 0.4) (width 0.2) (layer "F.Cu") (net "/D-")))"#,
        )
        .unwrap();
        // Production callers note the board's net names before measuring;
        // the test does the same (`normalize_net` asserts it in debug).
        crate::note_net_names(&pcb);
        // A via from F.Cu to B.Cu counts the whole stack, its copper
        // included (KiCad 10, `copper_heights`).
        let heights = copper_heights(&pcb);
        assert!(
            (heights["B.Cu"] - heights["F.Cu"] - 1.57).abs() < 1.0e-9,
            "{heights:?}"
        );
        let mut report = KiCadDiffPairs {
            pairs: vec![KiCadDiffPair {
                positive: "D+".into(),
                negative: "D-".into(),
                links: 1,
                links_coupled: 1,
                skew_target_mm: 0.2,
                ..KiCadDiffPair::default()
            }],
            ..KiCadDiffPairs::default()
        };
        measure_uncoupled(&mut report, &pcb);
        let skew = &report.skews[0];
        assert!((skew.lengths_mm[0] - 16.57).abs() < 1.0e-9, "{skew:?}");
        assert!((skew.lengths_mm[1] - 15.0).abs() < 1.0e-9, "{skew:?}");
        assert!((report.skew_mm - 1.57).abs() < 1.0e-9);
    }

    /// A fallen-back pair's leftovers: what joins nothing at one end goes
    /// (a track to nowhere, a via on no copper), what joins the two pads
    /// stays.
    /// A link's route that turns back on itself (a hairpin) or jogs aside
    /// for less than its offset: the offset tracks cross or near each
    /// other there (`own_copper_clear` says so), the despiked centreline's
    /// keep the pair's gap; `finish_pairs` takes the first clean one
    /// (pairs5: 0.000 mm between a pair's own nets refused jetson's,
    /// AMLing2's and the atmega board's runs).
    #[test]
    fn a_hairpin_in_a_links_route_is_straightened_before_its_tracks_are_offset() {
        let (board, config) = lowered();
        let pair = find_pairs(&board, &config).remove(0);
        let d = pair.offset();
        let path: Vec<(Point, usize)> = [
            [0.0, 0.0],
            [1.0, 0.0],
            [6.0, 0.0],
            // Back by 1 mm and aside: a hairpin.
            [5.0, 0.4],
            [5.0, 5.0],
            [5.0, 6.0],
        ]
        .into_iter()
        .map(|point| (point, 0))
        .collect();
        let crossing = link_copper(&path, &pair, 1.0, 1.0).unwrap();
        assert!(!own_copper_clear(&crossing, &pair));
        let straightened = despiked(&path, 2.5 * d);
        assert!(straightened.len() < path.len(), "{straightened:?}");
        let clean = link_copper(&straightened, &pair, 1.0, 1.0).unwrap();
        assert!(own_copper_clear(&clean, &pair));
        // A short jog aside (shorter than the offset) between two turns
        // the same way: its inner offset folds back toward the partner.
        let jog: Vec<(Point, usize)> = [
            [0.0, 0.0],
            [1.0, 0.0],
            [5.0, 0.0],
            [5.0 + 0.3 * d, 0.3 * d],
            [5.0 + 0.3 * d, 5.0],
            [5.0 + 0.3 * d, 6.0],
        ]
        .into_iter()
        .map(|point| (point, 0))
        .collect();
        let merged = despiked(&jog, 2.5 * d);
        let clean = link_copper(&merged, &pair, 1.0, 1.0).unwrap();
        assert!(own_copper_clear(&clean, &pair));
    }

    /// A link's route that changes layer in a turn: the vias stand on a
    /// straight piece once the change moves back along the segment before
    /// it (`vias_on_straights`), and the two nets keep their gap.
    #[test]
    fn a_links_layer_change_in_a_turn_moves_onto_a_straight_piece() {
        let (board, config) = lowered();
        let pair = find_pairs(&board, &config).remove(0);
        for turn in [[0.0, 1.0], [0.0, -1.0], [1.0, 1.0], [1.0, -1.0]] {
            let corner = [5.0, 0.0];
            let far = add(corner, scale(unit(turn), 3.0));
            let path: Vec<(Point, usize)> = vec![
                ([0.0, 0.0], 0),
                ([1.0, 0.0], 0),
                (corner, 0),
                (corner, 1),
                (far, 1),
                (add(far, scale(unit(turn), 1.0)), 1),
            ];
            let shift = 2.0 * pair.via_offset() + pair.width;
            let moved = vias_on_straights(&path, shift);
            assert_eq!(moved.len(), path.len() + 1, "{moved:?}");
            // The layer change is on the straight piece before the turn.
            assert!((moved[2].0[0] - (5.0 - shift)).abs() < 1.0e-9, "{moved:?}");
            assert_eq!((moved[2].1, moved[3].1, moved[4].1), (0, 1, 1));
            let copper = link_copper(&moved, &pair, 1.0, 1.0).unwrap();
            assert!(own_copper_clear(&copper, &pair), "turn {turn:?}");
            assert_eq!(copper.vias[0].len(), 1);
        }
    }

    /// The run of AMLing2's drone carrier's USB2 pair as the router routed
    /// it (pairs5): a layer change followed by a 0.14 mm diagonal and a
    /// turn. Offset as it is, the two nets' vias overlap; of the
    /// centrelines `finish_pairs` tries one keeps the pair's gap.
    #[test]
    fn a_routed_run_with_a_via_before_a_short_diagonal_gets_a_clean_centreline() {
        let (board, config) = lowered();
        let mut pair = find_pairs(&board, &config).remove(0);
        (pair.width, pair.gap, pair.clearance) = (0.21, 0.13, 0.149);
        (pair.via_diameter, pair.via_drill) = (0.45, 0.25);
        let path: Vec<(Point, usize)> = [
            ([158.033, 121.016], 0),
            ([157.818, 120.644], 0),
            ([157.498, 120.2], 0),
            ([157.198, 119.9], 0),
            ([157.198, 118.9], 0),
            ([160.398, 118.9], 0),
            ([160.398, 118.9], 1),
            ([160.498, 119.0], 1),
            ([160.498, 127.8], 1),
            ([159.598, 128.7], 1),
            ([159.598, 132.8], 1),
            ([160.398, 133.6], 1),
            ([160.398, 133.6], 0),
            ([161.298, 134.5], 0),
            ([164.498, 134.5], 0),
            ([165.498, 135.5], 0),
            ([165.608, 135.939], 0),
            ([165.608, 136.37], 0),
        ]
        .into_iter()
        .collect();
        let raw = link_copper(&path, &pair, -1.0, 1.0).unwrap();
        assert!(!own_copper_clear(&raw, &pair));
        let clean: Vec<Vec<(Point, usize)>> = centrelines(&path, &pair)
            .into_iter()
            .filter(|line| {
                link_copper(line, &pair, -1.0, 1.0)
                    .is_some_and(|copper| own_copper_clear(&copper, &pair))
            })
            .collect();
        assert!(!clean.is_empty());
        // The layer changes stay two.
        for line in &clean {
            let changes = line
                .windows(2)
                .filter(|step| step[0].1 != step[1].1)
                .count();
            assert_eq!(changes, 2, "{line:?}");
        }
    }

    /// A stub that reaches the run through another pad of its net on the
    /// way (an ESD diode's pin between the connector and the run: right_io's
    /// ETH_MDI pairs through U501 and U502) reaches it; through the pad of
    /// the link's other end it does not.
    #[test]
    fn a_stub_through_another_pad_of_its_net_reaches_the_run() {
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
        // The pad, an ESD pin on the way, the link's other end's pad.
        board.obstacles = vec![pad(0.0), pad(2.0), pad(9.0)];
        let segment = |start: Point, end: Point| core::Segment {
            layer: 0,
            start,
            end,
            width: 0.2,
        };
        let route = core::NetRoute {
            segments: vec![
                segment([0.0, 0.0], [1.8, 0.0]),
                segment([2.2, 0.0], [4.0, 0.0]),
            ],
            vias: Vec::new(),
        };
        let lead = ([4.0, 0.0], [4.2, 0.0]);
        assert!(stub_joined(
            &board,
            &route,
            0,
            ([0.0, 0.0], [9.0, 0.0]),
            lead,
            0.2
        ));
        // Without the ESD pin's copper the two tracks do not meet.
        board.obstacles.remove(1);
        assert!(!stub_joined(
            &board,
            &route,
            0,
            ([0.0, 0.0], [9.0, 0.0]),
            lead,
            0.2
        ));
        // Through the other end's pad: not this end's stub.
        board.obstacles = vec![pad(0.0), pad(2.0)];
        assert!(!stub_joined(
            &board,
            &route,
            0,
            ([0.0, 0.0], [2.0, 0.0]),
            lead,
            0.2
        ));
    }

    /// A link whose copper holds but whose pad's stub the attempt left out
    /// (the router joined the pad another way, or left it open): the stub
    /// is routed after the attempt from the pad to its end of the run, the
    /// link is kept, and the pad's old way, now a loop, goes.
    #[test]
    fn a_stub_the_attempt_left_out_is_routed_after_it() {
        let (mut board, config) = lowered();
        plan_pairs(&mut board, &config);
        let plan = plan_of(&board).unwrap();
        // The test board's link carries straight stubs at both ends: make
        // the second end's the router's.
        let mut link = plan.links[0].clone();
        assert_eq!(link.fixed, [true, true]);
        let mut result = core::route(&board, &core_config(&config));
        let (_, _, before) =
            finish_pairs(&board, &mut result.clone(), &config, &[0.0, 1.6]).unwrap();
        assert_eq!(before.links_coupled, 1);
        // The pads' stubs at the second end, dropped from a routed result:
        // a route from the far pad straight to the first end's pad instead.
        link.fixed = [true, false];
        let pair = &plan.pairs[link.pair];
        let mut changed = (*plan).clone();
        changed.links[0] = link.clone();
        let mut board = board.clone();
        board.attachment.data = Some(std::sync::Arc::new(changed));
        // D+ round below the board's middle, D- round above it.
        let detours: Vec<Vec<Point>> = (0..2)
            .map(|polarity| {
                let [a, b] = [link.pads[0][polarity], link.pads[1][polarity]];
                let (x, y) = if polarity == 0 {
                    (35.0, 7.0)
                } else {
                    (35.5, 14.0)
                };
                let left = if polarity == 0 { a[0] } else { 2.0 };
                vec![b, [x, b[1]], [x, y], [left, y], [left, a[1]], a]
            })
            .collect();
        for (polarity, net) in [(0, pair.positive), (1, pair.negative)] {
            result.routes[net as usize].segments = detours[polarity]
                .windows(2)
                .filter(|step| core::geometry::distance(step[0], step[1]) > 1.0e-9)
                .map(|step| core::Segment {
                    layer: 0,
                    start: step[0],
                    end: step[1],
                    width: 0.2,
                })
                .collect();
            result.routes[net as usize].vias.clear();
        }
        let (_, _, report) = finish_pairs(&board, &mut result, &config, &[0.0, 1.6]).unwrap();
        assert_eq!(report.links_coupled, 1, "{:?}", report.pairs);
        assert_eq!(report.stubs_routed, 2, "{:?}", report.pairs);
        // The old way round is gone: each net's tracks end at the run.
        for (polarity, net) in [(0, pair.positive), (1, pair.negative)] {
            let far = detours[polarity][2];
            assert!(
                !result.routes[net as usize].segments.iter().any(
                    |segment| core::geometry::distance(segment.start, far) < 1.0e-6
                        || core::geometry::distance(segment.end, far) < 1.0e-6
                ),
                "{:?}",
                result.routes[net as usize].segments
            );
        }
    }

    /// Two links' copper that meet: both are refused, but where one lies
    /// on the router's route and the other was moved off it
    /// (`centrelines`), the moved one only (pairs5: on jetson a link's
    /// moved via took a coupled neighbour down with it).
    #[test]
    fn copper_moved_off_the_route_is_refused_alone_where_it_meets_a_link_on_its_route() {
        let (board, config) = lowered();
        let pairs = find_pairs(&board, &config);
        let group = |segments: [Vec<(usize, Point, Point)>; 2], moved: bool| Group {
            pair: 0,
            segments,
            vias: Default::default(),
            run: [0..1, 0..1],
            moved,
        };
        let routes = vec![core::NetRoute::default(); board.nets.len()];
        let across = || [vec![(0, [20.0, 15.0], [24.0, 15.0])], Vec::new()];
        let along = || [Vec::new(), vec![(0, [22.0, 13.0], [22.0, 17.0])]];
        for (moved, refused) in [
            ([false, false], [true, true]),
            ([true, false], [true, false]),
            ([false, true], [false, true]),
            ([true, true], [true, true]),
        ] {
            let groups = [group(across(), moved[0]), group(along(), moved[1])];
            let why = refused_groups(&board, &routes, &pairs, &groups);
            assert_eq!(
                why.iter().map(Option::is_some).collect::<Vec<_>>(),
                refused,
                "{moved:?}: {why:?}"
            );
        }
    }

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
                    span: None,
                },
                // A via that joins nothing.
                core::Via {
                    at: [8.0, 5.0],
                    diameter: 0.6,
                    drill: 0.3,
                    span: None,
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
