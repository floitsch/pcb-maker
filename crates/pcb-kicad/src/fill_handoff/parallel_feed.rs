// Copyright (C) 2026 Florian Loitsch.

use super::{Value, fs_util, layout};
use pcb_kicad_route::{PadPathShortening, ParallelFeed, ParallelFeedFill};
use std::{fs, path::Path, time::Instant};

/// Add an independently routed feed and verify the stored board.
pub fn add_kicad_parallel_feed(
    project: &Path,
    board_id: &str,
    output: &Path,
    pads: [&str; 2],
    layer: &str,
    width: f64,
) -> Result<ParallelFeed, String> {
    pcb_kicad_route::add_kicad_parallel_feed(
        project,
        board_id,
        output,
        pads,
        layer,
        width,
        publish_fills,
    )
}

/// Add a direct decoupling shortcut with the same verified fill handoff.
pub fn shorten_kicad_decoupling_path(
    project: &Path,
    board_id: &str,
    output: &Path,
    pads: [&str; 2],
) -> Result<PadPathShortening, String> {
    pcb_kicad_route::shorten_kicad_decoupling_path(project, board_id, output, pads, publish_fills)
}

fn publish_fills(
    board: &Path,
    refilled: &Value,
    report_path: &Path,
    deadline: Instant,
) -> Result<ParallelFeedFill, String> {
    let project = board
        .parent()
        .ok_or("feed candidate needs a project directory")?;
    let board_id = board
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or("invalid board filename")?;
    let text = fs::read_to_string(board).map_err(|error| error.to_string())?;
    let authored = crate::parse(&text)?;
    let old_report = project.join("drc-saved-fill.json");
    let previous_report = match fs::read(&old_report) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.to_string()),
    };
    let publication = layout::publish_layout_fill(
        project,
        board_id,
        authored,
        fs_util::file_sha256(board)?,
        layout::FillRefillEvidence::CompletedRoute(refilled),
        false,
        || Instant::now() >= deadline,
    );
    if publication.status == "saved" {
        fs::rename(&old_report, report_path).map_err(|error| error.to_string())?;
    }
    if let Some(bytes) = previous_report {
        fs_util::write_atomically(&old_report, &bytes)?;
    }
    Ok(ParallelFeedFill {
        status: publication.status,
        reason: publication.reason,
        native_checks: publication.native_checks,
        filled_zones: publication.filled_zones,
        filled_polygons: publication.filled_polygons,
    })
}
