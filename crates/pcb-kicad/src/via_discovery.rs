// Copyright (C) 2026 Toit contributors.

use super::*;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadViaDiscoveryConfig {
    pub routing: KiCadGridRouteConfig,
    pub maximum_excursion_mm: f64,
    pub maximum_actions: usize,
    pub via_penalty_mm: f64,
}

fn blocking_owners(
    candidate: &KiCadRouteCandidate,
    affected: &[usize],
    model: &KiCadRoutingModel,
) -> Vec<serde_json::Value> {
    let mut owners = BTreeMap::<(KiCadViaLocalRerouteBlockerKind, String), BTreeSet<usize>>::new();
    for (index, obstacle) in model.obstacles.iter().enumerate() {
        let tracks = obstacle.blocks_tracks
            && affected.iter().any(|&i| {
                candidate.branches[i].path.windows(2).any(|pair| {
                    pair[0].layer == pair[1].layer
                        && copper_layer_index(&pair[0].layer).is_some_and(|layer| {
                            obstacle.layers[layer]
                                && obstacle.geometry.intersects_segment(
                                    pair[0].at,
                                    pair[1].at,
                                    obstacle.inflate(
                                        candidate.config.trace_width_mm / 2.0
                                            + candidate.config.clearance_mm,
                                        candidate.config.clearance_mm,
                                    ),
                                )
                        })
                })
            });
        let vias = obstacle.blocks_vias
            && (obstacle.layers[0] || obstacle.layers[1])
            && candidate.supplemental_vias.iter().any(|via| {
                obstacle.geometry.contains(
                    via.at,
                    obstacle.inflate(
                        candidate.config.via_size_mm / 2.0 + candidate.config.clearance_mm,
                        candidate.config.clearance_mm,
                    ),
                )
            });
        if tracks || vias {
            owners
                .entry((obstacle.kind, obstacle.object.clone()))
                .or_default()
                .insert(index);
        }
    }
    owners.into_iter().map(|((kind,object),indices)|serde_json::json!({"kind":kind,"object":object,"obstacle_indices":indices})).collect()
}

// Consecutive internal vias bound a layer excursion. A shared root path can
// expose the same physical excursion several times; deduplicate before work.
fn excursions(
    branches: &[KiCadRouteBranch],
    maximum_mm: f64,
) -> Result<Vec<([f64; 2], [f64; 2], String, f64)>, String> {
    let mut result = BTreeMap::new();
    for branch in branches {
        let topology = branch_topology(&branch.path)?;
        let vias = (1..topology.points.len() - 1)
            .filter(|&i| topology.points[i].was_via)
            .collect::<Vec<_>>();
        for pair in vias.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let outer = &topology.segment_layers[a - 1];
            if *outer != topology.segment_layers[b] {
                continue;
            }
            let length = topology.points[a..=b]
                .windows(2)
                .map(|p| distance_squared(p[0].at, p[1].at).sqrt())
                .sum::<f64>();
            if length > maximum_mm {
                continue;
            }
            let start = topology.points[a].at;
            let finish = topology.points[b].at;
            let key = |p: [f64; 2]| ((p[0] * 1e6).round() as i64, (p[1] * 1e6).round() as i64);
            let (lo, hi) = if key(start) <= key(finish) {
                (key(start), key(finish))
            } else {
                (key(finish), key(start))
            };
            result
                .entry((lo, hi, outer.clone()))
                .or_insert((start, finish, outer.clone(), length));
        }
    }
    Ok(result.into_values().collect())
}

