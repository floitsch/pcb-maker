// Copyright (C) 2026 Toit contributors.

//! The global route inside the placer's search (`global_guidance`, an
//! experiment, docs/global-route.md "Phase 2"): the anneal's evaluator
//! (the placement written into the board, lowered as the layout routes
//! it, routed on tiles) and the per-part attribution the coupled loop's
//! moves rank by.

use super::*;
use crate::board_layout::without_copper_texts;
use crate::board_router::{KiCadPourMode, LayerTable, add_pour_zones, lower, planned_pours, pours};
use pcb_placer as core;
use pcb_router as router;
use std::collections::HashMap;
use std::sync::Arc;

/// `KiCadBoardPlacerConfig::global_guidance`.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct KiCadGlobalGuidance {
    /// Millimetres of wirelength one track of mean overflow under a net's
    /// box adds per millimetre of the net (`pcb_placer::guidance`).
    pub weight: f64,
    /// The evaluations' share of the anneal's work at most.
    pub work_share: f64,
    /// Accepted moves between two evaluations at least.
    pub min_accepted: usize,
    /// Share of the anneal's moves whose part is drawn by its attribution.
    pub bias: f64,
    /// The share of a boundary's capacity above which its usage counts:
    /// 1 is the overflow, 0.5 the soft overflow at 0.5 (the measure that
    /// told our placements from the designers', docs/global-route.md).
    pub threshold: f64,
    /// The coupled loop's moves try the parts by their attribution in the
    /// global route of the current board first (instead of the router's
    /// congestion under their bodies).
    pub moves: bool,
}

impl Default for KiCadGlobalGuidance {
    fn default() -> Self {
        Self {
            weight: 1.0,
            work_share: 0.1,
            min_accepted: 200,
            bias: 0.5,
            threshold: 0.5,
            moves: true,
        }
    }
}

/// Placement work per unit of the global route's own counts (fitted on
/// 4in1-mini, DSP-ADAU1452 and video, docs/global-route.md "Phase 2"):
/// per obstacle lowered, per weighted distance test of the capacities,
/// per state popped by the negotiation.
const WORK_PER_OBSTACLE: u64 = 3_000;
const WORK_PER_CAPACITY_TEST: u64 = 5;
const WORK_PER_POP: u64 = 30;

/// Component index by reference, for the terminals' labels
/// (`REF.PAD`).
pub(crate) fn reference_index(references: &[String]) -> HashMap<String, usize> {
    references
        .iter()
        .enumerate()
        .map(|(index, reference)| (reference.clone(), index))
        .collect()
}

fn component_of(label: &str, index: &HashMap<String, usize>) -> Option<usize> {
    label
        .match_indices('.')
        .map(|(at, _)| &label[..at])
        .find_map(|reference| index.get(reference).copied())
}

/// Per boundary (`[direction][layer * tiles + tile]`), its usage above
/// `threshold` of its capacity (0 where below).
fn excess(route: &pcb_global::GlobalRoute, threshold: f64) -> [Vec<f32>; 2] {
    let threshold = threshold as f32;
    [0, 1].map(|direction| {
        route.usage[direction]
            .iter()
            .zip(&route.capacity.edges[direction])
            .map(|(used, capacity)| (used - threshold * capacity).max(0.0))
            .collect()
    })
}

