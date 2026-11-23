// Copyright (C) 2026 Florian Loitsch.

//! A native-verified local jog of generated copper around a different-net via.

use super::*;
use crate::open_repair::{finding_counts, in_repair_copy, preserved, report_capped};
use pcb_router::{Point, geometry::point_segment_distance};
use serde_json::Value;

pub const VIA_DETOUR_CHECKS: usize = 6;
const STEP_MM: f64 = 0.05;
const MARGIN_MM: f64 = 0.025;
const MAX_OFFSET_MM: f64 = 0.5;
const MAX_SPAN_MM: f64 = 3.0;

#[derive(Clone, Debug, Serialize)]
pub struct ViaDetourRepair {
    pub status: &'static str,
    pub reason: &'static str,
    pub native_checks: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detour: Option<ViaDetour>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub saved_copper: Option<board_stats::KiCadSavedCopper>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ViaDetour {
    pub track_uuid: String,
    pub via_uuid: String,
    pub track_net: String,
    pub via_net: String,
    pub layer: String,
    pub width_mm: f64,
    pub offset_mm: f64,
    pub added_length_mm: f64,
    pub replacement_tracks: Vec<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub stored_partition_verified: bool,
}

fn id(form: &Expr) -> Option<&str> {
    form_atom(form, "uuid", 1).or_else(|| form_atom(form, "tstamp", 1))
}

fn via_short(row: &Value) -> Option<[&str; 2]> {
    if row["type"] != "shorting_items" || row["severity"] != "error" {
        return None;
    }
    let [a, b] = row["items"].as_array()?.as_slice() else {
        return None;
    };
    Some([a["uuid"].as_str()?, b["uuid"].as_str()?])
}

pub fn has_via_short(report: &Value) -> bool {
    report["violations"].as_array().is_some_and(|rows| {
        rows.iter().any(|row| {
            via_short(row).is_some()
                && row["items"].as_array().is_some_and(|items| {
                    let describes = |prefix| {
                        items.iter().any(|item| {
                            item["description"]
                                .as_str()
                                .is_some_and(|s| s.starts_with(prefix))
                        })
                    };
                    describes("Via [") && describes("Track [")
                })
        })
    })
}

fn jog(track: &SupplementalSegment, via: Point, envelope: f64) -> Option<([Point; 6], f64)> {
    // Axis-aligned runs can retain both endpoints and use two exact 45°
    // transitions on KiCad's integer-nanometre grid.
    let axis = match (
        track.start[0] == track.end[0],
        track.start[1] == track.end[1],
    ) {
        (true, false) => 1,
        (false, true) => 0,
        _ => return None,
    };
    let across = 1 - axis;
    let direction = (track.end[axis] - track.start[axis]).signum();
    let away = (track.start[across] - via[across]).signum();
    let from_via = (track.start[across] - via[across]).abs();
    if from_via == 0.0 {
        return None;
    }
    let offset = ((envelope + MARGIN_MM - from_via) / STEP_MM).ceil() * STEP_MM;
    let half_span = envelope + STEP_MM;
    if !(STEP_MM..=MAX_OFFSET_MM).contains(&offset) || 2.0 * (half_span + offset) > MAX_SPAN_MM {
        return None;
    }
    let point = |along: f64, shifted: bool| {
        let mut point = track.start;
        point[axis] = via[axis] + direction * along;
        if shifted {
            point[across] += away * offset;
        }
        board_router::nanometres(point)
    };
    let before = point(-half_span - offset, false);
    let after = point(half_span + offset, false);
    if (before[axis] - track.start[axis]) * direction < track.width
        || (track.end[axis] - after[axis]) * direction < track.width
    {
        return None;
    }
    let points = [
        track.start,
        before,
        point(-half_span, true),
        point(half_span, true),
        after,
        track.end,
    ];
    points
        .windows(2)
        .all(|segment| point_segment_distance(via, segment[0], segment[1]) >= envelope)
        .then_some((points, offset))
}

fn proposal(
    pcb: &Expr,
    placed: &Expr,
    ids: [&str; 2],
    rules: &KiCadBoardRouterConfig,
) -> Option<(ViaDetour, Vec<Expr>)> {
    let find = |head| {
        let mut items = pcb.children().iter().filter(|item| {
            item.head() == Some(head) && id(item).is_some_and(|uuid| ids.contains(&uuid))
        });
        let item = items.next()?;
        items.next().is_none().then_some(item)
    };
    let via = find("via")?;
    let track = find("segment")?;
    let track_uuid = id(track)?;
    let via_uuid = id(via)?;
    if placed
        .children()
        .iter()
        .any(|item| id(item).is_some_and(|uuid| uuid.eq_ignore_ascii_case(track_uuid)))
        || track.children().iter().skip(1).any(|field| {
            !matches!(
                field.head(),
                Some("start" | "end" | "width" | "layer" | "net" | "uuid")
            )
        })
        || pcb
            .children()
            .iter()
            .filter(|item| item.head() == Some("group"))
            .any(|group| {
                group.child("members").is_some_and(|members| {
                    members
                        .children()
                        .iter()
                        .any(|member| member.atom() == Some(track_uuid))
                })
            })
    {
        return None;
    }
    let codes = net_codes(pcb);
    let names = NetNames::of(pcb);
    let original = stored_supplemental_segment(track, &codes, &names)?;
    let placed_codes = net_codes(placed);
    let coincides = |item: &Expr| {
        if item.head() != Some("segment")
            || form_atom(item, "layer", 1) != Some(&original.layer)
            || form_f64(item, "width", 1).ok() != Some(original.width)
            || item_net(item, &placed_codes).map(|net| names.normalize(net))
                != Some(&original.connection)
        {
            return false;
        }
        let points = [form_xy(item, "start").ok(), form_xy(item, "end").ok()];
        points == [Some(original.start), Some(original.end)]
            || points == [Some(original.end), Some(original.start)]
    };
    if project_rules::diff_pair_of(&original.connection).is_some()
        || placed.children().iter().any(coincides)
        || rules
            .length_rules
            .iter()
            .any(|rule| rule.nets.iter().any(|net| net.net == original.connection))
        || via.children().get(1)?.atom().is_some()
        || via.child("padstack").is_some()
        || via.child("remove_unused_layers").is_some()
        || !matches!(via.child("layers")?.children(), [_, front, back] if front.atom() == Some("F.Cu") && back.atom() == Some("B.Cu"))
    {
        return None;
    }
    let track_net = item_net(track, &codes)?.to_owned();
    let via_net = item_net(via, &codes)?.to_owned();
    if track_net.is_empty()
        || via_net.is_empty()
        || names.normalize(&track_net) == names.normalize(&via_net)
        || original.width <= 0.0
    {
        return None;
    }
    let via_at = form_xy(via, "at").ok()?;
    let via_radius = form_f64(via, "size", 1).ok()? / 2.0;
    let hole_radius = form_f64(via, "drill", 1).ok()? / 2.0;
    if hole_radius <= 0.0
        || hole_radius >= via_radius
        || point_segment_distance(via_at, original.start, original.end)
            >= via_radius + original.width / 2.0
    {
        return None;
    }
    let clearance = |net| {
        rules
            .connection_rules
            .get(names.normalize(net))
            .or(rules.default_rules.as_ref())
            .map_or(0.0, |rule| rule.clearance_mm)
    };
    let min_gap = clearance(&track_net).max(clearance(&via_net));
    let envelope =
        (via_radius + min_gap).max(hole_radius + rules.hole_clearance_mm) + original.width / 2.0;
    let (points, offset_mm) = jog(&original, via_at, envelope)?;
    let mut replacement_tracks = Vec::new();
    let mut replacements = Vec::new();
    let used_ids: BTreeSet<_> = pcb.children().iter().filter_map(id).collect();
    for (start, end) in points.into_iter().zip(points.into_iter().skip(1)) {
        let segment = SupplementalSegment {
            start,
            end,
            ..original.clone()
        };
        let uuid = supplemental_segment_id(&segment);
        if used_ids.contains(uuid.as_str()) || replacement_tracks.contains(&uuid) {
            return None;
        }
        let Expr::List(mut written) = supplemental_segment_expr(&segment) else {
            return None;
        };
        // Reuse the exact native spelling/code from the selected original.
        let net_slot = written
            .iter_mut()
            .find(|field| field.head() == Some("net"))?;
        *net_slot = track.child("net")?.clone();
        replacement_tracks.push(uuid);
        replacements.push(Expr::List(written));
    }
    Some((
        ViaDetour {
            track_uuid: track_uuid.into(),
            via_uuid: via_uuid.into(),
            track_net,
            via_net,
            layer: original.layer,
            width_mm: original.width,
            offset_mm,
            added_length_mm: 2.0 * offset_mm * (std::f64::consts::SQRT_2 - 1.0),
            replacement_tracks,
            stored_partition_verified: false,
        },
        replacements,
    ))
}

fn complete_evidence(report: &Value, refill: bool) -> bool {
    !report_capped(report)
        && report["coordinate_units"] == "mm"
        && report["schematic_parity"]
            .as_array()
            .is_some_and(Vec::is_empty)
        && report["unconnected_items"]
            .as_array()
            .is_some_and(|items| !refill || items.is_empty())
        && report["violations"].is_array()
}

fn contains_short(report: &Value, detour: &ViaDetour) -> bool {
    report["violations"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|row| {
            via_short(row).is_some_and(|ids| {
                ids.contains(&detour.track_uuid.as_str()) && ids.contains(&detour.via_uuid.as_str())
            })
        })
}

fn check_candidate(
    before: &Value,
    after: &Value,
    detour: &ViaDetour,
    refill: bool,
) -> Result<bool, String> {
    Ok(complete_evidence(after, refill)
        && !contains_short(after, detour)
        && finding_counts(before, "unconnected_items")?
            == finding_counts(after, "unconnected_items")?
        && preserved(before, after)?)
}

type Partition = Vec<(String, Vec<String>)>;

fn same_components(mut before: Partition, mut after: Partition, detour: &ViaDetour) -> bool {
    let mut replacements = 0;
    for (net, members) in &mut before {
        if let Some(index) = members.iter().position(|id| id == &detour.track_uuid) {
            if net != &detour.track_net {
                return false;
            }
            members.splice(index..=index, detour.replacement_tracks.iter().cloned());
            replacements += 1;
        }
    }
    let sort = |partition: &mut Partition| {
        for (_, members) in partition.iter_mut() {
            members.sort();
        }
        partition.sort();
    };
    sort(&mut before);
    sort(&mut after);
    replacements == 1 && before == after
}

fn comparable_stored_opens(
    before: &Value,
    after: &Value,
    detour: &ViaDetour,
) -> Result<bool, String> {
    let (Some(a), Some(b)) = (
        before["unconnected_items"].as_array(),
        after["unconnected_items"].as_array(),
    ) else {
        return Ok(false);
    };
    if a.len() != b.len()
        || !complete_evidence(before, false)
        || !complete_evidence(after, false)
        || [a, b].into_iter().flatten().any(|open| {
            open["type"] != "unconnected_items"
                || open["severity"] != "error"
                || open["excluded"] == true
                || open["items"]
                    .as_array()
                    .is_none_or(|items| items.len() != 2)
        })
    {
        return Ok(false);
    }
    // Native ratsnest representatives may change when unrelated copper is
    // replaced. Preserve every other report field and require an independent
    // exact native component comparison to establish that topology stayed put.
    let without_opens = |report: &Value| {
        let mut report = report.clone();
        report["unconnected_items"] = Value::Array(Vec::new());
        report
    };
    check_candidate(&without_opens(before), &without_opens(after), detour, false)
}

fn archive_routed_reports(directory: &Path) -> Result<(), String> {
    let baseline = directory.join("via-detour-input-evidence");
    fs::create_dir(&baseline).map_err(|error| error.to_string())?;
    for name in [
        "drc.json",
        "drc-saved-fill.json",
        "drc-refilled.json",
        "verification.json",
        "board-router.json",
        "board-layout.json",
        "verdict.json",
        "fill-handoff.json",
        "preview.svg",
        "preview.json",
        "preview-labels.json",
    ] {
        if directory.join(name).is_file() {
            fs::rename(directory.join(name), baseline.join(name))
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

fn placement_sides(board: &Expr) -> Vec<(&str, String, &str)> {
    let mut sides: Vec<_> = board
        .children()
        .iter()
        .filter(|item| matches!(item.head(), Some("footprint" | "module")))
        .map(|item| {
            (
                id(item).unwrap_or_default(),
                footprint_reference(item).unwrap_or_default(),
                form_atom(item, "layer", 1).unwrap_or_default(),
            )
        })
        .collect();
    sides.sort();
    sides
}

/// Evaluate one native via/track short in a private copy. On rejection the
/// output is the original board with the evidence that explains the result.
/// `placed` is the actual pre-routing project for the selected route.
pub fn repair_kicad_via_short(
    routed: &Path,
    board_id: &str,
    placed: &Path,
    output: &Path,
    deadline: Option<Instant>,
) -> Result<ViaDetourRepair, String> {
    check_board_id(board_id)?;
    let read_board = |directory: &Path| -> Result<String, String> {
        fs::read_to_string(directory.join(format!("{board_id}.kicad_pcb")))
            .map_err(|error| exit::input_error(error.to_string()))
    };
    let placed_pcb = parse(&read_board(placed)?)?;
    in_repair_copy(routed, output, |directory| {
        let file = directory.join(format!("{board_id}.kicad_pcb"));
        let original = read_board(directory)?;
        let mut pcb = parse(&original)?;
        let mut report = ViaDetourRepair {
            status: "unchanged",
            reason: "no_eligible_via_short",
            native_checks: 0,
            detour: None,
            saved_copper: None,
            detail: None,
        };
        let project = file.with_extension("kicad_pro");
        let drc = |name| directory.join(name);
        let companions = ["kicad_pro", "kicad_dru", "kicad_sch"].map(|extension| {
            let staged = file.with_extension(extension);
            let contents = fs::read(&staged).ok();
            (staged, contents)
        });
        let placed_matches = companions.iter().all(|(staged, contents)| {
            fs::read(placed.join(staged.file_name().unwrap())).ok() == *contents
        });
        let placements_match = board_stats::board_statistics(&placed_pcb)?
            .component_placement_sha256
            == board_stats::board_statistics(&pcb)?.component_placement_sha256
            && placement_sides(&placed_pcb) == placement_sides(&pcb);
        let expired = || deadline.is_some_and(|limit| Instant::now() >= limit);
        let mut checks = 0;
        let attempt = (|| -> Result<Option<ViaDetour>, String> {
            if !placed_matches
                || !placements_match
                || !project.is_file()
                || !file.with_extension("kicad_sch").is_file()
            {
                report.reason = "matching_placed_project_unavailable";
                return Ok(None);
            }
            let rules = project_rules::resolve_project_rules(&project, &file)?;
            let mut check = |name, refill| {
                if expired() {
                    return Err("wall guard expired".into());
                }
                checks += 1;
                native_report::check_kept_route_input(&file, &drc(name), true, refill, expired)
            };
            let before = check("drc-before-via-detour-refilled.json", true)?;
            if !complete_evidence(&before, true) || read_board(directory)? != original {
                report.reason = "native_baseline_open_or_incomplete";
                return Ok(None);
            }
            let Some((mut detour, replacements)) = before["violations"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(via_short)
                .find_map(|ids| proposal(&pcb, &placed_pcb, ids, &rules))
            else {
                return Ok(None);
            };
            let stored = check("drc-before-via-detour-stored.json", false)?;
            if !complete_evidence(&stored, false)
                || !contains_short(&stored, &detour)
                || read_board(directory)? != original
            {
                report.reason = "native_stored_baseline_does_not_confirm_short";
                return Ok(None);
            }
            let Expr::List(items) = &mut pcb else {
                return Err("PCB is not a list".into());
            };
            let index = items
                .iter()
                .position(|item| id(item) == Some(&detour.track_uuid))
                .ok_or("lost original track")?;
            items.splice(index..=index, replacements);
            let written = encode(&pcb);
            fs::write(&file, &written).map_err(|error| error.to_string())?;
            let after = check("drc-trial-via-detour-refilled.json", true)?;
            let reopened = check("drc-trial-via-detour-stored.json", false)?;
            if read_board(directory)? != written
                || companions
                    .iter()
                    .any(|(path, contents)| fs::read(path).ok().as_ref() != contents.as_ref())
            {
                return Err("native DRC changed the board or project".into());
            }
            if !check_candidate(&before, &after, &detour, true)? {
                report.reason = "native_trial_did_not_preserve_findings";
                return Ok(None);
            }
            if !check_candidate(&stored, &reopened, &detour, false)? {
                if !comparable_stored_opens(&stored, &reopened, &detour)? {
                    report.reason = "native_trial_did_not_preserve_findings";
                    return Ok(None);
                }
                let version = stored["kicad_version"]
                    .as_str()
                    .ok_or("missing native version")?;
                let partition_limit = Instant::now() + std::time::Duration::from_secs(20);
                let partition_expired = || expired() || Instant::now() >= partition_limit;
                let mut partition = |expected: &str| {
                    if partition_expired() {
                        return Err("native connectivity check wall guard expired".into());
                    }
                    write_atomically(&file, expected.as_bytes())?;
                    checks += 1;
                    let components = native_report::connectivity_partition(
                        &file,
                        version,
                        false,
                        partition_expired,
                    )?;
                    if read_board(directory)? != expected {
                        return Err("native connectivity check changed the board".into());
                    }
                    Ok::<_, String>(components)
                };
                let a = partition(&original)?;
                let b = partition(&written)?;
                if !same_components(a, b, &detour) {
                    report.reason = "native_trial_changed_stored_components";
                    return Ok(None);
                }
                detour.stored_partition_verified = true;
            }
            Ok(Some(detour))
        })();
        report.native_checks = checks;
        match attempt {
            Ok(Some(detour)) => {
                report.status = "improved";
                report.reason = "native_via_short_cleared";
                report.detour = Some(detour);
                report.saved_copper = Some(board_stats::saved_copper(&pcb)?);
                archive_routed_reports(directory)?;
                fs::copy(drc("drc-trial-via-detour-refilled.json"), drc("drc.json"))
                    .map_err(|error| error.to_string())?;
                fs::copy(
                    drc("drc-trial-via-detour-stored.json"),
                    drc("drc-via-detour-stored.json"),
                )
                .map_err(|error| error.to_string())?;
                if directory.join("erc.json").is_file() {
                    verification::record_verified_additive_board(directory, board_id, true)?;
                }
            }
            Ok(None) => fs::write(&file, original).map_err(|error| error.to_string())?,
            Err(error) => {
                if companions
                    .iter()
                    .any(|(path, contents)| fs::read(path).ok().as_ref() != contents.as_ref())
                {
                    return Err("native DRC changed the project inputs".into());
                }
                fs::write(&file, original).map_err(|error| error.to_string())?;
                report.reason = "native_evidence_unavailable";
                report.detail = Some(error);
            }
        }
        write_typed_json(&drc("via-detour.json"), &report)?;
        Ok(report)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_jog_preserves_both_endpoints_and_uses_two_45_degree_transitions() {
        let track = SupplementalSegment {
            connection: "SIG".into(),
            start: [3.0, 0.0],
            end: [3.0, 5.0],
            width: 0.2,
            layer: "F.Cu".into(),
        };
        let (points, offset) = jog(&track, [3.36, 2.0], 0.5).unwrap();
        assert_eq!(offset, 0.2);
        assert_eq!((points[0], points[5]), (track.start, track.end));
        assert_eq!(points[2], [2.8, 1.45]);
        assert_eq!(points[3], [2.8, 2.55]);
        assert!((points[2][0] - points[1][0] + points[2][1] - points[1][1]).abs() < 1e-10);
        assert!(jog(&track, [3.36, 0.2], 0.5).is_none());
        assert!(jog(&track, [3.0, 2.0], 0.5).is_none());
        assert!(jog(&track, [3.36, 2.0], 1.2).is_none());
    }

    #[test]
    fn only_a_new_generated_segment_can_become_the_detour() {
        let no_copper = parse("(kicad_pcb)").unwrap();
        note_net_names(&no_copper);
        let track = SupplementalSegment {
            connection: "SIG".into(),
            start: [3.0, 0.0],
            end: [3.0, 5.0],
            width: 0.2,
            layer: "F.Cu".into(),
        };
        let generated = supplemental_segment_expr(&track);
        let uuid = supplemental_segment_id(&track);
        let via = parse(r#"(via (at 3.36 2) (size 0.6) (drill 0.3) (layers "F.Cu" "B.Cu") (net "GND") (uuid "via-id"))"#).unwrap();
        let pcb = |track| Expr::List(vec![Expr::Atom("kicad_pcb".into()), track, via.clone()]);
        let rules = KiCadBoardRouterConfig::default();
        let ids = [uuid.as_str(), "via-id"];
        let (detour, written) = proposal(&pcb(generated.clone()), &no_copper, ids, &rules).unwrap();
        assert_eq!(written.len(), 5);
        assert_eq!(detour.layer, "F.Cu");
        assert_eq!(
            detour.track_net,
            item_net(&generated, &BTreeMap::new()).unwrap()
        );
        assert_eq!(detour.via_net, "GND");
        let authored = parse(&format!(
            "(kicad_pcb {})",
            encode(&generated).replace(&uuid, "other-id")
        ))
        .unwrap();
        assert!(proposal(&pcb(generated.clone()), &authored, ids, &rules).is_none());
        let locked = parse(&encode(&generated).replacen("(segment", "(segment locked", 1)).unwrap();
        assert!(proposal(&pcb(locked), &no_copper, ids, &rules).is_none());
    }

    #[test]
    fn native_trial_must_remove_the_short_and_preserve_all_other_findings_and_opens() {
        let detour = ViaDetour {
            track_uuid: "track".into(),
            via_uuid: "via".into(),
            track_net: "SIG".into(),
            via_net: "GND".into(),
            layer: "F.Cu".into(),
            width_mm: 0.2,
            offset_mm: 0.2,
            added_length_mm: 0.1,
            replacement_tracks: vec!["jog-a".into(), "jog-b".into()],
            stored_partition_verified: false,
        };
        let short = json!({"type":"shorting_items", "severity":"error", "items":[{"uuid":"track"},{"uuid":"via"}]});
        let warning = json!({"type":"foo", "severity":"warning", "items":[{"uuid":"other"}]});
        let before = json!({"coordinate_units":"mm", "unconnected_items":[], "schematic_parity":[], "violations":[warning, short]});
        let mut after = before.clone();
        assert!(!check_candidate(&before, &after, &detour, false).unwrap());
        after["violations"].as_array_mut().unwrap().pop();
        assert!(check_candidate(&before, &after, &detour, false).unwrap());
        let mut open = after.clone();
        open["unconnected_items"] = json!([{"type":"unconnected_items"}]);
        assert!(!check_candidate(&before, &open, &detour, false).unwrap());
        after["violations"]
            .as_array_mut()
            .unwrap()
            .push(json!({"type":"items_not_allowed", "severity":"error"}));
        assert!(!check_candidate(&before, &after, &detour, true).unwrap());

        let before = vec![
            ("SIG".into(), vec!["track".into(), "pad".into()]),
            ("GND".into(), vec!["via".into()]),
        ];
        let mut after = vec![
            (
                "SIG".into(),
                vec!["jog-a".into(), "pad".into(), "jog-b".into()],
            ),
            ("GND".into(), vec!["via".into()]),
        ];
        assert!(same_components(before.clone(), after.clone(), &detour));
        after[0].1.pop();
        after.push(("SIG".into(), vec!["jog-b".into()]));
        assert!(!same_components(before, after, &detour));
    }
}
