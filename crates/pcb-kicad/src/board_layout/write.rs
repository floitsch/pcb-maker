// Copyright (C) 2026 Toit contributors.

//! The kept routes written: the pairs' links finished, the labels and
//! copper texts placed, the lengths tuned, the copper emitted and the
//! board written to `result/` and checked by KiCad.

use super::candidates::Kept;
use super::placement::Copper;
use super::setup::{LayoutContext, WorkLedger};
use super::*;

/// The kept routes into the board written to `result/`: the pairs'
/// links finished (`finish_pairs`; the board from here on is the board as
/// lowered), the labels and copper texts placed on `kept.pcb`, the
/// lengths tuned, the copper emitted, the board written and checked (the
/// check charged), the pairs and the pour mode reported. Hands on the
/// route flow's result for the board kept and the label report.
#[allow(
    clippy::too_many_arguments,
    reason = "the layout's state and the first route's clocks are threaded through"
)]
pub(super) fn write_kept_board(
    ctx: &LayoutContext,
    ledger: &mut WorkLedger,
    kept: &mut Kept,
    result: &mut core::RoutingResult,
    copper: &Copper,
    attached_marks: &[String],
    first_route_seconds: f64,
    move_started: std::time::Instant,
) -> Result<(KiCadBoardRouterResult, crate::labels::KiCadLabelReport), String> {
    let router_config = &ctx.router_config;
    let layer_names = &copper.layers.names;
    // The pairs' links into the pairs' copper (`finish_pairs`), as route
    // mode writes an attempt: what a link left open is routed for the
    // pair's nets with everything else kept (the fallback), with its share
    // of the layout's work left (`work_budget`, its minimum where the
    // budget is spent: then the facts say so), its work the layout's. The
    // board from here on is the board as lowered.
    let pairs_left = ledger.left(ctx.work_deadline).max(0.0);
    let (pair_copper, pairs_report) = if crate::board_router::pairs_planned(router_config) {
        let heights = crate::board_router::copper_heights(&kept.pcb);
        let heights: Vec<f64> = layer_names
            .iter()
            .map(|name| heights.get(name).copied().unwrap_or(0.0))
            .collect();
        let mut pairs_config = router_config.clone();
        pairs_config.ladder_budget_seconds = Some(pairs_left);
        match crate::board_router::finish_pairs(&kept.board, result, &pairs_config, &heights) {
            Some((restored, copper, report)) => {
                kept.board = restored;
                ledger.work += report.work_seconds;
                (Some(copper), Some(report))
            }
            None => (None, None),
        }
    } else {
        (None, None)
    };
    // Fixed parts lay where they are in the input, marks and all (the
    // placement warned of them): only the parts placed are reported.
    let fixed = fixed_parts(&kept.problem);
    let mut labels = place_part_labels(ctx, &mut kept.pcb, &fixed, attached_marks)?;
    // Copper texts: kept, or else (decoration on copper) moved off the
    // copper and the parts.
    // (The pairs' copper is copper the texts keep off too.)
    let with_pair_copper;
    let copper_routes = match &pair_copper {
        Some(copper) => {
            with_pair_copper = result
                .routes
                .iter()
                .cloned()
                .chain(copper.routes())
                .collect::<Vec<_>>();
            &with_pair_copper
        }
        None => &result.routes,
    };
    place_copper_texts(
        ctx,
        &mut kept.pcb,
        copper_routes,
        layer_names,
        &fixed,
        &mut labels,
    )?;
    // The length and skew rules (`length_matching`): the meanders the
    // ladder's attempts get (`match_lengths`), on the layout's own routes
    // too, which it keeps when no ladder runs (link: the route took the
    // whole budget, its rule reported unmodelled and two nets short).
    // The meanders join the routes, so that the final route from the
    // router's state keeps them with the nets' copper.
    // (The pairs' coupled nets are the pairs' own, as in route mode.)
    let length_config = crate::board_router::with_lengths(router_config);
    let coupled: BTreeSet<String> = pairs_report
        .iter()
        .flat_map(|report| report.pairs.iter())
        .filter(|pair| pair.links_coupled > 0)
        .flat_map(|pair| [pair.positive.clone(), pair.negative.clone()])
        .collect();
    let (patterns, lengths) = crate::board_router::finish_lengths(
        &kept.pcb,
        &kept.board,
        result,
        router_config,
        layer_names,
        &coupled,
    );
    let nets = emit_routes(&mut kept.pcb, &kept.board, result, layer_names, false)?;
    // The tuning patterns and the pairs' copper go into the board written
    // only: `pcb` is the placement the ladder routes again, without copper.
    let written;
    let written_pcb = if patterns.is_empty() && pair_copper.is_none() {
        &kept.pcb
    } else {
        let mut copy = kept.pcb.clone();
        if let Some(copper) = &pair_copper {
            crate::board_router::write_pair_copper(&mut copy, copper, layer_names)?;
        }
        crate::board_router::write_length_patterns(&mut copy, &kept.board, &patterns, layer_names)?;
        written = copy;
        &written
    };
    let mut routed = finish_routed_board(
        written_pcb,
        &kept.board,
        result,
        nets,
        layer_names,
        &ctx.placed_directory,
        ctx.board_id,
        &ctx.output_directory.join("result"),
        &length_config,
        [
            0.0,
            first_route_seconds + move_started.elapsed().as_secs_f64(),
        ],
    )?;
    // The length reroute's work (`length_reroute`) counts as the layout's.
    ledger.work += lengths
        .iter()
        .map(|entry| entry.reroute_work_seconds)
        .sum::<f64>();
    routed.lengths = lengths;
    report_pairs(ctx, &mut routed, pairs_report, written_pcb, pairs_left);
    routed.pours = if !copper.has_pours {
        "none"
    } else if copper.connect {
        "connect"
    } else {
        "tracks"
    }
    .into();
    // KiCad's check of the board kept counts as work (`check_work`).
    if routed.native.is_some() {
        ledger.work += ledger.checks.add(1);
    }
    Ok((routed, labels))
}

