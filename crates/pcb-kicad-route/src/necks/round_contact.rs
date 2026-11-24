// Copyright (C) 2026 Florian Loitsch.

//! A short copper bridge where a flashed via nearly touches a round SMD pad.

use super::*;
use crate::open_repair::{finding_counts, finding_types_at_cap, report_capped, write_additions};
use serde_json::Value;

const MAX_RIM_GAP_MM: f64 = 0.02;
const OVERLAP_MM: f64 = 0.045;
const MAX_CHECKS: usize = 5;
const PARTITION_CHECKS: usize = 2;
const COPPER_FINDINGS: [&str; 3] = ["violations", "unconnected_items", "schematic_parity"];

#[derive(Clone, Debug, Serialize)]
pub struct RoundContactBridge {
    pub pad_uuid: String,
    pub via_uuid: String,
    pub route_uuids: Vec<String>,
    pub measured_mm: f64,
    pub minimum_mm: f64,
    pub rim_gap_mm: f64,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub stored_partition_verified: bool,
    pub segment: SupplementalSegment,
}

fn measurement(row: &Value, minimum: f64) -> Option<(&str, f64)> {
    if row["type"] != "connection_width" || row["severity"] != "warning" {
        return None;
    }
    let description = row["description"].as_str()?;
    let (_, layer) = description.rsplit_once(") (")?;
    let rules = description
        .split_once("(board setup constraints minimum connection width ")?
        .1;
    let (limit, measured) = rules.split_once(" mm; actual ")?;
    // KiCad writes this limit to four decimal places in millimetres.
    if (limit.parse::<f64>().ok()? - minimum).abs() > 0.000_050_001 {
        return None;
    }
    let measured: f64 = measured.split_once(" mm)")?.0.parse().ok()?;
    (measured >= 0.0 && measured < minimum).then_some((layer.strip_suffix(')')?, measured))
}

fn row_uuids(row: &Value) -> Option<[&str; 2]> {
    let [a, b] = row["items"].as_array()?.as_slice() else {
        return None;
    };
    Some([a["uuid"].as_str()?, b["uuid"].as_str()?])
}

fn other_end(track: &Expr, point: core::Point) -> Option<core::Point> {
    let a = form_xy(track, "start").ok()?;
    let b = form_xy(track, "end").ok()?;
    let near = |left, right| core::geometry::distance(left, right) <= 0.000_001_001;
    if near(a, point) {
        Some(b)
    } else if near(b, point) {
        Some(a)
    } else {
        None
    }
}

