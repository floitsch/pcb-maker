// Copyright (C) 2026 Florian Loitsch.

//! Finish width errors on an already connected layout under its hatch policy.

use super::setup::{LayoutContext, WorkLedger};
use super::*;
use pcb_kicad_file::kicad_sexpr::{Expr, form_atom};
use serde_json::Value;

#[derive(Clone, Debug, Serialize)]
pub struct LayoutUnchangedHatchRecovery {
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub native_checks_charged: usize,
    pub work_seconds: f64,
    pub remaining_work_seconds: f64,
    pub required_work_seconds: f64,
    pub connection_width_errors_before: usize,
    pub connection_width_errors_after: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report: Option<PathBuf>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub retry_argv: Vec<String>,
}

pub(super) fn recover(
    ctx: &LayoutContext,
    ledger: &mut WorkLedger,
    routed: &mut KiCadBoardRouterResult,
    fill: &mut Option<crate::FillPublication>,
) -> Result<Option<LayoutUnchangedHatchRecovery>, String> {
    use crate::fill_handoff::{
        UNCHANGED_HATCH_CHECKS_WITH_BASELINE, UnchangedHatchBaseline, UnchangedHatchBudget,
        repair_unchanged_hatch_widths,
    };

    if !ctx.router_config.post_route_repairs_on()
        || ctx.config.stop_after_first_route
        || ctx.router_config.skip_native_verification
        || !routed.native.as_ref().is_some_and(|native| {
            native.schematic_parity_checked
                && native.schematic_parity_issues == 0
                && native.selected_net_unconnected_items == 0
                && native.drc_design_violations > 0
        })
    {
        return Ok(None);
    }
    let result = ctx.output_directory.join("result");
    let board = result.join(format!("{}.kicad_pcb", ctx.board_id));
    let Ok(refilled) = native_report::read_kicad_report(&result.join("drc.json")) else {
        return Ok(None);
    };
    let count = width_errors(&refilled).count();
    if count == 0 {
        return Ok(None);
    }
    let pcb = crate::parse(&fs::read_to_string(&board).map_err(|error| error.to_string())?)?;
    if !hatch_has_width_error(&pcb, &refilled) {
        return Ok(None);
    }
    let limit = UNCHANGED_HATCH_CHECKS_WITH_BASELINE;
    let available = (ledger.left(ctx.work_deadline) - routed.work_seconds).max(0.0);
    let required = limit as f64 * ctx.check_seconds;
    let mut report = LayoutUnchangedHatchRecovery {
        status: "skipped",
        reason: None,
        native_checks_charged: 0,
        work_seconds: 0.0,
        remaining_work_seconds: available,
        required_work_seconds: required,
        connection_width_errors_before: count,
        connection_width_errors_after: count,
        report: None,
        retry_argv: Vec::new(),
    };
    let board_hash = crate::fs_util::file_sha256(&board)?;
    let saved = fill.as_ref().filter(|fill| {
        fill.status == "saved" && fill.saved_board_sha256.as_deref() == Some(board_hash.as_str())
    });
    let stored = native_report::read_kicad_report(&result.join("drc-saved-fill.json"));
    let Ok(stored) = stored else {
        report.reason = Some("selected_zone_fill_unverified".into());
        return Ok(Some(report));
    };
    if saved.is_none()
        || native_report::validate_drc_report(&refilled, &board).is_err()
        || native_report::validate_drc_report(&stored, &board).is_err()
        || crate::fill_handoff::compare_board_reports(&pcb, &refilled, &refilled, &stored).is_err()
    {
        report.reason = Some("selected_zone_fill_unverified".into());
        return Ok(Some(report));
    }
    let trial = ctx.output_directory.join("unchanged-hatch-width-repair");
    if ctx.wall_passed() || available < required {
        report.reason = Some(
            if ctx.wall_passed() {
                "wall_guard_expired"
            } else {
                "insufficient_remaining_work"
            }
            .into(),
        );
        report.retry_argv = vec![
            "pcb-maker".into(),
            "repair-kicad-necks".into(),
            result.display().to_string(),
            ctx.board_id.into(),
            "--preserve-hatch".into(),
            trial.display().to_string(),
        ];
        return Ok(Some(report));
    }
    let budget = UnchangedHatchBudget {
        max_checks: limit,
        deadline: ctx.router_config.deadline,
        verified_fill: Some(UnchangedHatchBaseline {
            board_sha256: &board_hash,
            refilled: &refilled,
            stored: &stored,
        }),
    };
    let repaired = repair_unchanged_hatch_widths(&result, ctx.board_id, &trial, &budget);
    report.native_checks_charged = repaired
        .as_ref()
        .map_or(limit, |repair| repair.native_checks_charged);
    report.work_seconds = ledger.checks.add(report.native_checks_charged);
    ledger.work += report.work_seconds;
    let repaired = match repaired {
        Ok(repaired) => repaired,
        Err(error) => {
            report.status = "failed";
            report.reason = Some(error);
            return Ok(Some(report));
        }
    };
    if report.native_checks_charged > limit || ctx.wall_passed() {
        report.status = "failed";
        report.reason =
            Some("the unchanged-hatch width repair exceeded its reserved bounds".into());
        return Ok(Some(report));
    }
    let log = Path::new("unchanged-hatch-width-repair.json");
    if trial.join(log).is_file() {
        report.report = Some(Path::new("unchanged-hatch-width-repair").join(log));
    }
    if !repaired.published_improvement() {
        report.status = "no_improvement";
        report.reason = Some(repaired.note);
        return Ok(Some(report));
    }
    if crate::fs_util::file_sha256(&board)? != board_hash {
        report.status = "failed";
        report.reason =
            Some("the selected PCB changed during the unchanged-hatch width repair".into());
        return Ok(Some(report));
    }
    let selected = adopt(
        ctx,
        &result,
        &trial,
        &stored,
        &refilled,
        routed,
        &repaired.added_zone_ids,
    );
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
            format!("could not adopt unchanged-hatch repair: {error}; restore: {restore}")
        })?;
        report.status = "failed";
        report.reason = Some(format!("adopt unchanged-hatch repair: {error}"));
        return Ok(Some(report));
    }
    let _ = fs::remove_dir_all(previous);
    *routed = selected.0;
    *fill = repaired.fill_publication;
    report.connection_width_errors_after = selected.1;
    report.status = "accepted";
    report.report = Some(Path::new("result").join(log));
    eprintln!(
        "layout: unchanged-hatch width repair accepted ({} to {} native width errors, {:.2} modeled seconds)",
        count, report.connection_width_errors_after, report.work_seconds
    );
    Ok(Some(report))
}

