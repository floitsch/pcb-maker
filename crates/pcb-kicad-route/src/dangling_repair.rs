// Copyright (C) 2026 Florian Loitsch.

//! Record generated pad leaves and optionally remove one KiCad rejects.

use super::*;
use crate::board_router::nanometres;
use crate::open_repair::{in_repair_copy, native_check, preserved, report_capped};
use pcb_router as core;
use serde_json::Value;

#[derive(Debug, Serialize)]
pub struct DanglingRepair {
    pub before: usize,
    pub after: usize,
    pub tried_track: Option<String>,
    pub removed_track: Option<String>,
    pub removed_mm: f64,
    pub native_checks: usize,
    pub check_wall_seconds: f64,
    pub note: &'static str,
}

/// Candidate IDs come from the route's current segments and must be present
/// in the written board. Existing copper is excluded before publishing IDs.
pub(crate) fn generated_pad_leaf_ids(
    pcb: &Expr,
    board: &core::Board,
    result: &core::RoutingResult,
    layer_names: &[String],
    source_board: &Path,
) -> Vec<String> {
    let mut candidates = BTreeSet::new();
    for (net, route) in result.routes.iter().enumerate() {
        let plane_layers: BTreeSet<_> = board
            .planes
            .iter()
            .filter(|plane| plane.connect && plane.net as usize == net)
            .map(|plane| plane.layer)
            .collect();
        if plane_layers.is_empty() {
            continue;
        }
        let description = &board.nets[net];
        let key = |point: core::Point| nanometres(point).map(f64::to_bits);
        let mut ends = BTreeMap::new();
        for segment in &route.segments {
            for point in [segment.start, segment.end] {
                *ends.entry((segment.layer, key(point))).or_insert(0) += 1;
            }
        }
        let vias: BTreeSet<_> = route.vias.iter().map(|via| key(via.at)).collect();
        for segment in &route.segments {
            // Restrict this to stubs on a pad edge, not a wire that reaches
            // away from the pad or a via the router intentionally placed.
            let length = core::geometry::distance(segment.start, segment.end);
            if !plane_layers.contains(&segment.layer)
                || length <= 0.0
                || length > 2.0 * segment.width
            {
                continue;
            }
            let on_pad = |point| {
                description.terminals.iter().any(|terminal| {
                    terminal.layers & (1 << segment.layer) != 0
                        && terminal
                            .contact
                            .as_ref()
                            .unwrap_or(&board.obstacles[terminal.pad].shape)
                            .contains(point)
                })
            };
            let tip = match (on_pad(segment.start), on_pad(segment.end)) {
                (true, false) => segment.end,
                (false, true) => segment.start,
                _ => continue,
            };
            if ends.get(&(segment.layer, key(tip))) != Some(&1) || vias.contains(&key(tip)) {
                continue;
            }
            candidates.insert(supplemental_segment_id(&SupplementalSegment {
                connection: description.name.clone(),
                start: nanometres(segment.start),
                end: nanometres(segment.end),
                width: segment.width,
                layer: layer_names[segment.layer].clone(),
            }));
        }
    }
    if candidates.is_empty() {
        return Vec::new();
    }
    let Some(source) = fs::read_to_string(source_board)
        .ok()
        .and_then(|text| parse(&text).ok())
    else {
        return Vec::new();
    };
    for item in source.children() {
        if let Some(id) = item_id(item) {
            candidates.remove(id);
        }
    }
    pcb.children()
        .iter()
        .filter_map(|item| item_id(item).filter(|id| candidates.contains(*id)))
        .map(str::to_owned)
        .collect()
}

fn item_id(item: &Expr) -> Option<&str> {
    form_atom(item, "uuid", 1).or_else(|| form_atom(item, "tstamp", 1))
}

fn dangling_ids(report: &Value) -> impl Iterator<Item = &str> {
    report["violations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|finding| {
            if finding["type"] != "track_dangling" || finding["severity"] != "error" {
                return None;
            }
            let [item] = finding["items"].as_array()?.as_slice() else {
                return None;
            };
            item["uuid"].as_str()
        })
}

fn candidate_track<'a>(pcb: &'a Expr, wanted: &BTreeSet<&str>) -> Option<(&'a str, f64)> {
    let codes = net_codes(pcb);
    let names = NetNames::of(pcb);
    let mut matches = BTreeMap::new();
    for item in pcb.children() {
        if let Some(id) = item_id(item).filter(|id| wanted.contains(id)) {
            matches
                .entry(id)
                .and_modify(|entry| *entry = None)
                .or_insert_with(|| {
                    let segment = stored_supplemental_segment(item, &codes, &names)?;
                    Some(core::geometry::distance(segment.start, segment.end))
                });
        }
    }
    matches
        .into_iter()
        .filter_map(|(id, length)| Some((id, length?)))
        .min_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(b.0)))
}

