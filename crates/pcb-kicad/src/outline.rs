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
const ARC_TOLERANCE: f64 = 0.005;

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
    for item in pcb.children() {
        if form_atom(item, "layer", 1) != Some("Edge.Cuts") {
            continue;
        }
        match item.head() {
            Some("gr_line") => open.push(vec![form_xy(item, "start")?, form_xy(item, "end")?]),
            Some("gr_arc") => open.push(arc_points(
                form_xy(item, "start")?,
                form_xy(item, "mid")?,
                form_xy(item, "end")?,
            )),
            Some("gr_rect") => {
                let (a, b) = (form_xy(item, "start")?, form_xy(item, "end")?);
                loops.push(vec![a, [b[0], a[1]], b, [a[0], b[1]]]);
            }
            Some("gr_circle") => {
                let center = form_xy(item, "center")?;
                let end = form_xy(item, "end")?;
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
            Some("gr_poly") => {
                let mut points = Vec::new();
                for point in item
                    .child("pts")
                    .map(Expr::children)
                    .unwrap_or_default()
                    .iter()
                    .filter(|point| point.head() == Some("xy"))
                {
                    points.push([
                        expression_coordinate(point, 1, "outline x")?,
                        expression_coordinate(point, 2, "outline y")?,
                    ]);
                }
                if points.len() >= 3 {
                    loops.push(points);
                }
            }
            Some("gr_curve") => {
                return Err("Edge.Cuts bezier curves are not supported yet".into());
            }
            _ => {}
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
                return Err("Edge.Cuts graphics do not form closed loops".into());
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
    let largest = (0..loops.len())
        .max_by(|a, b| polygon_area(&loops[*a]).total_cmp(&polygon_area(&loops[*b])))
        .unwrap();
    let outline = loops.swap_remove(largest);
    Ok(BoardLoops {
        outline,
        cutouts: loops,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
