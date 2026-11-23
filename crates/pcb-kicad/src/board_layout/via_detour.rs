// Copyright (C) 2026 Florian Loitsch.

//! A bounded local DRC repair of a generated track in the selected layout.

use super::setup::{LayoutContext, WorkLedger};
use super::*;
use pcb_kicad_route::via_detour::{
    VIA_DETOUR_CHECKS, ViaDetourRepair, has_via_short, repair_kicad_via_short,
};

pub(super) fn recover(
    ctx: &LayoutContext,
    ledger: &mut WorkLedger,
    routed: &mut KiCadBoardRouterResult,
) -> Result<Option<ViaDetourRepair>, String> {
    if ctx.config.stop_after_first_route
        || ctx.wall_passed()
        || routed.route_reuse.is_some()
        || routed.native.as_ref().is_none_or(|native| {
            native.selected_net_unconnected_items != 0
                || native.schematic_parity_issues != 0
                || native.drc_design_violations == 0
        })
    {
        return Ok(None);
    }
    let result = ctx.output_directory.join("result");
    if !native_report::read_kicad_report(&result.join("drc.json"))
        .is_ok_and(|drc| has_via_short(&drc))
    {
        return Ok(None);
    }
    // Keep the ordinary fill reserve available. A rejected jog still spends
    // its fresh native checks, with no change to the selected result.
    let reserved = if ctx.config.save_zone_fills {
        2.0 * ctx.check_seconds
    } else {
        0.0
    };
    let available = ledger.left(ctx.work_deadline) - routed.work_seconds - reserved;
    if available < VIA_DETOUR_CHECKS as f64 * ctx.check_seconds {
        return Ok(Some(ViaDetourRepair {
            status: "unchanged",
            reason: "insufficient_remaining_work",
            native_checks: 0,
            detour: None,
            saved_copper: None,
            detail: None,
        }));
    }
    let trial = ctx.output_directory.join("local-via-detour");
    let mut repair = match repair_kicad_via_short(
        &result,
        ctx.board_id,
        &ctx.placed_directory,
        &trial,
        ctx.router_config.deadline,
    ) {
        Ok(repair) => repair,
        Err(error) => {
            eprintln!("layout: could not stage via detour: {error}");
            return Ok(None);
        }
    };
    ledger.work += ledger.checks.add(repair.native_checks);
    if repair.status != "improved" {
        return Ok(Some(repair));
    }
    let selected = (|| {
        let mut selected = routed.clone();
        let native = serde_json::from_str(
            &fs::read_to_string(trial.join("verification.json"))
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        selected.native = Some(native);
        selected.saved_copper = repair.saved_copper;
        write_typed_json(&trial.join("board-router.json"), &selected)?;
        Ok::<_, String>(selected)
    })();
    let selected = match selected {
        Ok(selected) => selected,
        Err(error) => {
            repair.status = "unchanged";
            repair.reason = "selected_report_unavailable";
            repair.detail = Some(error);
            return Ok(Some(repair));
        }
    };
    let previous = crate::fs_util::temporary_next_to(&result);
    if let Err(error) = fs::rename(&result, &previous) {
        repair.status = "unchanged";
        repair.reason = "could_not_stage_previous_result";
        repair.detail = Some(error.to_string());
        return Ok(Some(repair));
    }
    if let Err(error) = fs::rename(&trial, &result) {
        fs::rename(&previous, &result).map_err(|restore| {
            format!("could not adopt verified via detour: {error}; restore: {restore}")
        })?;
        repair.status = "unchanged";
        repair.reason = "could_not_adopt_trial";
        repair.detail = Some(error.to_string());
        return Ok(Some(repair));
    }
    let _ = fs::remove_dir_all(previous);
    *routed = selected;
    eprintln!("layout: native via/track short cleared by a verified local detour");
    Ok(Some(repair))
}
