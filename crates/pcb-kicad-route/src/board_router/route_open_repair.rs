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

/// Route mode's repair after its ladder, on the board kept in `directory`:
/// not in a layout's own ladders (`kicad_kept`: the layout repairs the
/// board it keeps, `board_layout/additive_repair.rs`), a provisional
/// reroute or a run without KiCad; skipped with `tuning.post_route_repairs`
/// off and past the wall guard. Charges its work to the ladder and notes it
/// in `result.open_repair`. Returns whether it ran.
pub(super) fn after_ladder(
    result: &mut KiCadBoardRouterResult,
    directory: &Path,
    board_id: &str,
    config: &KiCadBoardRouterConfig,
    ladder: &mut Ladder<'_>,
) -> Result<bool, String> {
    if config.kicad_kept.is_some()
        || config.provisional_native_check_limit.is_some()
        || config.skip_native_verification
    {
        return Ok(false);
    }
    let repair = if ladder.past_deadline() && config.post_route_repairs_on() {
        result
            .native
            .as_ref()
            .filter(|native| native.selected_net_unconnected_items > 0)
            .map(|native| KiCadRouteOpenRepair::skipped(native, "wall_guard_expired"))
    } else {
        let left = (ladder.ladder_budget - ladder.work).max(0.0);
        repair_route_opens(result, directory, board_id, config, left)?
    };
    let Some(repair) = repair else {
        return Ok(false);
    };
    ladder.work += repair.search_work_seconds;
    ladder.charged.search_seconds += repair.search_work_seconds;
    ladder.charge_checks(repair.native_checks);
    eprintln!("{}", repair.line());
    result.open_repair = Some(repair);
    Ok(true)
}

/// Runs the repair on the board kept in `directory` (the run's staging
/// directory) when KiCad finds connections open there, and adopts the
/// repaired project in its place. With `tuning.post_route_repairs` off it
/// is skipped (`post_route_repairs_off`): no search, no KiCad check, the
/// board untouched. Returns the search work charged.
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
    // A repair from KiCad's report (docs/router.md, "Repairs from KiCad's
    // report").
    if !config.post_route_repairs_on() {
        return Ok(result
            .native
            .as_ref()
            .map(|native| KiCadRouteOpenRepair::skipped(native, "post_route_repairs_off")));
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_open_net_is_routed_alone_and_adopted_with_kicads_evidence() {
        if !std::process::Command::new(native_report::kicad_cli())
            .arg("version")
            .output()
            .is_ok_and(|output| output.status.success())
        {
            return;
        }
        let root = temporary_next_to(&std::env::temp_dir().join("route-open-repair"));
        let directory = root.join("route");
        fs::create_dir_all(&directory).unwrap();
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
        fs::write(directory.join("board.kicad_pcb"), pcb).unwrap();
        fs::write(directory.join("board.kicad_pro"), "{}\n").unwrap();
        let config = crate::resolve_project_rules(
            &directory.join("board.kicad_pro"),
            &directory.join("board.kicad_pcb"),
        )
        .unwrap();
        let native = verify_materialized_rung(&directory, "board").unwrap();
        assert_eq!(native.selected_net_unconnected_items, 1);
        let mut result = KiCadBoardRouterResult {
            native: Some(native),
            ..Default::default()
        };
        // With `tuning.post_route_repairs` false the repair is skipped with
        // its reason: no search, no KiCad check, the board untouched.
        let mut off = config.clone();
        off.tuning.post_route_repairs = Some(false);
        let skipped = repair_route_opens(&mut result, &directory, "board", &off, 0.0)
            .unwrap()
            .unwrap();
        assert_eq!(skipped.status, "skipped");
        assert_eq!(skipped.reason.as_deref(), Some("post_route_repairs_off"));
        assert_eq!((skipped.opens_before, skipped.opens_after), (1, 1));
        assert_eq!(
            (skipped.native_checks, skipped.search_work_seconds),
            (0, 0.0)
        );
        assert!(!result.model_precedes_additive_repair);
        assert!(result.saved_copper.is_none());
        assert_eq!(
            fs::read_to_string(directory.join("board.kicad_pcb")).unwrap(),
            pcb
        );
        assert!(!directory.join("open-repair.json").exists());
        // No work left: the floor still searches.
        let repair = repair_route_opens(&mut result, &directory, "board", &config, 0.0)
            .unwrap()
            .unwrap();
        assert_eq!(repair.status, "accepted", "{repair:?}");
        assert_eq!((repair.opens_before, repair.opens_after), (1, 0));
        assert_eq!(repair.nets, ["LINK"]);
        assert!(repair.native_checks >= 2);
        assert!(repair.search_work_seconds > 0.0);
        assert!(result.model_precedes_additive_repair);
        assert!(result.native.as_ref().unwrap().complete);
        assert!(result.saved_copper.as_ref().unwrap().segments > 0);
        assert!(directory.join("open-repair.json").is_file());
        let disk = fs::read_to_string(directory.join("board.kicad_pcb")).unwrap();
        assert!(disk.starts_with(pcb.strip_suffix(')').unwrap()));
        // A board KiCad finds connected has nothing to repair.
        assert!(
            repair_route_opens(&mut result, &directory, "board", &config, 0.0)
                .unwrap()
                .is_none()
        );
        fs::remove_dir_all(root).unwrap();
    }
}
