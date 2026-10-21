// Copyright (C) 2026 Toit contributors.

//! The coarse global route of a placed board (`pcb-global`,
//! docs/global-route.md) as a routability measure: the board lowered as
//! the layout routes a placement (tracks stripped, planned pours added,
//! copper texts removed), routed on tiles, and scored. Beside it the
//! race's RUDY score of the same lowering and, on request, the race's
//! router probe, so the three measures can be compared on one placement.

use super::*;
use crate::board_layout::without_copper_texts;
use crate::board_router::{KiCadPourMode, LayerTable, add_pour_zones, lower, planned_pours, pours};
use pcb_router as core;

#[derive(Clone, Debug, Serialize)]
pub struct KiCadGlobalRouteReport {
    pub board_id: String,
    pub layers: usize,
    pub tiles: [usize; 2],
    pub tile_mm: f64,
    /// Nets of the lowered board, and terminals.
    pub board_nets: usize,
    pub terminals: usize,
    pub score: pcb_global::Score,
    /// The placement race's RUDY score of the same board
    /// (`pcb_congestion::rudy_overflow` at 0.25).
    pub rudy: f64,
    pub lower_seconds: f64,
    pub capacity_seconds: f64,
    /// The global route, capacities included.
    pub global_seconds: f64,
    pub rudy_seconds: f64,
    /// The race's probe (`Router::probe`), when asked for.
    pub probe: Option<KiCadProbeReport>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KiCadProbeReport {
    pub work_seconds: f64,
    /// Nets conflicted or incomplete after the probe (the race's measure).
    pub unfinished: usize,
    pub seconds: f64,
}

/// The board as the layout routes a placement.
pub(crate) fn lower_placed(
    pcb: &Expr,
    router_config: &KiCadBoardRouterConfig,
) -> Result<core::Board, String> {
    let Expr::List(items) = pcb else {
        return Err("PCB root is not a list".into());
    };
    let mut routed = Expr::List(
        items
            .iter()
            .filter(|item| !matches!(item.head(), Some("segment" | "arc" | "via")))
            .cloned()
            .collect(),
    );
    let planned = planned_pours(&routed, router_config)?;
    add_pour_zones(&mut routed, &planned)?;
    let layers = LayerTable::from_pcb(&routed)?;
    let connect =
        !pours(&routed, &layers)?.is_empty() && router_config.pours != KiCadPourMode::Tracks;
    // Copper texts stay, as the layout keeps them by default
    // (`keep_board_marks`).
    Ok(lower(&without_copper_texts(&routed, true), router_config, connect)?.board)
}

/// Routes a placed board globally (and, with `probe_seconds`, probes it
/// as the placement race does) and reports the scores.
pub fn global_route_kicad_board(
    source_directory: &Path,
    board_id: &str,
    router_config: &KiCadBoardRouterConfig,
    config: &pcb_global::GlobalConfig,
    probe_seconds: Option<f64>,
    corridors: Option<&Path>,
) -> Result<KiCadGlobalRouteReport, String> {
    let board_path = source_directory.join(format!("{board_id}.kicad_pcb"));
    let text = fs::read_to_string(&board_path)
        .map_err(|error| format!("failed to read {}: {error}", board_path.display()))?;
    let pcb = crate::canonical_order::parse_board(&text)?;
    let lowered_at = Instant::now();
    let board = lower_placed(&pcb, router_config)?;
    let lower_seconds = lowered_at.elapsed().as_secs_f64();
    let route = pcb_global::route(&board, config);
    let core_config = crate::congestion_samples::race_core_config(router_config);
    let rudy_at = Instant::now();
    let frame = core::router::congestion::tile_frame(&board, &core_config);
    let rudy =
        pcb_congestion::rudy_overflow(&pcb_congestion::rasterize(&board, &frame), 0.25) as f64;
    let rudy_seconds = rudy_at.elapsed().as_secs_f64();
    let probe = probe_seconds.map(|work_seconds| {
        let started = Instant::now();
        let mut core_config = core_config.clone();
        core_config.verbose = false;
        let mut router = core::router::Router::new(&board, &core_config);
        let unfinished = router.probe(work_seconds);
        KiCadProbeReport {
            work_seconds,
            unfinished,
            seconds: started.elapsed().as_secs_f64(),
        }
    });
    if let Some(path) = corridors {
        let tiles = route.frame.tiles();
        let nets: Vec<serde_json::Value> = route
            .nets
            .iter()
            .map(|net| {
                serde_json::json!({
                    "net": board.nets[net.net].name,
                    "routed": net.routed,
                    "pour": net.pour,
                    "tiles": route.corridor(net.net),
                    "vias": net.vias,
                    "crossings": net.crossings.len(),
                })
            })
            .collect();
        let overflow = route.overflow();
        let per_layer: Vec<Vec<f32>> = (0..route.frame.layers)
            .map(|layer| {
                (0..tiles)
                    .map(|tile| {
                        overflow[0][layer * tiles + tile] + overflow[1][layer * tiles + tile]
                    })
                    .collect()
            })
            .collect();
        let value = serde_json::json!({
            "frame": {
                "origin": route.frame.origin, "tile_mm": route.frame.tile_mm,
                "tiles_x": route.frame.tiles_x, "tiles_y": route.frame.tiles_y,
                "layers": route.frame.layers,
            },
            "overflow": per_layer,
            "nets": nets,
        });
        fs::write(path, value.to_string())
            .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
    }
    Ok(KiCadGlobalRouteReport {
        board_id: board_id.to_string(),
        layers: board.layer_count,
        tiles: [route.frame.tiles_x, route.frame.tiles_y],
        tile_mm: route.frame.tile_mm,
        board_nets: board.nets.len(),
        terminals: board.nets.iter().map(|net| net.terminals.len()).sum(),
        score: route.score.clone(),
        rudy,
        lower_seconds,
        capacity_seconds: route.capacity_seconds,
        global_seconds: route.seconds,
        rudy_seconds,
        probe,
    })
}