/// Per component, the excess (`excess`) on the boundaries its nets cross,
/// summed over its nets; per tile the excess on its boundaries over the
/// layers.
pub(crate) fn attribution(
    board: &router::Board,
    route: &pcb_global::GlobalRoute,
    index: &HashMap<String, usize>,
    components: usize,
    threshold: f64,
) -> (Vec<f64>, Vec<f64>) {
    let overflow = excess(route, threshold);
    let tiles = route.frame.tiles();
    let per = route.frame.layers * tiles;
    let mut tile_overflow = vec![0.0f64; tiles];
    for boundaries in &overflow {
        for (state, over) in boundaries.iter().enumerate() {
            if *over > 0.0 {
                tile_overflow[state % tiles] += *over as f64;
            }
        }
    }
    let mut parts = vec![0.0f64; components];
    for net in &route.nets {
        let over: f64 = net
            .crossings
            .iter()
            .map(|edge| {
                let edge = *edge as usize;
                overflow[edge / per][edge % per] as f64
            })
            .sum();
        if over <= 0.0 {
            continue;
        }
        let mut seen: Vec<usize> = board.nets[net.net]
            .terminals
            .iter()
            .filter_map(|terminal| component_of(&terminal.label, index))
            .collect();
        seen.sort_unstable();
        seen.dedup();
        for component in seen {
            parts[component] += over;
        }
    }
    (parts, tile_overflow)
}

/// The board as the layout routes a placement (tracks stripped, planned
/// pours added, copper texts as the layout keeps them), whether its pours
/// connect, and the parts' references by index.
fn routed_base(
    pcb: &Expr,
    router_config: &KiCadBoardRouterConfig,
    keep_board_marks: bool,
) -> Result<(Expr, bool), String> {
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
    Ok((without_copper_texts(&routed, keep_board_marks), connect))
}

/// `base` with the footprints at `poses`, lowered as the layout routes it.
fn lowered_at(
    base: &Expr,
    poses: &[core::Pose],
    router_config: &KiCadBoardRouterConfig,
    connect: bool,
) -> Option<router::Board> {
    let mut candidate = base.clone();
    {
        let mut footprints = crate::board_layout::footprint_items(&mut candidate).ok()?;
        if footprints.len() > poses.len() {
            eprintln!(
                "escape demand: {} footprints for {} poses: not measured",
                footprints.len(),
                poses.len()
            );
            return None;
        }
        for (footprint, pose) in footprints.iter_mut().zip(poses) {
            write_footprint_pose(footprint, *pose).ok()?;
        }
    }
    match lower(&candidate, router_config, connect) {
        Ok(lowered) => Some(lowered.board),
        Err(error) => {
            eprintln!("escape demand: the lowering failed ({error}): not measured");
            None
        }
    }
}

/// The measured version of escape demand (`core::escape::Escape::measured`):
/// each session lowers the placement it first sees and routes it globally
/// (`pcb_global::Incremental`); later calls carry the parts that moved
/// since (their copper and terminals, rigidly) into that board and route
/// only their nets again. Each call returns the overflow attribution per
/// part (`attribution` at `threshold`) and its work.
pub(crate) fn measured_factory(
    pcb: &Expr,
    references: &[String],
    router_config: &KiCadBoardRouterConfig,
    keep_board_marks: bool,
    threshold: f64,
) -> Result<Arc<core::escape::MeasuredFactory>, String> {
    let (base, connect) = routed_base(pcb, router_config, keep_board_marks)?;
    let base = Arc::new(base);
    let index = Arc::new(reference_index(references));
    let components = references.len();
    let router_config = router_config.clone();
    Ok(Arc::new(move || -> core::escape::MeasuredSession {
        let base = base.clone();
        let index = index.clone();
        let router_config = router_config.clone();
        let mut state: Option<(
            router::Board,
            pcb_global::Incremental,
            Vec<crate::move_descent::PartCopper>,
            Vec<core::Pose>,
        )> = None;
        Box::new(move |poses: &[core::Pose]| {
            let work;
            match state.as_mut() {
                None => {
                    let board = lowered_at(&base, poses, &router_config, connect)?;
                    let global =
                        pcb_global::Incremental::new(&board, &pcb_global::GlobalConfig::default());
                    work = WORK_PER_OBSTACLE * board.obstacles.len() as u64
                        + WORK_PER_CAPACITY_TEST * global.capacity_tests
                        + WORK_PER_POP * global.pops;
                    let copper = crate::move_descent::part_copper(&board, &index, components);
                    state = Some((board, global, copper, poses.to_vec()));
                }
                Some((board, global, copper, last)) => {
                    if last.len() != poses.len() {
                        return None;
                    }
                    let moved: Vec<usize> = (0..components.min(poses.len()))
                        .filter(|part| last[*part] != poses[*part])
                        .collect();
                    if moved.is_empty() {
                        work = 0;
                    } else {
                        let (_, _, area, nets) =
                            crate::move_descent::apply(board, copper, last, poses, &moved);
                        let next = global.moved(board, area, &nets);
                        work = WORK_PER_CAPACITY_TEST
                            * (next.capacity_tests - global.capacity_tests)
                            + WORK_PER_POP * (next.pops - global.pops);
                        *global = next;
                        last.copy_from_slice(poses);
                    }
                }
            }
            let (board, global, _, _) = state.as_ref()?;
            let (parts, _) = attribution(board, &global.route, &index, components, threshold);
            Some((parts, work))
        })
    }))
}

