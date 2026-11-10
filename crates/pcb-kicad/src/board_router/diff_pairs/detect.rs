// Copyright (C) 2026 Toit contributors.

//! Detection: the pairs of a lowered board by name, with their rules, their
//! pair points (a positive and a negative pad side by side) and the links
//! between pair points of different parts (`find_pairs`).

use super::*;
use pcb_router as core;

/// The largest distance between the centres of a pair's two pads on one
/// part that makes them a pair point.
pub(super) const PAIR_POINT_SPAN: f64 = 3.0;
/// Room between the two stubs' ends beyond the router's clearance.
pub(super) const ENDS_MARGIN: f64 = 0.01;

/// A pair found on the board: its nets and its geometry.
#[derive(Clone)]
pub(super) struct Pair {
    pub positive: core::NetId,
    pub negative: core::NetId,
    /// The pair's track width and gap on the layers it may use.
    pub width: f64,
    pub gap: f64,
    /// Layers where the rules give that width and gap.
    pub layers: core::LayerMask,
    pub source: String,
    /// The clearance of the two nets to others (the larger class's).
    pub clearance: f64,
    pub via_diameter: f64,
    pub via_drill: f64,
    /// The board's hole clearance and hole to hole.
    pub hole_clearance: f64,
    pub hole_to_hole: f64,
    /// Pair points: (positive terminal, negative terminal), and what each
    /// is (`PointKind`).
    pub points: Vec<(usize, usize)>,
    pub kinds: Vec<PointKind>,
    /// Links between pair points of different parts: indices into
    /// `points`.
    pub links: Vec<(usize, usize)>,
    /// The custom rules' `skew` maximum and `length` (min, opt, max) for
    /// the pair's nets, if any.
    pub max_skew: Option<f64>,
    pub length: Option<(f64, Option<f64>, f64)>,
}

/// What makes a pair point (`Pair::points`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PointKind {
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
    pub(super) static EXTENDED: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

/// The part of a terminal's label (`REF.pad`): its reference, or the whole
/// label where it has no pad.
fn part_reference(label: &str) -> String {
    label
        .rsplit_once('.')
        .map_or(label, |(part, _)| part)
        .to_string()
}

