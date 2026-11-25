// Copyright (C) 2026 Florian Loitsch.

//! Try a single verified generated pad-via move on the finished layout.

use super::setup::{LayoutContext, WorkLedger};
use super::*;
use crate::relocated_open_repair::{
    GENERATED_PAD_VIA_CHECKS, MIN_GENERATED_PAD_VIA_SEARCH, RelocationBudget, repair_generated,
    suggest_generated_pad_via_repairs,
};

#[derive(Clone, Debug, Serialize)]
pub struct LayoutGeneratedPadViaRecovery {
    pub status: &'static str,
    pub reason: Option<String>,
    pub net: String,
    pub native_checks: usize,
    pub search_work_seconds: f64,
    pub work_seconds: f64,
    pub opens_before: usize,
    pub opens_after: usize,
    pub report: Option<&'static str>,
}

pub(super) fn recover(
    ctx: &LayoutContext,
    ledger: &mut WorkLedger,
    routed: &mut KiCadBoardRouterResult,
    fill: &mut Option<crate::FillPublication>,
) -> Result<Option<LayoutGeneratedPadViaRecovery>, String> {
    if !ctx.router_config.post_route_repairs_on()
        || ctx.config.stop_after_first_route
        || !ctx.config.save_zone_fills
        || routed.route_reuse.is_some()
    {
        return Ok(None);
    }
    let Some(before) = routed
        .native
        .as_ref()
        .filter(|native| native.selected_net_unconnected_items > 0)
    else {
        return Ok(None);
    };
    let result = ctx.output_directory.join("result");
    let trial = ctx.output_directory.join("generated-pad-via-repair");
    let Some(candidate) =
        suggest_generated_pad_via_repairs(&result, ctx.board_id, &ctx.placed_directory, &trial)
            .into_iter()
            .next()
    else {
        return Ok(None);
    };
    let opens = before.selected_net_unconnected_items;
    let mut report = LayoutGeneratedPadViaRecovery {
        status: "skipped",
        reason: None,
        net: candidate.net,
        native_checks: 0,
        search_work_seconds: 0.0,
        work_seconds: 0.0,
        opens_before: opens,
        opens_after: opens,
        report: None,
    };
    if let Some(reason) = candidate.native_evidence_limit {
        report.reason = Some(reason);
        return Ok(Some(report));
    }
    let available = ledger.left(ctx.work_deadline) - routed.work_seconds;
    let minimum =
        GENERATED_PAD_VIA_CHECKS as f64 * ctx.check_seconds + MIN_GENERATED_PAD_VIA_SEARCH;
    let stop = if ctx.wall_passed() {
        Some("wall_guard_expired")
    } else if available < minimum {
        Some("insufficient_remaining_work")
    } else if fill.as_ref().is_some_and(|fill| fill.status != "saved") {
        Some("selected_zone_fill_unverified")
    } else {
        None
    };
    if let Some(reason) = stop {
        report.reason = Some(reason.into());
        return Ok(Some(report));
    }
    let budget =
        RelocationBudget::for_layout(available, ctx.check_seconds, ctx.router_config.deadline);
    let repaired = repair_generated(
        &result,
        ctx.board_id,
        &ctx.placed_directory,
        &trial,
        &report.net,
        &budget,
    );
    report.native_checks = budget.native_checks();
    report.search_work_seconds = budget.search_work();
    report.work_seconds = ledger.checks.add(report.native_checks) + report.search_work_seconds;
    ledger.work += report.work_seconds;
    let repaired = match repaired {
        Ok(repaired) => repaired,
        Err(error) => {
            report.status = "failed";
            report.reason = Some(error);
            return Ok(Some(report));
        }
    };
    report.report = Some("generated-pad-via-repair/relocated-open-repair.json");
    if repaired.status != "improved" {
        report.status = "no_improvement";
        report.reason = repaired.reason;
        return Ok(Some(report));
    }
    let selected = (|| -> Result<KiCadBoardRouterResult, String> {
        fs::copy(result.join("erc.json"), trial.join("erc.json"))
            .map_err(|error| error.to_string())?;
        // The reports of an open repair adopted before, where there are any.
        preserve_local_repair_reports(&result, &trial)?;
        let native = verification::record_verified_additive_board(
            &trial,
            ctx.board_id,
            repaired.schematic_parity_checked,
        )?;
        if native.selected_net_unconnected_items >= opens {
            return Err("final native open count did not decrease".into());
        }
        if ctx.wall_passed() {
            return Err("wall guard expired while preparing the verified repair".into());
        }
        let mut selected = routed.clone();
        selected.native = Some(native);
        selected.closed_reuse_drc = None;
        selected.model_precedes_generated_pad_via_repair = true;
        selected.saved_copper = Some(pcb_kicad_file::board_stats::inspect_kicad_saved_copper(
            &trial.join(format!("{}.kicad_pcb", ctx.board_id)),
        )?);
        write_typed_json(&trial.join("board-router.json"), &selected)?;
        Ok(selected)
    })();
    let selected = match selected {
        Ok(selected) => selected,
        Err(error) => {
            report.status = "failed";
            report.reason = Some(error);
            return Ok(Some(report));
        }
    };
    let previous = crate::fs_util::temporary_next_to(&result);
    if let Err(error) = fs::rename(&result, &previous) {
        report.status = "failed";
        report.reason = Some(format!("stage selected layout: {error}"));
        return Ok(Some(report));
    }
    if let Err(error) = fs::rename(&trial, &result) {
        fs::rename(&previous, &result).map_err(|restore| {
            format!("could not adopt verified pad-via repair: {error}; restore: {restore}")
        })?;
        report.status = "failed";
        report.reason = Some(format!("adopt verified pad-via repair: {error}"));
        return Ok(Some(report));
    }
    let _ = fs::remove_dir_all(previous);
    *routed = selected;
    *fill = repaired.fill_publication;
    report.status = "accepted";
    report.opens_after = repaired.stored_opens_after;
    report.report = Some("result/relocated-open-repair.json");
    eprintln!(
        "layout: generated pad-via repair accepted ({} to {} native opens, {} added tracks or vias, {:.2} modeled seconds)",
        report.opens_before, report.opens_after, repaired.added_copper, report.work_seconds,
    );
    Ok(Some(report))
}

fn preserve_local_repair_reports(source: &Path, trial: &Path) -> Result<(), String> {
    for entry in fs::read_dir(source).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name();
        if name.to_str().is_some_and(|name| {
            matches!(name, "open-repair.json" | "drc-before-open-repair.json")
                || name.strip_prefix("drc-open-trial-").is_some_and(|name| {
                    name.strip_suffix(".json").is_some_and(|index| {
                        !index.is_empty() && index.bytes().all(|digit| digit.is_ascii_digit())
                    })
                })
        }) {
            fs::copy(entry.path(), trial.join(name)).map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}
