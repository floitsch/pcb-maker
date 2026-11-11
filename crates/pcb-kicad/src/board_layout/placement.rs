// Copyright (C) 2026 Toit contributors.

//! The layout's placement phase: the outline sized, the board placed, its
//! pins swapped, the pours and classes added and the placed board lowered
//! for the moves' legality.

use super::setup::{LayoutContext, WorkLedger};
use super::*;

/// What the placement phase hands on: the placer's result, the placer
/// configuration the later steps keep to (the outline sized, the edges and
/// marks the placement chose) and the outline sizes tried.
pub(super) struct Placed {
    pub placement: KiCadBoardPlacerResult,
    pub placer_config: KiCadBoardPlacerConfig,
    pub outline_sizing: Vec<KiCadOutlineTrial>,
}

/// The placed board's copper facts the routes and the ladder read: its
/// layers, whether it has pours and whether the routes connect them.
pub(super) struct Copper {
    pub layers: LayerTable,
    pub has_pours: bool,
    pub connect: bool,
}

/// The placed board as the routes start from it: the board (with its
/// pours), the placement problem lowered from it with the placement's
/// relaxation applied, that relaxation, and the copper facts.
pub(super) struct PlacedBoard {
    pub pcb: Expr,
    pub problem: crate::board_placer::LoweredPlacement,
    pub first_relaxation: placer::Relaxation,
    pub copper: Copper,
}

/// Sizes the outline (where it is automatic) and places the board with the
/// placer's configuration as the layout gives it, booking the placement's
/// work (its longest seed; the seed pool's excess apart). Hands on the
/// placement; an error where the placement is not legal.
pub(super) fn place(ctx: &LayoutContext, ledger: &mut WorkLedger) -> Result<Placed, String> {
    let config = ctx.config;
    let router_config = &ctx.router_config;
    let mut placer_config = layout_placer_config(config, router_config, ctx.source_directory)?;
    let placement_budget = placement_budget(config);
    eprintln!("layout: sizing the outline");
    let outline_sizing = size_outline(
        ctx.source_directory,
        ctx.board_id,
        ctx.output_directory,
        &mut placer_config,
        router_config,
        &mut ledger.work,
        ctx.work_deadline,
        &mut ledger.pool_excess,
    )?;
    eprintln!(
        "layout: placing ({:.0} s, work {:.0} s)",
        ctx.elapsed(),
        ledger.work
    );
    crate::mem_profile::phase("placing");
    // What sizing the outline left of the placement budget (a little at
    // least: without any, placement would not anneal at all).
    placer_config.work_seconds = Some((placement_budget - ledger.work).max(0.1 * placement_budget));
    // The pre-ranking ranks more of the placer's seeds than the race
    // could probe, only where the race runs: one router seed, no pin
    // swaps.
    let pre_ranking = pre_ranks(config, router_config);
    if pre_ranking {
        placer_config.placement_seeds = race_placement_seeds(config, &placer_config);
    }
    let placement = place_kicad_board(
        ctx.source_directory,
        ctx.board_id,
        &ctx.placed_directory,
        &placer_config,
    )?;
    ledger.work += placement.work_seconds;
    ledger.pool_excess += (placement.pool_work_seconds - placement.work_seconds).max(0.0);
    let work = ledger.work;
    // The marks that belong to a movable part were taken from the input;
    // the placed board's lowerings keep them so.
    placer_config.attached_marks = Some(placement.attached_mark_pairs.clone());
    if ctx.wall_passed() {
        core::router::note_wall_cut(
            "placement",
            format_args!("the placement ended past the deadline, {work:.0} s of work done"),
        );
    }
    eprintln!(
        "layout: placed ({:.0} s, work {work:.0} s, {} seeds kept of {} run, seed {} kept; {:.0} s more of it in the pool of seeds, {:.0} s of seeds in all)",
        ctx.elapsed(),
        placement.seeds,
        placement.seeds_run,
        placement.seed_kept,
        ledger.pool_excess,
        placement.seeds_work_seconds
    );
    // The moves keep parts on `"edge": "any"` at the edges they were given.
    if let Some(KiCadConstraintsSource::Inline(constraints)) = &mut placer_config.constraints {
        for entry in &mut constraints.edge {
            if let Some(edge) = placement.edges_chosen.get(&entry.part) {
                entry.edge = edge.clone();
            }
        }
    }
    if !placement.unplaced.is_empty() || !placement.illegal.is_empty() {
        return Err(format!(
            "placement is not legal (unplaced {:?}, illegal {:?}); {}",
            placement.unplaced,
            placement.illegal,
            placement.hints.join("; ")
        ));
    }
    Ok(Placed {
        placement,
        placer_config,
        outline_sizing,
    })
}

/// Pins assigned for the placement (`pin_swaps`, a spec file, or
/// `swappable`, groups of the placed board), in `placed/`: the placed
/// project (board and schematic) is what the result is built from. Hands
/// on what was swapped; `None` where neither is set.
pub(super) fn swap_pins(ctx: &LayoutContext) -> Result<Option<KiCadPinSwapResult>, String> {
    let config = ctx.config;
    Ok(match &config.pin_swaps {
        Some(path) => {
            let spec = read_pin_swap_spec(&ctx.source_directory.join(path))?;
            Some(swap_kicad_pins_in_place(
                &ctx.placed_directory,
                ctx.board_id,
                &spec,
                &config.pin_swap,
            )?)
        }
        None if !config.swappable.is_empty() => {
            let placed = &ctx.placed_board;
            let text = fs::read_to_string(placed)
                .map_err(|error| format!("failed to read {}: {error}", placed.display()))?;
            let spec = swappable_spec(
                &crate::canonical_order::parse_board(&text)?,
                &config.swappable,
            )?;
            Some(swap_kicad_pins_in_place(
                &ctx.placed_directory,
                ctx.board_id,
                &spec,
                &config.pin_swap,
            )?)
        }
        None => None,
    })
}

