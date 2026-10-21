// Copyright (C) 2026 Toit contributors.

//! Board outlines from arbitrary Edge.Cuts graphics: lines, arcs, rectangles,
//! polygons and circles are chained into closed loops. The largest loop is
//! the board; all others are cutouts.

use super::*;
use std::collections::HashMap;

pub(super) struct BoardLoops {
    pub outline: Vec<[f64; 2]>,
    pub cutouts: Vec<Vec<[f64; 2]>>,
    /// Bounding boxes ([min x, min y, max x, max y]) of the board's pieces,
    /// the largest first: one for an ordinary board.
    pub pieces: Vec<[f64; 4]>,
    /// The pieces' own loops, the largest first (`outline` joins them by
    /// zero-width bridges).
    pub piece_loops: Vec<Vec<[f64; 2]>>,
    /// Per cutout: drawn by the board's own graphics, not a footprint's (a
    /// slot a connector brings): the cutouts an `outline` constraint cut.
    pub board_cutouts: Vec<bool>,
}

/// Longest chord error accepted when flattening arcs.
pub(super) const ARC_TOLERANCE: f64 = 0.005;

/// The points a line, rectangle or arc passes through, an arc flattened
/// (its start, middle and end alone miss its bulge): for extents.
pub(super) fn outline_points(item: &Expr) -> Result<Vec<[f64; 2]>, String> {
    if item.child("start").is_some() && item.child("mid").is_some() && item.child("end").is_some() {
        return Ok(arc_points(
            form_xy(item, "start")?,
            form_xy(item, "mid")?,
            form_xy(item, "end")?,
        ));
    }
    let mut points = Vec::new();
    for head in ["start", "end"] {
        if item.child(head).is_some() {
            points.push(form_xy(item, head)?);
        }
    }
    Ok(points)
}

/// A full circle as a closed chain of points, the first repeated last
/// (chords within `ARC_TOLERANCE`, as `arc_points`).
pub(super) fn circle_points(center: [f64; 2], radius: f64) -> Vec<[f64; 2]> {
    let at = |dx: f64, dy: f64| [center[0] + dx, center[1] + dy];
    let mut points = arc_points(at(radius, 0.0), at(0.0, radius), at(-radius, 0.0));
    points.pop();
    points.extend(arc_points(
        at(-radius, 0.0),
        at(0.0, -radius),
        at(radius, 0.0),
    ));
    points
}

/// A polygon's corners from its `pts`: `xy` points and `arc` entries (a
/// polygon with rounded sides, KiCad 7 on), flattened as `arc_points` does.
pub(super) fn pts_points(item: &Expr) -> Result<Vec<[f64; 2]>, String> {
    let mut points: Vec<[f64; 2]> = Vec::new();
    for point in item
        .child("pts")
        .map(Expr::children)
        .unwrap_or_default()
        .iter()
    {
        match point.head() {
            Some("xy") => points.push([
                expression_coordinate(point, 1, "polygon x")?,
                expression_coordinate(point, 2, "polygon y")?,
            ]),
            Some("arc") => {
                let (start, mid, end) = (
                    form_xy(point, "start")?,
                    form_xy(point, "mid")?,
                    form_xy(point, "end")?,
                );
                // An arc that ends where it starts is a full circle, as
                // KiCad reads it (its arc centre is then halfway between
                // start and mid): rule areas drawn as one closed arc (the
                // 30led card's 26 mm NFC keepout, mounting-hole keepouts)
                // were two points.
                let arc = match full_circle(start, mid).filter(|_| closed_arc(start, end)) {
                    Some(mut circle) => {
                        circle.push(start);
                        circle
                    }
                    None => arc_points(start, mid, end),
                };
                for at in arc {
                    if points.last() != Some(&at) {
                        points.push(at);
                    }
                }
            }
            _ => {}
        }
    }
    // The closing point repeated is dropped, unless that leaves fewer than
    // three: a degenerate polygon (BratwurstPower's 2 nm wide copper
    // slivers, [a, b, a]) stays the zero-area shape KiCad keeps, not an
    // error.
    if points.len() > 3 && points.first() == points.last() {
        points.pop();
    }
    Ok(points)
}

/// A full circle through `start` and the opposite point `mid` (an arc
/// whose end is its start, as KiCad draws a circle with an arc), closed:
/// its points from `start` around and back to just before `start`. None
/// when `start` and `mid` coincide.
pub(super) fn full_circle(start: [f64; 2], mid: [f64; 2]) -> Option<Vec<[f64; 2]>> {
    if distance_squared(start, mid).sqrt() <= 1.0e-6 {
        return None;
    }
    let center = [(start[0] + mid[0]) / 2.0, (start[1] + mid[1]) / 2.0];
    let quarter = [
        center[0] - (start[1] - center[1]),
        center[1] + (start[0] - center[0]),
    ];
    let back = [2.0 * center[0] - quarter[0], 2.0 * center[1] - quarter[1]];
    let mut points = arc_points(start, quarter, mid);
    points.pop();
    points.extend(arc_points(mid, back, start));
    points.pop();
    Some(points)
}

