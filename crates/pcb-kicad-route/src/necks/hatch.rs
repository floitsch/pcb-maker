// Copyright (C) 2026 Florian Loitsch.

//! Native-checked repair of narrow connections in an explicitly adjusted hatch.

use super::*;
use crate::open_repair::{native_check, preserved, report_capped};
use serde_json::Value;
use std::collections::HashMap;

/// How the requested hatch change and any additive copper checked out.
#[derive(Debug, Serialize)]
pub struct HatchedNeckRepair {
    pub before: usize,
    pub after: usize,
    pub requested_min_hole_area: f64,
    pub prior_min_hole_area: Option<f64>,
    pub changed_zone: Option<String>,
    pub widened_tracks: usize,
    pub added_track_ids: Vec<String>,
    pub added_track_mm: f64,
    pub native_checks: usize,
    /// Scratch fills used to find an additive bridge.
    pub native_fills: usize,
    pub seconds: f64,
    pub note: &'static str,
}

fn zone_id(finding: &Value) -> Option<&str> {
    if finding["type"] != "connection_width"
        || finding["severity"] != "error"
        || finding["excluded"] == true
    {
        return None;
    }
    let [item] = finding["items"].as_array()?.as_slice() else {
        return None;
    };
    item["uuid"].as_str()
}

fn hatched_zone<'a>(pcb: &'a Expr, id: &str) -> Option<&'a Expr> {
    let mut zones = pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("zone") && form_atom(item, "uuid", 1) == Some(id));
    let (Some(zone), None) = (zones.next(), zones.next()) else {
        return None;
    };
    let fill = zone.child("fill")?;
    (zone.child("keepout").is_none()
        && form_atom(fill, "mode", 1) == Some("hatch")
        && (fill.child("hatch_smoothing_level").is_none()
            || form_f64(fill, "hatch_smoothing_level", 1).ok()? == 0.0))
        .then_some(zone)
}

/// The unique unsmoothed hatch named by native connection-width errors.
pub fn hatched_neck_target<'a>(pcb: &Expr, report: &'a Value) -> Option<(&'a str, f64)> {
    if report_capped(report) {
        return None;
    }
    let ids: BTreeSet<_> = report["violations"]
        .as_array()?
        .iter()
        .filter_map(zone_id)
        .collect();
    let mut targets = ids.into_iter().filter_map(|id| {
        let fill = hatched_zone(pcb, id)?.child("fill")?;
        let ratio = if fill.child("hatch_min_hole_area").is_some() {
            form_f64(fill, "hatch_min_hole_area", 1).ok()?
        } else {
            0.3
        };
        (0.0..1.0).contains(&ratio).then_some((id, ratio))
    });
    let (Some(target), None) = (targets.next(), targets.next()) else {
        return None;
    };
    Some(target)
}

fn change_hole_area(pcb: &mut Expr, id: &str, fraction: f64) {
    let Expr::List(items) = pcb else { return };
    for item in items
        .iter_mut()
        .filter(|item| item.head() == Some("zone") && form_atom(item, "uuid", 1) == Some(id))
    {
        let Expr::List(fields) = item else { continue };
        let Some(Expr::List(fill)) = fields.iter_mut().find(|field| field.head() == Some("fill"))
        else {
            continue;
        };
        fill.retain(|field| field.head() != Some("hatch_min_hole_area"));
        fill.push(Expr::List(vec![
            Expr::Atom("hatch_min_hole_area".into()),
            Expr::Atom(fraction.to_string()),
        ]));
    }
}

/// Prepare and refill-check a hatch repair in a caller-owned project copy.
pub fn prepare_kicad_hatched_neck_repair(
    stage: &Path,
    board_id: &str,
    fraction: f64,
    config: &KiCadBoardRouterConfig,
) -> Result<HatchedNeckRepair, String> {
    check_board_id(board_id).map_err(exit::input_error)?;
    if !fraction.is_finite() || !(0.0..1.0).contains(&fraction) || fraction == 0.0 {
        return Err(exit::input_error(
            "the minimum hatch hole area must be a fraction strictly between 0 and 1",
        ));
    }
    let started = Instant::now();
    let mut result = repair_copy(stage, board_id, fraction, config)?;
    result.seconds = started.elapsed().as_secs_f64();
    Ok(result)
}

fn write_board(path: &Path, pcb: &Expr) -> Result<String, String> {
    let text = format!("{}\n", encode(pcb));
    fs::write(path, &text).map_err(|error| error.to_string())?;
    Ok(text)
}

