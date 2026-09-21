// Copyright (C) 2026 Toit contributors.

//! Inspect an actual prepared branch request without changing its geometry.
//! Boundary samples are evidence of local obstruction, not a minimum cut.
use super::*;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct KiCadRoutingCutConfig {
    pub branch: usize,
    /// Sample each transition kind separately, nearest the opposite endpoint
    /// first. Full region sizes and boundary counts are retained independently.
    pub maximum_samples_per_kind: usize,
    pub routing: KiCadGridRouteConfig,
}

impl Default for KiCadRoutingCutConfig {
    fn default() -> Self {
        Self {
            branch: 0,
            maximum_samples_per_kind: 2048,
            routing: KiCadGridRouteConfig::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadRoutingCutReport {
    pub schema_version: u32,
    pub connection: String,
    pub branch: usize,
    pub attachment_rank: usize,
    pub origin_mm: [f64; 2],
    pub resolution_mm: f64,
    pub grid_size: [usize; 2],
    pub start_mm: [f64; 2],
    pub finish_mm: [f64; 2],
    pub connected: bool,
    pub start_reachable_states: usize,
    pub finish_reachable_states: usize,
    pub inspected_endpoint: &'static str,
    /// [layer, row, first column, last column], inclusive.
    pub reachable_runs: Vec<[usize; 4]>,
    pub planar_boundary_transitions: usize,
    pub rejected_via_entries: usize,
    pub sampled_planar_transitions: usize,
    pub sampled_via_entries: usize,
    pub samples_without_copper_attribution: usize,
    pub boundary_sampling_truncated: bool,
    pub blockers: Vec<KiCadRoutingCutBlocker>,
    pub interpretation: &'static str,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadRoutingCutBlocker {
    /// Index in this prepared model, disambiguating segments of the same net.
    pub obstacle_index: usize,
    pub object: String,
    pub kind: KiCadViaLocalRerouteBlockerKind,
    pub net: Option<String>,
    pub footprint: Option<String>,
    pub layers: [bool; 2],
    geometry: ObstacleGeometry,
    pub planar_hits: usize,
    pub via_hits: usize,
    pub examples_mm: Vec<[f64; 2]>,
}

impl KiCadRoutingCutReport {
    /// Advisory removal order from sampled copper boundary hits. Pads and
    /// other fixed objects are excluded; no foreign net is made ineligible.
    pub fn yielding_connection_priority(&self) -> Vec<String> {
        let mut scores = BTreeMap::<String, usize>::new();
        for blocker in &self.blockers {
            if blocker.kind == KiCadViaLocalRerouteBlockerKind::ForeignNetCopper
                && let Some(net) = &blocker.net
            {
                *scores.entry(net.clone()).or_default() += blocker.planar_hits + blocker.via_hits;
            }
        }
        let mut ordered: Vec<_> = scores.into_iter().collect();
        ordered.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        ordered.into_iter().map(|(net, _)| net).collect()
    }
}

pub(super) struct Inspector {
    pub branch: usize,
    maximum_samples: usize,
    connection: String,
    report: Option<KiCadRoutingCutReport>,
}

/// Prepare the requested branch using the production routing path, then flood
/// its exact grid. Earlier branches may be searched to construct a shared tree;
/// the inspected branch is not routed and no board is written.
pub fn inspect_kicad_routing_cut(
    board: &Path,
    connection: &str,
    config: &KiCadRoutingCutConfig,
) -> Result<KiCadRoutingCutReport, String> {
    if config.maximum_samples_per_kind == 0 {
        return Err("routing cut sample budget must be positive".into());
    }
    let mut inspector = Inspector {
        branch: config.branch,
        maximum_samples: config.maximum_samples_per_kind,
        connection: connection.into(),
        report: None,
    };
    let result = route_materialized_connection_inspected(
        board,
        connection,
        &[],
        &config.routing,
        Some(&mut inspector),
    );
    inspector.report.ok_or_else(|| match result {
        Ok(_) => format!("requested routing branch {} does not exist", config.branch),
        Err(error) => error.to_string(),
    })
}

#[derive(Clone)]
struct Sample {
    start: GridPosition,
    finish: GridPosition,
    via: bool,
    corners: Vec<GridPosition>,
}

impl Inspector {
    pub fn capture(
        &mut self,
        request: &GridRouteRequest,
        model: &KiCadRoutingModel,
        config: &KiCadGridRouteConfig,
        origin: [f64; 2],
    ) -> Result<(), String> {
        let from_start = pcb_grid_router::reachable_states(request)?;
        let mut reverse = request.clone();
        std::mem::swap(&mut reverse.start, &mut reverse.finish);
        std::mem::swap(
            &mut reverse.alternate_starts,
            &mut reverse.alternate_finishes,
        );
        let from_finish = pcb_grid_router::reachable_states(&reverse)?;
        let start_count = from_start.iter().filter(|&&v| v).count();
        let finish_count = from_finish.iter().filter(|&&v| v).count();
        let connected = std::iter::once(request.finish)
            .chain(request.alternate_finishes.iter().copied())
            .any(|p| from_start[request.state_index(p).expect("validated endpoint")]);
        let (states, endpoint, opposite) = if start_count <= finish_count {
            (&from_start, "start", request.finish)
        } else {
            (&from_finish, "finish", request.start)
        };
        let mut runs = Vec::new();
        for layer in 0..request.layers {
            for y in 0..request.height {
                let mut x = 0;
                while x < request.width {
                    if !states[(layer * request.height + y) * request.width + x] {
                        x += 1;
                        continue;
                    }
                    let first = x;
                    while x + 1 < request.width
                        && states[(layer * request.height + y) * request.width + x + 1]
                    {
                        x += 1;
                    }
                    runs.push([layer, y, first, x]);
                    x += 1;
                }
            }
        }
        let mut planar = Vec::new();
        let mut vias = Vec::new();
        for (state, &inside) in states.iter().enumerate() {
            if !inside {
                continue;
            }
            let p = request.position(state)?;
            for (dx, dy) in [
                (1, 0),
                (-1, 0),
                (0, 1),
                (0, -1),
                (1, 1),
                (1, -1),
                (-1, 1),
                (-1, -1),
            ] {
                let Some(x) = p.x.checked_add_signed(dx) else {
                    continue;
                };
                let Some(y) = p.y.checked_add_signed(dy) else {
                    continue;
                };
                if x >= request.width || y >= request.height {
                    continue;
                }
                let q = GridPosition { x, y, ..p };
                if states[request.state_index(q)?] {
                    continue;
                }
                let mut corners = Vec::new();
                if dx != 0 && dy != 0 {
                    for c in [GridPosition { x, ..p }, GridPosition { y, ..p }] {
                        if request.blocked[request.state_index(c)?] {
                            corners.push(c);
                        }
                    }
                }
                planar.push(Sample {
                    start: p,
                    finish: q,
                    via: false,
                    corners,
                });
            }
            if request.allow_vias {
                for layer in 0..request.layers {
                    if layer == p.layer {
                        continue;
                    }
                    let q = GridPosition { layer, ..p };
                    let next = request.state_index(q)?;
                    // Include forbidden vias even if the other layer is also
                    // reachable through another part of the component.
                    if request.via_enter_costs[next] == u32::MAX || request.blocked[next] {
                        vias.push(Sample {
                            start: p,
                            finish: q,
                            via: true,
                            corners: Vec::new(),
                        });
                    }
                }
            }
        }
        let planar_count = planar.len();
        let via_count = vias.len();
        let rank = |s: &Sample| {
            (
                s.start.x.abs_diff(opposite.x).pow(2) + s.start.y.abs_diff(opposite.y).pow(2),
                s.start.layer,
                s.start.y,
                s.start.x,
                s.finish.layer,
                s.finish.y,
                s.finish.x,
            )
        };
        for samples in [&mut planar, &mut vias] {
            samples.sort_by_key(&rank);
            samples.truncate(self.maximum_samples);
        }
        let grid = KiCadObstacleGrid::new(model, 4.0, config.clearance_mm);
        let at = |p| grid_position_at(p, origin, config.resolution_mm);
        let mut hits = BTreeMap::<usize, KiCadRoutingCutBlocker>::new();
        let mut unattributed = 0;
        for sample in planar.iter().chain(&vias) {
            let a = at(sample.start);
            let b = at(sample.finish);
            let radius = if sample.via {
                config.via_size_mm / 2.0
            } else {
                config.trace_width_mm / 2.0
            };
            let inflate = radius + config.clearance_mm;
            let layers: Vec<_> = if sample.via {
                (0..request.layers).collect()
            } else {
                vec![sample.start.layer]
            };
            let mut matching = BTreeSet::new();
            for layer in layers {
                let (candidates, _) =
                    grid.query(layer, GeometryAabb::around_segment(a, b, inflate));
                for i in candidates {
                    let o = &model.obstacles[i];
                    let relevant = if sample.via {
                        o.blocks_vias
                    } else {
                        o.blocks_tracks
                    };
                    if !relevant {
                        continue;
                    }
                    let amount = o.inflate(inflate, config.clearance_mm);
                    if o.geometry.intersects_segment(a, b, amount)
                        || (!sample.via
                            && sample
                                .corners
                                .iter()
                                .any(|&p| o.geometry.contains(at(p), amount)))
                    {
                        matching.insert(i);
                    }
                }
            }
            if matching.is_empty() {
                unattributed += 1;
            }
            for i in matching {
                let o = &model.obstacles[i];
                let row = hits.entry(i).or_insert_with(|| KiCadRoutingCutBlocker {
                    obstacle_index: i,
                    object: o.object.clone(),
                    kind: o.kind,
                    net: o.net.clone(),
                    footprint: o.footprint.clone(),
                    layers: o.layers,
                    geometry: o.geometry.clone(),
                    planar_hits: 0,
                    via_hits: 0,
                    examples_mm: Vec::new(),
                });
                if sample.via {
                    row.via_hits += 1;
                } else {
                    row.planar_hits += 1;
                }
                if row.examples_mm.len() < 4 {
                    row.examples_mm.push(a);
                }
            }
        }
        let mut blockers: Vec<_> = hits.into_values().collect();
        blockers.sort_by(|a, b| {
            (b.planar_hits + b.via_hits)
                .cmp(&(a.planar_hits + a.via_hits))
                .then(a.object.cmp(&b.object))
        });
        self.report = Some(KiCadRoutingCutReport {
            schema_version: 1,
            connection: self.connection.clone(),
            branch: self.branch,
            attachment_rank: 0,
            origin_mm: origin,
            resolution_mm: config.resolution_mm,
            grid_size: [request.width, request.height],
            start_mm: at(request.start),
            finish_mm: at(request.finish),
            connected,
            start_reachable_states: start_count,
            finish_reachable_states: finish_count,
            inspected_endpoint: endpoint,
            reachable_runs: runs,
            planar_boundary_transitions: planar_count,
            rejected_via_entries: via_count,
            sampled_planar_transitions: planar.len(),
            sampled_via_entries: vias.len(),
            samples_without_copper_attribution: unattributed,
            boundary_sampling_truncated: planar_count > planar.len() || via_count > vias.len(),
            blockers,
            interpretation: "Exact prepared-grid reachability for the first attachment request of this branch; ranked portfolios may try others. The smaller endpoint region is shown. Boundary and rejected-via samples are nearest the opposite primary endpoint first. Copper attribution is sampled, not a minimum cut or proof that moving a named object restores connectivity. Unattributed samples may involve outline, target holes or corner rules. Obstacle indices identify geometry within this prepared model, not native UUIDs. The inspected branch was not routed; no native admission is implied.",
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cut_attributes_a_fixed_wall_and_preserves_source() {
        let source = r#"(kicad_pcb (gr_rect (start 0 0) (end 10 8) (layer "Edge.Cuts"))
          (footprint "A" (at 1 2) (pad "1" thru_hole circle (at 0 0) (size 1 1) (drill 0.4) (layers "*.Cu" "*.Mask") (net "N")))
          (footprint "B" (at 8 2) (pad "1" thru_hole circle (at 0 0) (size 1 1) (drill 0.4) (layers "*.Cu" "*.Mask") (net "N")))
          (footprint "WALL" (at 5 4) (pad "1" thru_hole rect (at 0 0) (size 1 10) (drill 0.4) (layers "*.Cu" "*.Mask") (net "BLOCK"))))"#;
        let path = env::temp_dir().join(format!(
            "pcb-cut-{}.kicad_pcb",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::write(&path, source).unwrap();
        let config = KiCadRoutingCutConfig {
            routing: KiCadGridRouteConfig {
                resolution_mm: 0.5,
                trace_width_mm: 0.2,
                clearance_mm: 0.1,
                edge_clearance_mm: 0.1,
                terminal_contact_policy: KiCadTerminalContactPolicy::FirstPadContact,
                ..Default::default()
            },
            ..Default::default()
        };
        let report = inspect_kicad_routing_cut(&path, "N", &config).unwrap();
        assert!(!report.connected);
        assert!(
            report
                .blockers
                .iter()
                .any(|b| b.net.as_deref() == Some("BLOCK") && b.planar_hits > 0)
        );
        assert!(report.start_reachable_states > 0 && report.finish_reachable_states > 0);
        let limited = inspect_kicad_routing_cut(
            &path,
            "N",
            &KiCadRoutingCutConfig {
                maximum_samples_per_kind: 1,
                ..config.clone()
            },
        )
        .unwrap();
        assert!(limited.boundary_sampling_truncated);
        assert_eq!(limited.reachable_runs, report.reachable_runs);
        assert_eq!(
            limited.planar_boundary_transitions,
            report.planar_boundary_transitions
        );
        assert_eq!(limited.sampled_planar_transitions, 1);
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        assert_eq!(
            report
                .reachable_runs
                .iter()
                .map(|r| r[3] - r[2] + 1)
                .sum::<usize>(),
            report
                .start_reachable_states
                .min(report.finish_reachable_states)
        );
        fs::remove_file(path).unwrap();
    }
}
