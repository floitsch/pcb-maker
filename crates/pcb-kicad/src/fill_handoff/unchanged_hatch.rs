// Copyright (C) 2026 Florian Loitsch.

//! Native-checked narrow bridges and fill patches around a preserved hatch.

use super::{FillPublication, fs_util, keys, layout, native_report, same_metadata};
use pcb_kicad_file::{
    copper_emit::{SupplementalSegment, supplemental_segment_expr, supplemental_segment_id},
    kicad_nets::note_net_names,
    kicad_sexpr::{Expr, deterministic_uuid_for, encode, form_atom, form_f64, parse, quote},
};
use pcb_kicad_route::{
    necks::{hatched_neck_target, widen_recorded_neck_tracks},
    open_repair::{in_repair_copy, preserved, report_capped},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    time::Instant,
};

/// Scratch witness DRC and two native geometry reads, one trial DRC with
/// saved refill, and one reopen.
pub(crate) const UNCHANGED_HATCH_CHECKS_WITH_BASELINE: usize = 5;
const STANDALONE_CHECKS: usize = UNCHANGED_HATCH_CHECKS_WITH_BASELINE + 2;
const REPORT: &str = "unchanged-hatch-width-repair.json";

/// In-process evidence that the caller's exact saved fill passed both modes.
pub(crate) struct UnchangedHatchBaseline<'a> {
    pub board_sha256: &'a str,
    pub refilled: &'a Value,
    pub stored: &'a Value,
}

pub(crate) struct UnchangedHatchBudget<'a> {
    pub max_checks: usize,
    pub deadline: Option<Instant>,
    pub verified_fill: Option<UnchangedHatchBaseline<'a>>,
}

#[derive(Default, Serialize)]
pub struct UnchangedHatchRepair {
    pub before: usize,
    pub after: usize,
    pub native_checks_charged: usize,
    pub schematic_parity_checked: bool,
    pub note: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zone_uuid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preserved_hatch_fraction: Option<f64>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub widened_tracks: BTreeMap<String, f64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub added_track_ids: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub added_zone_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill_publication: Option<FillPublication>,
}

impl UnchangedHatchRepair {
    pub fn published_improvement(&self) -> bool {
        self.after < self.before
            && self
                .fill_publication
                .as_ref()
                .is_some_and(|fill| fill.status == "saved")
    }
}

pub fn repair_kicad_unchanged_hatch_widths(
    source: &Path,
    board_id: &str,
    output: &Path,
) -> Result<UnchangedHatchRepair, String> {
    repair_unchanged_hatch_widths(
        source,
        board_id,
        output,
        &UnchangedHatchBudget {
            max_checks: STANDALONE_CHECKS,
            deadline: None,
            verified_fill: None,
        },
    )
}