/// Discover candidates from native routed copper, without supplied via IDs,
/// net choices or route candidates. Unsupported import cases remain explicit.
pub fn discover_kicad_via_opportunities(
    board_path: &Path,
    output: &Path,
    config: &KiCadViaDiscoveryConfig,
) -> Result<serde_json::Value, String> {
    validate_grid_route_config(&config.routing)?;
    if !config.maximum_excursion_mm.is_finite()
        || config.maximum_excursion_mm <= 0.0
        || !(1..=1000).contains(&config.maximum_actions)
        || !config.via_penalty_mm.is_finite()
        || config.via_penalty_mm < 0.0
    {
        return Err("invalid via discovery limits".into());
    }
    if output.exists() {
        return Err("discovery output must be fresh".into());
    }
    let source = fs::read_to_string(board_path).map_err(|e| e.to_string())?;
    let pcb = parse(&source)?;
    let source_sha256 = file_sha256(board_path)?;
    let via_nets = pcb
        .children()
        .iter()
        .filter(|n| n.head() == Some("via"))
        .filter_map(node_net)
        .map(|n| normalize_net(n).to_string())
        .collect::<BTreeSet<_>>();
    let mut coverage = Vec::new();
    let mut actions = Vec::new();
    for net in &via_nets {
        let imported = (|| -> Result<KiCadRouteCandidate, String> {
            let mut candidate = import_route_candidate_from_expr(&pcb, net)?;
            let resolved = connection_rules::resolve(&config.routing, net)?;
            for (actual, required) in [
                (candidate.config.trace_width_mm, resolved.trace_width_mm),
                (candidate.config.via_size_mm, resolved.via_size_mm),
                (candidate.config.via_drill_mm, resolved.via_drill_mm),
            ] {
                if (actual - required).abs() > 1e-6 {
                    return Err(
                        "supplied routing dimensions differ from imported native copper".into(),
                    );
                }
            }
            candidate.config = resolved;
            Ok(candidate)
        })();
        let candidate = match imported {
            Ok(c) => c,
            Err(e) => {
                coverage
                    .push(serde_json::json!({"connection":net,"status":"unsupported","reason":e}));
                continue;
            }
        };
        let model = KiCadRoutingModel::from_pcb_with_config(&pcb, net, &candidate.config)?;
        let source_quality = route_candidate_quality(&candidate)?;
        let pairs = excursions(&candidate.branches, config.maximum_excursion_mm)?;
        let pair_vias = candidate
            .supplemental_vias
            .iter()
            .filter(|via| {
                pairs.iter().any(|(a, b, _, _)| {
                    same_native_position(via.at, *a) || same_native_position(via.at, *b)
                })
            })
            .count();
        coverage.push(serde_json::json!({"connection":net,"status":"scanned","vias":source_quality.vias,"excursions":pairs.len(),
            "vias_in_pair_excursions":pair_vias,"vias_outside_pair_excursions":source_quality.vias.saturating_sub(pair_vias)}));
        for (at, other, layer, length) in pairs {
            let via_actions = KiCadViaActionSearchConfig {
                target_at: at,
                target_layers: ["F.Cu".into(), "B.Cu".into()],
                removal_layers: vec![layer.clone()],
                relocation_radius_mm: 0.0,
                relocation_step_mm: 0.0,
                maximum_relocation_candidates: 0,
                feasible_frontier: None,
                local_reroute: Some(KiCadViaLocalRerouteSearchConfig {
                    merged_layers: vec![layer.clone()],
                    resolutions_mm: vec![
                        config.routing.resolution_mm,
                        config.routing.resolution_mm / 2.5,
                    ],
                    window_margin_mm: 5.0,
                    maximum_grid_states: 500_000,
                    maximum_expansions: 500_000,
                    maximum_exact_edge_retries: 16,
                    blocker_cut: Some(KiCadViaBlockerCutSearchConfig {
                        maximum_ranked_blockers: 5,
                        maximum_cut_size: 1,
                        maximum_trials: 5,
                    }),
                }),
                maximum_actions: 3,
                maximum_affected_branches: 100,
                via_penalty_mm: config.via_penalty_mm,
                minimum_score_improvement_mm: 0.001,
            };
            let (direct, affected) = match apply_via_topology_action(
                &candidate,
                &via_actions,
                &KiCadViaTopologyAction::Remove {
                    merged_layer: layer.clone(),
                },
            ) {
                Ok(v) => v,
                Err(e) => {
                    coverage.push(serde_json::json!({"connection":net,"status":"unsupported_action","target_at":at,"reason":e}));
                    continue;
                }
            };
            let quality = route_candidate_quality(&direct)?;
            let saved = source_quality.vias.saturating_sub(quality.vias);
            if saved == 0 {
                continue;
            }
            let mut blockers = selected_geometry_blockers(&direct, &affected, &model);
            blockers.extend(candidate_via_blockers(&direct, &model));
            blockers.sort();
            blockers.dedup();
            let owners = blocking_owners(&direct, &affected, &model);
            // Count semantic owners, not duplicate hits along shared root
            // paths. Keep non-object constraints separate from those owners.
            let mut other_constraints = Vec::new();
            let edge = direct.config.edge_clearance_mm;
            if affected.iter().any(|&i| {
                direct.branches[i].path.windows(2).any(|p| {
                    p[0].layer == p[1].layer
                        && !model.outline.contains_segment_with_clearance(
                            p[0].at,
                            p[1].at,
                            edge + direct.config.trace_width_mm / 2.0,
                        )
                })
            }) || direct.supplemental_vias.iter().any(|v| {
                !model
                    .outline
                    .contains_point_with_clearance(v.at, edge + direct.config.via_size_mm / 2.0)
            }) {
                other_constraints.push("board_edge_clearance");
            }
            if direct.supplemental_vias.iter().any(|v| {
                model.target_holes.iter().any(|hole| {
                    hole.contains(
                        v.at,
                        direct.config.via_drill_mm / 2.0 + direct.config.hole_to_hole_clearance_mm,
                    )
                })
            }) {
                other_constraints.push("target_hole_clearance");
            }
            let rank_blockers = owners.len() + usize::from(!other_constraints.is_empty());
            let label = |p| {
                pcb.children()
                    .iter()
                    .find(|v| {
                        v.head() == Some("via")
                            && node_net(v).is_some_and(|n| normalize_net(n) == net)
                            && form_xy(v, "at").is_ok_and(|q| same_native_position(p, q))
                    })
                    .and_then(|v| form_atom(v, "uuid", 1))
                    .map(|id| format!("V-{}", id.chars().take(6).collect::<String>()))
            };
            let mut reroute = config.routing.clone();
            reroute.via_cost_mm = Some(100.0);
            reroute.tree_attachment_objective = KiCadTreeAttachmentObjective::RouterCost;
            let repair = KiCadCoupledViaConfig {
                via_actions,
                reroute,
                maximum_coupled_trials: 4,
            };
            let evidence = serde_json::json!({"connection":net,"target_at":at,"other_via_at":other,
                "target_label":label(at),"other_label":label(other),"merged_layer":layer,
                "excursion_length_mm":length,"estimated_vias_removed":saved,
                "direct_length_change_mm":quality.length_mm-source_quality.length_mm,
                "direct_blockers":blockers,"blocking_owners":owners,"other_constraints":other_constraints,"direct_geometry_clear":blockers.is_empty(),
                "scope":"Potential topology improvement; neither blockers nor route quality prove a complete feasible repair."});
            actions.push((
                rank_blockers,
                length,
                net.clone(),
                at,
                evidence,
                candidate.clone(),
                repair,
            ));
        }
    }
    actions.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.total_cmp(&b.1))
            .then_with(|| a.2.cmp(&b.2))
            .then_with(|| a.3[0].total_cmp(&b.3[0]))
            .then_with(|| a.3[1].total_cmp(&b.3[1]))
    });
    let generated = actions.len();
    actions.truncate(config.maximum_actions);
    fs::create_dir_all(output).map_err(|e| e.to_string())?;
    let board_id = board_path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or("board name unavailable")?;
    fs::write(output.join(format!("{board_id}.kicad_pcb")), &source).map_err(|e| e.to_string())?;
    let _ = verification_preview::capture(output, board_id, || {
        Err("Read-only discovery; native verification was not run by this scan.".into())
    });
    let mut ranked = Vec::new();
    for (rank, (_, _, _, _, mut row, candidate, repair)) in actions.into_iter().enumerate() {
        let directory = output.join(format!("action-{rank:03}"));
        fs::create_dir(&directory).map_err(|e| e.to_string())?;
        write_pretty_json(&directory.join("candidate.json"), &candidate)?;
        write_pretty_json(&directory.join("config.json"), &repair)?;
        row["rank"] = serde_json::json!(rank + 1);
        row["artifacts"] = serde_json::json!(directory);
        ranked.push(row);
    }
    let report = serde_json::json!({"source":board_path,"source_sha256":source_sha256,
        "via_bearing_connections":via_nets.len(),"coverage":coverage,"generated_actions":generated,
        "ranking":"Fewest blocking semantic owners (plus any non-object constraints), then shortest layer excursion; deterministic net/coordinate tie breaks.",
        "truncated":generated>ranked.len(),"actions":ranked,
        "analysis_scope":"Consecutive internal via pairs within maximum_excursion_mm on a copper branch. Isolated vias and alternatives spanning plated pads are not yet proposed; zero actions is not proof that the board cannot improve.",
        "import_scope":"Uniform-width acyclic routed trees, including branch connections through plated pads; candidate import may normalize copper inside pads. The complete repair must be compared against the original board as well as its imported candidate."});
    write_pretty_json(&output.join("discovery.json"), &report)?;
    if file_sha256(board_path)? != source_sha256 {
        return Err("source board changed during scan".into());
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn excursion_detection_counts_shared_and_reversed_paths_once() {
        let point = |at, layer: &str| KiCadRoutePoint {
            at,
            layer: layer.into(),
        };
        let path = vec![
            point([0., 0.], "F.Cu"),
            point([2., 0.], "F.Cu"),
            point([2., 0.], "B.Cu"),
            point([3., 1.], "B.Cu"),
            point([4., 0.], "B.Cu"),
            point([4., 0.], "F.Cu"),
            point([6., 0.], "F.Cu"),
        ];
        let branch = |path| KiCadRouteBranch {
            start_terminal: [0., 0.],
            finish_terminal: [6., 0.],
            cost: 0,
            expansions: 0,
            path,
        };
        let branches = vec![
            branch(path.clone()),
            branch(path.iter().rev().cloned().collect()),
        ];
        assert_eq!(excursions(&branches, 3.).unwrap().len(), 1);
        assert!(excursions(&branches, 2.).unwrap().is_empty());
        let one_via = vec![
            point([0., 0.], "F.Cu"),
            point([2., 0.], "F.Cu"),
            point([2., 0.], "B.Cu"),
            point([4., 0.], "B.Cu"),
        ];
        assert!(excursions(&[branch(one_via)], 10.).unwrap().is_empty());
    }
}
