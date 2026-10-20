// Copyright (C) 2026 Toit contributors.

//! Courtyards: their shapes and slabs, and pads standing in pin rows.

use super::*;

/// Whether a footprint's pads (own-frame centres) stand in rows of pins:
/// at least four pads, three quarters of them in a row or column of three
/// or more whose neighbours are at most 2.6 mm apart (a QFP's sides, a
/// header's rows, a card slot). Pads at one position count once. A key
/// switch's or a hot-swap socket's few pads, spread over its box, do not.
pub(super) fn pads_in_pin_rows(centers: &[[f64; 2]]) -> bool {
    const PITCH: f64 = 2.6;
    const TOLERANCE: f64 = 0.05;
    let mut pads: Vec<[f64; 2]> = Vec::new();
    for center in centers {
        if !pads
            .iter()
            .any(|pad| (pad[0] - center[0]).abs() < 1.0e-3 && (pad[1] - center[1]).abs() < 1.0e-3)
        {
            pads.push(*center);
        }
    }
    if pads.len() < 4 {
        return false;
    }
    let mut member = vec![false; pads.len()];
    for axis in 0..2 {
        // Lines of pads sharing the other coordinate (within the tolerance
        // of the line's first pad).
        let mut lines: Vec<(f64, Vec<usize>)> = Vec::new();
        for (index, pad) in pads.iter().enumerate() {
            let across = pad[1 - axis];
            match lines
                .iter_mut()
                .find(|(at, _)| (at - across).abs() <= TOLERANCE)
            {
                Some((_, line)) => line.push(index),
                None => lines.push((across, vec![index])),
            }
        }
        for (_, mut line) in lines {
            line.sort_by(|a, b| pads[*a][axis].total_cmp(&pads[*b][axis]));
            let mut start = 0;
            for end in 1..=line.len() {
                let broken = end == line.len() || {
                    let gap = pads[line[end]][axis] - pads[line[end - 1]][axis];
                    gap <= 1.0e-6 || gap > PITCH
                };
                if broken {
                    if end - start >= 3 {
                        for index in &line[start..end] {
                            member[*index] = true;
                        }
                    }
                    start = end;
                }
            }
        }
    }
    member.iter().filter(|member| **member).count() as f64 >= 0.75 * pads.len() as f64
}

/// The courtyard (own frame) as horizontal strips, one per span between
/// the corners' heights, each as wide as the courtyard there (slanted
/// sides counted at their wider end); strips alike stacked into one.
/// Empty when the courtyard is not closed or has too many corners (arcs).
pub(super) fn courtyard_slabs(
    footprint: &Expr,
    skip: Option<&str>,
) -> Result<Vec<[f64; 4]>, String> {
    let mut edges: Vec<([f64; 2], [f64; 2])> = Vec::new();
    let chain = |points: &[[f64; 2]], closed: bool, edges: &mut Vec<([f64; 2], [f64; 2])>| {
        for pair in points.windows(2) {
            edges.push((pair[0], pair[1]));
        }
        if closed && points.len() > 2 {
            edges.push((points[points.len() - 1], points[0]));
        }
    };
    for child in footprint.children() {
        if !form_atom(child, "layer", 1).is_some_and(|layer| {
            layer.ends_with(".CrtYd") && !skip.is_some_and(|side| layer.starts_with(side))
        }) {
            continue;
        }
        match child.head() {
            Some("fp_line" | "fp_arc") => {
                chain(&outline::outline_points(child)?, false, &mut edges)
            }
            Some("fp_rect") => {
                let (a, b) = (form_xy(child, "start")?, form_xy(child, "end")?);
                chain(&[a, [b[0], a[1]], b, [a[0], b[1]]], true, &mut edges);
            }
            Some("fp_poly") => chain(&outline::pts_points(child)?, true, &mut edges),
            Some("fp_circle") => {
                let center = form_xy(child, "center")?;
                let radius = distance_squared(center, form_xy(child, "end")?).sqrt();
                chain(&outline::circle_points(center, radius), false, &mut edges);
            }
            _ => {}
        }
    }
    let mut heights: Vec<f64> = edges.iter().flat_map(|(a, b)| [a[1], b[1]]).collect();
    heights.sort_by(f64::total_cmp);
    heights.dedup_by(|a, b| (*a - *b).abs() < 1.0e-6);
    if edges.is_empty() || heights.len() > 64 {
        return Ok(Vec::new());
    }
    let x_at = |(a, b): ([f64; 2], [f64; 2]), y: f64| {
        let t = ((y - a[1]) / (b[1] - a[1])).clamp(0.0, 1.0);
        a[0] + t * (b[0] - a[0])
    };
    let mut slabs: Vec<[f64; 4]> = Vec::new();
    for span in heights.windows(2) {
        let (low, high) = (span[0], span[1]);
        let middle = (low + high) / 2.0;
        let mut crossing: Vec<([f64; 2], [f64; 2])> = edges
            .iter()
            .copied()
            .filter(|(a, b)| (a[1] > middle) != (b[1] > middle))
            .collect();
        if !crossing.len().is_multiple_of(2) {
            return Ok(Vec::new());
        }
        crossing.sort_by(|p, q| x_at(*p, middle).total_cmp(&x_at(*q, middle)));
        for pair in crossing.chunks(2) {
            let left = x_at(pair[0], low).min(x_at(pair[0], high));
            let right = x_at(pair[1], low).max(x_at(pair[1], high));
            match slabs.iter_mut().find(|slab| {
                (slab[3] - low).abs() < 1.0e-6
                    && (slab[0] - left).abs() < 1.0e-6
                    && (slab[2] - right).abs() < 1.0e-6
            }) {
                Some(slab) => slab[3] = high,
                None => slabs.push([left, low, right, high]),
            }
        }
    }
    Ok(slabs)
}

