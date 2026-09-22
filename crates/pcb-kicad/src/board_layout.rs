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
use crate::board_placer::{lower_placement, write_footprint_pose};
use crate::board_router::{LayerTable, core_config, emit_routes, finish_routed_board, lower, pours};
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
    pub placer: KiCadBoardPlacerConfig,
}

impl Default for KiCadBoardLayoutConfig {
    fn default() -> Self {
        Self {
            moves: 40,
            patience: 12,
            steps_mm: vec![0.5, 1.0, 2.0, 4.0],
            move_seconds: 600.0,
            placer: KiCadBoardPlacerConfig::default(),
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
    let mut placer_config = config.placer.clone();
    placer_config.edge_margin_mm = placer_config.edge_margin_mm.max(router_config.edge_clearance_mm);
    let placed_directory = output_directory.join("placed");
    let placement = place_kicad_board(source_directory, board_id, &placed_directory, &placer_config)?;
    if !placement.unplaced.is_empty() || !placement.illegal.is_empty() {
        return Err(format!(
            "placement is not legal (unplaced {:?}, illegal {:?})",
            placement.unplaced, placement.illegal
        ));
    }

    let placed_board = placed_directory.join(format!("{board_id}.kicad_pcb"));
    let source = fs::read_to_string(&placed_board)
        .map_err(|error| format!("failed to read {}: {error}", placed_board.display()))?;
    let mut pcb = parse(&source)?;
    let layers = LayerTable::from_pcb(&pcb)?;
    let has_pours = !pours(&pcb, &layers)?.is_empty();
    let connect = has_pours && router_config.pours != KiCadPourMode::Tracks;
    let mut problem = lower_placement(&pcb, &placer_config)?;
    let core_config = core_config(router_config);

    let first_started = std::time::Instant::now();
    let mut board = lower(&pcb, router_config, connect)?.board;
    let mut router = core::router::Router::new(&board, &core_config);
    let mut result = router.run_in_place();
    let first_route_seconds = first_started.elapsed().as_secs_f64();
    let mut best = score(&result, &board);
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
        if since_improvement >= 2 * config.patience
            || move_started.elapsed().as_secs_f64() > config.move_seconds
        {
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
        let grid = problem.problem.grid.max(0.05);
        let snap = |value: f64| (value / grid).round() * grid;

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
                        position: [snap(position[0]), snap(position[1])],
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
            {
                let mut footprints = footprint_items(&mut pcb)?;
                write_footprint_pose(footprints[index], pose)?;
            }
            problem.problem.poses[index] = pose;
            let trial_board = lower(&pcb, router_config, connect)?.board;
            let rerouted = match router.update(&trial_board) {
                Ok(count) => count,
                Err(error) => {
                    router = saved_router;
                    pcb = saved_pcb;
                    problem.problem.poses[index] = saved_pose;
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
            problem.problem.poses[index] = saved_pose;
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
