// Copyright (C) 2026 Toit contributors.

//! Coupled placement and routing.
//!
//! The board is placed, then routed once in full. From then on the router
//! and placer take turns on the same in-memory state: the router's
//! congestion history names the footprints sitting where nets fought for
//! room, the placer proposes small legal moves for them, and the router
//! reroutes only the nets the move invalidated. A move is kept when the
//! board improves (open connections first, then vias and copper). The final
//! board is verified natively once.

use super::*;
use crate::board_placer::{largest_clearance, lower_placement, write_footprint_pose};
use crate::placement_constraints::{KiCadConstraintStatus, constraint_report, resolve_constraints};
use crate::board_router::{LayerTable, add_pour_zones, core_config, emit_routes, finish_routed_board, lower, pours};
use pcb_placer as placer;
use pcb_router as core;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct KiCadBoardLayoutConfig {
    /// Footprint moves to try after the first full route.
    pub moves: usize,
    /// Stop after this many consecutive moves without improvement.
    pub patience: usize,
    /// Move distances tried, in millimetres, in the four axis directions.
    pub steps_mm: Vec<f64>,
    /// Wall-clock budget for the move phase, in seconds.
    pub move_seconds: f64,
    /// Once every connection is routed, moves only polish (fewer vias,
    /// less copper): they stop after this many seconds of the move phase.
    pub polish_seconds: f64,
    pub placer: KiCadBoardPlacerConfig,
    /// Interchangeable pins (`pin-swaps.json`; relative to the source
    /// directory). The nets on them are permuted after placement.
    pub pin_swaps: Option<PathBuf>,
    pub pin_swap: KiCadPinSwapConfig,
}