/// Boxes ([min x, min y, max x, max y], own frame) of the separate shapes
/// a footprint's courtyard is drawn as (empty when it is one shape, or
/// none), and whether the courtyard is malformed: lines and arcs whose end
/// points do not each join exactly two of them (a zero-length line does
/// not), which KiCad cannot build and does not check. Lines and arcs that
/// share end points form one shape. `skip`: leaves out the courtyard on
/// that side's layers.
pub(super) fn courtyard_shapes_on_side(
    footprint: &Expr,
    skip: Option<&str>,
) -> Result<(Vec<[f64; 4]>, bool), String> {
    let mut degrees: Vec<([f64; 2], usize)> = Vec::new();
    let mut shapes: Vec<(Vec<[f64; 2]>, [f64; 4])> = Vec::new();
    let grow = |bounds: &mut [f64; 4], point: [f64; 2], radius: f64| {
        bounds[0] = bounds[0].min(point[0] - radius);
        bounds[1] = bounds[1].min(point[1] - radius);
        bounds[2] = bounds[2].max(point[0] + radius);
        bounds[3] = bounds[3].max(point[1] + radius);
    };
    let empty = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for child in footprint.children() {
        if !form_atom(child, "layer", 1).is_some_and(|layer| {
            layer.ends_with(".CrtYd") && !skip.is_some_and(|side| layer.starts_with(side))
        }) {
            continue;
        }
        let mut bounds = empty;
        let mut ends = Vec::new();
        match child.head() {
            Some("fp_line" | "fp_arc") => {
                for point in outline::outline_points(child)? {
                    grow(&mut bounds, point, 0.0);
                }
                ends = vec![form_xy(child, "start")?, form_xy(child, "end")?];
                for end in &ends {
                    match degrees
                        .iter_mut()
                        .find(|(point, _)| distance_squared(*point, *end) < 1.0e-6)
                    {
                        Some((_, degree)) => *degree += 1,
                        None => degrees.push((*end, 1)),
                    }
                }
            }
            Some("fp_rect") => {
                grow(&mut bounds, form_xy(child, "start")?, 0.0);
                grow(&mut bounds, form_xy(child, "end")?, 0.0);
            }
            Some("fp_circle") => {
                let center = form_xy(child, "center")?;
                grow(
                    &mut bounds,
                    center,
                    distance_squared(center, form_xy(child, "end")?).sqrt(),
                );
            }
            Some("fp_poly") => {
                for point in outline::pts_points(child)? {
                    grow(&mut bounds, point, 0.0);
                }
            }
            _ => continue,
        }
        // Merge with every shape sharing an end point.
        let touches = |points: &[[f64; 2]]| {
            ends.iter().any(|end| {
                points
                    .iter()
                    .any(|point| distance_squared(*end, *point) < 1.0e-6)
            })
        };
        let (mut merged, rest): (Vec<_>, Vec<_>) =
            shapes.drain(..).partition(|(points, _)| touches(points));
        shapes = rest;
        let mut points = ends.clone();
        for (other_points, other_bounds) in merged.drain(..) {
            points.extend(other_points);
            grow(&mut bounds, [other_bounds[0], other_bounds[1]], 0.0);
            grow(&mut bounds, [other_bounds[2], other_bounds[3]], 0.0);
        }
        shapes.push((points, bounds));
    }
    let malformed = degrees.iter().any(|(_, degree)| *degree != 2);
    if shapes.len() < 2 {
        return Ok((Vec::new(), malformed));
    }
    Ok((
        shapes.into_iter().map(|(_, bounds)| bounds).collect(),
        malformed,
    ))
}