/// How many different points `points` holds (a polygon with fewer than
/// three is degenerate).
pub(super) fn distinct_points(points: &[[f64; 2]]) -> usize {
    let mut seen: Vec<[f64; 2]> = Vec::new();
    for point in points {
        if !seen.contains(point) {
            seen.push(*point);
            if seen.len() >= 3 {
                break;
            }
        }
    }
    seen.len()
}

/// Whether an arc ends where it starts.
fn closed_arc(start: [f64; 2], end: [f64; 2]) -> bool {
    distance_squared(start, end).sqrt() < 1.0e-6
}

pub(super) fn arc_points(start: [f64; 2], mid: [f64; 2], end: [f64; 2]) -> Vec<[f64; 2]> {
    // An arc ending where it starts is a whole circle to KiCad, `mid`
    // across from `start` (cnlohr's tadpoleswadge: MK1's ring pad, a custom
    // pad's `(gr_arc (start 0 0) (mid 2.25 0) (end 0 0))`, shorted a track
    // the two-point "arc" missed).
    if distance_squared(start, end) < 1.0e-12 && distance_squared(start, mid) > 1.0e-12 {
        let center = [(start[0] + mid[0]) / 2.0, (start[1] + mid[1]) / 2.0];
        return circle_points(center, distance_squared(center, start).sqrt());
    }
    // Circle through three points.
    let d = 2.0
        * (start[0] * (mid[1] - end[1])
            + mid[0] * (end[1] - start[1])
            + end[0] * (start[1] - mid[1]));
    if d.abs() < 1.0e-12 {
        return vec![start, end];
    }
    let square = |p: [f64; 2]| p[0] * p[0] + p[1] * p[1];
    let center = [
        (square(start) * (mid[1] - end[1])
            + square(mid) * (end[1] - start[1])
            + square(end) * (start[1] - mid[1]))
            / d,
        (square(start) * (end[0] - mid[0])
            + square(mid) * (start[0] - end[0])
            + square(end) * (mid[0] - start[0]))
            / d,
    ];
    let radius = distance_squared(center, start).sqrt();
    let angle = |p: [f64; 2]| (p[1] - center[1]).atan2(p[0] - center[0]);
    let (a0, a1, a2) = (angle(start), angle(mid), angle(end));
    let tau = std::f64::consts::TAU;
    // Sweep from start to end in the direction that passes through mid.
    let forward = (a2 - a0).rem_euclid(tau);
    let through = (a1 - a0).rem_euclid(tau);
    let sweep = if through <= forward {
        forward
    } else {
        forward - tau
    };
    let step = 2.0
        * (1.0 - ARC_TOLERANCE / radius.max(ARC_TOLERANCE))
            .clamp(-1.0, 1.0)
            .acos();
    let pieces = ((sweep.abs() / step.max(1.0e-3)).ceil() as usize).clamp(2, 720);
    let mut points = Vec::with_capacity(pieces + 1);
    points.push(start);
    for index in 1..pieces {
        let a = a0 + sweep * index as f64 / pieces as f64;
        points.push([center[0] + radius * a.cos(), center[1] + radius * a.sin()]);
    }
    points.push(end);
    points
}

/// Appends the points after the first of a cubic Bezier curve, flattened
/// until its control points lie within `ARC_TOLERANCE` of each chord (which
/// bounds the curve's distance from it), by halving (de Casteljau).
fn bezier_points(controls: [[f64; 2]; 4], depth: usize, points: &mut Vec<[f64; 2]>) {
    let [a, b, c, d] = controls;
    let deviation = |point: [f64; 2]| {
        let (dx, dy) = (d[0] - a[0], d[1] - a[1]);
        let length = (dx * dx + dy * dy).sqrt();
        if length < 1.0e-12 {
            distance_squared(point, a).sqrt()
        } else {
            ((point[0] - a[0]) * dy - (point[1] - a[1]) * dx).abs() / length
        }
    };
    if depth >= 16 || deviation(b).max(deviation(c)) <= ARC_TOLERANCE {
        points.push(d);
        return;
    }
    let half = |p: [f64; 2], q: [f64; 2]| [(p[0] + q[0]) / 2.0, (p[1] + q[1]) / 2.0];
    let (ab, bc, cd) = (half(a, b), half(b, c), half(c, d));
    let (abc, bcd) = (half(ab, bc), half(bc, cd));
    let middle = half(abc, bcd);
    bezier_points([a, ab, abc, middle], depth + 1, points);
    bezier_points([middle, bcd, cd, d], depth + 1, points);
}