fn candidate(
    pcb: &Expr,
    row: &Value,
    minimum: f64,
    layers: &LayerTable,
) -> Option<(RoundContactBridge, Expr)> {
    let (layer, measured_mm) = measurement(row, minimum)?;
    let mask = 1 << layers.index(layer)?;
    let ids = row_uuids(row)?;
    let mut vias = pcb.children().iter().filter(|item| {
        item.head() == Some("via")
            && footprint_uuid(item).is_some_and(|uuid| ids.contains(&uuid.as_str()))
    });
    let via = vias.next()?;
    let via_uuid = footprint_uuid(via)?;
    if vias.next().is_some()
        || layers.via_mask(via) & mask == 0
        || via.child("padstack").is_some()
        || form_atom(via, "remove_unused_layers", 1).is_some_and(|mode| mode != "no")
    {
        return None;
    }
    let via_at = form_xy(via, "at").ok()?;
    let radius = form_f64(via, "size", 1).ok()? / 2.0;
    let bore = form_f64(via, "drill", 1).ok()? / 2.0;
    let width = (minimum * 1000.0).ceil() / 1000.0;
    let overlap = OVERLAP_MM.min(width / 2.0);
    if bore <= 0.0 || radius - overlap < bore + width / 2.0 {
        return None;
    }
    let codes = net_codes(pcb);
    let net = item_net(via, &codes).filter(|net| !net.is_empty())?;
    let tracks: Vec<_> = pcb
        .children()
        .iter()
        .filter(|item| {
            item.head() == Some("segment")
                && form_atom(item, "layer", 1) == Some(layer)
                && item_net(item, &codes) == Some(net)
        })
        .collect();
    let routes: Vec<_> = tracks
        .iter()
        .filter_map(|track| {
            let joint = other_end(track, via_at)?;
            Some((*track, joint, footprint_uuid(track)?))
        })
        .collect();
    let mut selected = None;
    for footprint in pcb
        .children()
        .iter()
        .filter(|item| matches!(item.head(), Some("footprint" | "module")))
    {
        let Some(pose) = form_at(footprint).ok() else {
            continue;
        };
        for pad in footprint.children().iter().filter(|pad| {
            pad.head() == Some("pad")
                && pad.children().get(2).and_then(Expr::atom) == Some("smd")
                && pad.children().get(3).and_then(Expr::atom) == Some("circle")
                && pad.child("padstack").is_none()
                && pad.child("drill").is_none()
                && item_net(pad, &codes) == Some(net)
                && layers.pad_mask(pad).is_ok_and(|listed| listed == mask)
        }) {
            let (Ok([x, y]), Ok(lowered), Some(pad_uuid)) = (
                form_xy(pad, "size"),
                lower_pad(pad, pose),
                footprint_uuid(pad),
            ) else {
                continue;
            };
            let pad_at = lowered.center;
            let pad_radius = x / 2.0;
            let distance = core::geometry::distance(pad_at, via_at);
            let gap = distance - pad_radius - radius;
            if x != y || pad_radius < width || !gap.is_finite() || gap.abs() > MAX_RIM_GAP_MM {
                continue;
            }
            // The nearby pad must already be connected by one or two actual
            // track segments with exact endpoints on this layer. Never infer
            // a missing terminal or connect a different branch from proximity.
            let mut path = None;
            for (first, joint, first_id) in &routes {
                let near_pad = core::geometry::distance(*joint, pad_at) <= 0.000_001_001;
                let second = (!near_pad)
                    .then(|| {
                        tracks.iter().find_map(|track| {
                            if *track == *first {
                                return None;
                            }
                            other_end(track, *joint).filter(|end| {
                                core::geometry::distance(*end, pad_at) <= 0.000_001_001
                            })?;
                            footprint_uuid(track)
                        })
                    })
                    .flatten();
                if (near_pad || second.is_some())
                    && (ids.contains(&pad_uuid.as_str())
                        || ids.contains(&first_id.as_str())
                        || second.as_deref().is_some_and(|id| ids.contains(&id)))
                {
                    path = Some(std::iter::once(first_id.clone()).chain(second).collect());
                    break;
                }
            }
            let Some(route_uuids) = path else { continue };
            let lerp = |along: f64| {
                std::array::from_fn(|axis| {
                    let position = pad_at[axis] + (via_at[axis] - pad_at[axis]) * along / distance;
                    (position * 1e6).round() / 1e6
                })
            };
            let segment = SupplementalSegment {
                connection: net.into(),
                layer: layer.into(),
                start: lerp(pad_radius - overlap),
                end: lerp(distance - radius + overlap),
                width,
            };
            if selected.is_some() || segment.start == segment.end {
                return None;
            }
            let mut form = supplemental_segment_expr(&segment);
            if let Expr::List(items) = &mut form {
                let field = items.iter_mut().find(|item| item.head() == Some("net"))?;
                *field = via.child("net")?.clone();
            }
            selected = Some((
                RoundContactBridge {
                    pad_uuid,
                    via_uuid: via_uuid.clone(),
                    route_uuids,
                    measured_mm,
                    minimum_mm: minimum,
                    rim_gap_mm: gap,
                    stored_partition_verified: false,
                    segment,
                },
                form,
            ));
        }
    }
    selected
}

