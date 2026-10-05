// Copyright (C) 2026 Toit contributors.

//! KiCad's effective rounded-rectangle copper, expressed in existing primitives.
use super::*;

pub(super) fn geometry(
    pad: &Expr,
    center: [f64; 2],
    size: [f64; 2],
    angle_degrees: f64,
) -> Result<ObstacleGeometry, String> {
    geometry_with(pad, center, size, angle_degrees, true)
}

/// One layer's entry of a padstack, in board coordinates.
fn padstack_entry_geometry(entry: &Expr, center: [f64; 2], angle_degrees: f64) -> Result<ObstacleGeometry, String> {
    let size = form_xy(entry, "size")?;
    let shape = entry.child("shape").and_then(|shape| shape.children().get(1)).and_then(Expr::atom);
    match shape {
        Some("roundrect") if entry.child("chamfer").is_none() => geometry_with(entry, center, size, angle_degrees, false),
        Some("circle") => Ok(ObstacleGeometry::Circle { center, radius: size[0] / 2.0 }),
        // Rectangles, and conservatively anything else: the bounding
        // rectangle.
        _ => {
            let half = [size[0] / 2.0, size[1] / 2.0];
            let points = [[-half[0], -half[1]], [half[0], -half[1]], [half[0], half[1]], [-half[0], half[1]]]
                .map(|point| {
                    let rotated = rotate_vector(point, angle_degrees);
                    [center[0] + rotated[0], center[1] + rotated[1]]
                })
                .to_vec();
            Ok(ObstacleGeometry::Polygon { points })
        }
    }
}