/// A minimum spanning tree over a pair's pair points (Prim), where pair
/// points of one part are joined first (at no cost) and by ordinary
/// routing (a USB-C receptacle's two rows): only links between parts are
/// coupled.
fn spanning_links(p: &core::Net, n: &core::Net, points: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let middle = |point: &(usize, usize)| {
        scale(
            add(p.terminals[point.0].anchor, n.terminals[point.1].anchor),
            0.5,
        )
    };
    let part = |index: usize| {
        let point = &points[index];
        let (part_p, part_n) = (
            part_reference(&p.terminals[point.0].label),
            part_reference(&n.terminals[point.1].label),
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
    pub(super) fn offset(&self) -> f64 {
        (self.width + self.gap) / 2.0
    }
    pub(super) fn envelope(&self) -> f64 {
        2.0 * self.width + self.gap
    }
    /// How far from the run's centreline the pads' stubs end: the offset,
    /// or as far apart as the router keeps the two nets (its clearance, not
    /// the pair's gap, between their stubs), so that both ends have legal
    /// nodes; a jog joins them to the run.
    pub(super) fn ends_offset(&self) -> f64 {
        self.offset()
            .max((self.width + self.clearance) / 2.0 + ENDS_MARGIN)
    }
    /// Half the distance between the two vias' centres: the vias keep the
    /// clearance from each other and from the other net's track.
    pub(super) fn via_offset(&self) -> f64 {
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

/// A pair's width, gap, layers and where they came from
/// (`Pair::width`, `gap`, `layers`, `source`).
type PairGeometry = (f64, f64, core::LayerMask, String);

/// A pair's pair points, what each is and its links (`Pair::points`,
/// `kinds`, `links`).
type PairPoints = (Vec<(usize, usize)>, Vec<PointKind>, Vec<(usize, usize)>);

/// The pairs of a lowered board, by name, with their rules.
pub(super) fn find_pairs(board: &core::Board, config: &KiCadBoardRouterConfig) -> Vec<Pair> {
    let ids: BTreeMap<&str, core::NetId> = board
        .nets
        .iter()
        .enumerate()
        .map(|(index, net)| (net.name.as_str(), index as core::NetId))
        .collect();
    let mut pairs = Vec::new();
    let parts = part_pads(board);
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
        let clearance = pair_clearance(
            config,
            [n.name.as_str(), net.name.as_str()],
            class_p.clearance.max(class_n.clearance),
        );
        let (max_skew, length) = config
            .diff_pair_rules
            .get(&net.name)
            .map_or((None, None), |rule| (rule.max_skew, rule.length));
        let (width, gap, layers, source) =
            pair_geometry(board, config, &net.name, class_p.trace_width, clearance);
        let (points, kinds, links) = pair_points(board, &parts, [positive, negative]);
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

/// Per part (its reference): its pads' centres.
fn part_pads(board: &core::Board) -> BTreeMap<String, Vec<Point>> {
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
    parts
}

/// The classes' clearance (`classes`), raised to every custom pair
/// clearance that may hold for the pair's copper (ducktop2's 0.18 mm
/// between vias and tracks over 0.15 classes): the coupled run and the vias'
/// spacing keep it; the check of the copper holds the classes.
fn pair_clearance(config: &KiCadBoardRouterConfig, names: [&str; 2], classes: f64) -> f64 {
    config
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
        .fold(classes, f64::max)
}

/// The width, gap and layers of the pair of the positive net `name`: the
/// net class's width and the pair's clearance, on every layer, unless the
/// rules name the pair.
fn pair_geometry(
    board: &core::Board,
    config: &KiCadBoardRouterConfig,
    name: &str,
    class_width: f64,
    clearance: f64,
) -> PairGeometry {
    let (mut width, mut gap, mut layers, mut source) = (
        class_width,
        clearance,
        board.all_layers(),
        "net class width and clearance".to_string(),
    );
    if let Some(rule) = config.diff_pair_rules.get(name) {
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
    (width, gap, layers, source)
}

/// The pair points of the pair of `nets` (positive, negative), what each
/// is, and the links between them (`spanning_links`).
fn pair_points(
    board: &core::Board,
    parts: &BTreeMap<String, Vec<Point>>,
    nets: [core::NetId; 2],
) -> PairPoints {
    let (p, n) = (&board.nets[nets[0] as usize], &board.nets[nets[1] as usize]);
    let (mut used_p, mut used_n) = (BTreeSet::new(), BTreeSet::new());
    let mut points = side_points(board, parts, nets, (&mut used_p, &mut used_n));
    points.sort();
    let mut kinds = vec![PointKind::Side; points.len()];
    let mut links = spanning_links(p, n, &points);
    // A pair with no link (fewer than two parts with its pads side by
    // side): its pads of one part with another net's pin between them
    // are a pair point too (docs/router.md, "A pair point across a
    // pin").
    if links.is_empty() && extended_points() {
        for point in pin_points(board, nets, (&mut used_p, &mut used_n)) {
            points.push(point);
            kinds.push(PointKind::Pin);
        }
        links = spanning_links(p, n, &points);
    }
    if links.is_empty() && pairs_why() {
        side_why(board, parts, nets, &points);
    }
    if pairs_why() && kinds.iter().any(|kind| *kind != PointKind::Side) {
        points_why(p, n, &points, &kinds, links.len());
    }
    (points, kinds, links)
}

/// Pair points: a positive and a negative pad of one part, nearest first,
/// or a pad of each of two two-pad parts side by side (the pair's series
/// or AC-coupling capacitors, resistors, ESD diodes: each part's other pad
/// beside the other's), after those of one part: the pair is coupled up to
/// the parts and on from them. Each pad in one point at most (`used`).
fn side_points(
    board: &core::Board,
    parts: &BTreeMap<String, Vec<Point>>,
    nets: [core::NetId; 2],
    (used_p, used_n): (&mut BTreeSet<usize>, &mut BTreeSet<usize>),
) -> Vec<(usize, usize)> {
    let (p, n) = (&board.nets[nets[0] as usize], &board.nets[nets[1] as usize]);
    let mut candidates: Vec<(bool, f64, usize, usize)> = Vec::new();
    for (a, ta) in p.terminals.iter().enumerate() {
        for (b, tb) in n.terminals.iter().enumerate() {
            let span = core::geometry::distance(ta.anchor, tb.anchor);
            let (part_a, part_b) = (part_reference(&ta.label), part_reference(&tb.label));
            let series =
                part_a != part_b && series_parts(parts, &part_a, &part_b, ta.anchor, tb.anchor);
            if span <= PAIR_POINT_SPAN
                && span > EPSILON
                && (part_a == part_b || series)
                && ta.layers & tb.layers != 0
                && !straddles(board, ta, tb, nets)
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
    let mut points = Vec::new();
    for (_, _, a, b) in candidates {
        if !used_p.contains(&a) && !used_n.contains(&b) {
            used_p.insert(a);
            used_n.insert(b);
            points.push((a, b));
        }
    }
    points
}

/// The pair points across a pin (`PointKind::Pin`): pads of one part with
/// another net's copper between them, nearest first, of the pads not used.
fn pin_points(
    board: &core::Board,
    nets: [core::NetId; 2],
    (used_p, used_n): (&mut BTreeSet<usize>, &mut BTreeSet<usize>),
) -> Vec<(usize, usize)> {
    let (p, n) = (&board.nets[nets[0] as usize], &board.nets[nets[1] as usize]);
    let mut pins: Vec<(f64, usize, usize)> = Vec::new();
    for (a, ta) in p.terminals.iter().enumerate() {
        for (b, tb) in n.terminals.iter().enumerate() {
            let span = core::geometry::distance(ta.anchor, tb.anchor);
            if span <= PAIR_POINT_SPAN
                && span > EPSILON
                && part_reference(&ta.label) == part_reference(&tb.label)
                && ta.layers & tb.layers != 0
                && !used_p.contains(&a)
                && !used_n.contains(&b)
                && straddles(board, ta, tb, nets)
            {
                pins.push((span, a, b));
            }
        }
    }
    pins.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)).then(x.2.cmp(&y.2)));
    let mut points = Vec::new();
    for (_, a, b) in pins {
        if !used_p.contains(&a) && !used_n.contains(&b) {
            used_p.insert(a);
            used_n.insert(b);
            points.push((a, b));
        }
    }
    points
}

/// `PCB_ROUTER_PAIRS_WHY`: a pair's points with what each is, where one is
/// not of pads side by side (`pairs why: points`).
fn points_why(
    p: &core::Net,
    n: &core::Net,
    points: &[(usize, usize)],
    kinds: &[PointKind],
    links: usize,
) {
    let described: Vec<String> = points
        .iter()
        .zip(kinds)
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
        links,
        described.join(", ")
    );
}

/// Whether parts `a` and `b` (references) are two-pad parts side by side:
/// their pads at `at_a` and `at_b`, and their other pads as close
/// (`PAIR_POINT_SPAN`: a pair's series capacitors, not two parts that
/// merely touch at one end).
pub(super) fn series_parts(
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
    let pads_of = |label: &str| parts.get(&part_reference(label)).map_or(0, Vec::len);
    let mut straddled = Vec::new();
    let mut far = Vec::new();
    let mut loose = Vec::new();
    for ta in &p.terminals {
        let mut nearest: Option<(f64, &core::Terminal)> = None;
        for tb in &n.terminals {
            let span = core::geometry::distance(ta.anchor, tb.anchor);
            if part_reference(&ta.label) == part_reference(&tb.label) {
                if span <= PAIR_POINT_SPAN {
                    let between = copper_between(board, ta, tb, nets);
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

/// Another net's copper on the line between two pads, by label and net
/// (`side_why`).
fn copper_between(
    board: &core::Board,
    ta: &core::Terminal,
    tb: &core::Terminal,
    nets: [core::NetId; 2],
) -> Vec<String> {
    board
        .obstacles
        .iter()
        .filter(|obstacle| {
            obstacle.kind == core::ObstacleKind::Copper
                && obstacle.layers & ta.layers & tb.layers != 0
                && obstacle.net.is_none_or(|net| !nets.contains(&net))
                && obstacle.shape.distance_to_segment(ta.anchor, tb.anchor) < EPSILON
        })
        .map(|obstacle| {
            let net = obstacle
                .net
                .map_or("-".to_string(), |net| board.nets[net as usize].name.clone());
            format!("{} ({net})", obstacle.label)
        })
        .collect()
}

/// Whether another net's copper lies between two pads (a pin between
/// them: they are not side by side).
pub(super) fn straddles(
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