fn check_board(path: &Path, report: &Path, parity: bool, expected: &str) -> Result<Value, String> {
    let native = native_check(path, report, parity, true)?;
    if fs::read_to_string(path).map_err(|error| error.to_string())? != expected {
        return Err("native check modified the board".into());
    }
    Ok(native)
}

fn repair_copy(
    stage: &Path,
    board_id: &str,
    fraction: f64,
    config: &KiCadBoardRouterConfig,
) -> Result<HatchedNeckRepair, String> {
    let path = stage.join(format!("{board_id}.kicad_pcb"));
    let original =
        fs::read_to_string(&path).map_err(|error| exit::input_error(error.to_string()))?;
    let mut pcb = parse(&original).map_err(exit::input_error)?;
    let manifest = read_json(&stage.join("necks.json")).ok();
    native_report::remove_stale_board_run_reports(stage)?;
    let parity = path.with_extension("kicad_sch").is_file() || path.with_extension("sch").is_file();
    let baseline_path = stage.join("drc-before-hatched-necks.json");
    let baseline = check_board(&path, &baseline_path, parity, &original)?;
    let before = findings(&baseline).len();
    let mut repair = HatchedNeckRepair {
        before,
        after: before,
        requested_min_hole_area: fraction,
        prior_min_hole_area: None,
        changed_zone: None,
        widened_tracks: 0,
        added_track_ids: Vec::new(),
        added_track_mm: 0.0,
        native_checks: 1,
        native_fills: 0,
        seconds: 0.0,
        note: "requires one unsmoothed hatched zone that KiCad names by itself in a width error",
    };
    let kept = try_repair(
        stage,
        board_id,
        &path,
        &original,
        &mut pcb,
        config,
        parity,
        &baseline_path,
        &baseline,
        manifest.as_ref(),
        &mut repair,
    )?;
    fs::copy(kept, stage.join("drc.json")).map_err(|error| error.to_string())?;
    Ok(repair)
}

#[allow(clippy::too_many_arguments)]
fn try_repair(
    stage: &Path,
    board_id: &str,
    path: &Path,
    original: &str,
    pcb: &mut Expr,
    config: &KiCadBoardRouterConfig,
    parity: bool,
    baseline_path: &Path,
    baseline: &Value,
    manifest: Option<&Value>,
    repair: &mut HatchedNeckRepair,
) -> Result<PathBuf, String> {
    if report_capped(baseline) {
        repair.note = "native findings are capped";
        return Ok(baseline_path.into());
    }
    let Some((id, ratio)) = hatched_neck_target(pcb, baseline) else {
        return Ok(baseline_path.into());
    };
    repair.prior_min_hole_area = Some(ratio);
    if repair.requested_min_hole_area <= ratio {
        repair.note = "requested hatch hole area does not increase the current fraction";
        return Ok(baseline_path.into());
    }

    let plan = manifest
        .and_then(supplemental::recorded_tracks)
        .map(|recorded| supplemental::widening_plan(pcb, &recorded, baseline));
    if let Some(plan) = &plan {
        supplemental::apply_plan(pcb, plan);
    }
    change_hole_area(pcb, id, repair.requested_min_hole_area);
    let adjusted = write_board(path, pcb)?;
    let trial_path = stage.join("drc-hatched-necks-trial.json");
    let trial = check_board(path, &trial_path, parity, &adjusted)?;
    repair.native_checks += 1;
    let remaining = findings(&trial);
    if remaining.len() >= repair.before || !preserved(baseline, &trial)? {
        fs::write(path, original).map_err(|error| error.to_string())?;
        repair.note = "the hatch change did not reduce width errors without new native findings";
        return Ok(baseline_path.into());
    }
    repair.after = remaining.len();
    repair.changed_zone = Some(id.into());
    repair.widened_tracks = plan.map_or(0, |plan| plan.len());
    repair.note = "KiCad verified the hatch change";
    if let [finding] = remaining.as_slice()
        && let Some(minimum) = trial["violations"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|value| zone_id(value) == Some(id))
            .find_map(|value| {
                supplemental::number_after(
                    value["description"].as_str()?,
                    "minimum connection width ",
                )
            })
    {
        repair.native_fills += 1;
        let filled = saved_fill(stage, board_id)?;
        let target = BridgeTarget {
            finding,
            zone: id,
            minimum,
        };
        let bridges = candidate_bridges(pcb, &filled, config, &target)?;
        if let [bridge] = bridges.as_slice() {
            let Expr::List(parts) = pcb else {
                return Ok(trial_path);
            };
            parts.push(supplemental_segment_expr(bridge));
            let patched = write_board(path, pcb)?;
            let bridge_path = stage.join("drc-hatched-neck-bridges.json");
            let bridge_report = check_board(path, &bridge_path, parity, &patched)?;
            repair.native_checks += 1;
            if findings(&bridge_report).is_empty() && preserved(&trial, &bridge_report)? {
                repair.after = 0;
                repair.added_track_ids = vec![supplemental_segment_id(bridge)];
                repair.added_track_mm = core::geometry::distance(bridge.start, bridge.end);
                repair.note = "KiCad verified the hatch change and the additive repair";
                return Ok(bridge_path);
            }
            fs::write(path, adjusted).map_err(|error| error.to_string())?;
        }
    }
    Ok(trial_path)
}

