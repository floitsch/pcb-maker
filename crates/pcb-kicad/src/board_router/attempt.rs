// Copyright (C) 2026 Toit contributors.

//! One routing attempt: its run, its board written and checked.

use super::*;
use pcb_router as core;

/// Writes a routing result as copper into `pcb` and reports per net.
/// `keep_existing`: the board's own tracks and vias stay as they are
/// (route mode, `kept`); otherwise they are replaced.
pub(crate) fn emit_routes(
    pcb: &mut Expr,
    board: &core::Board,
    result: &core::RoutingResult,
    layer_names: &[String],
    keep_existing: bool,
) -> Result<Vec<KiCadBoardRouterNet>, String> {
    let copper_net_names = unambiguous_pad_net_names(pcb)?;
    let Expr::List(items) = pcb else {
        return Err("PCB root is not a list".into());
    };
    if !keep_existing {
        items.retain(|item| !matches!(item.head(), Some("segment" | "arc" | "via")));
    }
    let mut nets = Vec::new();
    for (net, route) in result.routes.iter().enumerate() {
        let description = &board.nets[net];
        for segment in &route.segments {
            items.push(supplemental_segment_expr(&SupplementalSegment {
                connection: description.name.clone(),
                start: nanometres(segment.start),
                end: nanometres(segment.end),
                width: segment.width,
                layer: layer_names[segment.layer].clone(),
            }));
        }
        for via in &route.vias {
            items.push(supplemental_via_expr(&SupplementalVia {
                connection: description.name.clone(),
                at: nanometres(via.at),
                size: via.diameter,
                drill: via.drill,
                layers: ["F.Cu".into(), "B.Cu".into()],
            }));
        }
        let (status, unconnected_terminals) = match result.status[net] {
            core::NetStatus::Trivial => ("trivial", 0),
            core::NetStatus::Routed => ("routed", 0),
            core::NetStatus::Partial {
                unconnected_terminals,
            } => ("partial", unconnected_terminals),
            core::NetStatus::Unreachable => {
                ("unreachable", description.terminals.len().saturating_sub(1))
            }
        };
        nets.push(KiCadBoardRouterNet {
            connection: description.name.clone(),
            terminals: description.terminals.len(),
            status: status.into(),
            unconnected_terminals,
            vias: route.vias.len(),
            length_mm: route
                .segments
                .iter()
                .map(|segment| distance_squared(segment.start, segment.end).sqrt())
                .sum(),
        });
    }
    canonicalize_copper_net_names(pcb, &copper_net_names);
    Ok(nets)
}

/// A fresh numbered subdirectory of `root` for one routing attempt's frames.
fn attempt_frame_directory(root: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(root)
        .map_err(|error| format!("failed to create {}: {error}", root.display()))?;
    let attempts = fs::read_dir(root)
        .map_err(|error| format!("failed to read {}: {error}", root.display()))?
        .count();
    let directory = root.join(format!("attempt-{attempts:02}"));
    fs::create_dir_all(&directory)
        .map_err(|error| format!("failed to create {}: {error}", directory.display()))?;
    Ok(directory)
}

pub(super) fn route_kicad_board_once(
    source_directory: &Path,
    board_id: &str,
    output_directory: &Path,
    config: &KiCadBoardRouterConfig,
    connect_pours: bool,
) -> Result<KiCadBoardRouterResult, String> {
    route_kicad_board_lowered(
        source_directory,
        board_id,
        output_directory,
        config,
        connect_pours,
        None,
    )
}