fn polygon_area(points: &[[f64; 2]]) -> f64 {
    (0..points.len())
        .map(|index| {
            let (a, b) = (points[index], points[(index + 1) % points.len()]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum::<f64>()
        .abs()
        / 2.0
}

/// Closed loops chained from open pieces, each with whether all its
/// pieces are the board's own graphics.
type Chained = Vec<(Vec<[f64; 2]>, bool)>;

const NOT_CLOSED: &str = "Edge.Cuts graphics do not form closed loops";

/// KiCad's chaining distance (`BOARD::GetBoardPolygonOutlines`), in
/// nanometres: an end joins the next piece's end within it.
const KICAD_CHAINING_EPSILON_NM: i64 = 20_000;

/// The open pieces chained as KiCad's `ConvertOutlineToPolygon` chains
/// them, in its integer nanometres: from the current end, an unused piece
/// with an end exactly there; else the nearest end of any other piece,
/// used or not, closer than the chaining distance. An unused one is
/// followed; otherwise the chain closes when its end is within the
/// distance of its start, and the outline is not closed when it is not.
/// Pieces start in their order (KiCad's first outline starts at the
/// leftmost piece; which loop starts where does not change the loops).
fn chain_as_kicad(open: &[Vec<[f64; 2]>], open_on_board: &[bool]) -> Result<Chained, String> {
    type Point = (i64, i64);
    let nm = |point: [f64; 2]| -> Point {
        (
            (point[0] * 1.0e6).round() as i64,
            (point[1] * 1.0e6).round() as i64,
        )
    };
    let ends: Vec<[Point; 2]> = open
        .iter()
        .map(|piece| [nm(piece[0]), nm(*piece.last().unwrap())])
        .collect();
    let cell = |point: Point| {
        (
            point.0.div_euclid(KICAD_CHAINING_EPSILON_NM),
            point.1.div_euclid(KICAD_CHAINING_EPSILON_NM),
        )
    };
    let mut exact: HashMap<Point, Vec<usize>> = HashMap::new();
    let mut cells: HashMap<Point, Vec<usize>> = HashMap::new();
    for (index, pair) in ends.iter().enumerate() {
        for end in pair {
            exact.entry(*end).or_default().push(index);
            let list = cells.entry(cell(*end)).or_default();
            if list.last() != Some(&index) {
                list.push(index);
            }
        }
    }
    let squared = |a: Point, b: Point| {
        let (dx, dy) = ((a.0 - b.0) as i128, (a.1 - b.1) as i128);
        dx * dx + dy * dy
    };
    let limit = (KICAD_CHAINING_EPSILON_NM as i128).pow(2);
    let mut used = vec![false; open.len()];
    let mut chained = Vec::new();
    for first in 0..open.len() {
        if used[first] {
            continue;
        }
        used[first] = true;
        let mut chain = open[first].clone();
        let mut on_board = open_on_board[first];
        let start = ends[first][0];
        let mut tail = ends[first][1];
        let mut current = first;
        loop {
            // An unused piece with an end exactly here, the first in order.
            let hit = exact.get(&tail).and_then(|indices| {
                indices
                    .iter()
                    .copied()
                    .find(|&index| index != current && !used[index])
            });
            // Else the nearest end of any other piece within the distance.
            let next = hit.or_else(|| {
                let (cx, cy) = cell(tail);
                let mut best: Option<(i128, usize)> = None;
                for x in cx - 1..=cx + 1 {
                    for y in cy - 1..=cy + 1 {
                        for &index in cells.get(&(x, y)).into_iter().flatten() {
                            if index == current {
                                continue;
                            }
                            for end in ends[index] {
                                let distance = squared(tail, end);
                                if distance < limit
                                    && best.is_none_or(|(nearest, at)| {
                                        (distance, index) < (nearest, at)
                                    })
                                {
                                    best = Some((distance, index));
                                }
                            }
                        }
                    }
                }
                best.map(|(_, index)| index)
            });
            match next {
                Some(index) if !used[index] => {
                    used[index] = true;
                    on_board &= open_on_board[index];
                    let mut piece = open[index].clone();
                    if squared(ends[index][1], tail) < squared(ends[index][0], tail) {
                        piece.reverse();
                    }
                    for point in &piece[1..] {
                        if chain.last() != Some(point) {
                            chain.push(*point);
                        }
                    }
                    tail = nm(*chain.last().unwrap());
                    current = index;
                }
                _ if squared(start, tail) <= limit => {
                    if chain.len() > 1 && nm(*chain.last().unwrap()) == nm(chain[0]) {
                        chain.pop();
                    }
                    break;
                }
                _ => return Err(NOT_CLOSED.into()),
            }
        }
        if chain.len() >= 3 {
            chained.push((chain, on_board));
        }
    }
    Ok(chained)
}

/// The open pieces (lines, arcs, curves as point chains) chained into
/// closed loops, with whether each loop is all the board's own graphics.
fn chain_pieces(
    mut open: Vec<Vec<[f64; 2]>>,
    mut open_on_board: Vec<bool>,
) -> Result<Chained, String> {
    let mut chained = Vec::new();
    // End points closer than this belong together: drawings exported by
    // other tools miss by a few micrometres, and KiCad chains them.
    // End points closer than this belong together: arcs converted from
    // older KiCad files miss their neighbours by up to 10 um, and KiCad
    // chains them. Each end joins the nearest point already seen, never
    // the other end of its own piece, so outlines drawn from 0.01 mm
    // segments keep their shape.
    const CHAIN_TOLERANCE: f64 = 0.02;
    // A piece shorter than that (a 3 um sliver some drawings carry) is
    // no edge: dropped, its neighbours' ends then chain to each other.
    let long: Vec<bool> = open
        .iter()
        .map(|piece| {
            piece
                .windows(2)
                .map(|pair| distance_squared(pair[0], pair[1]).sqrt())
                .sum::<f64>()
                > CHAIN_TOLERANCE
        })
        .collect();
    let mut keep = long.iter();
    open.retain(|_| *keep.next().unwrap());
    let mut keep = long.iter();
    open_on_board.retain(|_| *keep.next().unwrap());
    let mut anchors: Vec<[f64; 2]> = Vec::new();
    for piece in &mut open {
        let mut own: Option<usize> = None;
        for end in [0, piece.len() - 1] {
            let point = piece[end];
            let nearest = anchors
                .iter()
                .enumerate()
                .filter(|(index, _)| Some(*index) != own)
                .map(|(index, anchor)| (distance_squared(*anchor, point).sqrt(), index))
                .filter(|(distance, _)| *distance <= CHAIN_TOLERANCE)
                .min_by(|a, b| a.0.total_cmp(&b.0));
            match nearest {
                Some((_, index)) => {
                    piece[end] = anchors[index];
                    own.get_or_insert(index);
                }
                None => {
                    anchors.push(point);
                    own.get_or_insert(anchors.len() - 1);
                }
            }
        }
    }
    // Chain open pieces by their end points (matched to a micrometre).
    let key = |point: [f64; 2]| {
        (
            (point[0] * 1000.0).round() as i64,
            (point[1] * 1000.0).round() as i64,
        )
    };
    let mut used = vec![false; open.len()];
    for first in 0..open.len() {
        if used[first] {
            continue;
        }
        used[first] = true;
        let mut chain = open[first].clone();
        let mut chain_on_board = open_on_board[first];
        loop {
            let tail = key(*chain.last().unwrap());
            if tail == key(chain[0]) && chain.len() > 2 {
                chain.pop();
                break;
            }
            let next = (0..open.len()).find(|index| {
                !used[*index]
                    && (key(open[*index][0]) == tail || key(*open[*index].last().unwrap()) == tail)
            });
            let Some(next) = next else {
                // Stuck at this end: grow from the other end instead.
                let head = key(chain[0]);
                let continues = (0..open.len()).any(|index| {
                    !used[index]
                        && (key(open[index][0]) == head
                            || key(*open[index].last().unwrap()) == head)
                });
                if continues {
                    chain.reverse();
                    continue;
                }
                // A drawing with one gap (a missing edge) is closed with a
                // straight line, as KiCad's own outline tolerance would.
                let others_open = (0..open.len()).any(|index| !used[index]);
                if others_open {
                    return Err(NOT_CLOSED.into());
                }
                break;
            };
            used[next] = true;
            chain_on_board &= open_on_board[next];
            let mut piece = open[next].clone();
            if key(piece[0]) != tail {
                piece.reverse();
            }
            chain.extend_from_slice(&piece[1..]);
        }
        chained.push((chain, chain_on_board));
    }
    Ok(chained)
}

pub(super) fn board_loops(pcb: &Expr) -> Result<BoardLoops, String> {
    let mut loops: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut open: Vec<Vec<[f64; 2]>> = Vec::new();
    // Whether each loop and open piece is the board's own graphics.
    let mut loop_on_board: Vec<bool> = Vec::new();
    let mut open_on_board: Vec<bool> = Vec::new();
    // Footprints may carry part of the outline (card-edge notches); their
    // graphics are in the footprint frame.
    let identity = [0.0, 0.0, 0.0];
    let items: Vec<(&Expr, [f64; 3])> = pcb
        .children()
        .iter()
        .flat_map(|item| -> Vec<(&Expr, [f64; 3])> {
            if item.head() == Some("footprint") {
                let at = form_at(item).unwrap_or(identity);
                item.children().iter().map(|child| (child, at)).collect()
            } else {
                vec![(item, identity)]
            }
        })
        .collect();
    for (item, frame) in items {
        if form_atom(item, "layer", 1) != Some("Edge.Cuts") {
            continue;
        }
        let on_board = matches!(item.head(), Some(head) if head.starts_with("gr_"));
        let place = |local: [f64; 2]| -> [f64; 2] {
            let offset = rotate_vector(local, -frame[2]);
            [frame[0] + offset[0], frame[1] + offset[1]]
        };
        let xy = |head: &str| -> Result<[f64; 2], String> { Ok(place(form_xy(item, head)?)) };
        match item.head() {
            Some("gr_line" | "fp_line") => open.push(vec![xy("start")?, xy("end")?]),
            Some("gr_arc" | "fp_arc") => {
                let (start, mid, end) = (xy("start")?, xy("mid")?, xy("end")?);
                // An arc that ends where it starts is a full circle (start
                // and mid are opposite).
                if let Some(points) = full_circle(start, mid).filter(|_| closed_arc(start, end)) {
                    loops.push(points);
                } else {
                    open.push(arc_points(start, mid, end));
                }
            }
            Some("gr_rect" | "fp_rect") => {
                let (a, b) = (form_xy(item, "start")?, form_xy(item, "end")?);
                loops.push(
                    [a, [b[0], a[1]], b, [a[0], b[1]]]
                        .iter()
                        .map(|corner| place(*corner))
                        .collect(),
                );
            }
            Some("gr_circle" | "fp_circle") => {
                let center = xy("center")?;
                let end = xy("end")?;
                let opposite = [2.0 * center[0] - end[0], 2.0 * center[1] - end[1]];
                let quarter = [
                    center[0] - (end[1] - center[1]),
                    center[1] + (end[0] - center[0]),
                ];
                let back = [2.0 * center[0] - quarter[0], 2.0 * center[1] - quarter[1]];
                let mut points = arc_points(end, quarter, opposite);
                points.pop();
                points.extend(arc_points(opposite, back, end));
                points.pop();
                loops.push(points);
            }
            Some("gr_poly" | "fp_poly") => {
                let points: Vec<[f64; 2]> = pts_points(item)?.into_iter().map(place).collect();
                if distinct_points(&points) >= 3 {
                    loops.push(points);
                }
            }
            Some("gr_curve" | "fp_curve") => {
                let mut controls = Vec::new();
                for point in item
                    .child("pts")
                    .map(Expr::children)
                    .unwrap_or_default()
                    .iter()
                    .filter(|point| point.head() == Some("xy"))
                {
                    controls.push(place([
                        expression_coordinate(point, 1, "outline x")?,
                        expression_coordinate(point, 2, "outline y")?,
                    ]));
                }
                let [a, b, c, d] = controls[..] else {
                    return Err("Edge.Cuts bezier curve needs four points".into());
                };
                let mut points = vec![a];
                bezier_points([a, b, c, d], 0, &mut points);
                open.push(points);
            }
            _ => {}
        }
        loop_on_board.resize(loops.len(), on_board);
        open_on_board.resize(open.len(), on_board);
    }

    let chained = match chain_pieces(open.clone(), open_on_board.clone()) {
        Ok(chained) => chained,
        // The chaining above snaps each end to the nearest one seen before
        // and drops pieces shorter than its tolerance; outlines drawn from
        // segments of 10-20 um (keyboard halves' traced cutouts, panels of
        // thousands of segments) come apart under it although KiCad closes
        // them. Then KiCad's own chaining decides.
        Err(error) if error == NOT_CLOSED => chain_as_kicad(&open, &open_on_board)?,
        Err(error) => return Err(error),
    };
    for (chain, on_board) in chained {
        loops.push(chain);
        loop_on_board.push(on_board);
    }
    if loops.is_empty() {
        return Err("the board has no Edge.Cuts outline".into());
    }
    // The largest loop is the board. A loop inside a piece of board is a
    // cutout; a loop outside every piece is another piece (a module board
    // beside the main one, a panel): pieces are joined to the board by a
    // zero-width bridge between their closest vertices, so that one
    // polygon is their union (as the even-odd test and the edge distances
    // read it).
    let mut loops: Vec<(Vec<[f64; 2]>, bool)> = loops.into_iter().zip(loop_on_board).collect();
    loops.sort_by(|a, b| {
        polygon_area(&b.0)
            .abs()
            .total_cmp(&polygon_area(&a.0).abs())
    });
    let mut pieces: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut cutouts: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut board_cutouts = Vec::new();
    for (candidate, on_board) in loops {
        // Inside a piece when most of its vertices are: a cutout touching
        // the board's edge has vertices on that edge, which the even-odd
        // test may count either way.
        let inside = |piece: &Vec<[f64; 2]>| {
            let count = candidate
                .iter()
                .filter(|point| point_in_polygon(**point, piece))
                .count();
            2 * count >= candidate.len()
        };
        if pieces.iter().any(inside) {
            cutouts.push(candidate);
            board_cutouts.push(on_board);
        } else {
            pieces.push(candidate);
        }
    }
    let piece_boxes: Vec<[f64; 4]> = pieces
        .iter()
        .map(|piece| {
            piece.iter().fold(
                [
                    f64::INFINITY,
                    f64::INFINITY,
                    f64::NEG_INFINITY,
                    f64::NEG_INFINITY,
                ],
                |b, p| {
                    [
                        b[0].min(p[0]),
                        b[1].min(p[1]),
                        b[2].max(p[0]),
                        b[3].max(p[1]),
                    ]
                },
            )
        })
        .collect();
    let piece_loops = pieces.clone();
    let mut pieces = pieces.into_iter();
    let mut outline = pieces.next().expect("at least one loop");
    for piece in pieces {
        let (i, j) = (0..outline.len())
            .flat_map(|i| (0..piece.len()).map(move |j| (i, j)))
            .min_by(|(a, b), (c, d)| {
                distance_squared(outline[*a], piece[*b])
                    .total_cmp(&distance_squared(outline[*c], piece[*d]))
            })
            .unwrap();
        let mut joined = outline[..=i].to_vec();
        joined.extend(piece[j..].iter().chain(piece[..=j].iter()));
        joined.extend_from_slice(&outline[i..]);
        outline = joined;
    }
    if std::env::var_os("PCB_OUTLINE_DEBUG").is_some() {
        eprintln!(
            "outline: area {:.1} ({} vertices), {} cutouts",
            polygon_area(&outline).abs(),
            outline.len(),
            cutouts.len()
        );
    }
    let loops = cutouts;
    Ok(BoardLoops {
        outline,
        pieces: piece_boxes,
        piece_loops,
        cutouts: loops,
        board_cutouts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_loop_outside_the_board_is_another_piece_not_a_cutout() {
        let pcb = parse(
            r#"(kicad_pcb
          (gr_rect (start 0 0) (end 20 10) (layer "Edge.Cuts"))
          (gr_rect (start 30 0) (end 40 10) (layer "Edge.Cuts"))
          (gr_rect (start 2 2) (end 4 4) (layer "Edge.Cuts")))"#,
        )
        .unwrap();
        let loops = board_loops(&pcb).unwrap();
        // Both pieces are board; the small loop inside the first is a cutout.
        assert!((polygon_area(&loops.outline).abs() - 300.0).abs() < 0.1);
        assert_eq!(loops.cutouts.len(), 1);
        assert!(point_in_polygon([35.0, 5.0], &loops.outline));
        assert!(point_in_polygon([10.0, 5.0], &loops.outline));
        assert!(!point_in_polygon([25.0, 5.0], &loops.outline));
    }

    #[test]
    fn outlines_from_segments_shorter_than_the_tolerance_close_as_kicad_closes_them() {
        // A 10 x 10 board whose top and bottom edges are drawn from 0.01 mm
        // segments, as traced outlines are (keyboard halves, panels): the
        // snapping chain drops them as slivers and is left with two gaps;
        // KiCad chains them end to end.
        let mut text = String::from("(kicad_pcb");
        for y in [0.0, 10.0] {
            for step in 0..1000 {
                let (x0, x1) = (step as f64 * 0.01, (step + 1) as f64 * 0.01);
                text.push_str(&format!(
                    "(gr_line (start {x0:.2} {y}) (end {x1:.2} {y}) (layer \"Edge.Cuts\"))"
                ));
            }
        }
        text.push_str(
            r#"(gr_line (start 0 0) (end 0 10) (layer "Edge.Cuts"))
            (gr_line (start 10 0) (end 10 10) (layer "Edge.Cuts"))
            (gr_circle (center 5 5) (end 6 5) (layer "Edge.Cuts")))"#,
        );
        let pcb = parse(&text).unwrap();
        let loops = board_loops(&pcb).unwrap();
        assert!((polygon_area(&loops.outline) - 100.0).abs() < 1.0e-6);
        assert_eq!(loops.cutouts.len(), 1);
        // Ends further apart than KiCad's 0.02 mm stay open.
        let pcb = parse(
            r#"(kicad_pcb
          (gr_line (start 0 0) (end 10 0) (layer "Edge.Cuts"))
          (gr_line (start 10 0) (end 10 10) (layer "Edge.Cuts"))
          (gr_line (start 10 10) (end 0 10) (layer "Edge.Cuts"))
          (gr_line (start 0 10) (end 0 5.1) (layer "Edge.Cuts"))
          (gr_line (start 0 5) (end 0 0) (layer "Edge.Cuts"))
          (gr_line (start 20 0) (end 30 0) (layer "Edge.Cuts"))
          (gr_line (start 30 0) (end 30 10) (layer "Edge.Cuts")))"#,
        )
        .unwrap();
        assert_eq!(board_loops(&pcb).err().as_deref(), Some(NOT_CLOSED));
    }

    #[test]
    fn an_arc_that_closes_on_itself_in_a_polygon_is_a_circle() {
        // The 30led card's NFC keepout (McbeEringi/pcb-stuff,
        // proj/card/30led, MIT): one arc from (87, 100) through (113, 100)
        // back to (87, 100), which KiCad fills as a 26 mm circle.
        let zone = parse(
            r#"(zone (net 0) (net_name "") (layers "F.Cu" "B.Cu") (name "nfc")
          (keepout (tracks not_allowed) (vias not_allowed) (pads not_allowed)
            (copperpour not_allowed) (footprints not_allowed))
          (polygon (pts (arc (start 87 100) (mid 113 100) (end 87 100)))))"#,
        )
        .unwrap();
        let polygon = zone.child("polygon").unwrap();
        let points = pts_points(polygon).unwrap();
        let circle = std::f64::consts::PI * 13.0 * 13.0;
        assert!(
            (polygon_area(&points) - circle).abs() < 0.5,
            "{}",
            polygon_area(&points)
        );
        assert!(points.first() != points.last());
        assert!(point_in_polygon([100.0, 100.0], &points));
        assert!(!point_in_polygon([100.0, 113.5], &points));
    }

    #[test]
    fn a_degenerate_polygon_keeps_three_points() {
        // BratwurstPower's copper sliver (Qeteshpony/BratwurstPower,
        // pcb/BratwurstPower.kicad_pcb, BSD-2-Clause): three points, the
        // last the first, 2 nm wide. A zero-area shape, not an error; on
        // Edge.Cuts no outline.
        let polygon = parse(
            r#"(fp_poly (pts (xy -0.000385 -8.363353) (xy -0.000387 -8.363353)
          (xy -0.000385 -8.363353)) (stroke (width -0.000001) (type solid)) (fill solid)
          (layer "F.Cu"))"#,
        )
        .unwrap();
        let points = pts_points(&polygon).unwrap();
        assert_eq!(points.len(), 3);
        assert_eq!(polygon_area(&points), 0.0);
        let pcb = parse(
            r#"(kicad_pcb
          (gr_rect (start 0 0) (end 20 10) (layer "Edge.Cuts"))
          (gr_poly (pts (xy 5 5) (xy 6 5) (xy 5 5)) (layer "Edge.Cuts")))"#,
        )
        .unwrap();
        assert!(board_loops(&pcb).unwrap().cutouts.is_empty());
    }

    #[test]
    fn a_polygon_follows_its_arcs() {
        // A 10 x 10 square whose right side bulges out to a half circle (a
        // polygon of only arcs and points, as KiCad 7 on writes them).
        let polygon = parse(
            r#"(gr_poly (pts (xy 0 0) (arc (start 10 0) (mid 15 5) (end 10 10)) (xy 0 10)) (layer "F.Mask"))"#,
        )
        .unwrap();
        let points = pts_points(&polygon).unwrap();
        assert!(points.len() > 10);
        let half_circle = std::f64::consts::PI * 25.0 / 2.0;
        assert!((polygon_area(&points).abs() - (100.0 + half_circle)).abs() < 0.1);
        assert!(point_in_polygon([14.0, 5.0], &points));
    }

    #[test]
    fn a_sliver_shorter_than_the_chain_tolerance_is_dropped() {
        let pcb = parse(
            r#"(kicad_pcb
          (gr_line (start 0 0) (end 10 0) (layer "Edge.Cuts"))
          (gr_line (start 10 0) (end 10 10) (layer "Edge.Cuts"))
          (gr_line (start 10 10) (end 0.002 10.002) (layer "Edge.Cuts"))
          (gr_line (start 0.002 10.002) (end 0 10) (layer "Edge.Cuts"))
          (gr_line (start 0 10) (end 0 0) (layer "Edge.Cuts")))"#,
        )
        .unwrap();
        let loops = board_loops(&pcb).unwrap();
        assert!((polygon_area(&loops.outline) - 100.0).abs() < 0.1);
    }

    #[test]
    fn a_bezier_edge_is_flattened_within_the_tolerance() {
        let pcb = parse(
            r#"(kicad_pcb
          (gr_line (start 0 0) (end 10 0) (layer "Edge.Cuts"))
          (gr_curve (pts (xy 10 0) (xy 14 3) (xy 14 7) (xy 10 10)) (layer "Edge.Cuts"))
          (gr_line (start 10 10) (end 0 10) (layer "Edge.Cuts"))
          (gr_line (start 0 10) (end 0 0) (layer "Edge.Cuts")))"#,
        )
        .unwrap();
        let loops = board_loops(&pcb).unwrap();
        // The curve bulges 3 * 0.75 * 4 / 4 = 3 mm out at its middle.
        let widest = loops
            .outline
            .iter()
            .map(|point| point[0])
            .fold(0.0, f64::max);
        assert!((widest - 13.0).abs() < 0.01, "{widest}");
        assert!(loops.outline.len() > 10);
    }

    #[test]
    fn end_points_a_few_micrometres_apart_are_chained() {
        let pcb = parse(
            r#"(kicad_pcb
          (gr_line (start 0 0) (end 10 0) (layer "Edge.Cuts"))
          (gr_line (start 10.004 0) (end 10.004 5) (layer "Edge.Cuts"))
          (gr_line (start 10 5) (end 0 5) (layer "Edge.Cuts"))
          (gr_line (start 0 5) (end 0 0) (layer "Edge.Cuts")))"#,
        )
        .unwrap();
        let loops = board_loops(&pcb).unwrap();
        assert!((polygon_area(&loops.outline) - 50.0).abs() < 0.05);
        assert!(loops.cutouts.is_empty());
    }

    #[test]
    fn an_arc_that_ends_where_it_starts_is_a_circle() {
        let pcb = parse(
            r#"(kicad_pcb
          (gr_rect (start 0 0) (end 20 20) (layer "Edge.Cuts"))
          (gr_arc (start 5 10) (mid 15 10) (end 5 10) (layer "Edge.Cuts")))"#,
        )
        .unwrap();
        let loops = board_loops(&pcb).unwrap();
        assert_eq!(loops.cutouts.len(), 1);
        assert!((polygon_area(&loops.cutouts[0]) - std::f64::consts::PI * 25.0).abs() < 0.5);
    }

    #[test]
    fn rounded_rectangle_with_a_cutout() {
        let pcb = parse(
            r#"(kicad_pcb
          (gr_line (start 1 0) (end 9 0) (layer "Edge.Cuts"))
          (gr_arc (start 9 0) (mid 9.707107 0.292893) (end 10 1) (layer "Edge.Cuts"))
          (gr_line (start 10 1) (end 10 5) (layer "Edge.Cuts"))
          (gr_line (start 10 5) (end 0 5) (layer "Edge.Cuts"))
          (gr_line (start 0 5) (end 0 1) (layer "Edge.Cuts"))
          (gr_arc (start 0 1) (mid 0.292893 0.292893) (end 1 0) (layer "Edge.Cuts"))
          (gr_circle (center 5 2.5) (end 6 2.5) (layer "Edge.Cuts")))"#,
        )
        .unwrap();
        let loops = board_loops(&pcb).unwrap();
        let area = polygon_area(&loops.outline);
        let expected = 50.0 - 2.0 * (1.0 - std::f64::consts::PI / 4.0);
        assert!((area - expected).abs() < 0.03, "{area} vs {expected}");
        assert_eq!(loops.cutouts.len(), 1);
        assert!((polygon_area(&loops.cutouts[0]) - std::f64::consts::PI).abs() < 0.05);
    }

    #[test]
    fn an_arc_ending_where_it_starts_is_a_whole_circle() {
        // MK1's ring pad in cnlohr's tadpoleswadge.
        let points = arc_points([0.0, 0.0], [2.25, 0.0], [0.0, 0.0]);
        assert!(points.len() > 8);
        for point in points {
            let off = distance_squared(point, [1.125, 0.0]).sqrt() - 1.125;
            assert!(off.abs() <= ARC_TOLERANCE + 1.0e-9, "{point:?}");
        }
    }
}