pub fn repair_kicad_dangling(
    source: &Path,
    board_id: &str,
    output: &Path,
) -> Result<DanglingRepair, String> {
    check_board_id(board_id).map_err(exit::input_error)?;
    let routing = read_json(&source.join("board-router.json")).map_err(exit::input_error)?;
    if routing["board_id"] != board_id {
        return Err(exit::input_error("routing report is for a different board"));
    }
    let generated: BTreeSet<_> = routing["generated_pad_leaf_ids"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    in_repair_copy(source, output, |stage| {
        repair_in_copy(stage, board_id, &generated)
    })
}

fn repair_in_copy(
    stage: &Path,
    board_id: &str,
    generated: &BTreeSet<&str>,
) -> Result<DanglingRepair, String> {
    let board_path = stage.join(format!("{board_id}.kicad_pcb"));
    let original =
        fs::read_to_string(&board_path).map_err(|error| exit::input_error(error.to_string()))?;
    let mut pcb = parse(&original).map_err(exit::input_error)?;
    let parity = stage.join(format!("{board_id}.kicad_sch")).is_file();
    let refill = pcb.child("zone").is_some()
        || pcb
            .children()
            .iter()
            .any(|item| item.head() == Some("footprint") && item.child("zone").is_some());
    native_report::remove_stale_board_run_reports(stage)?;
    let started = Instant::now();
    let before_path = stage.join("drc-before-dangling-repair.json");
    let before = native_check(&board_path, &before_path, parity, refill)?;
    if fs::read_to_string(&board_path).map_err(|error| error.to_string())? != original {
        return Err("native check modified the original copy".into());
    }
    let count = dangling_ids(&before).count();
    let mut report = DanglingRepair {
        before: count,
        after: count,
        tried_track: None,
        removed_track: None,
        removed_mm: 0.0,
        native_checks: 1,
        check_wall_seconds: 0.0,
        note: "no dangling error names an intact router-generated pad stub",
    };
    let mut kept_report = before_path;
    if report_capped(&before) {
        report.note = "the baseline DRC report is capped";
    } else {
        let wanted = dangling_ids(&before)
            .filter(|id| generated.contains(id))
            .collect();
        if let Some((id, length)) = candidate_track(&pcb, &wanted) {
            let id = id.to_owned();
            report.tried_track = Some(id.clone());
            if let Expr::List(items) = &mut pcb {
                items.retain(|item| item_id(item) != Some(&id));
            }
            let proposed = format!("{}\n", encode(&pcb));
            fs::write(&board_path, &proposed).map_err(|error| error.to_string())?;
            let after_path = stage.join("drc-dangling-repair-trial.json");
            let after = native_check(&board_path, &after_path, parity, refill)?;
            report.native_checks += 1;
            if fs::read_to_string(&board_path).map_err(|error| error.to_string())? != proposed {
                return Err("native check modified the trial board".into());
            }
            let count_after = dangling_ids(&after).count();
            if count_after < count && preserved(&before, &after)? {
                report.after = count_after;
                report.removed_track = Some(id);
                report.removed_mm = length;
                report.note = "removed a generated pad stub with no new native finding";
                kept_report = after_path;
            } else {
                report.note = "the trial did not remove a dangling error without new or incomparable findings";
                fs::write(&board_path, &original).map_err(|error| error.to_string())?;
            }
        }
    }
    report.check_wall_seconds = started.elapsed().as_secs_f64();
    fs::copy(kept_report, stage.join("drc.json")).map_err(|error| error.to_string())?;
    write_typed_json(&stage.join("dangling-repair.json"), &report)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_requires_a_unique_unchanged_and_unlocked_generated_track() {
        note_net_names(&parse("(kicad_pcb (net 1 \"GND\"))").unwrap());
        let track = supplemental_segment_expr(&SupplementalSegment {
            connection: "GND".into(),
            start: [6.0, 4.0],
            end: [6.0, 4.25],
            width: 0.127,
            layer: "F.Cu".into(),
        });
        let id = item_id(&track).unwrap().to_owned();
        let wanted = BTreeSet::from([id.as_str()]);
        let pcb =
            |items: Vec<Expr>| Expr::List([vec![Expr::Atom("kicad_pcb".into())], items].concat());
        assert_eq!(
            candidate_track(&pcb(vec![track.clone()]), &wanted),
            Some((id.as_str(), 0.25))
        );
        assert_eq!(
            candidate_track(&pcb(vec![track.clone(), track.clone()]), &wanted),
            None
        );
        let mut locked = track.clone();
        if let Expr::List(items) = &mut locked {
            items.push(Expr::Atom("locked".into()));
        }
        assert_eq!(candidate_track(&pcb(vec![locked]), &wanted), None);
        let moved = parse(&encode(&track).replace("4.25", "4.30")).unwrap();
        assert_eq!(candidate_track(&pcb(vec![moved]), &wanted), None);
    }
}