impl Default for KiCadBoardLayoutConfig {
    fn default() -> Self {
        Self {
            moves: 40,
            patience: 12,
            steps_mm: vec![0.5, 1.0, 2.0, 4.0],
            move_seconds: 600.0,
            polish_seconds: 60.0,
            placer: KiCadBoardPlacerConfig::default(),
            pin_swaps: None,
            pin_swap: KiCadPinSwapConfig::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadBoardLayoutMove {
    pub reference: String,
    pub from: [f64; 3],
    pub to: [f64; 3],
    pub rerouted_nets: usize,
    pub seconds: f64,
    pub kept: bool,
    pub unconnected_terminals: usize,
    pub vias: usize,
    pub length_mm: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadBoardLayoutResult {
    pub board_id: String,
    pub placement_seconds: f64,
    pub first_route_seconds: f64,
    pub first_unconnected_terminals: usize,
    pub first_vias: usize,
    pub first_length_mm: f64,
    pub moves: Vec<KiCadBoardLayoutMove>,
    /// Open terminals after the first route of the kept placement and of
    /// each other seed's placement tried (only when the first left some).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub placement_race: Vec<usize>,
    pub pin_swaps: Option<KiCadPinSwapResult>,
    /// Every placement constraint with whether the final placement keeps it.
    pub constraints: Vec<KiCadConstraintStatus>,
    pub routed: KiCadBoardRouterResult,
}

/// Lower ordering is better: open connections first, then vias and copper.
fn score(result: &core::RoutingResult, board: &core::Board) -> (usize, f64) {
    let open: usize = result
        .status
        .iter()
        .map(|status| match status {
            core::NetStatus::Partial {
                unconnected_terminals,
            } => *unconnected_terminals,
            core::NetStatus::Unreachable => 1,
            _ => 0,
        })
        .sum::<usize>()
        + core::verify(board, &result.routes).len();
    let vias: usize = result.routes.iter().map(|route| route.vias.len()).sum();
    let length: f64 = result
        .routes
        .iter()
        .flat_map(|route| route.segments.iter())
        .map(|segment| distance_squared(segment.start, segment.end).sqrt())
        .sum();
    (open, length + 10.0 * vias as f64)
}

/// Mean congestion under a body plus a margin, on the routing lattice.
fn body_congestion(result: &core::RoutingResult, center: [f64; 2], half: [f64; 2], margin: f64) -> f64 {
    let grid = &result.grid;
    let range = |axis: usize, size: usize| {
        let low = center[axis] - half[axis] - margin;
        let high = center[axis] + half[axis] + margin;
        let first = ((low - grid.origin[axis]) / grid.pitch).ceil().max(0.0) as usize;
        let last = (((high - grid.origin[axis]) / grid.pitch).floor().max(0.0) as usize)
            .min(size.saturating_sub(1));
        (first, last)
    };
    let (x0, x1) = range(0, grid.nx);
    let (y0, y1) = range(1, grid.ny);
    let mut total = 0.0;
    let mut nodes = 0usize;
    for y in y0..=y1 {
        for x in x0..=x1 {
            total += result.congestion[y * grid.nx + x] as f64;
            nodes += 1;
        }
    }
    if nodes > 0 { total / nodes as f64 } else { 0.0 }
}

fn footprint_items(pcb: &mut Expr) -> Result<Vec<&mut Expr>, String> {
    let Expr::List(items) = pcb else {
        return Err("PCB root is not a list".into());
    };
    Ok(items
        .iter_mut()
        .filter(|item| item.head() == Some("footprint"))
        .collect())
}

/// Places and routes `<source>/<board_id>.kicad_pcb`. The placement goes to
/// `output/placed`, the final routed board to `output/result`.
pub fn layout_kicad_board(
    source_directory: &Path,
    board_id: &str,
    output_directory: &Path,
    config: &KiCadBoardLayoutConfig,
    router_config: &KiCadBoardRouterConfig,
) -> Result<KiCadBoardLayoutResult, String> {
    if output_directory.exists() {
        return Err(format!(
            "output directory {} already exists",
            output_directory.display()
        ));
    }
    fs::create_dir_all(output_directory)
        .map_err(|error| format!("failed to create {}: {error}", output_directory.display()))?;
    // Requested net classes set their nets' rules (and the spacing).
    let with_classes;
    let router_config = if router_config.net_classes.is_empty() {
        router_config
    } else {
        let board = source_directory.join(format!("{board_id}.kicad_pcb"));
        let text = fs::read_to_string(&board).map_err(|error| format!("failed to read {}: {error}", board.display()))?;
        with_classes = crate::net_classes::with_net_classes(router_config, &parse(&text)?)?;
        &with_classes
    };
    let mut placer_config = resolve_constraints(&config.placer, source_directory)?;
    // Bodies keep at least the largest copper clearance apart.
    if placer_config.copper_clearance_mm.is_none() {
        placer_config.copper_clearance_mm = Some(largest_clearance(router_config));
    }
    placer_config.edge_margin_mm = placer_config.edge_margin_mm.max(router_config.edge_clearance_mm);
    let placed_directory = output_directory.join("placed");
    let placement = place_kicad_board(source_directory, board_id, &placed_directory, &placer_config)?;
    if !placement.unplaced.is_empty() || !placement.illegal.is_empty() {
        return Err(format!(
            "placement is not legal (unplaced {:?}, illegal {:?}); {}",
            placement.unplaced,
            placement.illegal,
            placement.hints.join("; ")
        ));
    }

    // Pins are assigned for the placement; the placed project (board and
    // schematic) is what the result is built from.
    let pin_swaps = match &config.pin_swaps {
        Some(path) => {
            let spec = read_pin_swap_spec(&source_directory.join(path))?;
            Some(swap_kicad_pins_in_place(&placed_directory, board_id, &spec, &config.pin_swap)?)
        }
        None => None,
    };

    let placed_board = placed_directory.join(format!("{board_id}.kicad_pcb"));
    let source = fs::read_to_string(&placed_board)
        .map_err(|error| format!("failed to read {}: {error}", placed_board.display()))?;
    let mut pcb = parse(&source)?;
    // Requested pours (a ground plane) join the placed board.
    if !router_config.add_pours.is_empty() {
        add_pour_zones(&mut pcb, &router_config.add_pours)?;
        fs::write(&placed_board, format!("{}\n", encode(&pcb)))
            .map_err(|error| format!("failed to write {}: {error}", placed_board.display()))?;
    }
    if !router_config.net_classes.is_empty() {
        crate::net_classes::write_net_classes(&placed_directory, board_id, &pcb, &router_config.net_classes)?;
    }
    let layers = LayerTable::from_pcb(&pcb)?;
    let has_pours = !pours(&pcb, &layers)?.is_empty();
    let connect = has_pours && router_config.pours != KiCadPourMode::Tracks;
    // The moves that follow keep to the rules the placement needed: on a
    // board placed with relaxed spacing, a move checked against the full
    // spacing is never legal.
    placer_config.tight_bodies = Some(placement.tight_bodies);
    let mut problem = lower_placement(&pcb, &placer_config)?;
    placer::Relaxation {
        spacing: placement.spacing_mm,
        grid: placement.grid_mm,
        halo_scale: placement.halo_scale,
        edge_inset: placement.edge_inset,
        tight: placement.tight_bodies,
    }
    .apply(&mut problem.problem);
    let core_config = core_config(router_config);

    let first_started = std::time::Instant::now();
    let mut board = lower(&pcb, router_config, connect)?.board;
    // With seeds, the first route is done several ways at once; the best
    // router state carries the move phase.
    let seeds = router_config.seeds.unwrap_or(1).max(1) as u64;
    let route_once = |board: &core::Board| {
        std::thread::scope(|scope| {
            let handles: Vec<_> = (0..seeds)
                .map(|index| {
                    let mut seeded = core_config.clone();
                    seeded.seed = (index > 0).then_some(index);
                    seeded.verbose = core_config.verbose && index == 0;
                    scope.spawn(move || {
                        let mut router = core::router::Router::new(board, &seeded);
                        let result = router.run_in_place();
                        (router, result)
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| handle.join().expect("routing thread"))
                .min_by(|a, b| {
                    score(&a.1, board)
                        .partial_cmp(&score(&b.1, board))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .expect("at least one seed")
        })
    };
    let (mut router, mut result) = route_once(&board);
    let mut best = score(&result, &board);
    // Wirelength is not routability: when the placement kept for the least
    // wire leaves connections open, route the other seeds' placements once
    // and go on with the one that leaves fewer open.
    let mut race = Vec::new();
    if best.0 > 0 && pin_swaps.is_none() {
        race.push(best.0);
        for (poses, relaxation) in &placement.alternatives {
            let mut candidate = pcb.clone();
            {
                let mut footprints = footprint_items(&mut candidate)?;
                for (footprint, pose) in footprints.iter_mut().zip(poses) {
                    write_footprint_pose(footprint, *pose)?;
                }
            }
            let candidate_board = lower(&candidate, router_config, connect)?.board;
            let (candidate_router, candidate_result) = route_once(&candidate_board);
            let candidate_score = score(&candidate_result, &candidate_board);
            race.push(candidate_score.0);
            eprintln!("placement race: another seed's placement leaves {} open (best {})", candidate_score.0, best.0);
            if candidate_score.0 < best.0 {
                pcb = candidate;
                board = candidate_board;
                router = candidate_router;
                result = candidate_result;
                best = candidate_score;
                placer_config.tight_bodies = Some(relaxation.tight);
                problem = lower_placement(&pcb, &placer_config)?;
                relaxation.apply(&mut problem.problem);
            }
            if best.0 == 0 {
                break;
            }
        }
    }
    let first_route_seconds = first_started.elapsed().as_secs_f64();
    let first = (
        best.0,
        result.routes.iter().map(|route| route.vias.len()).sum::<usize>(),
        result
            .routes
            .iter()
            .flat_map(|route| route.segments.iter())
            .map(|segment| distance_squared(segment.start, segment.end).sqrt())
            .sum::<f64>(),
    );

    let move_started = std::time::Instant::now();
    let mut moves = Vec::new();
    let mut since_improvement = 0;
    let mut recently: Vec<usize> = Vec::new();
    let movable: Vec<usize> = (0..problem.problem.components.len())
        .filter(|index| !problem.problem.components[*index].fixed && !problem.problem.components[*index].pins.is_empty())
        .collect();
    for _ in 0..config.moves {
        if best.0 == 0 && since_improvement >= config.patience {
            break;
        }
        let budget = if best.0 == 0 {
            config.move_seconds.min(config.polish_seconds)
        } else {
            config.move_seconds
        };
        if since_improvement >= 2 * config.patience || move_started.elapsed().as_secs_f64() > budget {
            break;
        }
        // The most congested movable footprint that was not tried lately.
        let mut ranked: Vec<(f64, usize)> = movable
            .iter()
            .filter(|index| !recently.contains(index))
            .map(|index| {
                let component = &problem.problem.components[*index];
                let pose = problem.problem.poses[*index];
                (
                    body_congestion(&result, component.center(pose), component.half_extent(pose.angle), 2.0),
                    *index,
                )
            })
            .collect();
        ranked.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        let Some(&(congestion, index)) = ranked.first() else {
            break;
        };
        if congestion <= 0.0 && best.0 == 0 {
            break;
        }
        recently.push(index);
        if recently.len() > movable.len() / 3 + 1 {
            recently.remove(0);
        }
        let component = &problem.problem.components[index];
        let current = problem.problem.poses[index];

        // Candidate poses: steps along the axes and quarter turns, legal
        // against the placement, ranked by the congestion they land in.
        let mut candidates: Vec<(f64, placer::Pose)> = Vec::new();
        for angle in component.angle_options.iter().copied() {
            for step in config.steps_mm.iter().copied() {
                for (dx, dy) in [(step, 0.0), (-step, 0.0), (0.0, step), (0.0, -step)] {
                    let center = component.center(current);
                    let wanted = [center[0] + dx, center[1] + dy];
                    let position = component.position_for_center(wanted, angle);
                    let pose = placer::Pose {
                        position: placer::constraints::snap_position(&problem.problem, index, position, angle),
                        angle,
                    };
                    if pose == current {
                        continue;
                    }
                    if !placer::legal::is_legal(
                        &problem.problem,
                        &problem.problem.poses,
                        index,
                        pose,
                        0..problem.problem.components.len(),
                    ) {
                        continue;
                    }
                    // A nudge must not break a near or relative constraint
                    // further.
                    if !problem.problem.constraints.relations.is_empty() {
                        let before = placer::constraints::relation_penalty(
                            &problem.problem,
                            &problem.problem.poses,
                            Some(index),
                        );
                        let mut trial = problem.problem.poses.clone();
                        trial[index] = pose;
                        if placer::constraints::relation_penalty(&problem.problem, &trial, Some(index))
                            > before + 1.0e-9
                        {
                            continue;
                        }
                    }
                    let landing = body_congestion(
                        &result,
                        component.center(pose),
                        component.half_extent(pose.angle),
                        2.0,
                    );
                    candidates.push((landing + 0.01 * (dx.abs() + dy.abs()), pose));
                }
            }
        }
        candidates.sort_by(|a, b| a.0.total_cmp(&b.0));
        candidates.truncate(3);
        if candidates.is_empty() {
            since_improvement += 1;
            continue;
        }

        let mut improved = false;
        for (_, pose) in candidates {
            let trial_started = std::time::Instant::now();
            let saved_router = router.clone();
            let saved_pcb = pcb.clone();
            let saved_pose = problem.problem.poses[index];
            let saved_poses = problem.problem.poses.clone();
            // The part moves, and the parts its row carries with it.
            let mut poses = saved_poses.clone();
            poses[index] = pose;
            problem.problem.sync_followers(&mut poses);
            {
                let mut footprints = footprint_items(&mut pcb)?;
                for (part, (old, new)) in saved_poses.iter().zip(&poses).enumerate() {
                    if part == index || old != new {
                        write_footprint_pose(footprints[part], *new)?;
                    }
                }
            }
            problem.problem.poses = poses;
            let trial_board = lower(&pcb, router_config, connect)?.board;
            let rerouted = match router.update(&trial_board) {
                Ok(count) => count,
                Err(error) => {
                    router = saved_router;
                    pcb = saved_pcb;
                    problem.problem.poses = saved_poses.clone();
                    return Err(error);
                }
            };
            let trial = router.reroute(true);
            let trial_score = score(&trial, &trial_board);
            let kept = trial_score < best;
            moves.push(KiCadBoardLayoutMove {
                reference: problem.references[index].clone(),
                from: [saved_pose.position[0], saved_pose.position[1], saved_pose.angle],
                to: [pose.position[0], pose.position[1], pose.angle],
                rerouted_nets: rerouted,
                seconds: trial_started.elapsed().as_secs_f64(),
                kept,
                unconnected_terminals: trial_score.0,
                vias: trial.routes.iter().map(|route| route.vias.len()).sum(),
                length_mm: trial
                    .routes
                    .iter()
                    .flat_map(|route| route.segments.iter())
                    .map(|segment| distance_squared(segment.start, segment.end).sqrt())
                    .sum(),
            });
            if kept {
                best = trial_score;
                result = trial;
                board = trial_board;
                improved = true;
                break;
            }
            router = saved_router;
            pcb = saved_pcb;
            problem.problem.poses = saved_poses.clone();
        }
        if improved {
            since_improvement = 0;
        } else {
            since_improvement += 1;
        }
    }

    let layer_names = layers.names.clone();
    let nets = emit_routes(&mut pcb, &board, &result, &layer_names)?;
    let result_directory = output_directory.join("result");
    let mut routed = finish_routed_board(
        &pcb,
        &board,
        &result,
        nets,
        &layer_names,
        &placed_directory,
        board_id,
        &result_directory,
        router_config,
        [0.0, first_route_seconds + move_started.elapsed().as_secs_f64()],
    )?;
    routed.pours = if !has_pours {
        "none"
    } else if connect {
        "connect"
    } else {
        "tracks"
    }
    .into();
    // If something is still open (in KiCad's eyes too), route the final
    // placement with route mode's whole ladder (pours as tracks, finer
    // pitches, more seeds) and keep the better board. If only thermals are
    // starved (pads KiCad does not count as joined to their pour), route
    // the pour nets as tracks.
    let open = |result: &KiCadBoardRouterResult| {
        result.unconnected_terminals
            + result.internal_violations.len()
            + result
                .native
                .as_ref()
                .map_or(0, |native| native.selected_net_unconnected_items)
    };
    let quality = |result: &KiCadBoardRouterResult, directory: &Path| {
        (open(result), crate::board_router::starved_thermals(directory))
    };
    let current = quality(&routed, &result_directory);
    let fallback_config = if current.0 > 0 {
        Some(router_config.clone())
    } else if current.1 > 0 && connect && router_config.pours == KiCadPourMode::Auto {
        let mut tracks = router_config.clone();
        tracks.pours = KiCadPourMode::Tracks;
        Some(tracks)
    } else {
        None
    };
    if let Some(fallback_config) = fallback_config {
        let fallback_directory = output_directory.join("result-ladder");
        // The placement file already carries the final poses.
        let placed_pcb = {
            let mut copy = pcb.clone();
            if let Expr::List(items) = &mut copy {
                items.retain(|item| !matches!(item.head(), Some("segment" | "arc" | "via")));
            }
            copy
        };
        fs::write(&placed_board, format!("{}\n", encode(&placed_pcb)))
            .map_err(|error| format!("failed to write {}: {error}", placed_board.display()))?;
        let fallback = route_kicad_board(&placed_directory, board_id, &fallback_directory, &fallback_config)?;
        if quality(&fallback, &fallback_directory) < current {
            fs::remove_dir_all(&result_directory).map_err(|error| error.to_string())?;
            fs::rename(&fallback_directory, &result_directory).map_err(|error| error.to_string())?;
            routed = fallback;
        } else {
            fs::remove_dir_all(&fallback_directory).map_err(|error| error.to_string())?;
        }
    }
    // The placement file should show the final poses too.
    {
        let mut placed_pcb = pcb.clone();
        if let Expr::List(items) = &mut placed_pcb {
            items.retain(|item| !matches!(item.head(), Some("segment" | "arc" | "via")));
        }
        fs::write(&placed_board, format!("{}\n", encode(&placed_pcb)))
            .map_err(|error| format!("failed to write {}: {error}", placed_board.display()))?;
    }
    let layout = KiCadBoardLayoutResult {
        board_id: board_id.into(),
        placement_seconds: placement.seconds,
        first_route_seconds,
        first_unconnected_terminals: first.0,
        first_vias: first.1,
        first_length_mm: first.2,
        moves,
        placement_race: race,
        pin_swaps,
        constraints: constraint_report(&problem.problem, &problem.problem.poses, &problem.references),
        routed,
    };
    let report_path = output_directory.join("board-layout.json");
    fs::write(
        &report_path,
        serde_json::to_string_pretty(&layout).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("failed to write {}: {error}", report_path.display()))?;
    Ok(layout)
}
