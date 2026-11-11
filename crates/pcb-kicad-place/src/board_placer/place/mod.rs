// Copyright (C) 2026 Toit contributors.

//! `place_kicad_board`: the whole placement of a board.
//!
//! `place_kicad_board` runs the stages in order, each on a read-only
//! `PlaceContext` (the inputs and the settings resolved for the board)
//! and the board and its lowering, which the first stages change: the
//! board read and lowered (`prepare`), placed in pieces where it is a
//! panel (`pieces`), the routing demand the placer weighs (`demand`)
//! and its configuration (`placer_config`), the seeds placed and the best
//! picked (`seeds`), its poses written and the placed directory
//! (`output`), and board-placer.json (`report`). `placer_config` stays
//! in this file: `tools/config-reference` reads the settings it passes
//! to the core from the file that holds `place_kicad_board`.

use super::*;
use pcb_placer as core;

mod demand;
mod output;
mod pieces;
mod prepare;
mod report;
mod seeds;

use self::demand::add_routing_demand;
use self::output::{WrittenPoses, check_written_pads, write_placed_directory, write_poses};
use self::pieces::{BoardPieces, place_in_pieces};
pub use self::prepare::{PreparedBoard, prepare_board};
#[cfg(test)]
pub(crate) use self::report::placed_count;
pub use self::report::room_hint;
use self::report::{placement_report, write_report};
use self::seeds::{Picked, pick, place_seeds, skeleton_race};

/// What every stage of a placement reads: the inputs, the settings
/// resolved for the board (`prepare_board`), what its constraints chose,
/// and when the placement started.
pub(super) struct PlaceContext<'a> {
    pub source_directory: &'a Path,
    pub board_id: &'a str,
    pub output_directory: &'a Path,
    pub config: KiCadBoardPlacerConfig,
    /// The outline the constraints drew (`outline`), its size.
    pub outline_size: Option<[f64; 2]>,
    /// The edges the constraints left to the placer, as it chose them.
    pub edges_chosen: BTreeMap<String, String>,
    pub started: std::time::Instant,
}

pub fn place_kicad_board(
    source_directory: &Path,
    board_id: &str,
    output_directory: &Path,
    config: &KiCadBoardPlacerConfig,
) -> Result<KiCadBoardPlacerResult, String> {
    let started = std::time::Instant::now();
    let PreparedBoard {
        mut pcb,
        config,
        mut lowered,
        outline_size,
        edges_chosen,
    } = prepare_board(source_directory, board_id, config, None)?;
    let context = PlaceContext {
        source_directory,
        board_id,
        output_directory,
        config,
        outline_size,
        edges_chosen,
        started,
    };
    if let Some(designer) = std::env::var_os("PCB_PLACER_DESIGNER") {
        diagnose_designer(&lowered, Path::new(&designer))?;
    }
    for warning in &lowered.constraint_warnings {
        eprintln!("warning: {warning}");
    }
    let pieces = place_in_pieces(&context, &pcb, &mut lowered)?;
    add_routing_demand(&context, &pcb, &mut lowered)?;
    let lowered = lowered;
    let placer_config = placer_config(&context, &pcb, &lowered)?;
    let skeleton_race = skeleton_race(&context, &pcb, &lowered, &pieces);
    let seeds = place_seeds(
        &context,
        &lowered,
        &placer_config,
        &pieces,
        skeleton_race.as_ref(),
    );
    if let Some(trace) = std::env::var_os("PCB_PLACER_TRACE") {
        trace_placements(
            &lowered,
            &seeds.placements,
            &seeds.kept_seeds,
            &placer_config,
            Path::new(&trace),
        )?;
    }
    let picked = pick(
        &context,
        &pcb,
        &lowered,
        &pieces,
        skeleton_race.as_ref(),
        seeds,
    );
    let written = write_poses(&mut pcb, &lowered, &picked.placement)?;
    check_written_pads(&pcb, &lowered, &picked.placement.poses)?;
    write_placed_directory(&context, &pcb, &lowered, &picked.placement)?;
    let result = placement_report(
        &context,
        &pcb,
        &lowered,
        pieces.report,
        pieces.modules,
        picked,
        written,
    );
    write_report(&context, &result)?;
    Ok(result)
}

/// The placer's configuration for the board: the settings, the work
/// limit, the warm start, and the router's test where it is on (the pad
/// exits).
fn placer_config(
    context: &PlaceContext,
    pcb: &Expr,
    lowered: &LoweredPlacement,
) -> Result<core::Config, String> {
    let config = &context.config;
    let mut placer_config = core::Config::new();
    placer_config.global.whitespace_fill = config.whitespace_fill;
    placer_config.maximum_utilization = config.maximum_utilization;
    placer_config.global.seed = config.seed;
    placer_config.anneal.deadline = config.deadline;
    placer_config.work_limit = config
        .work_seconds
        .map(|seconds| (seconds.max(0.0) * core::work_per_second()) as u64);
    placer_config.overlap_far_side_pads = config.overlap_far_side_pads;
    placer_config.orientation_search = config.orientation_search;
    placer_config.bus_groups = config.bus_groups.as_ref().map(KiCadBusGroups::settings);
    placer_config.kicad_last_level = true;
    placer_config.start = warm_start(lowered)?.map(std::sync::Arc::new);
    placer_config.skip_anneal = std::env::var_os("PCB_PLACER_SKIP_ANNEAL").is_some();
    // Pads without a way out (`pad_exits`): the router's test after each
    // seed's placement, with the router's configuration the layout and the
    // place check give.
    if let Some(settings) = exits::pad_exits_settings(config) {
        match &config.measure_router {
            Some(router_config) => {
                placer_config.pad_exits = Some(core::exits::PadExits {
                    factory: exits::exit_factory(
                        pcb,
                        &lowered.references,
                        router_config,
                        config.keep_board_marks,
                    )?,
                    reach: settings.reach_mm,
                    candidates: settings.candidates,
                });
            }
            None => eprintln!(
                "placer: pad exits: no router configuration (the layout and the place check give one): not tested"
            ),
        }
    }
    Ok(placer_config)
}

#[cfg(test)]
mod tests;
