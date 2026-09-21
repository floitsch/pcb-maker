// Copyright (C) 2026 Toit contributors.

//! Local, advisory demand on a cut through an axis-aligned body passage.
//! This does not certify topology infeasibility or replace exact rerouting.

use super::{AxisAlignedBounds, DIRECTION_EPSILON};
use layout_trace_model::{Problem, Vec2};
use pcb_validate::SolvedTrace;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PassageCopperOccupant {
    pub electrical_net: String,
    /// Merged perpendicular projections of copper crossing the cut. Width is
    /// not multiplied by the number of tessellation vertices or branch aliases.
    pub projected_bands_mm: Vec<[f64; 2]>,
    pub projected_width_mm: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PassageCopperDemand {
    pub target_branch: String,
    pub target_electrical_net: String,
    pub layer: String,
    pub cut: [Vec2; 2],
    pub occupants: Vec<PassageCopperOccupant>,
    pub required_gap_mm: f64,
}

fn axis(p: Vec2, index: usize) -> f64 {
    if index == 0 { p.x } else { p.y }
}

fn merged_bands(mut bands: Vec<[f64; 2]>) -> Vec<[f64; 2]> {
    bands.sort_by(|a, b| a[0].total_cmp(&b[0]).then_with(|| a[1].total_cmp(&b[1])));
    let mut result: Vec<[f64; 2]> = Vec::new();
    for band in bands {
        if let Some(last) = result.last_mut()
            && band[0] <= last[1] + DIRECTION_EPSILON
        {
            last[1] = last[1].max(band[1]);
        } else {
            result.push(band);
        }
    }
    result
}

pub(super) fn demand(
    problem: &Problem,
    traces: &[SolvedTrace],
    branches: &BTreeSet<String>,
    mover: AxisAlignedBounds,
    direction: Vec2,
    gap: f64,
) -> Option<PassageCopperDemand> {
    let normal = usize::from(direction.x == 0.0);
    let tangent = 1 - normal;
    let positive = axis(direction, normal) > 0.0;
    let wall = axis(if positive { mover.min } else { mover.max }, normal);
    let (low, high) = if positive {
        (wall - gap, wall)
    } else {
        (wall, wall + gap)
    };
    let cross = (axis(mover.min, tangent) + axis(mover.max, tangent)) / 2.0;
    let point = |v| {
        if normal == 0 {
            Vec2::new(v, cross)
        } else {
            Vec2::new(cross, v)
        }
    };
    let mut best: Option<PassageCopperDemand> = None;
    for net in problem.nets.iter().filter(|n| branches.contains(&n.id)) {
        // An unresolved layer choice needs layer-specific failure evidence.
        // Do not add demands from mutually exclusive layer assignments.
        let layer = match net.allowed_layers.as_slice() {
            [] => net.layer.as_str(),
            [layer] => layer.as_str(),
            _ => continue,
        };
        let target_net = net.electrical_net.as_deref().unwrap_or(&net.id);
        let mut bands = BTreeMap::<String, Vec<[f64; 2]>>::new();
        for trace in traces.iter().filter(|t| t.electrical_net != target_net) {
            for (i, edge) in trace.points.windows(2).enumerate() {
                if trace
                    .segment_layers
                    .get(i)
                    .map(String::as_str)
                    .unwrap_or(&trace.layer)
                    != layer
                {
                    continue;
                }
                let (a, b) = (axis(edge[0], tangent), axis(edge[1], tangent));
                if cross < a.min(b) - DIRECTION_EPSILON || cross > a.max(b) + DIRECTION_EPSILON {
                    continue;
                }
                let band = if (b - a).abs() <= DIRECTION_EPSILON {
                    let (x, y) = (axis(edge[0], normal), axis(edge[1], normal));
                    [x.min(y) - trace.width / 2.0, x.max(y) + trace.width / 2.0]
                } else {
                    let t = ((cross - a) / (b - a)).clamp(0.0, 1.0);
                    let center =
                        axis(edge[0], normal) + t * (axis(edge[1], normal) - axis(edge[0], normal));
                    [center - trace.width / 2.0, center + trace.width / 2.0]
                };
                if band[1] < low || band[0] > high {
                    continue;
                }
                bands
                    .entry(trace.electrical_net.clone())
                    .or_default()
                    .push(band);
            }
        }
        let occupants: Vec<_> = bands
            .into_iter()
            .map(|(electrical_net, bands)| {
                let projected_bands_mm = merged_bands(bands);
                let projected_width_mm = projected_bands_mm.iter().map(|b| b[1] - b[0]).sum();
                PassageCopperOccupant {
                    electrical_net,
                    projected_bands_mm,
                    projected_width_mm,
                }
            })
            .collect();
        let required_gap_mm = net.width
            + 2.0 * problem.rules.clearance
            + occupants
                .iter()
                .map(|o| {
                    o.projected_width_mm
                        + o.projected_bands_mm.len() as f64 * problem.rules.clearance
                })
                .sum::<f64>();
        let candidate = PassageCopperDemand {
            target_branch: net.id.clone(),
            target_electrical_net: target_net.into(),
            layer: layer.into(),
            cut: [point(low), point(high)],
            occupants,
            required_gap_mm,
        };
        if best
            .as_ref()
            .is_none_or(|old| candidate.required_gap_mm > old.required_gap_mm)
        {
            best = Some(candidate);
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use layout_trace_model::topology::{RouteClass, TerminalSector};

    fn trace(net: &str, layer: &str, width: f64) -> SolvedTrace {
        SolvedTrace {
            branch: net.into(),
            electrical_net: net.into(),
            net: net.into(),
            from_node: "a".into(),
            to_node: "b".into(),
            width,
            tension_weight: 1.0,
            layer: "preferred-layer-is-not-authoritative".into(),
            segment_layers: vec![layer.into(); 2],
            vias: vec![],
            points: vec![
                Vec2::new(8.0, 0.7),
                Vec2::new(10.0, 0.7),
                Vec2::new(12.0, 0.7),
            ],
            route_class: RouteClass::direct(
                layer,
                TerminalSector {
                    component: "a".into(),
                    sector: "1".into(),
                },
                TerminalSector {
                    component: "b".into(),
                    sector: "1".into(),
                },
            ),
            route_basis_fingerprint: None,
        }
    }

    #[test]
    fn duplicate_vertices_and_branch_overlap_do_not_multiply_demand() {
        assert_eq!(
            merged_bands(vec![[1.0, 1.4], [1.0, 1.4], [1.2, 1.8], [3.0, 3.2]]),
            vec![[1.0, 1.8], [3.0, 3.2]]
        );
    }

    #[test]
    fn demand_respects_electrical_identity_actual_layers_and_retessellation() {
        let problem: Problem = serde_json::from_str(include_str!(
            "../../../benchmarks/small/passage-pressure-occupied.json"
        ))
        .unwrap();
        let mover = AxisAlignedBounds {
            min: Vec2::new(9.0, 1.15),
            max: Vec2::new(11.0, 10.85),
        };
        let branches = BTreeSet::from(["SIGNAL2".into()]);
        let route = trace("OTHER", "top", 0.4);
        let expected = demand(
            &problem,
            &[route.clone()],
            &branches,
            mover,
            Vec2::new(0.0, 1.0),
            1.15,
        )
        .unwrap();
        assert!((expected.required_gap_mm - 1.4).abs() < 1e-9);
        assert_eq!(expected.occupants.len(), 1);
        assert_eq!(expected.occupants[0].projected_bands_mm.len(), 1);
        let mut alias = route.clone();
        alias.branch = "other-branch-same-net".into();
        alias.points = vec![Vec2::new(8.0, 0.7), Vec2::new(12.0, 0.7)];
        alias.segment_layers = vec!["top".into()];
        let actual = demand(
            &problem,
            &[
                route,
                alias,
                trace("BOTTOM", "bottom", 0.6),
                trace("SIGNAL2", "top", 0.8),
            ],
            &branches,
            mover,
            Vec2::new(0.0, 1.0),
            1.15,
        )
        .unwrap();
        assert_eq!(actual, expected);
        let mut flexible = problem;
        flexible.nets[2].allowed_layers = vec!["top".into(), "bottom".into()];
        assert!(
            demand(
                &flexible,
                &[trace("OTHER", "top", 0.4)],
                &branches,
                mover,
                Vec2::new(0.0, 1.0),
                1.15
            )
            .is_none()
        );
    }
}
