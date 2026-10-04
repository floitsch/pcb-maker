// Copyright (C) 2026 Toit contributors.

//! Board outlines from arbitrary Edge.Cuts graphics: lines, arcs, rectangles,
//! polygons and circles are chained into closed loops. The largest loop is
//! the board; all others are cutouts.

use super::*;

pub(super) struct BoardLoops {
    pub outline: Vec<[f64; 2]>,
    pub cutouts: Vec<Vec<[f64; 2]>>,
}

/// Longest chord error accepted when flattening arcs.
pub(super) const ARC_TOLERANCE: f64 = 0.005;

/// The points a line, rectangle or arc passes through, an arc flattened
/// (its start, middle and end alone miss its bulge): for extents.
pub(super) fn outline_points(item: &Expr) -> Result<Vec<[f64; 2]>, String> {
    if item.child("start").is_some() && item.child("mid").is_some() && item.child("end").is_some() {
        return Ok(arc_points(form_xy(item, "start")?, form_xy(item, "mid")?, form_xy(item, "end")?));
    }
    let mut points = Vec::new();
    for head in ["start", "end"] {
        if item.child(head).is_some() {
            points.push(form_xy(item, head)?);
        }
    }
    Ok(points)
}

pub(super) fn arc_points(start: [f64; 2], mid: [f64; 2], end: [f64; 2]) -> Vec<[f64; 2]> {
    // Circle through three points.
    let d = 2.0
        * (start[0] * (mid[1] - end[1]) + mid[0] * (end[1] - start[1]) + end[0] * (start[1] - mid[1]));
    if d.abs() < 1.0e-12 {
        return vec![start, end];
    }
    let square = |p: [f64; 2]| p[0] * p[0] + p[1] * p[1];
    let center = [
        (square(start) * (mid[1] - end[1]) + square(mid) * (end[1] - start[1])
            + square(end) * (start[1] - mid[1]))
            / d,
        (square(start) * (end[0] - mid[0]) + square(mid) * (start[0] - end[0])
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
    let sweep = if through <= forward { forward } else { forward - tau };
    let step = 2.0 * (1.0 - ARC_TOLERANCE / radius.max(ARC_TOLERANCE)).clamp(-1.0, 1.0).acos();
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

pub(super) fn board_loops(pcb: &Expr) -> Result<BoardLoops, String> {
    let mut loops: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut open: Vec<Vec<[f64; 2]>> = Vec::new();
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
        let place = |local: [f64; 2]| -> [f64; 2] {
            let offset = rotate_vector(local, -frame[2]);
            [frame[0] + offset[0], frame[1] + offset[1]]
        };
        let xy = |head: &str| -> Result<[f64; 2], String> { Ok(place(form_xy(item, head)?)) };
        match item.head() {
            Some("gr_line" | "fp_line") => open.push(vec![xy("start")?, xy("end")?]),
            Some("gr_arc" | "fp_arc") => {
                let (start, mid, end) = (xy("start")?, xy("mid")?, xy("end")?);
                if distance_squared(start, end).sqrt() < 1.0e-6 && distance_squared(start, mid).sqrt() > 1.0e-6 {
                    // An arc that ends where it starts is a full circle
                    // (start and mid are opposite).
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
                let mut points = Vec::new();
                for point in item
                    .child("pts")
                    .map(Expr::children)
                    .unwrap_or_default()
                    .iter()
                    .filter(|point| point.head() == Some("xy"))
                {
                    points.push(place([
                        expression_coordinate(point, 1, "outline x")?,
                        expression_coordinate(point, 2, "outline y")?,
                    ]));
                }
                if points.len() >= 3 {
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
    }

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
    open.retain(|piece| {
        piece.windows(2).map(|pair| distance_squared(pair[0], pair[1]).sqrt()).sum::<f64>() > CHAIN_TOLERANCE
    });
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
                        && (key(open[index][0]) == head || key(*open[index].last().unwrap()) == head)
                });
                if continues {
                    chain.reverse();
                    continue;
                }
                // A drawing with one gap (a missing edge) is closed with a
                // straight line, as KiCad's own outline tolerance would.
                let others_open = (0..open.len()).any(|index| !used[index]);
                if others_open {
                    return Err("Edge.Cuts graphics do not form closed loops".into());
                }
                break;
            };
            used[next] = true;
            let mut piece = open[next].clone();
            if key(piece[0]) != tail {
                piece.reverse();
            }
            chain.extend_from_slice(&piece[1..]);
        }
        loops.push(chain);
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
    loops.sort_by(|a, b| polygon_area(b).abs().total_cmp(&polygon_area(a).abs()));
    let mut pieces: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut cutouts: Vec<Vec<[f64; 2]>> = Vec::new();
    for candidate in loops {
        if pieces.iter().any(|piece| point_in_polygon(candidate[0], piece)) {
            cutouts.push(candidate);
        } else {
            pieces.push(candidate);
        }
    }
    let mut pieces = pieces.into_iter();
    let mut outline = pieces.next().expect("at least one loop");
    for piece in pieces {
        let (i, j) = (0..outline.len())
            .flat_map(|i| (0..piece.len()).map(move |j| (i, j)))
            .min_by(|(a, b), (c, d)| {
                distance_squared(outline[*a], piece[*b]).total_cmp(&distance_squared(outline[*c], piece[*d]))
            })
            .unwrap();
        let mut joined = outline[..=i].to_vec();
        joined.extend(piece[j..].iter().chain(piece[..=j].iter()));
        joined.extend_from_slice(&outline[i..]);
        outline = joined;
    }
    if std::env::var_os("PCB_OUTLINE_DEBUG").is_some() {
        eprintln!("outline: area {:.1} ({} vertices), {} cutouts", polygon_area(&outline).abs(), outline.len(), cutouts.len());
    }
    let loops = cutouts;
    Ok(BoardLoops {
        outline,
        cutouts: loops,
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
        let widest = loops.outline.iter().map(|point| point[0]).fold(0.0, f64::max);
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
}