fn geometry_with(
    pad: &Expr,
    center: [f64; 2],
    size: [f64; 2],
    angle_degrees: f64,
    use_padstack: bool,
) -> Result<ObstacleGeometry, String> {
    // Chamfered corners (`chamfer_ratio`, `chamfer top_left ...`): each
    // named corner is cut straight across at chamfer_ratio * min(size)
    // from the corner along both edges; the other corners keep the
    // rounding. KiCad's corner names are in the pad's own frame, y down.
    let chamfered: Vec<&str> = pad
        .child("chamfer")
        .map(|list| list.children().iter().skip(1).filter_map(|item| item.atom()).collect())
        .unwrap_or_default();
    if !chamfered.is_empty() {
        // KiCad's default when the file does not say.
        let chamfer_ratio = form_f64(pad, "chamfer_ratio", 1).unwrap_or(0.2);
        if !chamfer_ratio.is_finite() || !(0.0..=0.5).contains(&chamfer_ratio) {
            return Err("chamfer_ratio must be finite and between zero and 0.5".into());
        }
        if chamfer_ratio > 0.0 {
            return chamfered_geometry(pad, center, size, angle_degrees, &chamfered, chamfer_ratio);
        }
    }
    let mut size = size;
    let mut ratio_of = pad;
    if let Some(padstack) = pad.child("padstack").filter(|_| use_padstack) {
        // A padstack (KiCad 9) gives other copper layers their own shapes;
        // the pad's own shape is the front one. On a front-only surface
        // pad the other entries are inert (footprints edited in that mode
        // keep them); a back-only surface pad (a flipped footprint) is
        // what its `B.Cu` entry says.
        let copper: Vec<&str> = pad
            .child("layers")
            .map(|layers| {
                layers.children().iter().skip(1).filter_map(Expr::atom).filter(|name| name.ends_with(".Cu")).collect()
            })
            .unwrap_or_default();
        let back = padstack
            .children()
            .iter()
            .find(|item| item.head() == Some("layer") && item.children().get(1).and_then(Expr::atom) == Some("B.Cu"));
        match (copper.as_slice(), back) {
            (["F.Cu"], _) => {}
            (["B.Cu"], Some(entry)) if entry.child("shape").and_then(|shape| shape.children().get(1)).and_then(Expr::atom) == Some("roundrect") && entry.child("chamfer").is_none() => {
                size = form_xy(entry, "size")?;
                ratio_of = entry;
            }
            // Different copper per layer (a through-hole pad's inner and
            // back shapes): the obstacle is their union, exact on the
            // largest layer and cautious on the others.
            _ => {
                let mut parts = vec![geometry_with(pad, center, size, angle_degrees, false)?];
                for entry in padstack.children().iter().filter(|item| item.head() == Some("layer")) {
                    parts.push(padstack_entry_geometry(entry, center, angle_degrees)?);
                }
                return Ok(ObstacleGeometry::Union { parts });
            }
        }
    }
    if !size.iter().all(|v| v.is_finite() && *v > 0.0)
        || !center.iter().all(|v| v.is_finite())
        || !angle_degrees.is_finite()
    {
        return Err("roundrect pad requires finite positive dimensions and finite pose".into());
    }
    let ratio = form_f64(ratio_of, "roundrect_rratio", 1)
        .map_err(|_| "roundrect pad requires an explicit numeric roundrect_rratio".to_string())?;
    if !ratio.is_finite() || !(0.0..=0.5).contains(&ratio) {
        return Err("roundrect_rratio must be finite and between zero and 0.5".into());
    }
    // PADSTACK::RoundRectRadius uses KiROUND in nanometre board units.
    // PAD::buildEffectiveShape uses integer half sizes and a polygon/capsule
    // union, including the near-circle specialization below.
    // https://docs.kicad.org/doxygen/padstack_8cpp_source.html
    // https://docs.kicad.org/doxygen/pad_8cpp_source.html
    let size_iu = size.map(|v| (v * 1_000_000.0).round());
    if size_iu.iter().any(|v| *v < 1.0 || *v > i32::MAX as f64) {
        return Err("roundrect pad dimensions exceed native integer coordinate range".into());
    }
    let radius_iu = (size_iu[0].min(size_iu[1]) * ratio).round();
    let radius = radius_iu / 1_000_000.0;
    let inset_iu = size_iu.map(|v| (v / 2.0).floor() - radius_iu);
    if radius_iu > 0.0 && inset_iu.iter().all(|v| *v < 100.0) {
        return Ok(ObstacleGeometry::Circle { center, radius });
    }
    let corners = [
        [-inset_iu[0], inset_iu[1]],
        [inset_iu[0], inset_iu[1]],
        [inset_iu[0], -inset_iu[1]],
        [-inset_iu[0], -inset_iu[1]],
    ]
    .map(|p| {
        let rotated = rotate_vector(p, angle_degrees);
        [
            center[0] + rotated[0].round() / 1_000_000.0,
            center[1] + rotated[1].round() / 1_000_000.0,
        ]
    });
    let mut parts = Vec::new();
    // A degenerate inset contributes only the capsules; do not hand an empty
    // polygon to the point-in-polygon predicate.
    if inset_iu[0] > 0.0 && inset_iu[1] > 0.0 {
        parts.push(ObstacleGeometry::Polygon {
            points: corners.to_vec(),
        });
    }
    if radius > 0.0 {
        for i in 0..4 {
            parts.push(ObstacleGeometry::Segment {
                start: corners[i],
                end: corners[(i + 1) % 4],
                radius,
            });
        }
    }
    if parts.is_empty() {
        return Err("roundrect pad has degenerate native copper geometry".into());
    }
    Ok(ObstacleGeometry::Union { parts })
}

