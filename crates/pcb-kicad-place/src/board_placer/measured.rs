// Copyright (C) 2026 Toit contributors.

//! The global route as the placer measures with it (docs/global-route.md):
//! the measured escape demand's sessions (`measured_factory`, the
//! placement lowered as the layout routes it and routed on tiles by
//! `pcb_global::Incremental`) and the per-part overflow attribution they
//! return; the lowering of a placement the pad exits share.

use super::*;
use crate::board_router::{KiCadPourMode, LayerTable, add_pour_zones, lower, planned_pours, pours};
use crate::labels::without_copper_texts;
use pcb_placer as core;
use pcb_router as router;
use std::collections::HashMap;
use std::sync::Arc;

/// Placement work per unit of the global route's own counts (fitted on
/// 4in1-mini, DSP-ADAU1452 and video, docs/global-route.md "Work"):
/// per obstacle lowered, per weighted distance test of the capacities,
/// per state popped by the negotiation.
const WORK_PER_OBSTACLE: u64 = 3_000;
const WORK_PER_CAPACITY_TEST: u64 = 5;
const WORK_PER_POP: u64 = 30;

/// Component index by reference, for the terminals' labels
/// (`REF.PAD`).
pub fn reference_index(references: &[String]) -> HashMap<String, usize> {
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
pub fn attribution(
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
pub(super) fn routed_base(
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
pub(super) fn lowered_at(
    base: &Expr,
    poses: &[core::Pose],
    router_config: &KiCadBoardRouterConfig,
    connect: bool,
) -> Option<router::Board> {
    let mut candidate = base.clone();
    {
        let mut footprints = crate::canonical_order::footprint_items(&mut candidate).ok()?;
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
            Vec<crate::part_motion::PartCopper>,
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
                    let copper = crate::part_motion::part_copper(&board, &index, components);
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
                        let (area, nets) =
                            crate::part_motion::apply(board, copper, last, poses, &moved);
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
