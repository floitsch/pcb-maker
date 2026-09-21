// Copyright (C) 2026 Toit contributors.

//! Native pad/footprint clearance overrides, shared by routing and repair.
//! The configured router clearance remains a conservative floor. This does
//! not resolve custom design rules, net classes, or pair-specific exceptions.

use super::{CopperObstacle, Expr, GeometryAabb, form_f64};

pub(super) fn pad_clearance(pad: &Expr, footprint: &Expr) -> Result<f64, String> {
    // KiCad 9+: an explicit pad zero overrides the footprint; an absent pad
    // value inherits it. See the native manual's pad clearance overrides:
    // https://docs.kicad.org/10.0/en/pcbnew/pcbnew.html
    let owner = if pad.child("clearance").is_some() {
        pad
    } else if footprint.child("clearance").is_some() {
        footprint
    } else {
        return Ok(0.0);
    };
    let clearance = form_f64(owner, "clearance", 1)?;
    if !clearance.is_finite() || clearance < 0.0 {
        return Err("unsupported negative or non-finite pad/footprint clearance".into());
    }
    Ok(clearance)
}

impl CopperObstacle {
    pub(super) fn clearance(&self, configured: f64) -> f64 {
        configured.max(self.local_clearance_mm)
    }

    pub(super) fn inflate(&self, radius_and_clearance: f64, configured: f64) -> f64 {
        radius_and_clearance + (self.local_clearance_mm - configured).max(0.0)
    }

    pub(super) fn query_bounds(&self, configured: f64) -> GeometryAabb {
        let mut bounds = self.geometry.aabb();
        let extra = (self.local_clearance_mm - configured).max(0.0);
        for axis in 0..2 {
            bounds.minimum[axis] -= extra;
            bounds.maximum[axis] += extra;
        }
        bounds
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CopperQueryKind, KiCadGridRouteConfig, KiCadObstacleGrid, KiCadRoutingModel, parse,
        route_segment_is_clear,
    };

    #[test]
    fn pad_override_precedes_footprint_including_explicit_zero() {
        let footprint = parse("(footprint (clearance 2.5))").unwrap();
        for (source, expected) in [
            ("(pad)", 2.5),
            ("(pad (clearance 1.85))", 1.85),
            ("(pad (clearance 0))", 0.0),
        ] {
            assert_eq!(
                pad_clearance(&parse(source).unwrap(), &footprint).unwrap(),
                expected
            );
        }
        assert!(pad_clearance(&parse("(pad (clearance NaN))").unwrap(), &footprint).is_err());
        assert!(pad_clearance(&parse("(pad (clearance -1))").unwrap(), &footprint).is_err());
    }

    #[test]
    fn clearance_halo_is_found_across_grid_cells_and_in_shortening_checks() {
        let pcb = parse(
            r#"(kicad_pcb
          (gr_rect (start 0 0) (end 30 20) (layer "Edge.Cuts"))
          (footprint "A" (at 2 2)
            (pad "1" smd circle (at 0 0) (size 1 1) (layers "F.Cu") (net "/TARGET")))
          (footprint "B" (at 28 2)
            (pad "1" smd circle (at 0 0) (size 1 1) (layers "B.Cu") (net "/TARGET")))
          (footprint "H" (at 10 10) (clearance 5.2)
            (pad "" np_thru_hole circle (at 0 0) (size 1 1) (drill 1)
              (layers "*.Cu" "*.Mask"))))"#,
        )
        .unwrap();
        let model = KiCadRoutingModel::from_pcb(&pcb, "TARGET").unwrap();
        let config = KiCadGridRouteConfig {
            trace_width_mm: 0.4,
            clearance_mm: 0.3,
            ..Default::default()
        };
        let grid = KiCadObstacleGrid::new(&model, 1.0, config.clearance_mm);
        for layer in 0..2 {
            for kind in [CopperQueryKind::Trace, CopperQueryKind::Via] {
                assert!(grid.any_contains(&model, layer, [15.0, 10.0], 0.5, kind));
                assert!(!grid.any_contains(&model, layer, [16.5, 10.0], 0.5, kind));
            }
            assert!(grid.any_intersects_segment(&model, layer, [14.0, 8.0], [14.0, 12.0], 0.5));
            assert!(!route_segment_is_clear(
                [14.0, 8.0],
                [14.0, 12.0],
                layer,
                &model,
                &config
            ));
            assert!(route_segment_is_clear(
                [17.0, 8.0],
                [17.0, 12.0],
                layer,
                &model,
                &config
            ));
        }
        // Physical copper remains its original size; only the derived query
        // envelope is expanded. The router default is a floor, not an addition.
        let obstacle = &model.obstacles[0];
        assert_eq!(obstacle.geometry.aabb().maximum, [10.5, 10.5]);
        assert!((obstacle.inflate(0.5, 0.3) - 5.4).abs() < 1e-9);
        assert_eq!(obstacle.inflate(6.2, 6.0), 6.2);
    }
}
