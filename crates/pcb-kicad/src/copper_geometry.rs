// Copyright (C) 2026 Toit contributors.

//! Copper and outline shapes as KiCad lowers them: pads (custom ones too),
//! rule areas and the board outline.

use super::*;

pub(crate) fn board_outline(pcb: &Expr) -> Result<Option<BoardOutline>, String> {
    let edge_shapes = pcb
        .children()
        .iter()
        .filter(|item| {
            matches!(
                item.head(),
                Some("gr_rect" | "gr_line" | "gr_arc" | "gr_poly" | "gr_curve")
            ) && form_atom(item, "layer", 1) == Some("Edge.Cuts")
        })
        .collect::<Vec<_>>();
    if edge_shapes.len() == 1 && edge_shapes[0].head() == Some("gr_rect") {
        let start = form_xy(edge_shapes[0], "start")?;
        let end = form_xy(edge_shapes[0], "end")?;
        return Ok(Some(BoardOutline::rectangle([
            start[0].min(end[0]),
            start[1].min(end[1]),
            start[0].max(end[0]),
            start[1].max(end[1]),
        ])?));
    }
    if edge_shapes.len() < 4
        || edge_shapes
            .iter()
            .any(|shape| shape.head() != Some("gr_line"))
    {
        return Ok(None);
    }
    let mut segments = Vec::with_capacity(edge_shapes.len());
    for shape in edge_shapes {
        let start = form_xy(shape, "start")?;
        let end = form_xy(shape, "end")?;
        if distance_squared(start, end) <= 1.0e-18 {
            return Ok(None);
        }
        segments.push((start, end));
    }
    let point = |coordinate: [f64; 2]| {
        Point(
            (coordinate[0] * 1_000_000.0).round() as i64,
            (coordinate[1] * 1_000_000.0).round() as i64,
        )
    };
    let segments = segments
        .into_iter()
        .map(|(start, end)| (point(start), point(end)))
        .collect::<Vec<_>>();
    let mut adjacency: BTreeMap<Point, Vec<Point>> = BTreeMap::new();
    for (start, end) in segments {
        adjacency.entry(start).or_default().push(end);
        adjacency.entry(end).or_default().push(start);
    }
    if adjacency.len() < 3 || adjacency.values().any(|neighbors| neighbors.len() != 2) {
        return Ok(None);
    }
    let start = *adjacency
        .keys()
        .next()
        .expect("non-empty outline adjacency");
    let mut previous = None;
    let mut current = start;
    let mut points = Vec::with_capacity(adjacency.len());
    loop {
        points.push([
            current.0 as f64 / 1_000_000.0,
            current.1 as f64 / 1_000_000.0,
        ]);
        let neighbors = &adjacency[&current];
        let next = if previous == Some(neighbors[0]) {
            neighbors[1]
        } else {
            neighbors[0]
        };
        previous = Some(current);
        current = next;
        if current == start {
            break;
        }
        if points.len() >= adjacency.len() {
            return Ok(None);
        }
    }
    if points.len() != adjacency.len() {
        return Ok(None);
    }
    Ok(Some(BoardOutline::new(points)?))
}

#[derive(Clone, Debug)]
pub(crate) struct BoardOutline {
    pub(crate) points: Vec<[f64; 2]>,
    pub(crate) bounds: [f64; 4],
}

impl BoardOutline {
    fn rectangle(bounds: [f64; 4]) -> Result<Self, String> {
        Self::new(vec![
            [bounds[0], bounds[1]],
            [bounds[2], bounds[1]],
            [bounds[2], bounds[3]],
            [bounds[0], bounds[3]],
        ])
    }

    fn new(points: Vec<[f64; 2]>) -> Result<Self, String> {
        if points.len() < 3
            || points
                .iter()
                .flatten()
                .any(|coordinate| !coordinate.is_finite())
        {
            return Err("board outline has invalid polygon coordinates".into());
        }
        let mut bounds = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for point in &points {
            bounds[0] = bounds[0].min(point[0]);
            bounds[1] = bounds[1].min(point[1]);
            bounds[2] = bounds[2].max(point[0]);
            bounds[3] = bounds[3].max(point[1]);
        }
        let outline = Self { points, bounds };
        if outline.area() <= 1.0e-12 {
            return Err("board outline polygon has zero area".into());
        }
        Ok(outline)
    }