/// Reads the placed board back, adds the requested pours (rewriting
/// `placed/`) and the net classes, and lowers it for the moves with the
/// rules the placement needed (`placer_config` gains them). Hands on the
/// board, its lowered placement and the copper facts.
pub(super) fn prepare_placed_board(
    ctx: &LayoutContext,
    placement: &KiCadBoardPlacerResult,
    placer_config: &mut KiCadBoardPlacerConfig,
) -> Result<PlacedBoard, String> {
    let router_config = &ctx.router_config;
    let placed_board = &ctx.placed_board;
    let source = fs::read_to_string(placed_board)
        .map_err(|error| format!("failed to read {}: {error}", placed_board.display()))?;
    let mut pcb = crate::canonical_order::parse_board(&source)?;
    // Requested pours (a ground plane) join the placed board.
    let planned = planned_pours(&pcb, router_config)?;
    if !planned.is_empty() {
        if router_config.add_pours.is_empty() {
            eprintln!(
                "automatic planes: {}",
                planned
                    .iter()
                    .map(|pour| format!("{} on {}", pour.net, pour.layers.join(", ")))
                    .collect::<Vec<_>>()
                    .join("; ")
            );
        }
        add_pour_zones(&mut pcb, &planned)?;
        fs::write(placed_board, format!("{}\n", encode(&pcb)))
            .map_err(|error| format!("failed to write {}: {error}", placed_board.display()))?;
    }
    if !router_config.net_classes.is_empty() {
        crate::net_classes::write_net_classes(
            &ctx.placed_directory,
            ctx.board_id,
            &pcb,
            &router_config.net_classes,
        )?;
    }
    let layers = LayerTable::from_pcb(&pcb)?;
    let has_pours = !pours(&pcb, &layers)?.is_empty();
    let connect = has_pours && router_config.pours != KiCadPourMode::Tracks;
    // The moves that follow keep to the rules the placement needed: on a
    // board placed with relaxed spacing, a move checked against the full
    // spacing is never legal.
    placer_config.tight_bodies = Some(placement.tight_bodies);
    if placer_config.holes_in_courtyards_error.is_none() {
        placer_config.holes_in_courtyards_error = Some(
            crate::board_placer::holes_in_courtyards_error(&ctx.project_file()),
        );
    }
    if placer_config.silk_over_parts_error.is_none() {
        placer_config.silk_over_parts_error = Some(crate::board_placer::silk_over_parts_error(
            &ctx.project_file(),
        ));
    }
    let mut problem = lower_placement(&pcb, placer_config)?;
    let first_relaxation = first_relaxation(placement);
    first_relaxation.apply(&mut problem.problem);
    Ok(PlacedBoard {
        pcb,
        problem,
        first_relaxation,
        copper: Copper {
            layers,
            has_pours,
            connect,
        },
    })
}

/// The relaxation the placement kept its first placement at (its spacing,
/// grid, halos, edge and courtyard rules, the parts left on marks).
pub(super) fn first_relaxation(placement: &KiCadBoardPlacerResult) -> placer::Relaxation {
    placer::Relaxation {
        spacing: placement.spacing_mm,
        grid: placement.grid_mm,
        halo_scale: placement.halo_scale,
        small_halo_scale: placement.small_halo_scale,
        edge_inset: placement.edge_inset,
        tight: placement.tight_bodies,
        edge_copper: placement.edge_copper,
        edge_rule: placement.edge_rule_mm,
        courtyard_spacing: placement.courtyard_spacing,
        pad_clearance: placement.pad_clearance,
        halos: placement.halos.clone(),
        on_marks: placement.on_marks.clone(),
    }
}

/// `pcb` with its footprints at `poses` (in the board's footprint order).
pub(super) fn posed(pcb: &Expr, poses: &[placer::Pose]) -> Result<Expr, String> {
    let mut candidate = pcb.clone();
    {
        let mut footprints = footprint_items(&mut candidate)?;
        for (footprint, pose) in footprints.iter_mut().zip(poses) {
            write_footprint_pose(footprint, *pose)?;
        }
    }
    Ok(candidate)
}

/// Writes `pcb` without its copper (segments, arcs, vias) to `placed/`:
/// the placement file shows the final poses.
pub(super) fn write_placement_without_copper(
    ctx: &LayoutContext,
    pcb: &Expr,
) -> Result<(), String> {
    let placed_board = &ctx.placed_board;
    let mut placed_pcb = pcb.clone();
    if let Expr::List(items) = &mut placed_pcb {
        items.retain(|item| !matches!(item.head(), Some("segment" | "arc" | "via")));
    }
    fs::write(placed_board, format!("{}\n", encode(&placed_pcb)))
        .map_err(|error| format!("failed to write {}: {error}", placed_board.display()))
}