/// `route_kicad_board_once`, on `lowered` (the board as `lower` gives it
/// for `config` and `connect_pours`, with its lowering's work) when the
/// caller lowered it already: the ladder lowers a rung's board for its
/// hold and routes that board (one lowering an attempt).
pub(super) fn route_kicad_board_lowered(
    source_directory: &Path,
    board_id: &str,
    output_directory: &Path,
    config: &KiCadBoardRouterConfig,
    connect_pours: bool,
    lowered: Option<(core::Board, f64)>,
) -> Result<KiCadBoardRouterResult, String> {
    let started = std::time::Instant::now();
    let source_board = source_directory.join(format!("{board_id}.kicad_pcb"));
    let source = fs::read_to_string(&source_board)
        .map_err(|error| format!("failed to read {}: {error}", source_board.display()))?;
    let pcb = crate::canonical_order::parse_board(&source)?;
    // The board's own tracks and vias: what they join is connected (a
    // board the ladder lowered had them kept there: the runner's `kept`
    // reports them).
    let (board, lowering_work, kept) = match lowered {
        Some((board, work)) => (board, work, KiCadKeptCopper::default()),
        None => {
            let Lowered {
                mut board,
                expressions,
            } = lower(&pcb, config, connect_pours)?;
            let kept = keep_existing_copper(&mut board, false);
            if config.diff_pairs_on() && config.engine.as_deref() != Some("topological") {
                plan_pairs(&mut board, config);
            }
            (board, lowering_work_of(expressions), kept)
        }
    };
    let layer_names = LayerTable::from_pcb(&pcb)?.names;
    let lowering_seconds = started.elapsed().as_secs_f64();
    let routing_started = std::time::Instant::now();
    // The router's lattice maps, for sizing routers side by side.
    let mut maps_mb = 0.0;
    let result = match &config.frame_directory {
        _ if config.engine.as_deref() == Some("topological") => {
            let mut topological = pcb_topo::Config {
                iterations: config.topological_rounds.unwrap_or(30),
                seed: config.first_seed.unwrap_or(1).max(1),
                verbose: true,
                ..pcb_topo::Config::default()
            };
            config.topological.apply(&mut topological);
            pcb_topo::route(&board, &topological)
        }
        None if config.seeds.unwrap_or(1) > 1 || config.first_seed.unwrap_or(0) > 0 => {
            let (result, maps) = route_seeds(
                &board,
                &core_config(config),
                config.first_seed.unwrap_or(0),
                config.seeds.unwrap_or(1),
                config.seeds_side_by_side.unwrap_or(usize::MAX),
            );
            maps_mb = maps;
            result
        }
        None => {
            let (result, maps) = route_measured(&board, &core_config(config));
            maps_mb = maps;
            result
        }
        Some(root) => {
            let directory = attempt_frame_directory(root)?;
            let mut router = core::router::Router::new(&board, &core_config(config));
            let frame_pcb = pcb.clone();
            let frame_board = board.clone();
            let frame_layers = layer_names.clone();
            router.set_frame_hook(std::sync::Arc::new(move |iteration, snapshot| {
                let mut frame = frame_pcb.clone();
                let written = emit_routes(&mut frame, &frame_board, snapshot, &frame_layers, true)
                    .and_then(|_| {
                        let path = directory.join(format!("frame-{iteration:04}.kicad_pcb"));
                        fs::write(&path, format!("{}\n", encode(&frame)))
                            .map_err(|error| format!("failed to write {}: {error}", path.display()))
                    });
                if let Err(error) = written {
                    eprintln!("frame {iteration}: {error}");
                }
            }));
            router.run()
        }
    };
    let routing_seconds = routing_started.elapsed().as_secs_f64();
    let mut written = write_attempt(
        pcb,
        &board,
        result,
        &layer_names,
        source_directory,
        board_id,
        output_directory,
        config,
        [lowering_seconds, routing_seconds],
    )?;
    written.maps_mb = maps_mb;
    written.lowering_work_seconds = lowering_work;
    if !kept.nets.is_empty() {
        written.kept_copper = Some(kept);
    }
    Ok(written)
}

/// Routes `board` as `core::route` does (`Router::new` and `run`), with
/// the router's lattice maps in megabytes (its memory report; 0 when the
/// report does not say).
pub(super) fn route_measured(
    board: &core::Board,
    config: &core::Config,
) -> (core::RoutingResult, f64) {
    let router = core::router::Router::new(board, config);
    let maps_mb = maps_mb_of(&router.memory_report()).unwrap_or(0.0);
    (router.run(), maps_mb)
}