fn target(row: &Value, bridge: &RoundContactBridge) -> bool {
    let Some((layer, measured)) = measurement(row, bridge.minimum_mm) else {
        return false;
    };
    let Some(ids) = row_uuids(row) else {
        return false;
    };
    layer == bridge.segment.layer
        && measured == bridge.measured_mm
        && ids.contains(&bridge.via_uuid.as_str())
        && (ids.contains(&bridge.pad_uuid.as_str())
            || bridge
                .route_uuids
                .iter()
                .any(|id| ids.contains(&id.as_str())))
}

fn removed_target(report: &Value, bridge: &RoundContactBridge) -> Option<Value> {
    let mut report = report.clone();
    let violations = report["violations"].as_array_mut()?;
    let mut matching = violations
        .iter()
        .enumerate()
        .filter(|(_, row)| target(row, bridge));
    let (index, _) = matching.next()?;
    if matching.next().is_some() {
        return None;
    }
    violations.remove(index);
    Some(report)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Comparison {
    Exact,
    OpenWitnessesChanged,
    Rejected,
}

fn compare(
    before: &Value,
    after: &Value,
    bridge: &RoundContactBridge,
) -> Result<Comparison, String> {
    use Comparison::*;
    for report in [before, after] {
        if report["coordinate_units"] != "mm"
            || capped(report)
            || COPPER_FINDINGS.iter().any(|key| !report[*key].is_array())
        {
            return Ok(Rejected);
        }
    }
    let Some(expected) = removed_target(before, bridge) else {
        return Ok(Rejected);
    };
    for field in ["kicad_version", "included_severities", "ignored_checks"] {
        if expected.get(field) != after.get(field) {
            return Ok(Rejected);
        }
    }
    for key in ["violations", "schematic_parity", outline_pieces::ACROSS_KEY] {
        if finding_counts(&expected, key)? != finding_counts(after, key)? {
            return Ok(Rejected);
        }
    }
    if finding_counts(before, "unconnected_items")? == finding_counts(after, "unconnected_items")? {
        return Ok(Exact);
    }
    let regular_opens = |report: &Value| {
        report["unconnected_items"]
            .as_array()
            .filter(|rows| {
                rows.iter().all(|finding| {
                    finding["type"] == "unconnected_items"
                        && finding["severity"] == "error"
                        && finding["excluded"] != true
                        && row_uuids(finding).is_some()
                })
            })
            .map(Vec::len)
    };
    Ok(match (regular_opens(&expected), regular_opens(after)) {
        (Some(a), Some(b)) if a == b => OpenWitnessesChanged,
        _ => Rejected,
    })
}

fn capped(report: &Value) -> bool {
    report_capped(report) || !finding_types_at_cap(report, "schematic_parity").is_empty()
}

fn same_partition(
    mut before: Vec<(String, Vec<String>)>,
    mut after: Vec<(String, Vec<String>)>,
    bridge: &RoundContactBridge,
    added: &str,
) -> bool {
    let mut matched = 0;
    for (net, members) in &mut after {
        if members.iter().any(|id| id == added) {
            if *net != bridge.segment.connection
                || !members.contains(&bridge.pad_uuid)
                || !members.contains(&bridge.via_uuid)
            {
                return false;
            }
            members.retain(|id| {
                matched += usize::from(id == added);
                id != added
            });
        }
    }
    for groups in [&mut before, &mut after] {
        for (_, ids) in groups.iter_mut() {
            ids.sort_unstable();
        }
        groups.sort_unstable();
    }
    matched == 1
        && before
            .iter()
            .all(|(_, ids)| ids.iter().all(|id| id != added))
        && before == after
}

pub(super) fn repair(
    directory: &Path,
    board_id: &str,
    native: &Value,
    max_checks: Option<usize>,
    deadline: Option<Instant>,
) -> Result<Option<NeckRepair>, String> {
    let minimum = minimum_connection(directory, board_id);
    if !(0.0 < minimum && minimum <= 0.25)
        || !directory.join(format!("{board_id}.kicad_sch")).is_file()
        || !native["violations"]
            .as_array()
            .is_some_and(|rows| rows.iter().any(|row| measurement(row, minimum).is_some()))
    {
        return Ok(None);
    }
    let board = directory.join(format!("{board_id}.kicad_pcb"));
    let original = fs::read_to_string(&board).map_err(|error| error.to_string())?;
    let pcb = parse(&original)?;
    note_net_names(&pcb);
    let layers = LayerTable::from_pcb(&pcb)?;
    let Some((bridge, addition)) = native["violations"]
        .as_array()
        .into_iter()
        .flatten()
        .find_map(|row| candidate(&pcb, row, minimum, &layers))
    else {
        return Ok(None);
    };
    let mut result = NeckRepair {
        before: 1,
        after: 1,
        found: 1,
        round_contact: Some(bridge),
        ..Default::default()
    };
    if let Some(checks) = max_checks.filter(|checks| *checks < MAX_CHECKS) {
        result.budget_checks_available = Some(checks);
        result.note = Some(format!(
            "round pad-to-via repair requires {MAX_CHECKS} native checks"
        ));
        return Ok(Some(result));
    }
    let started = std::time::Instant::now();
    let scratch = temporary_next_to(directory);
    let staged = scratch.join(board.file_name().expect("named input PCB"));
    let snapshot = Saved::take(directory, board_id)?;
    let mut changed = false;
    let checked = (|| -> Result<(), String> {
        copy_directory_tree(directory, &scratch)?;
        let mut baselines = Vec::new();
        for (mode, refill) in [("stored", false), ("refilled", true)] {
            result.round_contact_native_checks += 1;
            let report = scratch.join(format!("baseline-{mode}.json"));
            let baseline = native_report::run_route_reuse_drc(&staged, &report, refill)?;
            if capped(&baseline) {
                return Err(format!(
                    "the {mode} native baseline report reached a finding cap"
                ));
            }
            baselines.push((mode, refill, baseline));
        }
        write_additions(&staged, &original, std::slice::from_ref(&addition))?;
        let exact = fs::read(&staged).map_err(|error| error.to_string())?;
        let bridge = result
            .round_contact
            .as_mut()
            .expect("round contact evidence");
        for (mode, refill, baseline) in baselines {
            result.round_contact_native_checks += 1;
            let report = scratch.join(format!("trial-{mode}.json"));
            let trial = native_report::run_route_reuse_drc(&staged, &report, refill)?;
            if capped(&trial) {
                return Err(format!(
                    "the {mode} native trial report reached a finding cap"
                ));
            }
            match compare(&baseline, &trial, bridge)? {
                Comparison::Exact => {}
                Comparison::OpenWitnessesChanged if !refill => {
                    if max_checks.is_some_and(|remaining| remaining < MAX_CHECKS + PARTITION_CHECKS)
                    {
                        return Err("remaining layout work does not cover native connectivity comparison for the changed stored open witnesses".into());
                    }
                    let version = baseline["kicad_version"]
                        .as_str()
                        .ok_or("missing KiCad version")?;
                    let added = footprint_uuid(&addition).ok_or("missing bridge UUID")?;
                    let wall = Instant::now() + std::time::Duration::from_secs(20);
                    let wall = deadline.map_or(wall, |limit| wall.min(limit));
                    let expired = || Instant::now() >= wall;
                    result.round_contact_native_checks += 1;
                    let trial =
                        native_report::connectivity_partition(&staged, version, false, expired)?;
                    write_atomically(&staged, original.as_bytes())?;
                    result.round_contact_native_checks += 1;
                    let initial =
                        native_report::connectivity_partition(&staged, version, false, expired)?;
                    write_atomically(&staged, &exact)?;
                    if !same_partition(initial, trial, bridge, &added) {
                        return Err("the stored native copper partitions changed".into());
                    }
                    bridge.stored_partition_verified = true;
                }
                _ => {
                    return Err(format!(
                        "{mode} native check did not confirm exactly one target warning removed"
                    ));
                }
            }
        }
        if fs::read(&staged).map_err(|error| error.to_string())? != exact
            || fs::read(&board).map_err(|error| error.to_string())? != original.as_bytes()
        {
            return Err("the checked board or original board changed during verification".into());
        }
        write_atomically(&board, &exact)?;
        changed = true;
        verify_materialized_rung(directory, board_id)?;
        if fs::read(&board).map_err(|error| error.to_string())? != exact {
            return Err("normal verification changed the checked board".into());
        }
        let final_report = read_json(&directory.join("drc.json"))?;
        if final_report["violations"]
            .as_array()
            .is_none_or(|rows| rows.iter().any(|row| target(row, bridge)))
        {
            return Err(
                "the materialized board still reports the selected junction warning".into(),
            );
        }
        result.tracks = 1;
        result.track_ids.extend(footprint_uuid(&addition));
        result.track_mm = core::geometry::distance(bridge.segment.start, bridge.segment.end);
        result.after = 0;
        result.kept = true;
        Ok(())
    })();
    let _ = fs::remove_dir_all(scratch);
    if let Err(reason) = checked {
        if changed {
            snapshot.restore()?;
        }
        result.note = Some(reason);
    }
    result.seconds = started.elapsed().as_secs_f64();
    Ok(Some(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const BOARD: &str = r#"(kicad_pcb
      (layers (0 "F.Cu" signal) (2 "B.Cu" signal))
      (footprint "TP" (at 3 4) (layer "B.Cu")
        (pad "1" smd circle (at 0 0) (size 1.5 1.5) (layers "B.Cu") (net "CC2") (uuid pad)))
      (via (at 4.05 4) (size 0.6) (drill 0.3) (layers "F.Cu" "B.Cu") (net "CC2") (uuid via))
      (segment (start 4.05 4) (end 3.95 4.5) (width 0.2) (layer "B.Cu") (net "CC2") (uuid launch))
      (segment (start 3.95 4.5) (end 3 4) (width 0.2) (layer "B.Cu") (net "CC2") (uuid route)))"#;

    fn warning(other: &str) -> Value {
        json!({
            "type": "connection_width",
            "severity": "warning",
            "description": "Copper connection too narrow (board setup constraints minimum connection width 0.1000 mm; actual 0.0165 mm) (B.Cu)",
            "items": [{"uuid": "via"}, {"uuid": other}],
        })
    }

    fn measure(source: &str, other: &str) -> Option<RoundContactBridge> {
        let pcb = parse(source).unwrap();
        note_net_names(&pcb);
        let layers = LayerTable::from_pcb(&pcb).unwrap();
        candidate(&pcb, &warning(other), 0.1, &layers).map(|(bridge, _)| bridge)
    }

    #[test]
    fn near_rims_use_a_short_span_on_a_proven_existing_pad_to_via_route() {
        for other in ["pad", "launch"] {
            let bridge = measure(BOARD, other).unwrap();
            assert_eq!(bridge.route_uuids, ["launch", "route"]);
            assert_eq!(bridge.via_uuid, "via");
            assert_eq!(bridge.pad_uuid, "pad");
            assert_eq!(bridge.segment.start, [3.705, 4.0]);
            assert_eq!(bridge.segment.end, [3.795, 4.0]);
            assert_eq!(bridge.segment.width, 0.1);
        }
        assert!(measure(BOARD, "unrelated").is_none());
        for changed in [
            BOARD
                .replace("(at 4.05 4)", "(at 4.15 4)")
                .replace("(start 4.05 4)", "(start 4.15 4)"),
            BOARD.replace("(end 3 4)", "(end 3 4.1)"),
            BOARD.replace("smd circle", "smd oval"),
            BOARD.replace(
                r#"(layers "B.Cu") (net "CC2")"#,
                r#"(layers "B.Cu") (net "CC1")"#,
            ),
        ] {
            assert!(measure(&changed, "pad").is_none());
        }
    }

    /// The route's neck repair (`repair_necks`, as route-kicad-board and a
    /// layout's ladder call it) on a board whose report holds one round
    /// pad-to-via warning: by default the bridge is tried (here it stops at
    /// the check budget before any KiCad check); with
    /// `tuning.post_route_repairs` false it is not, no check is spent and
    /// the board is untouched.
    #[test]
    fn the_round_contact_bridge_is_skipped_without_post_route_repairs() {
        let directory = pcb_kicad_file::fs_util::temporary_next_to(
            &std::env::temp_dir().join("round-contact-gate"),
        );
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("b.kicad_pcb"), BOARD).unwrap();
        fs::write(
            directory.join("b.kicad_pro"),
            r#"{"board": {"design_settings": {"rules": {"min_connection": 0.1}}}}"#,
        )
        .unwrap();
        fs::write(directory.join("b.kicad_sch"), "(kicad_sch)\n").unwrap();
        let report = json!({
            "violations": [warning("pad")],
            "coordinate_units": "mm",
            "unconnected_items": [],
            "schematic_parity": [],
        });
        fs::write(directory.join("drc.json"), report.to_string()).unwrap();
        let run = |config: &KiCadBoardRouterConfig| {
            let mut result = crate::board_router::KiCadBoardRouterResult::default();
            let checks =
                crate::board_router::repair_necks(&mut result, &directory, "b", config, Some(0))
                    .unwrap();
            (result.neck_repair, checks)
        };

        let default = KiCadBoardRouterConfig::default();
        assert!(default.post_route_repairs_on());
        let (repair, checks) = run(&default);
        let repair = repair.expect("the round contact repair runs by default");
        assert!(repair.round_contact.is_some());
        assert!(!repair.kept);
        assert_eq!(checks, 0);

        let mut off = KiCadBoardRouterConfig::default();
        off.tuning.post_route_repairs = Some(false);
        assert!(!off.post_route_repairs_on());
        let (repair, checks) = run(&off);
        assert!(repair.is_none());
        assert_eq!(checks, 0);
        assert_eq!(
            fs::read_to_string(directory.join("b.kicad_pcb")).unwrap(),
            BOARD
        );
        assert!(!directory.join("necks.json").exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn only_removes_the_exact_warning_and_preserves_electrical_partitions() {
        let bridge = measure(BOARD, "launch").unwrap();
        let report = |contact: Option<Value>| {
            json!({
                "violations": contact.into_iter().collect::<Vec<_>>(),
                "coordinate_units": "mm",
                "unconnected_items": [],
                "schematic_parity": [],
            })
        };
        let baseline = report(Some(warning("pad")));
        let after = report(None);
        assert_eq!(
            compare(&baseline, &after, &bridge).unwrap(),
            Comparison::Exact
        );
        assert_eq!(
            compare(&baseline, &report(Some(warning("route"))), &bridge).unwrap(),
            Comparison::Rejected
        );
        let mut new_clearance = after.clone();
        new_clearance["violations"] =
            json!([{"type": "clearance", "severity": "warning", "items": []}]);
        assert_eq!(
            compare(&baseline, &new_clearance, &bridge).unwrap(),
            Comparison::Rejected
        );
        let mut unknown = after;
        unknown.as_object_mut().unwrap().remove("schematic_parity");
        assert_eq!(
            compare(&baseline, &unknown, &bridge).unwrap(),
            Comparison::Rejected
        );

        let part = |net: &str, ids: &[&str]| {
            (
                net.to_owned(),
                ids.iter().map(|id| id.to_string()).collect(),
            )
        };
        let before = vec![
            part("CC2", &["via", "pad", "launch"]),
            part("CC2", &["old_open"]),
        ];
        assert!(same_partition(
            before.clone(),
            vec![
                part("CC2", &["bridge", "pad", "via", "launch"]),
                part("CC2", &["old_open"]),
            ],
            &bridge,
            "bridge"
        ));
        assert!(!same_partition(
            before.clone(),
            vec![part("CC2", &["bridge", "pad", "via", "launch", "old_open"]),],
            &bridge,
            "bridge"
        ));
        assert!(!same_partition(
            before,
            vec![
                part("CC2", &["pad", "via", "launch"]),
                part("CC2", &["old_open", "bridge"]),
            ],
            &bridge,
            "bridge"
        ));
    }
}