/// The references of the fixed parts.
fn fixed_parts(problem: &crate::board_placer::LoweredPlacement) -> BTreeSet<String> {
    problem
        .references
        .iter()
        .zip(&problem.problem.components)
        .filter(|(_, component)| component.fixed)
        .map(|(reference, _)| reference.clone())
        .collect()
}

/// Reference labels off pads and other silkscreen; the board's own texts
/// and silkscreen graphics stay where the input put them
/// (`keep_board_marks`), and the parts on them are reported (the `fixed`
/// parts' not).
fn place_part_labels(
    ctx: &LayoutContext,
    pcb: &mut Expr,
    fixed: &BTreeSet<String>,
    attached_marks: &[String],
) -> Result<crate::labels::KiCadLabelReport, String> {
    let field_options = crate::silk_fields::FieldOptions {
        rules: crate::silk_fields::SilkRules::from_project(&ctx.project_file()),
        shrink: true,
        hide_when_blocked: ctx.config.silkscreen.hide_when_blocked,
    };
    crate::mem_profile::phase("labels and the board kept");
    let labels = crate::labels::place_labels(
        pcb,
        ctx.config.placer.keep_board_marks,
        fixed,
        attached_marks,
        field_options,
    )?;
    if !labels.stuck.is_empty() {
        eprintln!("labels without a free spot: {:?}", labels.stuck);
    }
    Ok(labels)
}

/// Copper texts kept, or else moved off the copper (`copper_routes`) and
/// the parts; their overlaps join the label report's, every overlap
/// logged.
fn place_copper_texts(
    ctx: &LayoutContext,
    pcb: &mut Expr,
    copper_routes: &[core::NetRoute],
    layer_names: &[String],
    fixed: &BTreeSet<String>,
    labels: &mut crate::labels::KiCadLabelReport,
) -> Result<(), String> {
    let copper_texts = crate::labels::place_copper_texts(
        pcb,
        copper_routes,
        layer_names,
        ctx.config.placer.keep_board_marks,
        fixed,
    )?;
    labels
        .overlaps
        .extend(copper_texts.overlaps.iter().cloned());
    for overlap in &labels.overlaps {
        eprintln!("board mark under parts: {overlap}");
    }
    if copper_texts.moved > 0 || !copper_texts.stuck.is_empty() {
        eprintln!(
            "copper texts: {} moved, {} without a free spot",
            copper_texts.moved,
            copper_texts.stuck.len()
        );
    }
    Ok(())
}

/// The pairs as the board written has them, with the setting (the
/// verdict's `diff_pairs`: filled whenever the setting is on, so that it
/// never reads as off), logged; where the fallback had only its minimum
/// (`pairs_left` the work it was planned with), the board says so.
fn report_pairs(
    ctx: &LayoutContext,
    routed: &mut KiCadBoardRouterResult,
    mut pairs_report: Option<crate::KiCadDiffPairs>,
    written_pcb: &Expr,
    pairs_left: f64,
) {
    let choice = ctx.router_config.diff_pairs_choice();
    if choice.on {
        let report = pairs_report.get_or_insert_with(crate::KiCadDiffPairs::default);
        report.on = choice.on;
        report.default = choice.default.to_string();
        report.reason = choice.reason;
        crate::board_router::measure_uncoupled(report, written_pcb);
        let fallen = report.fallen_back + report.partly_coupled;
        eprintln!(
            "layout: diff pairs on the layout's own routes: {} found, {} coupled, {} partly, {} fallen back ({} of {} links coupled; fallback {:.1} s of work)",
            report.found,
            report.coupled,
            report.partly_coupled,
            report.fallen_back,
            report.links_coupled,
            report.links,
            report.work_seconds
        );
        // The fallback at its minimum: the layout's budget was spent.
        if fallen > 0 && crate::board_router::pairs_fallback_short(pairs_left) {
            routed.skipped.push(format!(
                "the differential pairs' fallback (the nets of the {fallen} pairs not wholly coupled) had only {:.0} s of work: the layout's budget was spent ({pairs_left:.0} s left); a larger total_seconds in the layout config gives it its share",
                crate::board_router::pairs_fallback_work(pairs_left)
            ));
        }
    }
    routed.diff_pairs = pairs_report;
}
