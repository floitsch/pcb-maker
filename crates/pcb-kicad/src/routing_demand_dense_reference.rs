// Copyright (C) 2026 Toit contributors.

// Frozen dense implementation for differential tests of sparse accumulation.
use super::*;

pub(super) fn costs(
    config: &KiCadGridRouteConfig,
    connection: &str,
    origin: [f64; 2],
    size: [usize; 2],
) -> (Vec<u32>, Vec<u32>) {
    let [width, height] = size;
    let plane = width * height;
    let mut trace_total = vec![0.0; 2 * plane];
    let mut via_total = vec![0.0; 2 * plane];
    if let Some(demand) = &config.routing_demand {
        for alternative in demand.alternatives.iter().filter(|a| {
            identity(&a.connection) != identity(connection)
                && a.weight > 0.0
                && demand.strength > 0.0
        }) {
            let mut trace = vec![0.0_f64; 2 * plane];
            let mut via = vec![0.0_f64; 2 * plane];
            for path in &alternative.paths {
                for pair in path.windows(2) {
                    let changing_layer = pair[0].layer != pair[1].layer;
                    let foreign_radius = if changing_layer {
                        alternative.via_size_mm
                    } else {
                        alternative.trace_width_mm
                    } / 2.0;
                    let clearance = config.clearance_mm.max(alternative.clearance_mm);
                    let trace_core = foreign_radius + config.trace_width_mm / 2.0 + clearance;
                    let via_core = foreign_radius + config.via_size_mm / 2.0 + clearance;
                    let reach = trace_core.max(via_core) + demand.shoulder_mm;
                    let low = [0, 1].map(|axis| {
                        (((pair[0].at[axis].min(pair[1].at[axis]) - reach - origin[axis])
                            / config.resolution_mm)
                            .floor()
                            .max(0.0) as usize)
                            .min(size[axis])
                    });
                    let high = [0, 1].map(|axis| {
                        (((pair[0].at[axis].max(pair[1].at[axis]) + reach - origin[axis])
                            / config.resolution_mm)
                            .ceil()
                            .max(0.0) as usize)
                            .min(size[axis].saturating_sub(1))
                    });
                    for layer in 0..2 {
                        if !changing_layer && (pair[0].layer == "B.Cu") != (layer == 1) {
                            continue;
                        }
                        for y in low[1]..=high[1] {
                            for x in low[0]..=high[0] {
                                let point = [
                                    origin[0] + x as f64 * config.resolution_mm,
                                    origin[1] + y as f64 * config.resolution_mm,
                                ];
                                let distance =
                                    point_segment_distance_squared(point, pair[0].at, pair[1].at)
                                        .sqrt();
                                let falloff = |core: f64| {
                                    (1.0 - (distance - core).max(0.0) / demand.shoulder_mm).max(0.0)
                                };
                                let state = layer * plane + y * width + x;
                                trace[state] = trace[state].max(falloff(trace_core));
                                via[state] = via[state].max(falloff(via_core));
                            }
                        }
                    }
                }
            }
            let scale = demand.strength * alternative.weight * f64::from(config.straight_cost);
            for state in 0..2 * plane {
                trace_total[state] += trace[state] * scale;
                // A via occupies both layers; express its footprint cost in
                // straight-track-equivalent travel across its diameter.
                via_total[state] +=
                    (via[state % plane] + via[plane + state % plane]) * scale * config.via_size_mm
                        / config.resolution_mm;
            }
        }
    }
    // u32::MAX means forbidden to the grid backend. Forecasts stay soft.
    let quantize = |v: f64| v.round().min(f64::from(u32::MAX / 4)) as u32;
    (
        trace_total.into_iter().map(quantize).collect(),
        via_total.into_iter().map(quantize).collect(),
    )
}