pub(crate) fn repair_unchanged_hatch_widths(
    source: &Path,
    board_id: &str,
    output: &Path,
    budget: &UnchangedHatchBudget<'_>,
) -> Result<UnchangedHatchRepair, String> {
    fs_util::check_board_id(board_id)?;
    in_repair_copy(source, output, |stage| {
        let board = stage.join(format!("{board_id}.kicad_pcb"));
        let original = fs::read_to_string(&board).map_err(|error| error.to_string())?;
        let reports = ["drc.json", "drc-saved-fill.json"]
            .map(|name| stage.join(name))
            .into_iter()
            .map(|path| {
                let bytes = match fs::read(&path) {
                    Ok(bytes) => Some(bytes),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                    Err(error) => return Err(error.to_string()),
                };
                Ok((path, bytes))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let mut report = UnchangedHatchRepair::default();
        if let Err(reason) = attempt(stage, board_id, &board, &original, budget, &mut report) {
            if fs::read_to_string(&board).map_err(|error| error.to_string())? != original {
                fs_util::write_atomically(&board, original.as_bytes())?;
                for (path, bytes) in reports {
                    match bytes {
                        Some(bytes) => fs_util::write_atomically(&path, &bytes)?,
                        None => native_report::remove_if_present(&path)?,
                    }
                }
            }
            report.note = reason;
        }
        fs_util::write_typed_json(&stage.join(REPORT), &report)?;
        Ok(report)
    })
}

fn active_width(finding: &Value) -> bool {
    finding["type"] == "connection_width"
        && finding["severity"] == "error"
        && finding["excluded"] != true
}

fn width_count(report: &Value) -> usize {
    report["violations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|finding| active_width(finding))
        .count()
}

fn complete_closed_report(report: &Value) -> bool {
    !report_capped(report)
        && ["unconnected_items", "schematic_parity"]
            .iter()
            .all(|field| report[*field].as_array().is_some_and(Vec::is_empty))
}

fn only_width_improves(before: &Value, trial: &Value) -> Result<bool, String> {
    same_metadata(before, before, trial)?;
    if !preserved(before, trial)? || width_count(trial) >= width_count(before) {
        return Ok(false);
    }
    let without_active_widths = |report: &Value| {
        let mut report = report.clone();
        if let Some(violations) = report["violations"].as_array_mut() {
            violations.retain(|row| !active_width(row));
        }
        keys(&report, "violations")
    };
    Ok(
        without_active_widths(before) == without_active_widths(trial)
            && keys(before, "unconnected_items") == keys(trial, "unconnected_items")
            && keys(before, "schematic_parity") == keys(trial, "schematic_parity"),
    )
}

fn minimum_width(finding: &Value) -> Option<f64> {
    let width = finding["description"]
        .as_str()?
        .split_once("minimum connection width ")?
        .1
        .split_whitespace()
        .next()?
        .parse::<f64>()
        .ok()?;
    // This repair is for a local pinch. A large region needs a designer's
    // zone edit, even when its width is covered by the same native check.
    (width > 0.0 && width <= 0.5).then_some(width)
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Bridge {
    Track {
        start_nm: [i64; 2],
        end_nm: [i64; 2],
        width_nm: i64,
    },
    SolidPatch {
        patch_nm: [[i64; 2]; 4],
        clearance_nm: i64,
    },
}

struct BridgeTarget<'a> {
    location: &'a native_report::ZoneWidthLocation,
    minimum: f64,
}

fn append_bridges(
    pcb: &mut Expr,
    zone_id: &str,
    targets: &[BridgeTarget<'_>],
    bridges: Vec<Option<Bridge>>,
    original: &str,
) -> Result<(Vec<String>, Vec<String>), String> {
    let zone = pcb
        .children()
        .iter()
        .find(|item| item.head() == Some("zone") && form_atom(item, "uuid", 1) == Some(zone_id))
        .ok_or("native hatch zone no longer matches the source")?;
    let priority = form_atom(zone, "priority", 1)
        .unwrap_or("0")
        .parse::<u32>()
        .ok()
        .and_then(|value| value.checked_add(1))
        .ok_or("the hatch zone priority does not allow a supplemental patch")?;
    let fill = zone
        .child("fill")
        .ok_or("the hatch zone has no fill policy")?;
    let thermal_gap = form_f64(fill, "thermal_gap", 1)?;
    let thermal_width = form_f64(fill, "thermal_bridge_width", 1)?;
    let mm = |nm| nm as f64 / 1_000_000.0;
    note_net_names(pcb);
    let Expr::List(items) = pcb else {
        return Err("missing source PCB forms".into());
    };
    let mut added_tracks = Vec::new();
    let mut added_zones = Vec::new();
    let mut added_ids = BTreeSet::new();
    for (target, bridge) in targets.iter().zip(bridges) {
        let location = target.location;
        let (id, item, ids) = match bridge {
            Some(Bridge::Track {
                start_nm,
                end_nm,
                width_nm,
            }) => {
                let segment = SupplementalSegment {
                    connection: location.net.clone(),
                    start: start_nm.map(mm),
                    end: end_nm.map(mm),
                    width: mm(width_nm),
                    layer: location.layer.clone(),
                };
                (
                    supplemental_segment_id(&segment),
                    supplemental_segment_expr(&segment),
                    &mut added_tracks,
                )
            }
            Some(Bridge::SolidPatch {
                patch_nm,
                clearance_nm,
            }) => {
                let id = deterministic_uuid_for(&format!(
                    "hatch width patch:{zone_id}:{}:{patch_nm:?}",
                    location.layer
                ));
                let xy = patch_nm
                    .into_iter()
                    .map(|[x, y]| format!("(xy {} {})", mm(x), mm(y)))
                    .collect::<Vec<_>>()
                    .join(" ");
                let net = quote(&location.net);
                let layer = quote(&location.layer);
                let minimum = target.minimum;
                let clearance = mm(clearance_nm);
                let thermal_gap = thermal_gap.max(thermal_width).max(minimum);
                let form = parse(&format!(
                    r#"(zone (net {net}) (layer {layer}) (uuid "{id}")
                        (name "Local hatch width patch") (hatch edge 0.5) (priority {priority})
                        (connect_pads (clearance {clearance})) (min_thickness {minimum})
                        (fill yes (thermal_gap {thermal_gap}) (thermal_bridge_width {minimum}) (island_removal_mode 1))
                        (polygon (pts {xy})))"#
                ))?;
                (id, form, &mut added_zones)
            }
            None => continue,
        };
        if original.contains(&id) || !added_ids.insert(id.clone()) {
            return Err("supplemental hatch repair has a conflicting PCB item UUID".into());
        }
        items.push(item);
        ids.push(id);
    }
    Ok((added_tracks, added_zones))
}

fn attempt(
    stage: &Path,
    board_id: &str,
    board: &Path,
    original: &str,
    budget: &UnchangedHatchBudget<'_>,
    result: &mut UnchangedHatchRepair,
) -> Result<(), String> {
    let required = if budget.verified_fill.is_some() {
        UNCHANGED_HATCH_CHECKS_WITH_BASELINE
    } else {
        STANDALONE_CHECKS
    };
    let expired = || {
        budget
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
    };
    if budget.max_checks < required || expired() {
        return Err(
            "the repair budget does not cover the native searches and two-mode verification".into(),
        );
    }
    let parity =
        board.with_extension("kicad_sch").is_file() || board.with_extension("sch").is_file();
    let hash = fs_util::file_sha256(board)?;
    let refilled_report;
    let refilled = if let Some(baseline) = &budget.verified_fill {
        if baseline.board_sha256 != hash {
            return Err("verified fill evidence does not match the current source PCB".into());
        }
        baseline.refilled
    } else {
        result.native_checks_charged += 1;
        refilled_report = native_report::check_kept_route_input(
            board,
            &stage.join("drc-before-unchanged-hatch-refilled.json"),
            parity,
            true,
            expired,
        )?;
        &refilled_report
    };
    result.schematic_parity_checked = parity;
    result.before = width_count(refilled);
    result.after = result.before;
    if result.before == 0 {
        return Err("KiCad reports no active connection width errors".into());
    }
    if !complete_closed_report(refilled) {
        return Err("the repair requires complete native findings, closed connectivity, and no reported parity mismatch".into());
    }
    let mut pcb = parse(original)?;
    let (zone_id, fraction) = hatched_neck_target(&pcb, refilled)
        .ok_or("requires a unique unsmoothed hatched zone named by a width error")?;
    result.zone_uuid = Some(zone_id.into());
    result.preserved_hatch_fraction = Some(fraction);
    let stored_report;
    let stored = if let Some(baseline) = &budget.verified_fill {
        baseline.stored
    } else {
        result.native_checks_charged += 1;
        stored_report = native_report::check_kept_route_input(
            board,
            &stage.join("drc-before-unchanged-hatch-stored.json"),
            parity,
            false,
            expired,
        )?;
        &stored_report
    };
    same_metadata(refilled, stored, stored)?;
    if !complete_closed_report(stored) {
        return Err("saved copper must be closed and match the verified source; use a verified --save-filled copy".into());
    }
    let localized = native_report::localize_zone_widths_from_checked_fill(
        board, &hash, refilled, stored, expired,
    );
    result.native_checks_charged += localized.native_checks + localized.native_geometry_reads;
    if let Some(reason) = localized.reason {
        return Err(reason);
    }
    let targets: Vec<_> = localized
        .findings
        .iter()
        .filter_map(|row| {
            let location = row
                .location
                .as_ref()
                .filter(|place| place.zone_uuid == zone_id)?;
            Some(BridgeTarget {
                location,
                minimum: minimum_width(&row.finding)?,
            })
        })
        .collect();
    if targets.is_empty() || targets.len() != localized.findings.len() || expired() {
        return Err(
            "the localized native width findings do not identify one eligible hatch and width rule"
                .into(),
        );
    }
    let request = targets
        .iter()
        .map(|target| {
            let place = &target.location;
            json!({
                "zone_uuid": place.zone_uuid,
                "layer": place.layer,
                "net": place.net,
                "chord_mm": place.chord_mm,
                "minimum_mm": target.minimum,
            })
        })
        .collect::<Vec<_>>();
    result.native_checks_charged += 1;
    let (bridges, current_hash) = native_report::hatch_neck_geometry(
        board,
        refilled["kicad_version"]
            .as_str()
            .ok_or("missing KiCad version")?,
        &json!(request),
        expired,
    )?;
    let bridges: Vec<Option<Bridge>> =
        serde_json::from_value(bridges).map_err(|error| error.to_string())?;
    if current_hash != hash || bridges.len() != targets.len() || expired() {
        return Err("the board changed or the native bridge geometry could not be verified".into());
    }
    let (added_tracks, added_zones) =
        append_bridges(&mut pcb, zone_id, &targets, bridges, original)?;
    let widened_tracks = fs_util::read_typed_json::<Value>(&stage.join("necks.json"))
        .ok()
        .map(|manifest| widen_recorded_neck_tracks(&mut pcb, &manifest, refilled))
        .unwrap_or_default();
    if added_tracks.is_empty() && added_zones.is_empty() && widened_tracks.is_empty() {
        return Err(
            "no legal local bridge, solid patch or generated-track widening was found".into(),
        );
    }
    let candidate = format!("{}\n", encode(&pcb));
    fs_util::write_atomically(board, candidate.as_bytes())?;
    let trial_path = stage.join("drc-unchanged-hatch-trial-refilled.json");
    let gate = |trial: &Value| {
        fs_util::write_typed_json(&trial_path, trial)?;
        if !only_width_improves(refilled, trial)? || !only_width_improves(stored, trial)? {
            return Err("the proposed copper did not remove width errors while preserving all other native findings".into());
        }
        Ok(())
    };
    let fill = layout::publish_layout_fill(
        stage,
        board_id,
        pcb,
        fs_util::file_sha256(board)?,
        layout::FillRefillEvidence::GuardedTrial(&gate),
        false,
        expired,
    );
    result.native_checks_charged += fill.native_checks_charged;
    if fill.status != "saved" {
        return Err(fill
            .reason
            .unwrap_or("the verified repair fill could not be published".into()));
    }
    let trial = native_report::read_kicad_report(&trial_path)?;
    fs::copy(&trial_path, stage.join("drc.json")).map_err(|error| error.to_string())?;
    result.after = width_count(&trial);
    result.widened_tracks = widened_tracks;
    result.added_track_ids = added_tracks;
    result.added_zone_ids = added_zones;
    result.fill_publication = Some(fill);
    result.note = "verified stored and refilled copper with the original hatch settings".into();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removing_width_errors_cannot_change_other_findings() {
        let width = json!({
            "type": "connection_width", "severity": "error", "items": [], "description": "width"
        });
        let inherited = json!({
            "type": "clearance", "severity": "error", "items": [], "description": "existing"
        });
        let report = |violations| json!({"violations": violations, "unconnected_items": [], "schematic_parity": []});
        let before = report(vec![width.clone(), inherited.clone()]);
        assert!(only_width_improves(&before, &report(vec![inherited.clone()])).unwrap());
        assert!(!only_width_improves(&before, &report(Vec::<Value>::new())).unwrap());
        assert!(!only_width_improves(&before, &report(vec![inherited, width])).unwrap());
        let mut opened = before.clone();
        opened["violations"].as_array_mut().unwrap().remove(0);
        opened["unconnected_items"] = json!([{"type": "unconnected_items", "items": []}]);
        assert!(!only_width_improves(&before, &opened).unwrap());
    }
}