    pub(crate) fn area(&self) -> f64 {
        self.points
            .iter()
            .zip(self.points.iter().cycle().skip(1))
            .take(self.points.len())
            .map(|(left, right)| left[0] * right[1] - right[0] * left[1])
            .sum::<f64>()
            .abs()
            / 2.0
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum ObstacleGeometry {
    Circle {
        center: [f64; 2],
        radius: f64,
    },
    Segment {
        start: [f64; 2],
        end: [f64; 2],
        radius: f64,
    },
    Rectangle {
        center: [f64; 2],
        half_size: [f64; 2],
        angle_degrees: f64,
    },
    Polygon {
        points: Vec<[f64; 2]>,
    },
    Union {
        parts: Vec<ObstacleGeometry>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GeometryAabb {
    pub(crate) minimum: [f64; 2],
    pub(crate) maximum: [f64; 2],
}

impl GeometryAabb {
    fn around_segment(start: [f64; 2], end: [f64; 2], inflate: f64) -> Self {
        Self {
            minimum: [
                start[0].min(end[0]) - inflate,
                start[1].min(end[1]) - inflate,
            ],
            maximum: [
                start[0].max(end[0]) + inflate,
                start[1].max(end[1]) + inflate,
            ],
        }
    }
}

impl ObstacleGeometry {
    pub(crate) fn aabb(&self) -> GeometryAabb {
        match self {
            Self::Circle { center, radius } => GeometryAabb {
                minimum: [center[0] - radius, center[1] - radius],
                maximum: [center[0] + radius, center[1] + radius],
            },
            Self::Segment { start, end, radius } => {
                GeometryAabb::around_segment(*start, *end, *radius)
            }
            Self::Rectangle {
                center,
                half_size,
                angle_degrees,
            } => {
                let (sine, cosine) = quarter_sin_cos(*angle_degrees);
                let extent = [
                    cosine.abs() * half_size[0] + sine.abs() * half_size[1],
                    sine.abs() * half_size[0] + cosine.abs() * half_size[1],
                ];
                GeometryAabb {
                    minimum: [center[0] - extent[0], center[1] - extent[1]],
                    maximum: [center[0] + extent[0], center[1] + extent[1]],
                }
            }
            Self::Polygon { points } => polygon_aabb(points),
            Self::Union { parts } => parts
                .iter()
                .map(Self::aabb)
                .reduce(|left, right| GeometryAabb {
                    minimum: [
                        left.minimum[0].min(right.minimum[0]),
                        left.minimum[1].min(right.minimum[1]),
                    ],
                    maximum: [
                        left.maximum[0].max(right.maximum[0]),
                        left.maximum[1].max(right.maximum[1]),
                    ],
                })
                .expect("compound pad geometry has an anchor"),
        }
    }
}

fn polygon_aabb(points: &[[f64; 2]]) -> GeometryAabb {
    let mut minimum = [f64::INFINITY; 2];
    let mut maximum = [f64::NEG_INFINITY; 2];
    for point in points {
        for axis in 0..2 {
            minimum[axis] = minimum[axis].min(point[axis]);
            maximum[axis] = maximum[axis].max(point[axis]);
        }
    }
    GeometryAabb { minimum, maximum }
}

pub(crate) fn rule_area_layer_mask(zone: &Expr) -> Result<[bool; 2], String> {
    let mut mask = [false; 2];
    if let Some(layer) = form_atom(zone, "layer", 1) {
        let index = copper_layer_index(layer)
            .ok_or_else(|| format!("unsupported rule-area layer {layer:?}"))?;
        mask[index] = true;
    } else if let Some(layers) = zone.child("layers") {
        for layer in layers.children().iter().skip(1).filter_map(Expr::atom) {
            match layer {
                "F&B.Cu" | "*.Cu" => mask = [true, true],
                "F.Cu" => mask[0] = true,
                "B.Cu" => mask[1] = true,
                _ => return Err(format!("unsupported rule-area layer {layer:?}")),
            }
        }
    }
    if mask == [false, false] {
        Err("rule area has no supported copper layer".into())
    } else {
        Ok(mask)
    }
}

/// A rule area's outline as one simple polygon. KiCad writes the zone's
/// outline as its first `polygon` and every hole as a further `polygon`
/// (Glasgow keeps tracks off a 1.7 mm rim this way: a board-sized outline
/// with the inside cut out). Each hole is joined to the outline by a slit
/// between their closest vertices, walked in and out again, which the
/// even-odd point test and the edge distances treat as the ring it is.
pub(crate) fn rule_area_polygon_points(zone: &Expr) -> Result<Vec<[f64; 2]>, String> {
    let mut polygons: Vec<Vec<[f64; 2]>> = Vec::new();
    for polygon in zone
        .children()
        .iter()
        .filter(|item| item.head() == Some("polygon"))
    {
        // Sides drawn as arcs (Telemetry's GND zones along its round
        // edge), flattened as KiCad's filler does (ClearArcs); they were
        // left out, the zone then a polygon through the other corners.
        let arcs = polygon
            .child("pts")
            .is_some_and(|pts| pts.children().iter().any(|item| item.head() == Some("arc")));
        if arcs {
            let points = outline::pts_points(polygon)?;
            if points.len() >= 3 {
                polygons.push(points);
            }
            continue;
        }
        let points = polygon
            .child("pts")
            .ok_or_else(|| "rule area has no polygon points".to_string())?
            .children()
            .iter()
            .skip(1)
            .filter(|point| point.head() == Some("xy"))
            .map(|point| {
                let coordinate = |index: usize| {
                    point
                        .children()
                        .get(index)
                        .and_then(Expr::atom)
                        .ok_or_else(|| "rule-area xy point is missing a coordinate".to_string())?
                        .parse::<f64>()
                        .map_err(|error| format!("invalid rule-area xy coordinate: {error}"))
                };
                Ok::<[f64; 2], String>([coordinate(1)?, coordinate(2)?])
            })
            .collect::<Result<Vec<_>, _>>()?;
        if points.len() >= 3 {
            polygons.push(points);
        }
    }
    let mut polygons = polygons.into_iter();
    let mut outline = polygons
        .next()
        .ok_or_else(|| "rule area polygon has fewer than three points".to_string())?;
    for hole in polygons {
        let (i, j) = (0..outline.len())
            .flat_map(|i| (0..hole.len()).map(move |j| (i, j)))
            .min_by(|(a, b), (c, d)| {
                let da =
                    (outline[*a][0] - hole[*b][0]).powi(2) + (outline[*a][1] - hole[*b][1]).powi(2);
                let dc =
                    (outline[*c][0] - hole[*d][0]).powi(2) + (outline[*c][1] - hole[*d][1]).powi(2);
                da.partial_cmp(&dc).unwrap()
            })
            .unwrap();
        let mut joined = outline[..=i].to_vec();
        joined.extend(hole[j..].iter().chain(hole[..=j].iter()));
        joined.extend_from_slice(&outline[i..]);
        outline = joined;
    }
    Ok(outline)
}

pub(crate) struct LoweredPad {
    pub(crate) center: [f64; 2],
    pub(crate) geometry: ObstacleGeometry,
}

/// Board-space copper of one footprint pad, as KiCad's effective shape.
pub(crate) fn lower_pad(pad: &Expr, footprint_at: [f64; 3]) -> Result<LoweredPad, String> {
    let pad_at = form_at(pad)?;
    // KiCad's board coordinates have Y pointing down: positive footprint
    // angles therefore rotate local pad coordinates clockwise.
    let center_offset = rotate_vector([pad_at[0], pad_at[1]], -footprint_at[2]);
    let center = [
        footprint_at[0] + center_offset[0],
        footprint_at[1] + center_offset[1],
    ];
    let size = form_xy(pad, "size")?;
    let shape = pad.children().get(3).and_then(Expr::atom).unwrap_or("rect");
    // KiCad's drill offset displaces copper relative to the drill/pad
    // anchor. Pad angles in board files are absolute, and Y points down.
    let copper_offset = match pad.child("drill").and_then(|d| d.child("offset")) {
        Some(offset) => rotate_vector(
            [
                expression_coordinate(offset, 1, "pad copper offset x")?,
                expression_coordinate(offset, 2, "pad copper offset y")?,
            ],
            -pad_at[2],
        ),
        None => [0.0, 0.0],
    };
    let copper_center = [center[0] + copper_offset[0], center[1] + copper_offset[1]];
    let anchor_geometry = match shape {
        "roundrect" => roundrect_pad::geometry(pad, copper_center, size, -pad_at[2])?,
        "circle" => ObstacleGeometry::Circle {
            center: copper_center,
            radius: size[0].max(size[1]) / 2.0,
        },
        "oval" => {
            // Pad angles in a board file are already absolute, rather
            // than relative to the footprint angle.
            let angle = -pad_at[2];
            let (half_length, radius, local_end) = if size[0] >= size[1] {
                ((size[0] - size[1]) / 2.0, size[1] / 2.0, [1.0, 0.0])
            } else {
                ((size[1] - size[0]) / 2.0, size[0] / 2.0, [0.0, 1.0])
            };
            let offset = rotate_vector(
                [local_end[0] * half_length, local_end[1] * half_length],
                angle,
            );
            ObstacleGeometry::Segment {
                start: [copper_center[0] - offset[0], copper_center[1] - offset[1]],
                end: [copper_center[0] + offset[0], copper_center[1] + offset[1]],
                radius,
            }
        }
        "trapezoid" => {
            // KiCad's corners (PAD::BuildPadPolygon), with half the delta.
            let delta = form_xy(pad, "rect_delta").unwrap_or([0.0, 0.0]);
            let (half, d) = (
                [size[0] / 2.0, size[1] / 2.0],
                [delta[0] / 2.0, delta[1] / 2.0],
            );
            let corners = [
                [-half[0] - d[1], half[1] + d[0]],
                [-half[0] + d[1], -half[1] - d[0]],
                [half[0] - d[1], -half[1] + d[0]],
                [half[0] + d[1], half[1] - d[0]],
            ];
            ObstacleGeometry::Polygon {
                points: corners
                    .iter()
                    .map(|corner| {
                        let offset = rotate_vector(*corner, -pad_at[2]);
                        [copper_center[0] + offset[0], copper_center[1] + offset[1]]
                    })
                    .collect(),
            }
        }
        _ => ObstacleGeometry::Rectangle {
            center: copper_center,
            half_size: [size[0] / 2.0, size[1] / 2.0],
            angle_degrees: -pad_at[2],
        },
    };
    let geometry = if shape == "custom" {
        custom_pad_geometry(pad, copper_center, pad_at[2], anchor_geometry)?
    } else {
        anchor_geometry
    };
    Ok(LoweredPad { center, geometry })
}

fn custom_pad_geometry(
    pad: &Expr,
    center: [f64; 2],
    pad_angle_degrees: f64,
    anchor: ObstacleGeometry,
) -> Result<ObstacleGeometry, String> {
    let mut parts = vec![anchor];
    let primitives = pad
        .child("primitives")
        .ok_or_else(|| "custom pad has no primitives form".to_string())?;
    for primitive in primitives.children().iter().skip(1) {
        let width = primitive
            .child("stroke")
            .and_then(|stroke| form_f64(stroke, "width", 1).ok())
            .or_else(|| form_f64(primitive, "width", 1).ok())
            .unwrap_or(0.0)
            .max(0.0);
        let filled = !matches!(form_atom(primitive, "fill", 1), None | Some("no" | "none"));
        let local_point = |point: &Expr, label: &str| -> Result<[f64; 2], String> {
            let local = [
                expression_coordinate(point, 1, label)?,
                expression_coordinate(point, 2, label)?,
            ];
            let offset = rotate_vector(local, -pad_angle_degrees);
            Ok([center[0] + offset[0], center[1] + offset[1]])
        };
        let xy = |head: &str| -> Result<[f64; 2], String> {
            let form = primitive
                .child(head)
                .ok_or_else(|| format!("custom-pad primitive has no {head}"))?;
            local_point(form, head)
        };
        match primitive.head() {
            Some("gr_poly") => {
                let points_form = primitive
                    .child("pts")
                    .ok_or_else(|| "custom-pad polygon has no pts form".to_string())?;
                let mut points = Vec::new();
                for point in points_form.children().iter().skip(1) {
                    match point.head() {
                        Some("xy") => points.push(local_point(point, "custom-pad point")?),
                        // A rounded side, flattened.
                        Some("arc") => {
                            let corner = |head: &str| {
                                point
                                    .child(head)
                                    .ok_or_else(|| format!("custom-pad arc has no {head}"))
                                    .and_then(|form| local_point(form, head))
                            };
                            for at in outline::arc_points(
                                corner("start")?,
                                corner("mid")?,
                                corner("end")?,
                            ) {
                                if points.last() != Some(&at) {
                                    points.push(at);
                                }
                            }
                        }
                        _ => {
                            return Err(
                                "custom-pad polygon contains a point that is neither xy nor arc"
                                    .into(),
                            );
                        }
                    }
                }
                if points.len() < 3 {
                    return Err("custom-pad polygon has fewer than three points".into());
                }
                if filled {
                    parts.push(ObstacleGeometry::Polygon {
                        points: points.clone(),
                    });
                }
                // A stroked outline is copper too: one capsule per edge.
                if width > 0.0 {
                    for index in 0..points.len() {
                        parts.push(ObstacleGeometry::Segment {
                            start: points[index],
                            end: points[(index + 1) % points.len()],
                            radius: width / 2.0,
                        });
                    }
                }
            }
            Some("gr_line") => parts.push(ObstacleGeometry::Segment {
                start: xy("start")?,
                end: xy("end")?,
                radius: width.max(0.01) / 2.0,
            }),
            Some("gr_rect") => {
                let (a, b) = (xy("start")?, xy("end")?);
                // Rotated pads make the rectangle a general quadrilateral.
                let local_a = [
                    expression_coordinate(primitive.child("start").unwrap(), 1, "x")?,
                    expression_coordinate(primitive.child("start").unwrap(), 2, "y")?,
                ];
                let local_b = [
                    expression_coordinate(primitive.child("end").unwrap(), 1, "x")?,
                    expression_coordinate(primitive.child("end").unwrap(), 2, "y")?,
                ];
                let corners: Vec<[f64; 2]> = [
                    local_a,
                    [local_b[0], local_a[1]],
                    local_b,
                    [local_a[0], local_b[1]],
                ]
                .iter()
                .map(|corner| {
                    let offset = rotate_vector(*corner, -pad_angle_degrees);
                    [center[0] + offset[0], center[1] + offset[1]]
                })
                .collect();
                let _ = (a, b);
                if filled {
                    parts.push(ObstacleGeometry::Polygon {
                        points: corners.clone(),
                    });
                }
                if width > 0.0 {
                    for index in 0..4 {
                        parts.push(ObstacleGeometry::Segment {
                            start: corners[index],
                            end: corners[(index + 1) % 4],
                            radius: width / 2.0,
                        });
                    }
                }
            }
            Some("gr_circle") => {
                let circle_center = xy("center")?;
                let end = xy("end")?;
                let radius = distance_squared(circle_center, end).sqrt();
                if filled || width <= 0.0 {
                    parts.push(ObstacleGeometry::Circle {
                        center: circle_center,
                        radius: radius + width / 2.0,
                    });
                } else {
                    // An unfilled circle is its ring (a dome switch's outer
                    // contact around its inner pad, Sisu's keypad).
                    for pair in outline::circle_points(circle_center, radius).windows(2) {
                        parts.push(ObstacleGeometry::Segment {
                            start: pair[0],
                            end: pair[1],
                            radius: width / 2.0 + 2.0 * outline::ARC_TOLERANCE,
                        });
                    }
                }
            }
            Some("gr_arc") => {
                // Our chords lie inside the arc (by up to ARC_TOLERANCE) and
                // KiCad's approximation outside it (by up to its own 5 um):
                // the swept width covers both, or tracks end up a fraction
                // of a micrometre too close (capacitive sensor pads drawn
                // from arcs: 0.1993 mm against 0.2).
                let (start, mid, end) = (xy("start")?, xy("mid")?, xy("end")?);
                for pair in outline::arc_points(start, mid, end).windows(2) {
                    parts.push(ObstacleGeometry::Segment {
                        start: pair[0],
                        end: pair[1],
                        radius: width.max(0.01) / 2.0 + 2.0 * outline::ARC_TOLERANCE,
                    });
                }
            }
            // The pad editor's number box: KiCad reads it as a proxy shape
            // (SetIsProxyItem) that its pad outline leaves out, no copper.
            Some("gr_bbox") => {}
            other => {
                return Err(format!(
                    "KiCad grid adapter does not yet lower custom-pad primitive {}",
                    other.unwrap_or("unknown")
                ));
            }
        }
    }
    if parts.len() == 1 {
        // No primitives: KiCad draws just the anchor shape.
        return Ok(parts.pop().expect("the anchor"));
    }
    Ok(ObstacleGeometry::Union { parts })
}

pub(crate) fn pad_copper_layers(pad: &Expr) -> [bool; 2] {
    let Some(layers) = pad.child("layers") else {
        return [false, false];
    };
    let names: Vec<_> = layers
        .children()
        .iter()
        .skip(1)
        .filter_map(Expr::atom)
        .collect();
    [
        names.iter().any(|name| matches!(*name, "F.Cu" | "*.Cu")),
        names.iter().any(|name| matches!(*name, "B.Cu" | "*.Cu")),
    ]
}

pub(crate) fn copper_layer_index(layer: &str) -> Option<usize> {
    match layer {
        "F.Cu" => Some(0),
        "B.Cu" => Some(1),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_closed_edge_lines_define_the_same_rectangular_bounds_and_area() {
        let pcb = parse(
            r#"(kicad_pcb
                (gr_line (start 10 20) (end 30 20) (layer "Edge.Cuts"))
                (gr_line (start 30 40) (end 10 40) (layer "Edge.Cuts"))
                (gr_line (start 10 40) (end 10 20) (layer "Edge.Cuts"))
                (gr_line (start 30 20) (end 30 40) (layer "Edge.Cuts")))"#,
        )
        .unwrap();

        assert_eq!(
            rectangular_board_bounds(&pcb).unwrap(),
            Some([10.0, 20.0, 30.0, 40.0])
        );
        assert_eq!(rectangular_board_outline_area(&pcb).unwrap(), Some(400.0));
    }

    #[test]
    fn collinearly_split_edge_lines_still_define_a_rectangle() {
        let pcb = parse(
            r#"(kicad_pcb
                (gr_line (start 10 20) (end 17 20) (layer "Edge.Cuts"))
                (gr_line (start 17 20) (end 30 20) (layer "Edge.Cuts"))
                (gr_line (start 30 20) (end 30 40) (layer "Edge.Cuts"))
                (gr_line (start 30 40) (end 10 40) (layer "Edge.Cuts"))
                (gr_line (start 10 40) (end 10 20) (layer "Edge.Cuts")))"#,
        )
        .unwrap();

        assert_eq!(
            rectangular_board_bounds(&pcb).unwrap(),
            Some([10.0, 20.0, 30.0, 40.0])
        );
        assert_eq!(rectangular_board_outline_area(&pcb).unwrap(), Some(400.0));
    }

    #[test]
    fn non_convex_closed_line_outline_keeps_exact_area_and_blocks_corner_cutting() {
        let pcb = parse(
            r#"(kicad_pcb
                (gr_line (start 0 0) (end 10 0) (layer "Edge.Cuts"))
                (gr_line (start 10 0) (end 10 10) (layer "Edge.Cuts"))
                (gr_line (start 10 10) (end 6 10) (layer "Edge.Cuts"))
                (gr_line (start 6 10) (end 6 14) (layer "Edge.Cuts"))
                (gr_line (start 6 14) (end 4 14) (layer "Edge.Cuts"))
                (gr_line (start 4 14) (end 4 10) (layer "Edge.Cuts"))
                (gr_line (start 4 10) (end 0 10) (layer "Edge.Cuts"))
                (gr_line (start 0 10) (end 0 0) (layer "Edge.Cuts")))"#,
        )
        .unwrap();

        let outline = board_outline(&pcb).unwrap().unwrap();
        assert_eq!(outline.bounds, [0.0, 0.0, 10.0, 14.0]);
        assert!((outline.area() - 108.0).abs() < 1.0e-12);
        assert_eq!(board_outline_area(&pcb).unwrap(), Some(108.0));
        assert_eq!(rectangular_board_bounds(&pcb).unwrap(), None);
        assert!(outline.contains_point_with_clearance([5.0, 12.0], 0.1));
        assert!(!outline.contains_point_with_clearance([2.0, 12.0], 0.1));
        assert!(outline.contains_segment_with_clearance([5.0, 9.0], [5.0, 12.0], 0.1));
        assert!(!outline.contains_segment_with_clearance([2.0, 9.0], [5.0, 12.0], 0.1));
    }

    #[test]
    fn continuous_obstacle_check_rejects_crossing_shortcuts() {
        let obstacle = ObstacleGeometry::Circle {
            center: [5.0, 5.0],
            radius: 0.5,
        };
        assert!(obstacle.intersects_segment([1.0, 5.0], [9.0, 5.0], 0.2));
        assert!(!obstacle.intersects_segment([1.0, 7.0], [9.0, 7.0], 0.2));
    }

    #[test]
    fn custom_polygon_geometry_is_not_reduced_to_its_anchor() {
        let pad = parse(
            r#"(pad "2" smd custom
                (at 0 0)
                (size 0.3 0.3)
                (primitives
                    (gr_poly
                        (pts (xy 0.5 -0.75) (xy -0.65 -0.75) (xy -0.15 0)
                             (xy -0.65 0.75) (xy 0.5 0.75))
                        (width 0)
                        (fill yes))))"#,
        )
        .unwrap();
        let geometry = custom_pad_geometry(
            &pad,
            [10.0, 20.0],
            0.0,
            ObstacleGeometry::Rectangle {
                center: [10.0, 20.0],
                half_size: [0.15, 0.15],
                angle_degrees: 0.0,
            },
        )
        .unwrap();
        assert!(geometry.contains([10.45, 20.5], 0.0));
        assert!(!geometry.contains([9.0, 20.7], 0.0));
        assert_eq!(geometry.aabb().minimum, [9.35, 19.25]);
        assert_eq!(geometry.aabb().maximum, [10.5, 20.75]);
    }

    #[test]
    fn a_custom_pads_number_box_is_no_copper() {
        // A heatsink pad whose primitives are only the pad editor's number
        // box (gr_bbox), as KiCad 7 writes it: the anchor alone is copper.
        let pad = parse(
            r#"(pad "5" thru_hole custom (at 1.2 -1.5 90) (size 0.4 0.4) (drill 0.2)
                (layers "*.Cu") (options (clearance outline) (anchor rect))
                (primitives
                    (gr_bbox (start -0.75 -0.75) (end 0.75 0.75) (width 0.000001) (fill none))))"#,
        )
        .unwrap();
        let anchor = ObstacleGeometry::Rectangle {
            center: [10.0, 20.0],
            half_size: [0.2, 0.2],
            angle_degrees: 90.0,
        };
        let geometry = custom_pad_geometry(&pad, [10.0, 20.0], 90.0, anchor).unwrap();
        assert!(matches!(geometry, ObstacleGeometry::Rectangle { .. }));
        assert!(!geometry.contains([10.5, 20.5], 0.0));
    }

    #[test]
    fn custom_polygon_follows_its_arcs() {
        // A square with a half circle bulging out to the right.
        let pad = parse(
            r#"(pad "1" smd custom
                (at 0 0)
                (size 0.3 0.3)
                (primitives
                    (gr_poly
                        (pts (xy -1 -1) (arc (start 1 -1) (mid 2 0) (end 1 1)) (xy -1 1))
                        (width 0)
                        (fill yes))))"#,
        )
        .unwrap();
        let geometry = custom_pad_geometry(
            &pad,
            [10.0, 20.0],
            0.0,
            ObstacleGeometry::Rectangle {
                center: [10.0, 20.0],
                half_size: [0.15, 0.15],
                angle_degrees: 0.0,
            },
        )
        .unwrap();
        assert!(geometry.contains([11.9, 20.0], 0.0));
        assert!(!geometry.contains([11.9, 20.9], 0.0));
        assert!((geometry.aabb().maximum[0] - 12.0).abs() < 0.01);
    }
}
