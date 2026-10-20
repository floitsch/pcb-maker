// Copyright (C) 2026 Toit contributors.

//! Diagnostic commands on a lowered board (pour maps, search benchmarks)
//! and a board without its tracks.

use super::geometry::polygon_area;
use super::*;
use pcb_router as core;

/// Lowers the board as it is, its tracks and vias kept as fixed copper,
/// with the pours connected, and writes the router's pour map before any
/// routing (`Router::dump_pours`) into `output_directory`, with
/// `layers.json` naming the layers. For comparing the pour model with
/// KiCad's fill on the same copper (experiments/pour-fidelity).
pub fn dump_kicad_pour_map(
    source_directory: &Path,
    board_id: &str,
    output_directory: &Path,
    config: &KiCadBoardRouterConfig,
) -> Result<(), String> {
    let source_board = source_directory.join(format!("{board_id}.kicad_pcb"));
    let source = fs::read_to_string(&source_board)
        .map_err(|error| format!("failed to read {}: {error}", source_board.display()))?;
    let pcb = crate::canonical_order::parse_board(&source)?;
    let Lowered { board } = lower(&pcb, config, true)?;
    let layer_names = LayerTable::from_pcb(&pcb)?.names;
    let mut router = core::router::Router::new(&board, &core_config(config));
    router.adopt_existing_copper();
    if std::env::var_os("PCB_ROUTER_DUMP_MANUAL_SPOKES").is_some() {
        eprintln!("manual spokes: {} added", router.add_manual_spokes());
    }
    router
        .dump_pours(output_directory)
        .map_err(|error| error.to_string())?;
    let names: Vec<String> = layer_names
        .iter()
        .map(|name| format!("\"{name}\""))
        .collect();
    fs::write(
        output_directory.join("layers.json"),
        format!("[{}]\n", names.join(", ")),
    )
    .map_err(|error| error.to_string())
}

/// The search benchmark: the board lowered with its pours connected, a
/// router built, and `iterations` negotiation iterations run with an
/// unbounded work budget (the run stops at the iteration count). Returns
/// the work done and a fingerprint of the routes, which must not change
/// when only the speed of the search does.
pub fn bench_kicad_search(
    source_directory: &Path,
    board_id: &str,
    iterations: usize,
    config: &KiCadBoardRouterConfig,
) -> Result<String, String> {
    let source_board = source_directory.join(format!("{board_id}.kicad_pcb"));
    let source = fs::read_to_string(&source_board)
        .map_err(|error| format!("failed to read {}: {error}", source_board.display()))?;
    let pcb = crate::canonical_order::parse_board(&source)?;
    let Lowered { board } = lower(&pcb, config, true)?;
    let mut router_config = core_config(config);
    router_config.max_iterations = iterations;
    let mut router = core::router::Router::new(&board, &router_config);
    let started = std::time::Instant::now();
    let unfinished = router.probe(1.0e7);
    let seconds = started.elapsed().as_secs_f64();
    let snapshot = router.snapshot();
    let vias: usize = snapshot.routes.iter().map(|route| route.vias.len()).sum();
    let length: f64 = snapshot
        .routes
        .iter()
        .flat_map(|route| route.segments.iter())
        .map(|segment| distance_squared(segment.start, segment.end).sqrt())
        .sum();
    Ok(format!(
        "bench: {} expansions, {unfinished} unfinished, {vias} vias, {length:.4} mm, {seconds:.1} s",
        router.expansions()
    ))
}

/// Copies `source` to `destination` without tracks, vias and stale pour
/// fills. Unlike the cold-board strip, copper pours themselves are kept:
/// where copper is poured is a design decision like the placement, and the
/// router connects to pours instead of replacing them by tracks.
pub fn write_kicad_board_without_tracks(source: &Path, destination: &Path) -> Result<(), String> {
    let text = fs::read_to_string(source)
        .map_err(|error| format!("failed to read {}: {error}", source.display()))?;
    let mut pcb = parse(&text)?;
    let board_area = polygon_area(&outline::board_loops(&pcb)?.outline);
    let Expr::List(items) = &mut pcb else {
        return Err("PCB root is not a list".into());
    };
    items.retain(|item| !matches!(item.head(), Some("segment" | "arc" | "via")));
    // Small filled zones are hand-drawn copper, i.e. routing; only pours
    // covering a good part of the board are kept.
    items.retain(|item| {
        item.head() != Some("zone")
            || is_rule_area(item)
            || rule_area_polygon_points(item)
                .is_ok_and(|points| polygon_area(&points) >= POUR_SHARE * board_area)
    });
    for zone in items.iter_mut().filter(|item| item.head() == Some("zone")) {
        if let Expr::List(children) = zone {
            children
                .retain(|child| !matches!(child.head(), Some("filled_polygon" | "fill_segments")));
        }
    }
    // KiCad refuses a board with items on undefined layers; the stripped
    // board loads like the routed one will (koeg-board's Rescue layer).
    move_undefined_layers(&mut pcb);
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    }
    fs::write(destination, format!("{}\n", encode(&pcb)))
        .map_err(|error| format!("failed to write {}: {error}", destination.display()))
}