fn saved_fill(stage: &Path, board_id: &str) -> Result<Expr, String> {
    let scratch = stage.join("hatch-fill");
    fs::create_dir(&scratch).map_err(|error| error.to_string())?;
    let filled = (|| {
        for ext in ["kicad_pcb", "kicad_pro", "kicad_dru"] {
            let name = format!("{board_id}.{ext}");
            let source = stage.join(&name);
            if source.is_file() {
                fs::copy(source, scratch.join(&name)).map_err(|error| error.to_string())?;
            }
        }
        let board = scratch.join(format!("{board_id}.kicad_pcb"));
        native_report::run_kicad_save_board(&board, &scratch.join("drc.json"))?;
        let text = fs::read_to_string(&board).map_err(|error| error.to_string())?;
        parse(&text)
    })();
    let _ = fs::remove_dir_all(scratch);
    filled
}

struct BridgeTarget<'a> {
    finding: &'a Finding,
    zone: &'a str,
    minimum: f64,
}

fn midpoint(a: core::Point, b: core::Point) -> core::Point {
    [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5]
}

fn long_flanks(outline: &[core::Point], a: usize, b: usize, reach: f64) -> bool {
    let origin = outline[a];
    let size = outline.len();
    for step in [1, size - 1] {
        let mut i = (a + step) % size;
        let mut spread = [false; 2];
        while i != b && i != a && !spread.into_iter().all(|value| value) {
            for axis in 0..2 {
                spread[axis] |= (outline[i][axis] - origin[axis]).abs() >= reach;
            }
            i = (i + step) % size;
        }
        if !spread.into_iter().all(|value| value) {
            return false;
        }
    }
    true
}

/// A native width gives the separation but zone-only findings name the zone
/// origin. Nominate outline chords matching the reported span; KiCad checks the
/// actual copper union to decide whether the additive geometry helps.
fn matching_spans(
    outline: &[core::Point],
    actual: f64,
    minimum: f64,
    comparisons_left: &mut usize,
) -> Option<Vec<[core::Point; 2]>> {
    let mut spatial: HashMap<[i64; 2], Vec<usize>> = HashMap::new();
    let mut result = Vec::new();
    for (i, &point) in outline.iter().enumerate() {
        let [cell_x, cell_y] = point.map(|coord| (coord / minimum).floor() as i64);
        for x in cell_x - 1..=cell_x + 1 {
            for y in cell_y - 1..=cell_y + 1 {
                for &j in spatial.get(&[x, y]).into_iter().flatten() {
                    *comparisons_left = comparisons_left.checked_sub(1)?;
                    let other = outline[j];
                    if (i - j).min(outline.len() - (i - j)) <= 1
                        || (core::geometry::distance(point, other) - actual).abs() > 0.000051
                        || !long_flanks(outline, i, j, minimum * 0.9)
                        || !long_flanks(outline, j, i, minimum * 0.9)
                        || !core::geometry::point_in_polygon(midpoint(point, other), outline)
                    {
                        continue;
                    }
                    result.push([point, other]);
                }
            }
        }
        spatial.entry([cell_x, cell_y]).or_default().push(i);
    }
    Some(result)
}