fn width_errors(report: &Value) -> impl Iterator<Item = &Value> {
    report["violations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|finding| {
            finding["type"] == "connection_width"
                && finding["severity"] == "error"
                && finding["excluded"] != true
        })
}

fn hatch_has_width_error(pcb: &Expr, report: &Value) -> bool {
    let zones: BTreeSet<_> = pcb
        .children()
        .iter()
        .filter_map(|node| {
            if node.head() != Some("zone")
                || form_atom(node.child("fill")?, "mode", 1) != Some("hatch")
            {
                return None;
            }
            form_atom(node, "uuid", 1).or_else(|| form_atom(node, "tstamp", 1))
        })
        .collect();
    width_errors(report).any(|finding| {
        let Some([zone]) = finding["items"].as_array().map(Vec::as_slice) else {
            return false;
        };
        zone["uuid"].as_str().is_some_and(|id| zones.contains(id))
    })
}

fn verify_same_native_scope(before: &Value, after: &Value) -> Result<(), String> {
    for key in [
        "kicad_version",
        "coordinate_units",
        "included_severities",
        "ignored_checks",
    ] {
        if before[key] != after[key] {
            return Err(format!("the repaired board's native check changed {key}"));
        }
    }
    if pcb_kicad_route::open_repair::report_capped(after)
        || !after["unconnected_items"]
            .as_array()
            .is_some_and(Vec::is_empty)
        || !after["schematic_parity"]
            .as_array()
            .is_some_and(Vec::is_empty)
    {
        return Err(
            "the repaired board needs an uncapped check with zero opens and clean parity".into(),
        );
    }
    if width_errors(after).count() >= width_errors(before).count() {
        return Err("the repaired board did not reduce native connection-width errors".into());
    }
    let old = crate::fill_handoff::keys(before, "violations");
    if crate::fill_handoff::keys(after, "violations")
        .iter()
        .any(|(key, count)| *count > old.get(key).copied().unwrap_or(0))
    {
        return Err("the repaired board added a native finding".into());
    }
    Ok(())
}