/// The anneal's evaluator for the board `pcb` as placed (before the
/// lowering of the placement): each call writes the poses into it,
/// lowers it as the layout routes a placement and routes it globally.
pub(crate) fn evaluator(
    pcb: &Expr,
    references: &[String],
    router_config: &KiCadBoardRouterConfig,
    keep_board_marks: bool,
    guidance_threshold: f64,
) -> Result<Arc<core::guidance::GuidanceFn>, String> {
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
    let base = without_copper_texts(&routed, keep_board_marks);
    let index = reference_index(references);
    let components = references.len();
    let router_config = router_config.clone();
    let config = pcb_global::GlobalConfig::default();
    let threshold = guidance_threshold;
    let debug = std::env::var_os("PCB_PLACER_DEBUG").is_some();
    Ok(Arc::new(move |poses: &[core::Pose]| {
        let started = std::time::Instant::now();
        let mut candidate = base.clone();
        {
            let mut footprints = crate::board_layout::footprint_items(&mut candidate).ok()?;
            // The footprints come first; the board's marks follow as
            // fixed components.
            if footprints.len() > poses.len() {
                eprintln!(
                    "guidance: {} footprints for {} poses: no guidance",
                    footprints.len(),
                    poses.len()
                );
                return None;
            }
            for (footprint, pose) in footprints.iter_mut().zip(poses) {
                write_footprint_pose(footprint, *pose).ok()?;
            }
        }
        let board = match lower(&candidate, &router_config, connect) {
            Ok(lowered) => lowered.board,
            Err(error) => {
                eprintln!("guidance: the lowering failed ({error}): no guidance");
                return None;
            }
        };
        let lowered = started.elapsed().as_secs_f64();
        let route = pcb_global::route(&board, &config);
        let (parts, tile_overflow) = attribution(&board, &route, &index, components, threshold);
        let total: f64 = tile_overflow.iter().sum();
        let work = WORK_PER_OBSTACLE * board.obstacles.len() as u64
            + WORK_PER_CAPACITY_TEST * route.capacity.work
            + WORK_PER_POP * route.pops;
        if debug {
            eprintln!(
                "guidance: evaluation lowered in {lowered:.3} s, routed in {:.3} s (capacity {:.3} s); obstacles {}, capacity tests {}, pops {}, work {:.3} s; total overflow {:.0}, excess at {threshold} {total:.0}",
                route.seconds,
                route.capacity_seconds,
                board.obstacles.len(),
                route.capacity.work,
                route.pops,
                work as f64 / core::work_per_second(),
                route.score.total_overflow
            );
        }
        Some(core::guidance::GuidanceField {
            origin: route.frame.origin,
            tile_mm: route.frame.tile_mm,
            tiles_x: route.frame.tiles_x,
            tiles_y: route.frame.tiles_y,
            overflow: tile_overflow,
            attribution: parts,
            total_overflow: total,
            work,
        })
    }))
}
