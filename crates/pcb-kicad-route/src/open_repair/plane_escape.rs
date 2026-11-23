// Copyright (C) 2026 Florian Loitsch.

//! Retry a plane-pad open with compatible early access for two nearby pads.

use super::*;
pub(super) mod geometry;
pub(crate) use self::geometry::legacy_escape_segment;
use self::geometry::{open_pad_ids, pads, plan};
use pcb_router::Point;

#[derive(Clone, Debug, Serialize)]
pub struct PlaneEscapeRepair {
    pub status: &'static str,
    pub reason: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub opens_before: usize,
    pub opens_after: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trial_refilled_opens: Option<usize>,
    pub work_seconds: f64,
    pub budget_seconds: f64,
    pub route_work_seconds: f64,
    pub route_native_checks_charged: usize,
    #[serde(skip)]
    pub routed: Option<KiCadBoardRouterResult>,
    pub native_raw_errors_before: Option<usize>,
    pub native_raw_errors_after: Option<usize>,
    /// Refill is used for the acceptance comparison; store verified fill
    /// through the normal layout output or the separate fill handoff.
    pub native_refill: bool,
    pub planning_probes: u64,
    pub native_checks: usize,
    pub escapes: Vec<ProtectedEscape>,
}

impl PlaneEscapeRepair {
    pub fn skipped(budget: f64, reason: &'static str, detail: Option<String>) -> Self {
        Self {
            status: "skipped",
            reason,
            detail,
            opens_before: 0,
            opens_after: 0,
            trial_refilled_opens: None,
            work_seconds: 0.0,
            budget_seconds: budget,
            route_work_seconds: 0.0,
            route_native_checks_charged: 0,
            routed: None,
            native_raw_errors_before: None,
            native_raw_errors_after: None,
            native_refill: true,
            planning_probes: 0,
            native_checks: 0,
            escapes: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ProtectedEscape {
    pub terminal: String,
    pub net: String,
    pub layer: String,
    pub path_mm: Vec<Point>,
    pub via_mm: Point,
}

/// Whether the current native report names a surface pad on a plane net.
/// Full geometry and compatibility are checked only when a retry runs.
pub fn has_plane_escape_open(directory: &Path, board_id: &str) -> bool {
    let check = || -> Option<bool> {
        let (_, board) = read_board(&directory.join(format!("{board_id}.kicad_pcb"))).ok()?;
        let report = read_json(&directory.join("drc.json")).ok()?;
        if open_count(&report) != 1 || !report["schematic_parity"].as_array()?.is_empty() {
            return Some(false);
        }
        let open = open_pad_ids(&report);
        let layers = board_router::LayerTable::from_pcb(&board).ok()?;
        let plane_nets: BTreeSet<_> = board_router::pours(&board, &layers)
            .ok()?
            .into_iter()
            .map(|pour| pour.net)
            .collect();
        let pads = pads(&board);
        Some(pads.iter().any(|pad| {
            open.contains(&pad.id)
                && plane_nets.contains(&pad.net)
                && pads.iter().any(|neighbor| {
                    neighbor.reference == pad.reference
                        && neighbor.net != pad.net
                        && plane_nets.contains(&neighbor.net)
                })
        }))
    };
    check().unwrap_or(false)
}

pub fn repair_kicad_plane_escape(
    routed: &Path,
    placed: &Path,
    board_id: &str,
    output: &Path,
    config: &KiCadBoardRouterConfig,
) -> Result<PlaneEscapeRepair, String> {
    check_board_id(board_id).map_err(exit::input_error)?;
    let board_name = format!("{board_id}.kicad_pcb");
    let (original_text, original) = read_board(&routed.join(&board_name))?;
    let (placed_text, initial) = read_board(&placed.join(&board_name))?;
    let budget = config
        .ladder_budget_seconds
        .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
        .ok_or_else(|| {
            exit::input_error("plane escape repair needs a positive ladder_budget_seconds")
        })?;
    if config.skip_native_verification
        || config.existing_tracks != KiCadExistingTracks::Keep
        || config.pours == KiCadPourMode::Tracks
        || config.pin_swaps.is_some()
        || !config.add_pours.is_empty()
        || !config.net_classes.is_empty()
        || config.tuning.engine.as_deref() == Some("topological")
    {
        return Err(exit::input_error(
            "plane escape repair needs native checks, retained tracks, and the placed project's existing pins, rules and zones",
        ));
    }
    if initial.children().iter().any(route_item) {
        return Err(exit::input_error(
            "plane escape repair needs the original copper-free placed board; a whole reroute would otherwise replace existing authored routes",
        ));
    }
    if snapshot(&initial) != snapshot(&original) {
        return Err(exit::input_error(
            "the placed and routed boards differ outside generated copper or zone fill",
        ));
    }
    // Always route and check in copies of the *same* current project. An
    // alternate donor board cannot bring its schematic, libraries or rules.
    let schematic = routed.join(format!("{board_id}.kicad_sch"));
    let parity = schematic.is_file();
    if !parity {
        return Err(exit::input_error(
            "plane escape repair needs the source KiCad schematic for parity",
        ));
    }
    let native_cost = check_work::check_seconds(check_work::check_inputs(&initial));
    in_repair_copy(routed, output, |directory| {
        let source = directory.join(&board_name);
        let before_path = directory.join("drc-before-plane-escape.json");
        let mut report = PlaneEscapeRepair::skipped(budget, "budget_too_small", None);
        if budget > 2.0 * native_cost {
            report.native_checks += 1;
            report.work_seconds += native_cost;
            let before = native_check(&source, &before_path, parity, true)?;
            report.native_raw_errors_before = Some(native_error_count(&before));
            report.opens_before = open_count(&before);
            report.opens_after = report.opens_before;
            if !unchanged(&source, &original_text)? {
                return Err("KiCad rewrote the baseline board during its check".into());
            }
            report.reason = if report_capped(&before) {
                "capped_baseline_report"
            } else if before["coordinate_units"] != "mm" {
                "unsupported_native_units"
            } else if !before["schematic_parity"]
                .as_array()
                .is_some_and(Vec::is_empty)
            {
                "baseline_schematic_parity"
            } else if report.opens_before == 0 {
                "already_closed"
            } else {
                "native_open_to_check"
            };
            if report.reason == "native_open_to_check" {
                report.status = "no_improvement";
                trial_in_copy(
                    directory,
                    board_id,
                    &board_name,
                    &initial,
                    &placed_text,
                    &original,
                    &before,
                    config,
                    native_cost,
                    &mut report,
                )?;
            }
        }
        let json = serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?;
        fs::write(directory.join("plane-escape.json"), json).map_err(|error| error.to_string())?;
        Ok(report)
    })
}

#[allow(clippy::too_many_arguments)]
fn trial_in_copy(
    directory: &Path,
    board_id: &str,
    board_name: &str,
    initial: &Expr,
    placed_text: &str,
    original: &Expr,
    before: &Value,
    config: &KiCadBoardRouterConfig,
    native_cost: f64,
    report: &mut PlaneEscapeRepair,
) -> Result<(), String> {
    let (escapes, pieces, probes) = plan(initial, original, before, config)?;
    report.planning_probes = probes;
    let rate = work_rate::work_rate().expansions_per_second;
    report.work_seconds += probes as f64 / rate;
    report.escapes = escapes;
    if pieces.is_empty() {
        report.reason = "no_compatible_escape_pair";
        return Ok(());
    }
    // The normal route driver owns its own soft ladder budget. Reserve the
    // two independent all-track comparisons here and verify actual work
    // again before publishing, even if a ladder overran its last stage.
    let route_allowance = report.budget_seconds - report.work_seconds - 2.0 * native_cost;
    if route_allowance <= 0.0 {
        report.reason = "budget_too_small";
        return Ok(());
    }
    let scratch = temporary_next_to(directory);
    let result = (|| {
        fs::create_dir(&scratch).map_err(|error| error.to_string())?;
        let input = scratch.join("input");
        copy_directory_tree(directory, &input)?;
        let seeded = input.join(board_name);
        write_additions(&seeded, placed_text, &pieces)?;
        let seeded_text = fs::read_to_string(&seeded).map_err(|error| error.to_string())?;
        let seed_path = directory.join("drc-plane-escape-seed.json");
        report.native_checks += 1;
        report.work_seconds += native_cost;
        let seeded_drc = native_check(&seeded, &seed_path, true, true)?;
        // The placed board still has open nets, including plane nets with
        // other bare pads. Check for new electrical or rule errors here.
        // The final comparison requires all native opens to close.
        if !unchanged(&seeded, &seeded_text)?
            || !native_errors_preserved(before, &seeded_drc)?
            || !seeded_drc["schematic_parity"]
                .as_array()
                .is_some_and(Vec::is_empty)
        {
            report.reason = "native_check_rejected_seed";
            return Ok(());
        }
        let mut attempt = config.clone();
        attempt.ladder_budget_seconds = Some(route_allowance);
        attempt.deadline_work = None;
        attempt.ladder_work_ceiling = Some(route_allowance);
        let trial = scratch.join("route");
        let result = match route_kicad_board(&input, board_id, &trial, &attempt) {
            Ok(result) => result,
            Err(error) => {
                // This driver cannot return partial accounting on an IO
                // error. Retire the whole trial slice conservatively.
                report.route_work_seconds = route_allowance;
                report.work_seconds += route_allowance;
                return Err(error);
            }
        };
        report.route_work_seconds = result.work_seconds;
        report.route_native_checks_charged = result.kicad_checks_charged;
        report.work_seconds += result.work_seconds;
        if report.work_seconds + native_cost > report.budget_seconds {
            report.reason = "trial_used_remaining_budget";
            return Ok(());
        }
        let trial_board = trial.join(board_name);
        let (text, pcb) = read_board(&trial_board)?;
        if snapshot(&pcb) != snapshot(original) {
            report.reason = "trial_changed_placement_or_rules";
            return Ok(());
        }
        let after_path = directory.join("drc-after-plane-escape.json");
        report.native_checks += 1;
        report.work_seconds += native_cost;
        let after = native_check(&trial_board, &after_path, true, true)?;
        report.trial_refilled_opens = Some(open_count(&after));
        report.native_raw_errors_after = Some(native_error_count(&after));
        if result.native.is_none() || !unchanged(&trial_board, &text)? {
            report.reason = "trial_native_attestation_failed";
            return Ok(());
        }
        if let Some(reason) = rejection(before, &after)? {
            report.reason = reason;
            return Ok(());
        }
        // A complete recovery has no remaining opens, including any
        // that the new route might otherwise move to a different net.
        native_report::remove_stale_board_run_reports(directory)?;
        for name in [
            "board-router.json",
            "drc.json",
            "erc.json",
            "verification.json",
            "preview.json",
            "preview.svg",
            "preview-labels.json",
        ] {
            let path = trial.join(name);
            if path.is_file() {
                fs::copy(path, directory.join(name)).map_err(|error| error.to_string())?;
            }
        }
        fs::write(directory.join(board_name), text).map_err(|error| error.to_string())?;
        report.routed = Some(result);
        report.status = "accepted";
        report.reason = "all_opens_closed_without_new_native_findings";
        report.opens_after = open_count(&after);
        Ok(())
    })();
    let _ = fs::remove_dir_all(&scratch);
    result
}

fn unchanged(board: &Path, expected: &str) -> Result<bool, String> {
    Ok(fs::read_to_string(board).map_err(|error| error.to_string())? == expected)
}

pub(super) fn open_count(report: &Value) -> usize {
    report["unconnected_items"].as_array().map_or(0, Vec::len)
}

fn native_error_count(report: &Value) -> usize {
    report["violations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|finding| finding["severity"] == "error")
        .count()
}

pub(super) fn rejection(before: &Value, after: &Value) -> Result<Option<&'static str>, String> {
    if open_count(before) == 0 || open_count(after) != 0 {
        return Ok(Some("trial_has_native_opens"));
    }
    if report_capped(after) || !finding_types_at_cap(after, "schematic_parity").is_empty() {
        return Ok(Some("capped_trial_report"));
    }
    if !after["schematic_parity"]
        .as_array()
        .is_some_and(Vec::is_empty)
    {
        return Ok(Some("trial_schematic_parity"));
    }
    Ok((!preserved(before, after)?).then_some("trial_added_native_findings"))
}

fn native_errors_preserved(before: &Value, after: &Value) -> Result<bool, String> {
    let errors = |report: &Value| {
        let mut report = report.clone();
        if let Some(findings) = report["violations"].as_array_mut() {
            findings.retain(|finding| finding["severity"] == "error");
        }
        report
    };
    Ok(before["kicad_version"] == after["kicad_version"]
        && before["ignored_checks"] == after["ignored_checks"]
        && before["included_severities"] == after["included_severities"]
        && finding_types_at_cap(after, "violations").is_empty()
        && !adds_findings(&errors(before), &errors(after), "violations")?)
}

pub(super) fn snapshot(pcb: &Expr) -> Vec<String> {
    fn scrub(node: &Expr) -> Expr {
        if let Expr::List(forms) = node {
            let forms = forms
                .iter()
                .filter(|form| {
                    node.head() != Some("zone")
                        || !matches!(
                            form.head(),
                            Some("filled_polygon" | "fill_segments" | "filled_areas_thickness")
                        )
                })
                .map(scrub)
                .collect();
            Expr::List(forms)
        } else {
            node.clone()
        }
    }
    pcb.children()
        .iter()
        .filter(|item| !route_item(item))
        .map(|item| encode(&scrub(item)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(opens: &[Value], extra: &[Value]) -> Value {
        serde_json::json!({
            "coordinate_units": "mm",
            "kicad_version": "10.0.8",
            "included_severities": ["error", "warning", "exclusion"],
            "ignored_checks": [],
            "unconnected_items": opens,
            "schematic_parity": [],
            "violations": extra
        })
    }

    #[test]
    fn native_acceptance_requires_all_opens_to_close_without_new_findings() {
        let open = serde_json::json!({"type": "unconnected_items", "items": ["pad"]});
        let inherited = serde_json::json!({"type": "annular_width", "severity": "error"});
        let before = report(&[open], std::slice::from_ref(&inherited));
        let success = report(&[], std::slice::from_ref(&inherited));
        assert_eq!(rejection(&before, &success).unwrap(), None);
        assert_eq!(
            rejection(&before, &before).unwrap(),
            Some("trial_has_native_opens")
        );
        let warning = serde_json::json!({"type":"isolated_copper","severity":"warning"});
        assert_eq!(
            rejection(&before, &report(&[], &[inherited, warning])).unwrap(),
            Some("trial_added_native_findings")
        );
        let mut parity = success;
        parity["schematic_parity"] = serde_json::json!([{"type":"missing_footprint"}]);
        assert_eq!(
            rejection(&before, &parity).unwrap(),
            Some("trial_schematic_parity")
        );
    }

    #[test]
    fn placement_identity_keeps_mechanical_geometry_and_omits_only_route_or_fill() {
        let placed = parse(
            r#"(kicad_pcb (footprint "f" (at 2 3))
          (zone (net "GND") (layer "In1.Cu") (polygon (pts (xy 0 0) (xy 3 3)))))"#,
        )
        .unwrap();
        let routed = parse(
            r#"(kicad_pcb (footprint "f" (at 2 3))
          (segment (start 2 3) (end 2 4))
          (zone (net "GND") (layer "In1.Cu") (polygon (pts (xy 0 0) (xy 3 3)))
                (filled_polygon (pts (xy 1 1) (xy 2 2)))))"#,
        )
        .unwrap();
        let moved = parse(
            r#"(kicad_pcb (footprint "f" (at 2 4))
          (zone (net "GND") (layer "In1.Cu") (polygon (pts (xy 0 0) (xy 3 3)))))"#,
        )
        .unwrap();
        assert_eq!(snapshot(&placed), snapshot(&routed));
        assert_ne!(snapshot(&placed), snapshot(&moved));
    }
}
