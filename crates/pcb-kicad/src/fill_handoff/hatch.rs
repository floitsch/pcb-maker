// Copyright (C) 2026 Florian Loitsch.

use super::{FillDrcCounts, FillPublication, counts, fs_util, layout, native_report};
use crate::board_router::KiCadBoardRouterConfig;
use pcb_kicad_route::{
    necks::{HatchedNeckRepair as RefillRepair, prepare_kicad_hatched_neck_repair},
    open_repair::in_repair_copy,
};
use serde::Serialize;
use std::{fs, path::Path, time::Instant};

#[derive(Debug, Serialize)]
pub struct HatchedNeckRepair {
    #[serde(flatten)]
    pub repair: RefillRepair,
    pub as_refilled: FillDrcCounts,
    pub as_stored: FillDrcCounts,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill_publication: Option<FillPublication>,
}

impl HatchedNeckRepair {
    pub fn published_improvement(&self) -> bool {
        self.fill_publication
            .as_ref()
            .is_some_and(|fill| fill.status == "saved")
    }
}

/// Repair in a private copy and publish verified stored fill with the accepted copper.
pub fn repair_kicad_hatched_necks(
    source: &Path,
    board_id: &str,
    output: &Path,
    fraction: f64,
    config: &KiCadBoardRouterConfig,
) -> Result<HatchedNeckRepair, String> {
    in_repair_copy(source, output, |stage| {
        let started = Instant::now();
        let mut repair = prepare_kicad_hatched_neck_repair(stage, board_id, fraction, config)?;
        let board = stage.join(format!("{board_id}.kicad_pcb"));
        let drc = stage.join("drc.json");
        let refilled_path = stage.join("drc-refilled-after-hatch-repair.json");
        fs::rename(&drc, &refilled_path).map_err(|error| error.to_string())?;
        let refilled = native_report::read_kicad_report(&refilled_path)?;
        let mut fill_publication = None;
        if repair.after < repair.before {
            let text = fs::read_to_string(&board).map_err(|error| error.to_string())?;
            let mut fill = layout::publish_layout_fill(
                stage,
                board_id,
                crate::parse(&text)?,
                fs_util::file_sha256(&board)?,
                layout::FillRefillEvidence::AcceptedRepair(&refilled),
                false,
                || false,
            );
            repair.native_checks += fill.native_checks;
            if fill.status == "saved" {
                fs::copy(stage.join("drc-saved-fill.json"), &drc)
                    .map_err(|error| error.to_string())?;
                repair.note = "KiCad reopened and verified the stored fill of the repaired PCB";
            } else {
                repair.note =
                    "the repair improved refilled DRC, but the saved PCB needs a verified fill";
                let name = output.file_name().unwrap_or_default().to_string_lossy();
                fill.retry_argv = vec![
                    "pcb-maker".into(),
                    "drc-kicad-board".into(),
                    output.display().to_string(),
                    board_id.into(),
                    output
                        .with_file_name(format!("{name}-filled-drc.json"))
                        .display()
                        .to_string(),
                    "--save-filled".into(),
                    output
                        .with_file_name(format!("{name}-filled"))
                        .display()
                        .to_string(),
                ];
            }
            fill_publication = Some(fill);
        }
        let stored = if drc.is_file() {
            native_report::read_kicad_report(&drc)?
        } else {
            let parity = board.with_extension("kicad_sch").is_file()
                || board.with_extension("sch").is_file();
            repair.native_checks += 1;
            native_report::run_fill_handoff_drc(&board, &drc, parity, false)?
        };
        repair.seconds = started.elapsed().as_secs_f64();
        let report = HatchedNeckRepair {
            repair,
            as_refilled: counts(&refilled),
            as_stored: counts(&stored),
            fill_publication,
        };
        fs_util::write_typed_json(&stage.join("hatched-neck-repair.json"), &report)?;
        Ok(report)
    })
}
