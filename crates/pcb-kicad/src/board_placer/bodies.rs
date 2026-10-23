// Copyright (C) 2026 Toit contributors.

//! Footprint bodies, pads and their poses: what a part occupies on its
//! side and how it is written back to the board.

use super::*;
use pcb_placer as core;

pub(super) fn normalize_angle(angle: f64) -> f64 {
    let wrapped = angle.rem_euclid(360.0);
    if wrapped.abs() < 1.0e-9 || (wrapped - 360.0).abs() < 1.0e-9 {
        0.0
    } else {
        wrapped
    }
}

/// Bounding box, in the footprint's own frame, of its courtyard and pads.
pub(crate) fn local_body(footprint: &Expr) -> Result<([f64; 2], [f64; 2], bool), String> {
    body_with_outline(footprint, ".CrtYd").map(|(center, size, round, _)| (center, size, round))
}

/// The box ([min x, min y, max x, max y], own frame) of a footprint
/// graphic (line, rectangle, arc, circle or polygon) with its stroke;
/// `None` for anything else.
pub(super) fn graphic_box(child: &Expr) -> Result<Option<[f64; 4]>, String> {
    if !matches!(
        child.head(),
        Some("fp_line" | "fp_rect" | "fp_arc" | "fp_circle" | "fp_poly")
    ) {
        return Ok(None);
    }
    let width = child
        .child("stroke")
        .and_then(|stroke| form_atom(stroke, "width", 1))
        .or_else(|| form_atom(child, "width", 1))
        .and_then(|width| width.parse::<f64>().ok())
        .unwrap_or(0.0);
    let mut points = outline::outline_points(child)?;
    if child.head() == Some("fp_circle") {
        let center = form_xy(child, "center")?;
        let radius = distance_squared(center, form_xy(child, "end")?).sqrt();
        points.extend([
            [center[0] - radius, center[1] - radius],
            [center[0] + radius, center[1] + radius],
        ]);
    }
    points.extend(outline::pts_points(child)?);
    let bounds = points.into_iter().fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |bounds, point| {
            [
                bounds[0].min(point[0] - width / 2.0),
                bounds[1].min(point[1] - width / 2.0),
                bounds[2].max(point[0] + width / 2.0),
                bounds[3].max(point[1] + width / 2.0),
            ]
        },
    );
    Ok(bounds[0].is_finite().then_some(bounds))
}

/// The body without the courtyard's margin: fabrication outline and pads
/// ([min x, min y, max x, max y], own frame). `None` without a fabrication
/// outline, or where it is no smaller than the courtyard.
pub(super) fn tight_body(footprint: &Expr, skip: Option<&str>) -> Result<Option<[f64; 4]>, String> {
    let (center, size, _, outlined) = body_on_side(footprint, ".Fab", skip)?;
    let (_, courtyard, _, _) = body_on_side(footprint, ".CrtYd", skip)?;
    if outlined == 0 || size[0] * size[1] >= courtyard[0] * courtyard[1] - 1.0e-9 {
        return Ok(None);
    }
    Ok(Some([
        center[0] - size[0] / 2.0,
        center[1] - size[1] / 2.0,
        center[0] + size[0] / 2.0,
        center[1] + size[1] / 2.0,
    ]))
}

/// The box around the graphics on layers ending in `outline` and the pads,
/// whether it is a disc, and how many outline graphics there are.
fn body_with_outline(
    footprint: &Expr,
    outline: &str,
) -> Result<([f64; 2], [f64; 2], bool, usize), String> {
    body_on_side(footprint, outline, None)
}

