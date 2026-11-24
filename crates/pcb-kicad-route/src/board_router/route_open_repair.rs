// Copyright (C) 2026 Florian Loitsch.

//! Route mode's open repair: after the ladder, each net KiCad still finds
//! open is routed alone against all the copper kept, pours lowered as
//! the local repair lowers them (`open_repair::lower_for_local`), and
//! kept only where KiCad finds the board better (`repair_layout_opens`, the
//! layout's additive repair).

use super::*;
use crate::open_repair::{LayoutRepairBudget, repair_layout_opens};

/// The search work the repair may take when the ladder left less: route
/// mode's other repairs after the ladder (solid thermals, necks) count
/// their checks as no work either. One net's search is at most 20 s of
/// work (the local repair's own limit).
const SEARCH_FLOOR_SECONDS: f64 = 20.0;

/// What route mode's open repair did (`board-router.json`'s `open_repair`).
#[derive(Clone, Debug, Serialize)]
pub struct KiCadRouteOpenRepair {
    /// `accepted`, `no_improvement`, `skipped` or `failed`.
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// KiCad's unconnected items before and after.
    pub opens_before: usize,
    pub opens_after: usize,
    /// The nets whose copper KiCad accepted, and the objects added.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub nets: Vec<String>,
    pub added_copper: usize,
    pub native_checks: usize,
    pub search_work_seconds: f64,
}

/// Runs the repair on the board kept in `directory` (the run's staging
/// directory) when KiCad finds connections open there, and adopts the
/// repaired project in its place. Returns the search work charged.
pub(super) fn repair_route_opens(
    result: &mut KiCadBoardRouterResult,
    directory: &Path,
    board_id: &str,
    config: &KiCadBoardRouterConfig,
    work_left: f64,
) -> Result<Option<KiCadRouteOpenRepair>, String> {
    let Some(before) = result
        .native
        .as_ref()
        .filter(|native| native.selected_net_unconnected_items > 0)
        .map(|native| native.selected_net_unconnected_items)
    else {
        return Ok(None);
    };
    let mut repair = KiCadRouteOpenRepair {
        status: "skipped",
        reason: None,
        opens_before: before,
        opens_after: before,
        nets: Vec::new(),
        added_copper: 0,
        native_checks: 0,
        search_work_seconds: 0.0,
    };
    let mut budget = LayoutRepairBudget {
        router: config,
        available_seconds: work_left.max(SEARCH_FLOOR_SECONDS),
        // Route mode counts KiCad's checks as no work (`checks_charged`).
        check_seconds: 0.0,
        native_checks: 0,
        search_work_seconds: 0.0,
        branch_fill_reserve_checks: Some(0),
    };
    let trial = temporary_next_to(directory);
    let candidate = repair_layout_opens(directory, board_id, &trial, &mut budget);
    repair.native_checks = budget.native_checks;
    repair.search_work_seconds = budget.search_work_seconds;
    let candidate = match candidate {
        Ok(candidate) => candidate,
        Err(error) => {
            let _ = fs::remove_dir_all(&trial);
            repair.status = "failed";
            repair.reason = Some(error);
            return Ok(Some(repair));
        }
    };
    if candidate.status != "improved" {
        let _ = fs::remove_dir_all(&trial);
        repair.status = "no_improvement";
        repair.reason = Some(
            candidate
                .note
                .or_else(|| candidate.trials.first().map(|trial| trial.status))
                .unwrap_or("no_eligible_native_open")
                .to_string(),
        );
        return Ok(Some(repair));
    }
    let adopted = (|| -> Result<VerificationReport, String> {
        let native = verification::record_verified_additive_board(
            &trial,
            board_id,
            candidate.schematic_parity_checked,
        )?;
        if native.selected_net_unconnected_items >= before {
            return Err("selected_native_open_count_did_not_decrease".into());
        }
        Ok(native)
    })();
    let native = match adopted {
        Ok(native) => native,
        Err(error) => {
            let _ = fs::remove_dir_all(&trial);
            repair.status = "no_improvement";
            repair.reason = Some(error);
            return Ok(Some(repair));
        }
    };
    let copper =
        board_stats::inspect_kicad_saved_copper(&trial.join(format!("{board_id}.kicad_pcb")))?;
    let previous = temporary_next_to(directory);
    fs::rename(directory, &previous)
        .map_err(|error| format!("stage the routed board for the open repair: {error}"))?;
    if let Err(error) = fs::rename(&trial, directory) {
        fs::rename(&previous, directory).map_err(|restore| {
            format!("could not adopt the open repair: {error}; restore: {restore}")
        })?;
        let _ = fs::remove_dir_all(&trial);
        repair.status = "failed";
        repair.reason = Some(format!("adopt the open repair: {error}"));
        return Ok(Some(repair));
    }
    let _ = fs::remove_dir_all(previous);
    repair.status = "accepted";
    repair.opens_after = native.selected_net_unconnected_items;
    repair.added_copper = candidate.added_copper;
    repair.nets = candidate
        .trials
        .iter()
        .filter(|trial| trial.status == "accepted")
        .map(|trial| trial.net.clone())
        .collect();
    result.native = Some(native);
    result.model_precedes_additive_repair = true;
    result.closed_reuse_drc = None;
    result.saved_copper = Some(copper);
    Ok(Some(repair))
}

impl KiCadRouteOpenRepair {
    pub(super) fn skipped(native: &VerificationReport, reason: &str) -> Self {
        Self {
            status: "skipped",
            reason: Some(reason.into()),
            opens_before: native.selected_net_unconnected_items,
            opens_after: native.selected_net_unconnected_items,
            nets: Vec::new(),
            added_copper: 0,
            native_checks: 0,
            search_work_seconds: 0.0,
        }
    }

    /// The log line.
    pub(super) fn line(&self) -> String {
        format!(
            "open repair: {}{}, KiCad's unconnected items {} -> {}{} ({} KiCad checks, {:.1} s of search work)",
            self.status,
            self.reason
                .as_ref()
                .map_or(String::new(), |reason| format!(" ({reason})")),
            self.opens_before,
            self.opens_after,
            if self.nets.is_empty() {
                String::new()
            } else {
                format!(
                    ", {} objects on {}",
                    self.added_copper,
                    self.nets.join(", ")
                )
            },
            self.native_checks,
            self.search_work_seconds
        )
    }
}
