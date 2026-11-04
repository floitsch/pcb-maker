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
    let Lowered { board, .. } = lower(&pcb, config, true)?;
    let layer_names = LayerTable::from_pcb(&pcb)?.names;
    // The pour model as the measurement takes it (`measured_thermals`).
    let mut router_config = core_config(config);
    router_config.measured_thermals = true;
    let mut router = core::router::Router::new(&board, &router_config);
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
    let Lowered { board, .. } = lower(&pcb, config, true)?;
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
    write_kicad_board_without_tracks_except(source, destination, &[]).map(|_| ())
}

/// `write_kicad_board_without_tracks`, keeping the tracks, arcs and vias of
/// the nets `keep_nets` names (KiCad wildcards, `*` and `?`; a net's
/// sheet path's leading `/` may be left out), so that route mode keeps
/// them and routes the rest: a designer's length-matched bus (fresh-agent
/// trial 6's firestarter: route mode left 6 of the ten DVP nets under
/// their 25 mm minimum, the designer's own meanders meet it). Returns the
/// tracks, arcs and vias kept and removed.
pub fn write_kicad_board_without_tracks_except(
    source: &Path,
    destination: &Path,
    keep_nets: &[String],
) -> Result<[usize; 2], String> {
    let text = fs::read_to_string(source)
        .map_err(|error| format!("failed to read {}: {error}", source.display()))?;
    let mut pcb = parse(&text)?;
    let board_area = polygon_area(&outline::board_loops(&pcb)?.outline);
    // Nets by number, for boards before KiCad 10 (`(net 3)`).
    let numbered: BTreeMap<String, String> = pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("net"))
        .filter_map(|net| {
            let children = net.children();
            Some((
                children.get(1)?.atom()?.to_string(),
                children.get(2)?.atom()?.to_string(),
            ))
        })
        .collect();
    let kept = |item: &Expr| {
        let Some(net) = item
            .child("net")
            .and_then(|net| net.children().get(1).and_then(Expr::atom))
        else {
            return false;
        };
        let name = numbered.get(net).map_or(net, String::as_str);
        keep_nets.iter().any(|pattern| {
            crate::board_placer::glob_matches(pattern, name)
                || name
                    .strip_prefix('/')
                    .is_some_and(|bare| crate::board_placer::glob_matches(pattern, bare))
        })
    };
    let Expr::List(items) = &mut pcb else {
        return Err("PCB root is not a list".into());
    };
    let before = items.len();
    let mut counts = [0, 0];
    items.retain(|item| {
        if !matches!(item.head(), Some("segment" | "arc" | "via")) {
            return true;
        }
        let keep = kept(item);
        counts[usize::from(!keep)] += 1;
        keep
    });
    debug_assert_eq!(before - items.len(), counts[1]);
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
        .map_err(|error| format!("failed to write {}: {error}", destination.display()))?;
    Ok(counts)
}