/// `body_with_outline` leaving out the outline and mask graphics on the
/// layers of one side (`"F."` or `"B."`).
pub(super) fn body_on_side(
    footprint: &Expr,
    outline: &str,
    skip: Option<&str>,
) -> Result<([f64; 2], [f64; 2], bool, usize), String> {
    let mut minimum = [f64::INFINITY; 2];
    let mut maximum = [f64::NEG_INFINITY; 2];
    let mut include = |point: [f64; 2], radius: f64| {
        for axis in 0..2 {
            minimum[axis] = minimum[axis].min(point[axis] - radius);
            maximum[axis] = maximum[axis].max(point[axis] + radius);
        }
    };
    let footprint_angle = form_at(footprint)?[2];
    let mut circles = 0;
    let mut straight = 0;
    // A footprint's own mask graphics open the mask over its area: another
    // part's pads placed there bridge (SNSP-CPU-01's J2 polygon: 63 pads
    // of other parts under it). They are part of the body, tight or not.
    // So is its copper: a printed antenna's 15 mm of copper beside its
    // three small pads and without a courtyard (the Business-Cards' AE1,
    // whose copper the body of its pads alone let hang over the board
    // edge: copper_edge_clearance on WLED_Matrix and Batch_1).
    let counts = |child: &Expr| {
        form_atom(child, "layer", 1).is_some_and(|layer| {
            (layer.ends_with(outline) || layer.ends_with(".Mask") || layer.ends_with(".Cu"))
                && !skip.is_some_and(|side| layer.starts_with(side))
        })
    };
    for child in footprint.children() {
        match child.head() {
            Some("fp_line" | "fp_rect" | "fp_arc") if counts(child) => {
                straight += 1;
                for point in outline::outline_points(child)? {
                    include(point, 0.0);
                }
            }
            Some("fp_circle") if counts(child) => {
                circles += 1;
                let center = form_xy(child, "center")?;
                let end = form_xy(child, "end")?;
                include(center, distance_squared(center, end).sqrt());
            }
            Some("fp_poly") if counts(child) => {
                straight += 1;
                for point in outline::pts_points(child)? {
                    include(point, 0.0);
                }
            }
            Some("pad") => {
                let at = form_at(child)?;
                let size = pad_extent(child);
                // Pad angles are absolute; bring the pad box into the local frame.
                let (sin, cos) = quarter_sin_cos(-(at[2] - footprint_angle));
                // Copper must keep a hole's local clearance, so no other
                // part's pads may come that close either.
                let keep_away =
                    if child.children().get(2).and_then(Expr::atom) == Some("np_thru_hole") {
                        local_clearance::pad_clearance(child, footprint)?
                    } else {
                        0.0
                    };
                let half = [
                    (cos.abs() * size[0] + sin.abs() * size[1]) / 2.0 + keep_away,
                    (sin.abs() * size[0] + cos.abs() * size[1]) / 2.0 + keep_away,
                ];
                // A drill offset moves the copper away from the anchor, also
                // on SMD pads.
                let offset = child
                    .child("drill")
                    .and_then(|drill| drill.child("offset"))
                    .and_then(|offset| {
                        Some([
                            expression_coordinate(offset, 1, "pad offset x").ok()?,
                            expression_coordinate(offset, 2, "pad offset y").ok()?,
                        ])
                    })
                    .unwrap_or([0.0, 0.0]);
                let center = [
                    at[0] + cos * offset[0] - sin * offset[1],
                    at[1] + sin * offset[0] + cos * offset[1],
                ];
                for corner in [[-1.0, -1.0], [1.0, 1.0]] {
                    include(
                        [
                            center[0] + corner[0] * half[0],
                            center[1] + corner[1] * half[1],
                        ],
                        0.0,
                    );
                }
                // A custom pad's copper is its primitives too (Sisu's dome
                // switch: a 0.7 mm anchor and a ring of 3.1 mm radius around
                // the switch centre), in the pad's own frame.
                for (point, radius) in custom_pad_points(child) {
                    include(
                        [
                            at[0] + cos * point[0] - sin * point[1],
                            at[1] + sin * point[0] + cos * point[1],
                        ],
                        radius,
                    );
                }
            }
            _ => {}
        }
    }
    if !minimum[0].is_finite() {
        // Neither courtyard nor pads: copper artwork (a logo on copper)
        // still takes its room.
        for child in footprint.children() {
            if !(matches!(
                child.head(),
                Some("fp_line" | "fp_rect" | "fp_arc" | "fp_circle" | "fp_poly")
            ) && form_atom(child, "layer", 1).is_some_and(|layer| layer.ends_with(".Cu")))
            {
                continue;
            }
            let mut points = outline::outline_points(child)?;
            if child.child("center").is_some() {
                points.push(form_xy(child, "center")?);
            }
            for point in outline::pts_points(child)? {
                points.push(point);
            }
            for point in points {
                for axis in 0..2 {
                    minimum[axis] = minimum[axis].min(point[axis]);
                    maximum[axis] = maximum[axis].max(point[axis]);
                }
            }
        }
    }
    if !minimum[0].is_finite() {
        return Ok(([0.0, 0.0], [1.0, 1.0], false, 0));
    }
    let size = [
        (maximum[0] - minimum[0]).max(0.2),
        (maximum[1] - minimum[1]).max(0.2),
    ];
    // A single circular courtyard containing all pads is a disc.
    let round = circles == 1 && straight == 0 && (size[0] - size[1]).abs() < 1.0e-6;
    Ok((
        [
            (minimum[0] + maximum[0]) / 2.0,
            (minimum[1] + maximum[1]) / 2.0,
        ],
        size,
        round,
        circles + straight,
    ))
}

