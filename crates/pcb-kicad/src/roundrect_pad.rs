// Copyright (C) 2026 Toit contributors.

//! KiCad's effective rounded-rectangle copper, expressed in existing primitives.
use super::*;

pub(super) fn geometry(
    pad: &Expr,
    center: [f64; 2],
    size: [f64; 2],
    angle_degrees: f64,
) -> Result<ObstacleGeometry, String> {
    if pad.child("chamfer").is_some() || pad.child("chamfer_ratio").is_some() {
        return Err("chamfered roundrect pads have no exact geometry lowering".into());
    }
    if pad.child("padstack").is_some() {
        return Err("per-layer roundrect padstacks have no exact geometry lowering".into());
    }
    if !size.iter().all(|v| v.is_finite() && *v > 0.0)
        || !center.iter().all(|v| v.is_finite())
        || !angle_degrees.is_finite()
    {
        return Err("roundrect pad requires finite positive dimensions and finite pose".into());
    }
    let ratio = form_f64(pad, "roundrect_rratio", 1)
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

#[cfg(test)]
mod tests {
    use super::*;
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
    fn roundrect_invalid_and_chamfered_shapes_are_rejected() {
        for ratio in ["NaN", "inf", "-0.1", "0.6", "garbage"] {
            assert!(geometry(&pad(ratio), [0.0; 2], [2.0; 2], 0.0).is_err());
        }
        for extra in ["(chamfer top_left)", "(chamfer_ratio 0.2)", "(padstack)"] {
            let p = parse(&format!(
                "(pad \"1\" smd roundrect (roundrect_rratio 0.25) {extra})"
            ))
            .unwrap();
            assert!(geometry(&p, [0.0; 2], [2.0; 2], 0.0).is_err());
        }
        assert!(geometry(&pad("0.25"), [0.0; 2], [0.0, 2.0], 0.0).is_err());
    }
}