/// Moves every `(layer "X")` that names a layer the board's layer table
/// does not define to `Cmts.User`. Some boards carry such items (a
/// `Rescue` layer from an old KiCad, a user layer removed later); KiCad's
/// editor asks what to do with them and kicad-cli refuses the board.
/// Returns how many were moved.
pub(super) fn move_undefined_layers(pcb: &mut Expr) -> usize {
    let defined: std::collections::HashSet<String> = pcb
        .child("layers")
        .map(|layers| {
            layers
                .children()
                .iter()
                .skip(1)
                .flat_map(|entry| {
                    entry
                        .children()
                        .iter()
                        .skip(1)
                        .take(3)
                        .filter_map(Expr::atom)
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default();
    if defined.is_empty() {
        return 0;
    }
    fn walk(node: &mut Expr, defined: &std::collections::HashSet<String>, top: bool) -> usize {
        let Expr::List(items) = node else {
            return 0;
        };
        // The layer table itself and the stackup name layers by design.
        if !top
            && matches!(
                items.first().and_then(Expr::atom),
                Some("layers" | "stackup")
            )
        {
            return 0;
        }
        let mut moved = 0;
        if items.len() == 2 && items[0].atom() == Some("layer") {
            if let Some(name) = items[1].atom()
                && !defined.contains(name)
                && !name.contains('*')
                && !name.contains('&')
            {
                items[1] = Expr::Atom("\"Cmts.User\"".into());
                moved += 1;
            }
            return moved;
        }
        for item in items.iter_mut() {
            moved += walk(item, defined, false);
        }
        moved
    }
    walk(pcb, &defined, true)
}

/// Writes a routed board: the optional tightening, the copper into the
/// board file, and the checks (`finish_routed_board`).
#[allow(clippy::too_many_arguments)]
pub(super) fn write_attempt(
    mut pcb: Expr,
    board: &core::Board,
    result: core::RoutingResult,
    layer_names: &[String],
    source_directory: &Path,
    board_id: &str,
    output_directory: &Path,
    config: &KiCadBoardRouterConfig,
    seconds: [f64; 2],
) -> Result<KiCadBoardRouterResult, String> {
    crate::mem_profile::phase("write-back");
    let [lowering_seconds, mut routing_seconds] = seconds;
    // Read before the items on undefined layers move away.
    let warnings = input_warnings(&pcb);
    let moved = move_undefined_layers(&mut pcb);
    if moved > 0 {
        eprintln!(
            "{moved} items on layers the board does not define moved to Cmts.User (KiCad would not load the board)"
        );
    }
    let mut result = result;
    if config.tighten && config.engine.as_deref() != Some("topological") {
        let tightening = std::time::Instant::now();
        let (routes, report) = pcb_topo::tighten::tighten(board, &result.routes);
        eprintln!(
            "tighten: {} of {} pieces pulled tight, {:.1} mm -> {:.1} mm, {:.2}s",
            report.tightened,
            report.pieces,
            report.length_before,
            report.length_after,
            tightening.elapsed().as_secs_f64()
        );
        result.routes = routes;
        routing_seconds += tightening.elapsed().as_secs_f64();
    }
    // Differential pairs: the links' routes into the pairs' copper (written
    // first, so that the nets' names are made canonical with the rest), the
    // board as lowered.
    let heights = copper_heights(&pcb);
    let heights: Vec<f64> = layer_names
        .iter()
        .map(|name| heights.get(name).copied().unwrap_or(0.0))
        .collect();
    let paired = finish_pairs(board, &mut result, config, &heights);
    let board = paired.as_ref().map_or(board, |(restored, _, _)| restored);
    if let Some((_, copper, _)) = &paired {
        write_pair_copper(&mut pcb, copper, layer_names)?;
    }
    let nets = emit_routes(&mut pcb, board, &result, layer_names, true)?;
    let mut report = finish_routed_board(
        &pcb,
        board,
        &result,
        nets,
        layer_names,
        source_directory,
        board_id,
        output_directory,
        config,
        [lowering_seconds, routing_seconds],
    )?;
    report.warnings = warnings;
    report.diff_pairs = paired.map(|(_, _, pairs)| pairs);
    Ok(report)
}
