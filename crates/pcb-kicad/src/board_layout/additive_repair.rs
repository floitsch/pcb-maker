// Copyright (C) 2026 Florian Loitsch.

//! Budgeted native additive repair of the selected layout.

use super::setup::{LayoutContext, WorkLedger};
use super::*;
use pcb_kicad_route::open_repair::{LayoutRepairBudget, repair_layout_opens, same_as_standalone};
use sha2::{Digest, Sha256};

/// The optional post-route repair. The result's ordinary routing metrics
/// still describe the core route. Native and board quality describe the
/// selected board after any accepted additional copper.
#[derive(Clone, Debug, Serialize)]
pub struct LayoutOpenRepair {
    pub status: &'static str,
    pub reason: Option<String>,
    pub native_checks: usize,
    pub search_work_seconds: f64,
    pub work_seconds: f64,
    pub opens_before: usize,
    pub opens_after: usize,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub model_precedes_additive_repair: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report: Option<&'static str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub retry_argv: Vec<String>,
    /// The standalone command would repeat a completed search on this board.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub same_board_search_complete: bool,
    #[serde(skip)]
    completed_search: bool,
}

const MIN_SEARCH_SECONDS: f64 = 4.0;

pub(super) fn recover(
    ctx: &LayoutContext,
    ledger: &mut WorkLedger,
    routed: &mut KiCadBoardRouterResult,
) -> Result<Option<LayoutOpenRepair>, String> {
    if !ctx.router_config.post_route_repairs_on() {
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
    let trial = ctx.output_directory.join("local-open-repair");
    let mut repair = LayoutOpenRepair {
        status: "skipped",
        reason: None,
        native_checks: 0,
        search_work_seconds: 0.0,
        work_seconds: 0.0,
        opens_before: before.selected_net_unconnected_items,
        opens_after: before.selected_net_unconnected_items,
        model_precedes_additive_repair: false,
        report: None,
        same_board_search_complete: false,
        completed_search: false,
        retry_argv: vec![
            "pcb-maker".into(),
            "repair-kicad-opens".into(),
            result.display().to_string(),
            ctx.board_id.into(),
            "--local".into(),
            ctx.output_directory
                .join("manual-open-repair")
                .display()
                .to_string(),
        ],
    };
    let skip = |mut repair: LayoutOpenRepair, reason: &str| {
        repair.reason = Some(reason.into());
        Some(repair)
    };
    if ctx.config.stop_after_first_route {
        return Ok(skip(repair, "first_route_measurement"));
    }
    if ctx.wall_passed() {
        return Ok(skip(repair, "wall_guard_expired"));
    }
    let remaining = ledger.left(ctx.work_deadline) - routed.work_seconds;
    if remaining < 2.0 * ctx.check_seconds + MIN_SEARCH_SECONDS {
        return Ok(skip(repair, "insufficient_remaining_work"));
    }
    let has_fills = if ctx.config.save_zone_fills {
        let board = result.join(format!("{}.kicad_pcb", ctx.board_id));
        let text = match fs::read_to_string(board) {
            Ok(text) => text,
            Err(error) => return Ok(skip(repair, &format!("read selected PCB: {error}"))),
        };
        match parse(&text) {
            Ok(pcb) => crate::fill_handoff::has_fillable_zones(&pcb),
            Err(error) => return Ok(skip(repair, &error)),
        }
    } else {
        false
    };
    let available = remaining
        - if has_fills {
            2.0 * ctx.check_seconds
        } else {
            0.0
        };
    if available < 2.0 * ctx.check_seconds + MIN_SEARCH_SECONDS {
        return Ok(skip(
            repair,
            "insufficient_remaining_work_after_fill_reserve",
        ));
    }
    let mut budget = LayoutRepairBudget {
        router: &ctx.router_config,
        available_seconds: available,
        check_seconds: ctx.check_seconds,
        native_checks: 0,
        search_work_seconds: 0.0,
        branch_fill_reserve_checks: Some(if has_fills { 2 } else { 0 }),
    };
    let candidate = repair_layout_opens(&result, ctx.board_id, &trial, &mut budget);
    repair.native_checks = budget.native_checks;
    repair.search_work_seconds = budget.search_work_seconds;
    repair.work_seconds = budget.work_seconds();
    ledger.work += ledger.checks.add(budget.native_checks) + budget.search_work_seconds;
    let candidate = match candidate {
        Ok(candidate) => candidate,
        Err(error) => {
            repair.status = "failed";
            eprintln!("layout: local open repair failed: {error}");
            return Ok(skip(repair, &error));
        }
    };
    repair.report = Some("local-open-repair/open-repair.json");
    if candidate.status != "improved" {
        repair.status = "no_improvement";
        repair.completed_search = candidate.completed_without_local_path();
        return Ok(skip(
            repair,
            candidate
                .note
                .or_else(|| candidate.trials.first().map(|trial| trial.status))
                .unwrap_or("no_eligible_native_open"),
        ));
    }
    let native = match verification::record_verified_additive_board(
        &trial,
        ctx.board_id,
        candidate.schematic_parity_checked,
    ) {
        Ok(native) => native,
        Err(error) => {
            repair.status = "failed";
            return Ok(skip(repair, &error));
        }
    };
    if native.selected_net_unconnected_items >= before.selected_net_unconnected_items {
        repair.status = "no_improvement";
        return Ok(skip(repair, "selected_native_open_count_did_not_decrease"));
    }
    // Write all canonical evidence before switching directories. Retain
    // the old physical board and its reports through any recoverable failure.
    let mut selected = routed.clone();
    let accepted_opens = native.selected_net_unconnected_items;
    selected.native = Some(native);
    selected.model_precedes_additive_repair = true;
    selected.closed_reuse_drc = None;
    match pcb_kicad_file::board_stats::inspect_kicad_saved_copper(
        &trial.join(format!("{}.kicad_pcb", ctx.board_id)),
    ) {
        Ok(copper) => selected.saved_copper = Some(copper),
        Err(error) => {
            repair.status = "failed";
            return Ok(skip(repair, &error));
        }
    }
    if let Err(error) = write_typed_json(&trial.join("board-router.json"), &selected) {
        repair.status = "failed";
        return Ok(skip(repair, &error));
    }
    let previous = crate::fs_util::temporary_next_to(&result);
    if let Err(error) = fs::rename(&result, &previous) {
        repair.status = "failed";
        return Ok(skip(repair, &format!("stage selected layout: {error}")));
    }
    if let Err(error) = fs::rename(&trial, &result) {
        fs::rename(&previous, &result).map_err(|restore| {
            format!("could not adopt verified local open repair: {error}; restore: {restore}")
        })?;
        repair.status = "failed";
        return Ok(skip(
            repair,
            &format!("adopt verified local open repair: {error}"),
        ));
    }
    let _ = fs::remove_dir_all(previous);
    *routed = selected;
    repair.status = "accepted";
    repair.opens_after = accepted_opens;
    repair.model_precedes_additive_repair = true;
    repair.report = Some("result/open-repair.json");
    if repair.opens_after == 0 {
        repair.retry_argv.clear();
    }
    eprintln!(
        "layout: local open repair accepted ({} to {} native opens, {} copper objects, {:.2} modeled seconds)",
        candidate.opens_before, candidate.opens_after, candidate.added_copper, repair.work_seconds
    );
    Ok(Some(repair))
}

/// Suppress only a replay on the same selected board and with the same
/// router inputs. Fill publication may change the PCB bytes, so account
/// for its verified source and output explicitly.
pub(super) fn finalize_retry(
    ctx: &LayoutContext,
    ledger: &mut WorkLedger,
    repair: &mut LayoutOpenRepair,
    fill: Option<&crate::FillPublication>,
    work_deadline: f64,
) {
    if !repair.completed_search || ctx.wall_passed() || ledger.left(work_deadline) <= 0.0 {
        return;
    }
    let searched = ctx.output_directory.join("local-open-repair");
    let selected = ctx.output_directory.join("result");
    let board_name = format!("{}.kicad_pcb", ctx.board_id);
    let searched_board = searched.join(&board_name);
    let selected_board = selected.join(&board_name);
    let prove_unchanged = || -> Option<bool> {
        let old = fs::read(&searched_board).ok()?;
        let new = fs::read(&selected_board).ok()?;
        let same_hash = |bytes: &[u8], hash: Option<&str>| {
            hash.is_some_and(|hash| format!("{:x}", Sha256::digest(bytes)) == hash)
        };
        if old != new
            && !fill.is_some_and(|fill| {
                fill.status == "saved"
                    && same_hash(&old, fill.source_board_sha256.as_deref())
                    && same_hash(&new, fill.saved_board_sha256.as_deref())
            })
        {
            return Some(false);
        }
        for extension in ["kicad_pro", "kicad_sch", "kicad_dru"] {
            match (
                fs::read(searched_board.with_extension(extension)),
                fs::read(selected_board.with_extension(extension)),
            ) {
                (Ok(a), Ok(b)) if a == b => {}
                (Err(a), Err(b))
                    if a.kind() == std::io::ErrorKind::NotFound
                        && b.kind() == std::io::ErrorKind::NotFound => {}
                _ => return Some(false),
            }
        }
        Some(true)
    };
    if prove_unchanged() != Some(true) {
        return;
    }
    // The standalone repair diagnoses after refill. A saved board's
    // stored-only report can name different copper at the open endpoint.
    let Ok(before) =
        native_report::read_kicad_report(&searched.join("drc-before-open-repair.json"))
    else {
        return;
    };
    let Ok(after) = native_report::read_kicad_report(&selected.join("drc.json")) else {
        return;
    };
    let (equivalent, work) = same_as_standalone(
        &searched_board,
        &selected_board,
        [&before, &after],
        &ctx.router_config,
        ledger.left(work_deadline),
    );
    ledger.work += work;
    repair.work_seconds += work;
    if equivalent {
        repair.same_board_search_complete = true;
        repair.retry_argv.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_layout_repairs_one_net_with_native_evidence_and_budget() {
        if !std::process::Command::new(native_report::kicad_cli())
            .arg("version")
            .output()
            .is_ok_and(|output| output.status.success())
        {
            return;
        }
        let root = temporary_next_to(&std::env::temp_dir().join("layout-open-repair"));
        let source = root.join("source");
        fs::create_dir_all(&source).unwrap();
        let pcb = r#"(kicad_pcb (version 20241229) (generator "pcbnew")
          (general (thickness 1.6)) (paper "A4")
          (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
          (gr_rect (start 0 0) (end 20 10) (stroke (width 0.05) (type default)) (layer "Edge.Cuts"))
          (footprint "test" (layer "F.Cu") (at 3 5)
            (uuid "bf2ff1e8-13dd-462e-9898-234e65f5ff01")
            (property "Reference" "J1" (at 0 -2) (layer "F.SilkS") (effects (font (size 1 1) (thickness 0.15))))
            (pad "1" smd circle (at 0 0) (size 1 1) (layers "F.Cu") (net "LINK")
              (uuid "1b0aa000-13dd-462e-9898-234e65f5ff01")))
          (footprint "test" (layer "F.Cu") (at 16 5)
            (uuid "bf2ff1e8-13dd-462e-9898-234e65f5ff02")
            (property "Reference" "J2" (at 0 -2) (layer "F.SilkS") (effects (font (size 1 1) (thickness 0.15))))
            (pad "1" smd circle (at 0 0) (size 1 1) (layers "F.Cu") (net "LINK")
              (uuid "1b0aa000-13dd-462e-9898-234e65f5ff02"))))"#;
        fs::write(source.join("board.kicad_pcb"), pcb).unwrap();
        fs::write(source.join("board.kicad_pro"), "{}\n").unwrap();
        let output = root.join("layout");
        let result = output.join("result");
        let config = KiCadBoardLayoutConfig {
            total_seconds: 40.0,
            ..Default::default()
        };
        let router = crate::resolve_project_rules(
            &source.join("board.kicad_pro"),
            &source.join("board.kicad_pcb"),
        )
        .unwrap();
        let (mut ctx, mut work) =
            super::super::setup::prepare(&source, "board", &output, &config, &router).unwrap();
        copy_directory_tree(&source, &result).unwrap();
        let verified = verification::verify_materialized_rung(&result, "board").unwrap();
        assert_eq!(verified.selected_net_unconnected_items, 1);
        let mut routed = KiCadBoardRouterResult {
            native: Some(verified),
            ..Default::default()
        };
        work.work = ctx.work_deadline;
        let skipped = recover(&ctx, &mut work, &mut routed).unwrap().unwrap();
        assert_eq!(
            skipped.reason.as_deref(),
            Some("insufficient_remaining_work")
        );
        assert_eq!(skipped.native_checks, 0);
        assert_eq!(work.work, ctx.work_deadline);
        assert_eq!(
            fs::read_to_string(result.join("board.kicad_pcb")).unwrap(),
            pcb
        );
        work.work = 0.0;
        // With `tuning.post_route_repairs` false no repair from KiCad's
        // report runs: no record, no check, no work, the board untouched.
        ctx.router_config.tuning.post_route_repairs = Some(false);
        let mut fill = None;
        assert!(recover(&ctx, &mut work, &mut routed).unwrap().is_none());
        assert!(
            super::super::via_detour::recover(&ctx, &mut work, &mut routed)
                .unwrap()
                .is_none()
        );
        assert!(
            super::super::generated_pad_via::recover(&ctx, &mut work, &mut routed, &mut fill)
                .unwrap()
                .is_none()
        );
        assert!(
            super::super::unchanged_hatch::recover(&ctx, &mut work, &mut routed, &mut fill)
                .unwrap()
                .is_none()
        );
        assert_eq!((work.work, work.checks.checks), (0.0, 0));
        assert_eq!(
            fs::read_to_string(result.join("board.kicad_pcb")).unwrap(),
            pcb
        );
        assert!(!output.join("local-open-repair").exists());
        // The default: the repair runs as before.
        ctx.router_config.tuning.post_route_repairs = None;
        let accepted = recover(&ctx, &mut work, &mut routed).unwrap().unwrap();
        assert_eq!(accepted.status, "accepted", "{accepted:?}");
        assert_eq!((accepted.opens_before, accepted.opens_after), (1, 0));
        assert_eq!(accepted.native_checks, 2);
        assert_eq!(accepted.work_seconds, work.work);
        assert!(accepted.search_work_seconds > 0.0);
        assert!(routed.model_precedes_additive_repair);
        let copper = routed.saved_copper.unwrap();
        assert!(copper.segments > 0 && copper.track_length_mm > 10.0);
        let report: serde_json::Value =
            crate::fs_util::read_typed_json(&result.join("board-router.json")).unwrap();
        assert_eq!(
            report["saved_copper"],
            serde_json::to_value(copper).unwrap()
        );
        assert!(routed.native.unwrap().complete);
        assert!(result.join("open-repair.json").is_file());
        let disk = fs::read_to_string(result.join("board.kicad_pcb")).unwrap();
        assert!(disk.starts_with(pcb.strip_suffix(')').unwrap()));
        assert_eq!(
            fs::read_to_string(source.join("board.kicad_pcb")).unwrap(),
            pcb
        );
        fs::remove_dir_all(root).unwrap();
    }
}
