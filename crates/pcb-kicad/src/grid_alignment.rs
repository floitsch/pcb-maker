// Copyright (C) 2026 Toit contributors.

//! A bounded alternative to globally refining a grid that misses a pad gap.
//! Bounding boxes conservatively identify channels; the existing continuous
//! collision checker must accept the entire passage before it can steer the grid.

use super::{
    KiCadGridAlignment, KiCadGridAlignmentEvidence, KiCadGridRouteConfig, KiCadRoutingModel,
    KiCadViaLocalRerouteBlockerKind, copper_layer_name, distance_squared, route_segment_is_clear,
};

pub(super) fn aligned_origin(
    model: &KiCadRoutingModel,
    config: &KiCadGridRouteConfig,
    mut origin: [f64; 2],
) -> ([f64; 2], Option<KiCadGridAlignmentEvidence>) {
    if config.grid_alignment == KiCadGridAlignment::BoardOrigin {
        return (origin, None);
    }
    let pads: Vec<_> = model
        .obstacles
        .iter()
        .filter(|o| {
            o.kind == KiCadViaLocalRerouteBlockerKind::FootprintPad
                && o.blocks_tracks
                && o.footprint.is_some()
        })
        .collect();
    let mut best: Option<KiCadGridAlignmentEvidence> = None;
    for (i, a) in pads.iter().enumerate() {
        for b in &pads[i + 1..] {
            if a.footprint != b.footprint {
                continue;
            }
            for axis in 0..2 {
                let cross = 1 - axis;
                let (left, right) =
                    if a.geometry.aabb().minimum[axis] <= b.geometry.aabb().minimum[axis] {
                        (a, b)
                    } else {
                        (b, a)
                    };
                let lb = left.geometry.aabb();
                let rb = right.geometry.aabb();
                let low = lb.maximum[axis]
                    + config.trace_width_mm / 2.0
                    + left.clearance(config.clearance_mm);
                let high = rb.minimum[axis]
                    - config.trace_width_mm / 2.0
                    - right.clearance(config.clearance_mm);
                // Only narrow, positive-width bands missed by the current
                // lattice need another phase. Exact tangencies are not free.
                if high - low <= 1e-8 || high - low >= config.resolution_mm {
                    continue;
                }
                let first_line = origin[axis]
                    + ((low - origin[axis]) / config.resolution_mm).ceil() * config.resolution_mm;
                if first_line > low + 1e-8 && first_line < high - 1e-8 {
                    continue;
                }
                if lb.maximum[cross].min(rb.maximum[cross])
                    - lb.minimum[cross].max(rb.minimum[cross])
                    <= config.resolution_mm
                {
                    continue;
                }
                let center = (low + high) / 2.0;
                let mut entry = [0.0; 2];
                let mut exit = [0.0; 2];
                entry[axis] = center;
                exit[axis] = center;
                entry[cross] = lb.minimum[cross].min(rb.minimum[cross]);
                exit[cross] = lb.maximum[cross].max(rb.maximum[cross]);
                let mut detour = f64::INFINITY;
                for start in &model.terminals {
                    for finish in &model.terminals {
                        if start[cross] >= entry[cross] || finish[cross] <= exit[cross] {
                            continue;
                        }
                        detour = detour.min(
                            distance_squared(*start, entry).sqrt()
                                + distance_squared(entry, exit).sqrt()
                                + distance_squared(exit, *finish).sqrt()
                                - distance_squared(*start, *finish).sqrt(),
                        );
                    }
                }
                if !detour.is_finite() {
                    continue;
                }
                for layer in 0..2 {
                    if !left.layers[layer]
                        || !right.layers[layer]
                        || !route_segment_is_clear(entry, exit, layer, model, config)
                    {
                        continue;
                    }
                    let candidate = KiCadGridAlignmentEvidence {
                        footprint: left.footprint.clone().unwrap(),
                        layer: copper_layer_name(layer).into(),
                        axis,
                        center_band_mm: [low, high],
                        passage: [entry, exit],
                        detour_mm: detour.max(0.0),
                        offset_mm: (center - origin[axis]).rem_euclid(config.resolution_mm),
                    };
                    // Geometry breaks ties independently of native object order.
                    let rank = |e: &KiCadGridAlignmentEvidence| {
                        (
                            e.axis,
                            e.passage.map(|p| p.map(f64::to_bits)),
                            e.layer.clone(),
                        )
                    };
                    if best.as_ref().is_none_or(|old| {
                        candidate.detour_mm.total_cmp(&old.detour_mm).is_lt()
                            || (candidate.detour_mm == old.detour_mm
                                && rank(&candidate) < rank(old))
                    }) {
                        best = Some(candidate);
                    }
                }
            }
        }
    }
    if let Some(evidence) = &best {
        origin[evidence.axis] += evidence.offset_mm;
    }
    (origin, best)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CopperObstacle, ObstacleGeometry, parse};

    fn fixture() -> (KiCadRoutingModel, KiCadGridRouteConfig) {
        let pcb = parse(
            r#"(kicad_pcb
          (net 1 "SIGNAL")
          (gr_rect (start 0 0) (end 12 8) (layer "Edge.Cuts"))
          (footprint "Left" (layer "F.Cu") (at 2 4)
            (property "Reference" "J1")
            (pad "1" smd circle (at 0 0) (size 0.8 0.8) (layers "F.Cu") (net 1 "SIGNAL")))
          (footprint "Right" (layer "F.Cu") (at 10 4)
            (property "Reference" "J2")
            (pad "1" smd circle (at 0 0) (size 0.8 0.8) (layers "F.Cu") (net 1 "SIGNAL")))
          (footprint "Row" (layer "F.Cu") (at 6 4)
            (property "Reference" "U1")
            (pad "1" smd rect (at 0 -0.635) (size 1.5 0.9) (layers "F.Cu"))
            (pad "2" smd rect (at 0 0.635) (size 1.5 0.9) (layers "F.Cu"))))"#,
        )
        .unwrap();
        let model = KiCadRoutingModel::from_pcb(&pcb, "SIGNAL").unwrap();
        let config = KiCadGridRouteConfig {
            resolution_mm: 0.05,
            trace_width_mm: 0.15,
            clearance_mm: 0.1,
            grid_alignment: KiCadGridAlignment::NearestNarrowPadGap,
            ..Default::default()
        };
        (model, config)
    }

    #[test]
    fn missed_gap_alignment_is_translation_invariant_and_keeps_grid_spacing() {
        let (model, config) = fixture();
        let origin = [0.575, 0.575];
        for shift in [[0.0, 0.0], [-17.023, 45.007], [0.011, -0.019]] {
            let mut moved = model.clone();
            moved.bounds = [
                model.bounds[0] + shift[0],
                model.bounds[1] + shift[1],
                model.bounds[2] + shift[0],
                model.bounds[3] + shift[1],
            ];
            moved.outline = crate::BoardOutline::rectangle(moved.bounds).unwrap();
            for terminal in &mut moved.terminals {
                for axis in 0..2 {
                    terminal[axis] += shift[axis];
                }
            }
            for obstacle in &mut moved.obstacles {
                if let ObstacleGeometry::Rectangle { center, .. } = &mut obstacle.geometry {
                    for axis in 0..2 {
                        center[axis] += shift[axis];
                    }
                }
            }
            let input = [origin[0] + shift[0], origin[1] + shift[1]];
            let (aligned, evidence) = aligned_origin(&moved, &config, input);
            let evidence = evidence.unwrap();
            assert_eq!(evidence.axis, 1);
            assert_eq!(aligned[0], input[0]);
            assert!((evidence.offset_mm - 0.025).abs() < 1e-10);
            assert!(evidence.detour_mm.abs() < 1e-10);
            let line = aligned[1]
                + ((4.0 + shift[1] - aligned[1]) / config.resolution_mm).round()
                    * config.resolution_mm;
            assert!((line - 4.0 - shift[1]).abs() < 1e-10);
        }
    }

    #[test]
    fn blocked_occluded_already_aligned_and_irrelevant_gaps_do_not_shift() {
        let (mut model, mut config) = fixture();
        let origin = [0.575, 0.575];
        config.clearance_mm = 0.2;
        assert!(aligned_origin(&model, &config, origin).1.is_none());
        config.clearance_mm = 0.1;
        model.obstacles[0].local_clearance_mm = 0.2;
        assert!(aligned_origin(&model, &config, origin).1.is_none());
        model.obstacles[0].local_clearance_mm = 0.0;
        assert!(aligned_origin(&model, &config, [0.575, 0.6]).1.is_none());
        let blocker = CopperObstacle {
            source_uuid: None,
            geometry: ObstacleGeometry::Circle {
                center: [6.0, 4.0],
                radius: 0.05,
            },
            kind: KiCadViaLocalRerouteBlockerKind::ForeignNetCopper,
            ..model.obstacles[0].clone()
        };
        model.obstacles.push(blocker);
        assert!(aligned_origin(&model, &config, origin).1.is_none());
        model.obstacles.pop();
        model.terminals[1] = [3.0, 4.0];
        assert!(aligned_origin(&model, &config, origin).1.is_none());
        config.grid_alignment = KiCadGridAlignment::BoardOrigin;
        assert_eq!(aligned_origin(&model, &config, origin).0, origin);
    }
}