fn candidate_bridges(
    pcb: &Expr,
    filled: &Expr,
    config: &KiCadBoardRouterConfig,
    target: &BridgeTarget,
) -> Result<Vec<SupplementalSegment>, String> {
    let finding = target.finding;
    let actual = finding.width;
    let minimum = target.minimum;
    if !actual.is_finite() || actual <= 0.0 || !minimum.is_finite() || minimum <= actual {
        return Ok(Vec::new());
    }
    let Some(zone) = hatched_zone(filled, target.zone) else {
        return Ok(Vec::new());
    };
    let layer_table = LayerTable::from_pcb(pcb)?;
    let Some(layer) = layer_table.index(&finding.layer) else {
        return Ok(Vec::new());
    };
    let lowered = crate::board_router::lower(pcb, config, true)?;
    let board = &lowered.board;
    let names = NetNames::of(pcb);
    let Some(net) = board
        .nets
        .iter()
        .position(|net| net_matches(&names, &net.name, &finding.net))
    else {
        return Ok(Vec::new());
    };
    let clearance = Clearances {
        board,
        net: net as core::NetId,
        layer,
        fills: zone_fills(filled)
            .into_iter()
            .filter(|(name, layer, _)| {
                *layer == finding.layer && !net_matches(&names, name, &finding.net)
            })
            .map(|(name, _, polygon)| {
                let theirs = board
                    .nets
                    .iter()
                    .find(|net| net_matches(&names, &net.name, &name))
                    .map_or(0.0, |net| board.classes[net.class].clearance);
                (polygon, theirs)
            })
            .collect(),
        drills: via_drills(pcb)
            .into_iter()
            .map(|(centre, radius, name)| {
                let n = name
                    .as_ref()
                    .and_then(|name| {
                        board
                            .nets
                            .iter()
                            .position(|net| net_matches(&names, &net.name, name))
                    })
                    .map(|n| n as core::NetId);
                (centre, radius, n)
            })
            .collect(),
    };
    let zone_pcb = Expr::List(vec![Expr::Atom("kicad_pcb".into()), zone.clone()]);
    let outlines: Vec<_> = zone_fills(&zone_pcb)
        .into_iter()
        .filter(|(_, layer, _)| *layer == finding.layer)
        .map(|(_, _, outline)| outline)
        .collect();
    let width = ((minimum + 0.01) * 1_000.0).round() / 1_000.0;
    let mut bridges: Vec<SupplementalSegment> = Vec::new();
    // A dense cluster of vertices can put most of a polygon in one cell.
    // Keep the optional geometry search bounded in that case as well.
    let mut comparisons_left = 2_000_000;
    for outline in outlines {
        let Some(spans) = matching_spans(&outline, actual, minimum, &mut comparisons_left) else {
            return Ok(Vec::new());
        };
        for [a, b] in spans {
            let middle = midpoint(a, b);
            let normal: core::Point = [(b[0] - a[0]) / actual, (b[1] - a[1]) / actual];
            let tangent = [-normal[1] * actual, normal[0] * actual];
            let round = |point: core::Point| point.map(|x| (x * 1.0e6).round() / 1.0e6);
            let ends = |offset: f64| {
                let centre = [
                    middle[0] + normal[0] * offset,
                    middle[1] + normal[1] * offset,
                ];
                [
                    round([centre[0] - tangent[0], centre[1] - tangent[1]]),
                    round([centre[0] + tangent[0], centre[1] + tangent[1]]),
                ]
            };
            let [a, b] = ends(0.0);
            if clearance.track_fits(a, b, width) {
                continue;
            }
            for offset in [0.65, -0.65, 0.70, -0.70] {
                let [start, end] = ends(offset * minimum);
                if clearance.track_fits(start, end, width)
                    && !bridges.iter().any(|bridge| {
                        core::geometry::distance(
                            midpoint(bridge.start, bridge.end),
                            midpoint(start, end),
                        ) < minimum
                    })
                {
                    bridges.push(SupplementalSegment {
                        connection: board.nets[net].name.clone(),
                        start,
                        end,
                        width,
                        layer: finding.layer.clone(),
                    });
                    break;
                }
            }
            if bridges.len() > 1 {
                return Ok(Vec::new());
            }
        }
    }
    Ok(bridges)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nominates_an_interior_chord_at_the_exact_reported_width() {
        let outline = [
            [-1.0, -1.0],
            [-0.1, -1.0],
            [0.0, -0.02745],
            [0.1, -1.0],
            [1.0, -1.0],
            [1.0, 1.0],
            [0.1, 1.0],
            [0.0, 0.02745],
            [-0.1, 1.0],
            [-1.0, 1.0],
        ];
        assert_eq!(
            matching_spans(&outline, 0.0549, 0.127, &mut 100),
            Some(vec![[[0.0, 0.02745], [0.0, -0.02745]]])
        );
        assert_eq!(
            matching_spans(&outline, 0.0551, 0.127, &mut 100),
            Some(Vec::new())
        );
        assert_eq!(matching_spans(&outline, 0.0549, 0.127, &mut 0), None);
    }
}