/// A rectangle with the `chamfered` corners cut straight and the others
/// rounded by `roundrect_rratio`: the polygon with every corner cut (by the
/// chamfer, or by the radius) plus a disc of the radius at each rounded
/// corner, which is exactly the rounded corner's quarter disc there and
/// inside the polygon elsewhere.
fn chamfered_geometry(
    pad: &Expr,
    center: [f64; 2],
    size: [f64; 2],
    angle_degrees: f64,
    chamfered: &[&str],
    chamfer_ratio: f64,
) -> Result<ObstacleGeometry, String> {
    let ratio = form_f64(pad, "roundrect_rratio", 1).unwrap_or(0.0);
    if !ratio.is_finite() || !(0.0..=0.5).contains(&ratio) {
        return Err("roundrect_rratio must be finite and between zero and 0.5".into());
    }
    let size_iu = size.map(|v| (v * 1_000_000.0).round());
    if size_iu.iter().any(|v| *v < 1.0 || *v > i32::MAX as f64) {
        return Err("roundrect pad dimensions exceed native integer coordinate range".into());
    }
    let shortest = size_iu[0].min(size_iu[1]);
    let radius_iu = (shortest * ratio).round();
    let chamfer_iu = (shortest * chamfer_ratio).round();
    let half = size_iu.map(|v| (v / 2.0).floor());
    let place = |p: [f64; 2]| {
        let rotated = rotate_vector(p, angle_degrees);
        [
            center[0] + rotated[0].round() / 1_000_000.0,
            center[1] + rotated[1].round() / 1_000_000.0,
        ]
    };
    // Corners counter-clockwise in the pad frame (y down): top left, bottom
    // left, bottom right, top right.
    let corners = [
        ("top_left", [-1.0, -1.0]),
        ("bottom_left", [-1.0, 1.0]),
        ("bottom_right", [1.0, 1.0]),
        ("top_right", [1.0, -1.0]),
    ];
    let mut points = Vec::new();
    let mut parts = Vec::new();
    for (index, (name, sign)) in corners.iter().enumerate() {
        let cut = if chamfered.contains(name) {
            chamfer_iu
        } else {
            radius_iu
        };
        let corner = [sign[0] * half[0], sign[1] * half[1]];
        if cut <= 0.0 {
            points.push(place(corner));
            continue;
        }
        // The two cut points, in the polygon's winding: coming along the
        // previous edge, leaving along the next.
        let along_x = [corner[0] - sign[0] * cut, corner[1]];
        let along_y = [corner[0], corner[1] - sign[1] * cut];
        let (first, second) = if index % 2 == 0 { (along_x, along_y) } else { (along_y, along_x) };
        points.push(place(first));
        points.push(place(second));
        if !chamfered.contains(name) {
            parts.push(ObstacleGeometry::Circle {
                center: place([corner[0] - sign[0] * cut, corner[1] - sign[1] * cut]),
                radius: cut / 1_000_000.0,
            });
        }
    }
    parts.insert(0, ObstacleGeometry::Polygon { points });
    Ok(ObstacleGeometry::Union { parts })
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_chamfered_corner_is_cut_straight_and_the_others_stay_rounded() {
        let pad = parse(
            "(pad \"1\" smd roundrect (at 0 0) (size 2 1) (layers \"F.Cu\") (roundrect_rratio 0.25) (chamfer_ratio 0.2) (chamfer top_left))",
        )
        .unwrap();
        let geometry = geometry(&pad, [0.0, 0.0], [2.0, 1.0], 0.0).unwrap();
        let ObstacleGeometry::Union { parts } = geometry else {
            panic!("union expected");
        };
        // The polygon plus three discs (the three rounded corners).
        assert_eq!(parts.len(), 4);
        let ObstacleGeometry::Polygon { points } = &parts[0] else {
            panic!("polygon first");
        };
        // Top left is cut 0.2 mm in from the corner (-1, -0.5), the others
        // 0.25 mm (the radius).
        assert!(points.contains(&[-0.8, -0.5]) && points.contains(&[-1.0, -0.3]), "{points:?}");
        assert!(points.contains(&[-1.0, 0.25]) && points.contains(&[-0.75, 0.5]), "{points:?}");
        assert!(matches!(parts[1], ObstacleGeometry::Circle { center: [-0.75, 0.25], radius } if (radius - 0.25).abs() < 1e-9));
        assert!(geometry_contains(&parts, [-0.95, -0.45]) == false, "the chamfer cuts the corner");
        assert!(geometry_contains(&parts, [0.97, 0.47]) == false, "a rounded corner is round");
        assert!(geometry_contains(&parts, [0.9, 0.4]), "inside the rounded corner's disc");
    }

    use super::*;
    fn geometry_contains(parts: &[ObstacleGeometry], point: [f64; 2]) -> bool {
        parts.iter().any(|part| part.contains(point, 0.0))
    }
    fn pad(ratio: &str) -> Expr {
        parse(&format!("(pad \"1\" smd roundrect (at 0 0) (size 2 2) (layers \"F.Cu\") (roundrect_rratio {ratio}))")).unwrap()
    }
    #[test]
    fn roundrect_corner_gap_is_not_its_square_envelope() {
        let g = geometry(&pad("0.25"), [0.0; 2], [2.0; 2], 0.0).unwrap();
        assert!(!g.contains([0.95, 0.95], 0.1));
        assert!(g.contains([0.95, 0.95], 0.14));
        assert!(g.intersects_segment([0.85, 0.85], [1.1, 1.1], 0.0));
        assert!(!g.intersects_segment([0.96, 0.94], [0.94, 0.96], 0.1));
    }
    #[test]
    fn roundrect_rotates_about_its_copper_center() {
        let center = [10.25, -2.5];
        let g = geometry(&pad("0.25"), center, [4.0, 2.0], -37.0).unwrap();
        for (p, inside) in [([1.9, 0.9], false), ([1.5, 0.8], true), ([0.0, 0.9], true)] {
            let p = rotate_vector(p, -37.0);
            assert_eq!(
                g.contains([center[0] + p[0], center[1] + p[1]], 0.0),
                inside
            );
        }
    }
    #[test]
    fn roundrect_zero_radius_and_maximum_radius_remain_valid() {
        let square = geometry(&pad("0"), [0.0; 2], [2.0; 2], 0.0).unwrap();
        assert!(square.contains([0.99, 0.99], 0.0));
        let circle = geometry(&pad("0.5"), [0.0; 2], [2.0; 2], 0.0).unwrap();
        assert!(matches!(
            circle,
            ObstacleGeometry::Circle { radius: 1.0, .. }
        ));
        assert!(!circle.contains([0.8, 0.8], 0.0));
        let oval = geometry(&pad("0.5"), [0.0; 2], [4.0, 2.0], 0.0).unwrap();
        assert!(oval.contains([1.9, 0.0], 0.0));
        assert!(!oval.contains([1.9, 0.9], 0.0));
    }
    #[test]
    fn roundrect_radius_uses_native_integer_rounding() {
        let g = geometry(&pad("0.1234567"), [0.0; 2], [2.0; 2], 0.0).unwrap();
        let ObstacleGeometry::Union { parts } = g else {
            panic!()
        };
        assert!(
            parts
                .iter()
                .any(|p| matches!(p,ObstacleGeometry::Segment{radius,..} if *radius==0.246913))
        );
    }
    #[test]
    fn odd_nanometre_rotated_pad_matches_kicad_10_effective_capsules() {
        // Installed KiCad10.0.6 GetEffectiveShape/GetRoundRectCornerRadius.
        let g = geometry(&pad("0.1234567"), [10.0, 10.0], [2.000001, 1.500003], -37.0).unwrap();
        let ObstacleGeometry::Union { parts } = g else {
            panic!()
        };
        let expected = [
            [9.689175, 10.941450],
            [10.990655, 9.960714],
            [10.310825, 9.058550],
            [9.009345, 10.039286],
        ];
        let capsules = parts
            .iter()
            .filter_map(|p| match p {
                ObstacleGeometry::Segment { start, end, radius } => Some((start, end, radius)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(capsules.len(), 4);
        for (i, (start, end, radius)) in capsules.iter().enumerate() {
            for axis in 0..2 {
                assert!((start[axis] - expected[i][axis]).abs() < 1e-12);
                assert!((end[axis] - expected[(i + 1) % 4][axis]).abs() < 1e-12);
            }
            assert_eq!(**radius, 0.185185);
        }
    }

    #[test]
    fn roundrect_invalid_and_padstack_shapes_are_rejected() {
        for ratio in ["NaN", "inf", "-0.1", "0.6", "garbage"] {
            assert!(geometry(&pad(ratio), [0.0; 2], [2.0; 2], 0.0).is_err());
        }
        let p = parse("(pad \"1\" smd roundrect (roundrect_rratio 0.25) (chamfer top_left) (chamfer_ratio 0.7))").unwrap();
        assert!(geometry(&p, [0.0; 2], [2.0; 2], 0.0).is_err());
        assert!(geometry(&pad("0.25"), [0.0; 2], [0.0, 2.0], 0.0).is_err());
    }

    #[test]
    fn per_layer_padstacks_are_the_union_of_their_layers() {
        let p = parse(
            "(pad \"1\" thru_hole roundrect (roundrect_rratio 0.25) (layers \"*.Cu\")
               (padstack (mode front_inner_back) (layer \"B.Cu\" (shape circle) (size 3 3))))",
        )
        .unwrap();
        let ObstacleGeometry::Union { parts } = geometry(&p, [0.0; 2], [2.0; 2], 0.0).unwrap() else {
            panic!("expected a union");
        };
        assert_eq!(parts.len(), 2);
        assert!(matches!(parts[1], ObstacleGeometry::Circle { radius, .. } if (radius - 1.5).abs() < 1e-12));
    }
}
