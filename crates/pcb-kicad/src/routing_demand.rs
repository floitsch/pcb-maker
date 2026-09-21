// Copyright (C) 2026 Toit contributors.

//! Advisory forecasts, never obstacles or evidence of electrical completion.
use super::*;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadRoutingDemand {
    /// Extra cost relative to ordinary straight travel at full demand.
    pub strength: f64,
    /// Linear falloff outside the physical clearance envelope.
    pub shoulder_mm: f64,
    pub alternatives: Vec<KiCadDemandAlternative>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadDemandAlternative {
    pub connection: String,
    /// Alternatives for one net should sum to one. Larger sums explicitly
    /// express greater criticality; the router does not infer probabilities.
    pub weight: f64,
    pub trace_width_mm: f64,
    pub clearance_mm: f64,
    pub via_size_mm: f64,
    pub paths: Vec<Vec<KiCadRoutePoint>>,
}

pub(super) fn check(demand: &KiCadRoutingDemand) -> Result<(), String> {
    if !demand.strength.is_finite()
        || !(0.0..=1e6).contains(&demand.strength)
        || !demand.shoulder_mm.is_finite()
        || demand.shoulder_mm <= 0.0
    {
        return Err(
            "routing demand requires finite nonnegative strength and positive shoulder".into(),
        );
    }
    for alternative in &demand.alternatives {
        if alternative.connection.is_empty()
            || !alternative.weight.is_finite()
            || !(0.0..=1e6).contains(&alternative.weight)
            || [alternative.trace_width_mm, alternative.via_size_mm]
                .iter()
                .any(|v| !v.is_finite() || *v <= 0.0)
            || !alternative.clearance_mm.is_finite()
            || alternative.clearance_mm < 0.0
            || alternative.paths.iter().any(|path| {
                path.len() < 2
                    || path.iter().any(|point| {
                        !matches!(point.layer.as_str(), "F.Cu" | "B.Cu")
                            || point.at.iter().any(|v| !v.is_finite())
                    })
            })
        {
            return Err("invalid routing demand alternative".into());
        }
        for path in &alternative.paths {
            for pair in path.windows(2) {
                if pair[0].layer != pair[1].layer && !same_native_position(pair[0].at, pair[1].at) {
                    return Err("demand layer changes must have coincident endpoints".into());
                }
            }
        }
    }
    Ok(())
}

fn identity(net: &str) -> &str {
    net.strip_prefix('/').unwrap_or(net)
}

pub(super) fn remove_completed(config: &mut KiCadGridRouteConfig, completed: &[String]) {
    if let Some(demand) = &mut config.routing_demand {
        demand.alternatives.retain(|a| {
            !completed
                .iter()
                .any(|net| identity(net) == identity(&a.connection))
        });
    }
}

/// Max-union within an alternative prevents shared tree branches and extra
/// vertices from multiplying demand. Different alternatives add by weight.
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
        // Reuse scratch storage across alternatives. Most forecasts cover a
        // small fraction of the board; only accumulate and clear those cells.
        // Alternative order stays unchanged so floating-point sums are exact.
        let mut trace = vec![0.0_f64; 2 * plane];
        let mut via = vec![0.0_f64; 2 * plane];
        let mut touched = Vec::new();
        for alternative in demand.alternatives.iter().filter(|a| {
            identity(&a.connection) != identity(connection)
                && a.weight > 0.0
                && demand.strength > 0.0
        }) {
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
                                let t = falloff(trace_core);
                                let v = falloff(via_core);
                                if (t > 0.0 || v > 0.0) && trace[state] == 0.0 && via[state] == 0.0
                                {
                                    touched.push(state);
                                }
                                trace[state] = trace[state].max(t);
                                via[state] = via[state].max(v);
                            }
                        }
                    }
                }
            }
            let scale = demand.strength * alternative.weight * f64::from(config.straight_cost);
            for &state in &touched {
                trace_total[state] += trace[state] * scale;
                let xy = state % plane;
                // Charge both via layers exactly once per touched XY. Read
                // both scratch planes before either plane is cleared.
                if state < plane || (trace[xy] == 0.0 && via[xy] == 0.0) {
                    let value = (via[xy] + via[plane + xy]) * scale * config.via_size_mm
                        / config.resolution_mm;
                    via_total[xy] += value;
                    via_total[plane + xy] += value;
                }
            }
            for state in touched.drain(..) {
                trace[state] = 0.0;
                via[state] = 0.0;
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

#[cfg(test)]
#[path = "routing_demand_dense_reference.rs"]
mod dense_reference;

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> KiCadGridRouteConfig {
        KiCadGridRouteConfig {
            resolution_mm: 0.5,
            routing_demand: Some(KiCadRoutingDemand {
                strength: 2.0,
                shoulder_mm: 1.0,
                alternatives: vec![KiCadDemandAlternative {
                    connection: "/future".into(),
                    weight: 1.0,
                    trace_width_mm: 0.5,
                    clearance_mm: 0.2,
                    via_size_mm: 0.8,
                    paths: vec![vec![
                        KiCadRoutePoint {
                            at: [1.0, 2.0],
                            layer: "F.Cu".into(),
                        },
                        KiCadRoutePoint {
                            at: [4.0, 2.0],
                            layer: "F.Cu".into(),
                        },
                    ]],
                }],
            }),
            ..Default::default()
        }
    }
    #[test]
    fn demand_preserves_identity_layer_and_shared_copper() {
        let config = fixture();
        check(config.routing_demand.as_ref().unwrap()).unwrap();
        let base = costs(&config, "present", [0.0, 0.0], [12, 12]);
        assert!(base.0[4 * 12 + 4] > 0);
        assert!(base.0[144..].iter().all(|v| *v == 0));
        assert_eq!(base.1[..144], base.1[144..]);
        assert!(
            costs(&config, "future", [0.0, 0.0], [12, 12])
                .0
                .iter()
                .all(|v| *v == 0)
        );
        let mut split = config.clone();
        let paths = &mut split.routing_demand.as_mut().unwrap().alternatives[0].paths;
        paths[0].insert(
            1,
            KiCadRoutePoint {
                at: [2.5, 2.0],
                layer: "F.Cu".into(),
            },
        );
        paths.push(paths[0].clone());
        assert_eq!(base, costs(&split, "present", [0.0, 0.0], [12, 12]));
        remove_completed(&mut split, &["future".into()]);
        assert!(
            costs(&split, "present", [0.0, 0.0], [12, 12])
                .0
                .iter()
                .all(|v| *v == 0)
        );
    }
    #[test]
    fn demand_rejects_nonphysical_layer_jumps_and_remains_soft() {
        let mut config = fixture();
        config.routing_demand.as_mut().unwrap().alternatives[0].paths[0][1].layer = "B.Cu".into();
        assert!(check(config.routing_demand.as_ref().unwrap()).is_err());
        let mut config = fixture();
        config.routing_demand.as_mut().unwrap().strength = 1e6;
        config.routing_demand.as_mut().unwrap().alternatives[0].weight = 1e6;
        assert!(
            costs(&config, "present", [0.0, 0.0], [12, 12])
                .0
                .iter()
                .all(|v| *v < u32::MAX)
        );
    }

    #[test]
    fn sparse_accumulation_matches_dense_for_layers_crops_weights_and_shared_trees() {
        for resolution in [0.125, 0.3, 0.5] {
            for trace_width in [0.2, 1.6] {
                for strength in [0.0, 0.7, 1e6] {
                    let mut config = fixture();
                    config.resolution_mm = resolution;
                    config.trace_width_mm = trace_width;
                    let demand = config.routing_demand.as_mut().unwrap();
                    demand.strength = strength;
                    let template = demand.alternatives[0].clone();
                    demand.alternatives.clear();
                    for i in 0..9 {
                        let mut alternative = template.clone();
                        alternative.weight = [0.0, 0.3, 1.0, 1e6][i % 4];
                        alternative.connection =
                            if i % 3 == 0 { "/present" } else { "future" }.into();
                        let point = |at, back| KiCadRoutePoint {
                            at,
                            layer: if back { "B.Cu" } else { "F.Cu" }.into(),
                        };
                        let x = i as f64 - 4.0;
                        let start = [x, 1.5];
                        let end = [x + 2.3, 3.1];
                        alternative.paths = vec![
                            vec![point(start, i % 2 == 0), point(end, i % 2 == 0)],
                            vec![point(end, false), point(end, true)],
                            vec![point(end, true), point([x + 4.0, -1.0], true)],
                        ];
                        alternative.paths.push(alternative.paths[0].clone());
                        demand.alternatives.push(alternative);
                    }
                    for origin in [[0.0, 0.0], [-2.3, -1.7], [100.0, 100.0]] {
                        let expected = dense_reference::costs(&config, "present", origin, [37, 29]);
                        assert_eq!(costs(&config, "present", origin, [37, 29]), expected);
                    }
                }
            }
        }
        let mut config = fixture();
        config.routing_demand = None;
        assert_eq!(
            costs(&config, "present", [0.0, 0.0], [13, 17]),
            dense_reference::costs(&config, "present", [0.0, 0.0], [13, 17])
        );
    }
}