fn adopt(
    ctx: &LayoutContext,
    source: &Path,
    trial: &Path,
    stored_before: &Value,
    refill_before: &Value,
    routed: &KiCadBoardRouterResult,
    added_zone_uuids: &[String],
) -> Result<(KiCadBoardRouterResult, usize), String> {
    use crate::fill_handoff::layout::project_inputs;

    // Preserve authored geometry and policy while permitting the repair's
    // explicitly identified new solid copper supplements.
    if project_inputs(source)? != project_inputs(trial)? {
        return Err("the unchanged-hatch repair changed project inputs".into());
    }
    let filename = format!("{}.kicad_pcb", ctx.board_id);
    let candidate = trial.join(&filename);
    let original = crate::parse(
        &fs::read_to_string(source.join(&filename)).map_err(|error| error.to_string())?,
    )?;
    let repaired =
        crate::parse(&fs::read_to_string(&candidate).map_err(|error| error.to_string())?)?;
    unchanged_design(&original, repaired, added_zone_uuids)?;
    let stored = native_report::read_kicad_report(&trial.join("drc-saved-fill.json"))?;
    let refilled = native_report::read_kicad_report(&trial.join("drc.json"))?;
    native_report::validate_drc_report(&stored, &candidate)?;
    native_report::validate_drc_report(&refilled, &candidate)?;
    verify_same_native_scope(stored_before, &stored)?;
    verify_same_native_scope(refill_before, &refilled)?;
    let count = width_errors(&stored).count();
    fs::copy(source.join("erc.json"), trial.join("erc.json")).map_err(|error| error.to_string())?;
    let mut next = routed.clone();
    next.native = Some(verification::record_verified_additive_board(
        trial,
        ctx.board_id,
        true,
    )?);
    next.closed_reuse_drc = None;
    next.model_precedes_additive_repair = true;
    next.saved_copper = Some(pcb_kicad_file::board_stats::inspect_kicad_saved_copper(
        &trial.join(format!("{}.kicad_pcb", ctx.board_id)),
    )?);
    write_typed_json(&trial.join("board-router.json"), &next)?;
    Ok((next, count))
}

fn unchanged_design(
    original: &Expr,
    mut repaired: Expr,
    added_zone_uuids: &[String],
) -> Result<(), String> {
    use crate::fixed_route_comparison::design;
    use pcb_kicad_file::board_stats::is_rule_area;

    let Expr::List(items) = &mut repaired else {
        return Err("the repaired PCB is not a board list".into());
    };
    let mut remaining: BTreeSet<_> = added_zone_uuids.iter().map(String::as_str).collect();
    if remaining.len() != added_zone_uuids.len() {
        return Err("the repaired PCB has repeated supplemental zone identities".into());
    }
    items.retain(|item| {
        let uuid = form_atom(item, "uuid", 1).or_else(|| form_atom(item, "tstamp", 1));
        let solid = item.head() == Some("zone")
            && !is_rule_area(item)
            && item
                .child("fill")
                .is_some_and(|fill| form_atom(fill, "mode", 1).is_none_or(|mode| mode == "solid"));
        !solid || !uuid.is_some_and(|id| remaining.remove(id))
    });
    if !remaining.is_empty() || design(&repaired) != design(original) {
        return Err("the repaired PCB changed authored geometry or hatch policy".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn needs_an_active_zone_width_finding_on_the_authored_hatch() {
        let pcb = crate::parse(
            r#"(kicad_pcb
            (zone (uuid "hatched") (fill yes (mode hatch)))
            (zone (uuid "solid") (fill yes)))"#,
        )
        .unwrap();
        let drc = |id, severity| {
            serde_json::json!({"violations": [{
                "type": "connection_width", "severity": severity, "items":[{"uuid":id}]
            }]})
        };
        assert!(hatch_has_width_error(&pcb, &drc("hatched", "error")));
        assert!(!hatch_has_width_error(&pcb, &drc("solid", "error")));
        assert!(!hatch_has_width_error(&pcb, &drc("hatched", "warning")));
    }

    #[test]
    fn admits_only_declared_supplemental_solid_zones_alongside_unchanged_authored_geometry() {
        let source = crate::parse(
            r#"(kicad_pcb
            (footprint "FP" (at 1 2))
            (zone (uuid "original") (fill yes (mode hatch) (hatch_hole_min_area 0.1))))"#,
        )
        .unwrap();
        let candidate = |fraction: &str, addition: &str| {
            crate::parse(&format!(
                r#"(kicad_pcb
                (footprint "FP" (at 1 2))
                (zone (uuid "original") (fill yes (mode hatch) (hatch_hole_min_area {fraction})))
                (zone (uuid "supplement") (fill yes {addition})))"#
            ))
            .unwrap()
        };
        let allowed = vec!["supplement".into()];
        assert!(unchanged_design(&source, candidate("0.1", ""), &allowed).is_ok());
        assert!(unchanged_design(&source, candidate("0.1", ""), &[]).is_err());
        assert!(unchanged_design(&source, candidate("0.1", "(mode hatch)"), &allowed).is_err());
        assert!(unchanged_design(&source, candidate("0.2", ""), &allowed).is_err());
    }
}