/// Moves a footprint to `pose`, keeping pad and text angles (which KiCad
/// stores as absolute values) consistent. Returns the written pose.
pub(crate) fn write_footprint_pose(
    footprint: &mut Expr,
    pose: core::Pose,
) -> Result<[f64; 3], String> {
    let current = form_at(footprint)?;
    let at = [
        (pose.position[0] * 1.0e6).round() / 1.0e6,
        (pose.position[1] * 1.0e6).round() / 1.0e6,
        normalize_angle(pose.angle),
    ];
    let delta = normalize_angle(at[2] - current[2]);
    set_form_at(footprint, at)?;
    // Zones inside a footprint (a U.FL's keepout) are in board
    // coordinates: they move and turn with it. MokyaLora's moved U.FL left
    // its keepout behind, and KiCad found another part's pad in it.
    if (at[0] != current[0] || at[1] != current[1] || delta != 0.0)
        && let Expr::List(children) = footprint
    {
        for child in children
            .iter_mut()
            .filter(|child| child.head() == Some("zone"))
        {
            move_points(child, current, at);
        }
    }
    if delta != 0.0
        && let Expr::List(children) = footprint
    {
        for child in children.iter_mut() {
            if matches!(child.head(), Some("pad" | "property" | "fp_text"))
                && child.child("at").is_some()
            {
                let child_at = form_at(child)?;
                set_form_at(
                    child,
                    [
                        child_at[0],
                        child_at[1],
                        normalize_angle(child_at[2] + delta),
                    ],
                )?;
            }
        }
    }
    Ok(at)
}

/// Moves every `xy` point below `node` (board coordinates) from a
/// footprint at `from` to the same place relative to it at `to`.
fn move_points(node: &mut Expr, from: [f64; 3], to: [f64; 3]) {
    let Expr::List(items) = node else {
        return;
    };
    if items.first().and_then(Expr::atom) == Some("xy") {
        let value = |items: &[Expr], index: usize| {
            items
                .get(index)
                .and_then(Expr::atom)
                .and_then(|atom| atom.parse::<f64>().ok())
        };
        if let (Some(x), Some(y)) = (value(items, 1), value(items, 2)) {
            let local = core::problem::rotate([x - from[0], y - from[1]], from[2]);
            let board = core::problem::rotate(local, -to[2]);
            let round =
                |value: f64| Expr::Atom((((value * 1.0e6).round() / 1.0e6) + 0.0).to_string());
            items[1] = round(to[0] + board[0]);
            items[2] = round(to[1] + board[1]);
        }
        return;
    }
    for item in items.iter_mut().skip(1) {
        move_points(item, from, to);
    }
}

/// A pad's size in its own frame, at least its drill (a hole may be
/// drawn with a token pad).
/// The points (with a radius around them) that bound a custom pad's
/// primitives, in the pad's frame; empty for other pads.
pub(super) fn custom_pad_points(pad: &Expr) -> Vec<([f64; 2], f64)> {
    let Some(primitives) = pad.child("primitives") else {
        return Vec::new();
    };
    let mut points = Vec::new();
    for primitive in primitives.children().iter().skip(1) {
        let width = primitive
            .child("stroke")
            .and_then(|stroke| form_f64(stroke, "width", 1).ok())
            .or_else(|| form_f64(primitive, "width", 1).ok())
            .unwrap_or(0.0)
            .max(0.0);
        match primitive.head() {
            Some("gr_circle") => {
                if let (Ok(center), Ok(end)) =
                    (form_xy(primitive, "center"), form_xy(primitive, "end"))
                {
                    points.push((center, distance_squared(center, end).sqrt() + width / 2.0));
                }
            }
            _ => {
                for head in ["start", "mid", "end"] {
                    if let Ok(point) = form_xy(primitive, head) {
                        points.push((point, width / 2.0));
                    }
                }
                for point in outline::pts_points(primitive).unwrap_or_default() {
                    points.push((point, width / 2.0));
                }
            }
        }
    }
    points
}

pub(super) fn pad_extent(pad: &Expr) -> [f64; 2] {
    let size = form_xy(pad, "size").unwrap_or([0.0, 0.0]);
    let drill: Vec<f64> = pad
        .child("drill")
        .map(|drill| {
            drill
                .children()
                .iter()
                .skip(1)
                .filter_map(Expr::atom)
                .filter_map(|atom| atom.parse().ok())
                .collect()
        })
        .unwrap_or_default();
    let drill = match drill.as_slice() {
        [diameter] => [*diameter, *diameter],
        [width, height, ..] => [*width, *height],
        [] => [0.0, 0.0],
    };
    [size[0].max(drill[0]), size[1].max(drill[1])]
}

/// Whether a footprint sits on the bottom side. Its SMD pads decide when
/// they all sit on one outer layer: converted footprints (EasyEDA) may claim
/// the front while their copper is on the back.
pub(crate) fn on_back(footprint: &Expr) -> bool {
    let (mut front, mut back) = (0, 0);
    for pad in footprint
        .children()
        .iter()
        .filter(|child| child.head() == Some("pad"))
    {
        if pad.children().get(2).and_then(Expr::atom) != Some("smd") {
            continue;
        }
        let layers: Vec<&str> = pad
            .child("layers")
            .map(|layers| {
                layers
                    .children()
                    .iter()
                    .skip(1)
                    .filter_map(Expr::atom)
                    .collect()
            })
            .unwrap_or_default();
        front += layers.contains(&"F.Cu") as usize;
        back += layers.contains(&"B.Cu") as usize;
    }
    match (front, back) {
        (0, 1..) => true,
        (1.., 0) => false,
        _ => form_atom(footprint, "layer", 1) == Some("B.Cu"),
    }
}

/// Which way a connector opens, in its own frame: the side where the
/// courtyard reaches farthest beyond the pads (a USB receptacle's shell, a
/// barrel jack's body), if one side clearly does.
pub(crate) fn connector_mouth(footprint: &Expr) -> Result<Option<[f64; 2]>, String> {
    let footprint_angle = form_at(footprint)?[2];
    let mut courtyard = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    let mut pads = courtyard;
    let grow = |bounds: &mut [f64; 4], point: [f64; 2], half: [f64; 2]| {
        bounds[0] = bounds[0].min(point[0] - half[0]);
        bounds[1] = bounds[1].min(point[1] - half[1]);
        bounds[2] = bounds[2].max(point[0] + half[0]);
        bounds[3] = bounds[3].max(point[1] + half[1]);
    };
    for child in footprint.children() {
        let on_courtyard =
            form_atom(child, "layer", 1).is_some_and(|layer| layer.ends_with(".CrtYd"));
        match child.head() {
            Some("fp_line" | "fp_rect" | "fp_arc") if on_courtyard => {
                for point in outline::outline_points(child)? {
                    grow(&mut courtyard, point, [0.0, 0.0]);
                }
            }
            Some("fp_poly") if on_courtyard => {
                for point in outline::pts_points(child)? {
                    grow(&mut courtyard, point, [0.0, 0.0]);
                }
            }
            Some("pad") => {
                let at = form_at(child)?;
                let size = form_xy(child, "size").unwrap_or([0.0, 0.0]);
                let (sin, cos) = quarter_sin_cos(-(at[2] - footprint_angle));
                grow(
                    &mut pads,
                    [at[0], at[1]],
                    [
                        (cos.abs() * size[0] + sin.abs() * size[1]) / 2.0,
                        (sin.abs() * size[0] + cos.abs() * size[1]) / 2.0,
                    ],
                );
            }
            _ => {}
        }
    }
    if !courtyard[0].is_finite() || !pads[0].is_finite() {
        return Ok(None);
    }
    let mut sides = [
        (pads[0] - courtyard[0], [-1.0, 0.0]),
        (pads[1] - courtyard[1], [0.0, -1.0]),
        (courtyard[2] - pads[2], [1.0, 0.0]),
        (courtyard[3] - pads[3], [0.0, 1.0]),
    ];
    sides.sort_by(|a, b| b.0.total_cmp(&a.0));
    Ok((sides[0].0 >= 1.0 && sides[0].0 >= 1.5 * sides[1].0.max(0.0)).then_some(sides[0].1))
}

/// Bounding box, in the footprint's own frame, of its keepout zones.
pub(super) fn footprint_keepout_box(
    footprint: &Expr,
    at: [f64; 3],
) -> Result<Option<[f64; 4]>, String> {
    let mut bounds: Option<[f64; 4]> = None;
    for zone in footprint
        .children()
        .iter()
        .filter(|child| child.head() == Some("zone") && child.child("keepout").is_some())
    {
        let Some(points) = zone
            .child("polygon")
            .and_then(|polygon| polygon.child("pts"))
        else {
            continue;
        };
        for point in points
            .children()
            .iter()
            .filter(|child| child.head() == Some("xy"))
        {
            let board = [
                expression_coordinate(point, 1, "keepout x")?,
                expression_coordinate(point, 2, "keepout y")?,
            ];
            // Zones inside footprints are stored in board coordinates.
            let local = core::problem::rotate([board[0] - at[0], board[1] - at[1]], at[2]);
            let entry = bounds.get_or_insert([local[0], local[1], local[0], local[1]]);
            entry[0] = entry[0].min(local[0]);
            entry[1] = entry[1].min(local[1]);
            entry[2] = entry[2].max(local[0]);
            entry[3] = entry[3].max(local[1]);
        }
    }
    Ok(bounds)
}
