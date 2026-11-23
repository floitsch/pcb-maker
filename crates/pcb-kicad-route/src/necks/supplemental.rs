// Copyright (C) 2026 Florian Loitsch.

//! A separate native-checked pass for two repair tracks that meet too narrowly.

use super::*;
use crate::open_repair::{native_check, preserved, report_capped};
use serde_json::Value;

#[derive(Debug, Serialize)]
pub struct SupplementalNeckRepair {
    pub before: usize,
    pub after: usize,
    pub proposed_tracks: Vec<String>,
    pub widened_tracks: usize,
    pub note: &'static str,
}

pub(super) fn number_after(text: &str, label: &str) -> Option<f64> {
    text.split_once(label)?
        .1
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

fn width_increment(finding: &Value) -> Option<f64> {
    let text = finding["description"].as_str()?;
    let required = number_after(text, "minimum connection width ")?;
    let actual = number_after(text, "actual ")?;
    (required.is_finite() && actual.is_finite() && actual > 0.0 && actual < required)
        .then_some(required - actual)
}

pub(super) fn widening_plan(
    pcb: &Expr,
    manifest: &BTreeSet<&str>,
    native: &Value,
) -> BTreeMap<String, f64> {
    let codes = net_codes(pcb);
    let names = NetNames::of(pcb);
    let mut tracks = BTreeMap::new();
    for item in pcb.children() {
        if let Some(id) = form_atom(item, "uuid", 1).filter(|id| manifest.contains(id)) {
            tracks
                .entry(id)
                .and_modify(|previous| *previous = None)
                .or_insert_with(|| stored_supplemental_segment(item, &codes, &names));
        }
    }
    let mut planned = BTreeMap::<String, f64>::new();
    let violations = native["violations"].as_array().into_iter().flatten();
    for finding in violations {
        if finding["type"] != "connection_width" || finding["severity"] != "error" {
            continue;
        }
        let Some([a, b]) = finding["items"].as_array().map(Vec::as_slice) else {
            continue;
        };
        let (Some(a), Some(b), Some(increment)) = (
            a["uuid"].as_str(),
            b["uuid"].as_str(),
            width_increment(finding),
        ) else {
            continue;
        };
        let (Some(Some(first)), Some(Some(second))) = (tracks.get(a), tracks.get(b)) else {
            continue;
        };
        if a == b || first.connection != second.connection || first.layer != second.layer {
            continue;
        }
        for (id, segment) in [(a, first), (b, second)] {
            let widened = ((segment.width + increment) * 1_000.0).ceil() / 1_000.0;
            if widened.is_finite() {
                planned
                    .entry(id.to_owned())
                    .and_modify(|best| *best = best.max(widened))
                    .or_insert(widened);
            }
        }
    }
    planned
}

pub(super) fn apply_plan(board: &mut Expr, planned: &BTreeMap<String, f64>) {
    let Expr::List(items) = board else { return };
    for item in items {
        let Some(width) = form_atom(item, "uuid", 1).and_then(|id| planned.get(id)) else {
            continue;
        };
        let Expr::List(forms) = item else { continue };
        if let Some(Expr::List(width_form)) =
            forms.iter_mut().find(|form| form.head() == Some("width"))
        {
            width_form[1] = Expr::Atom(width.to_string());
        }
    }
}

pub(super) fn recorded_tracks(manifest: &Value) -> Option<BTreeSet<&str>> {
    let ids = manifest["track_ids"].as_array()?;
    if ids.is_empty()
        || manifest["kept"] != true
        || manifest["tracks"].as_u64()? != ids.len() as u64
    {
        return None;
    }
    let unique: BTreeSet<&str> = ids.iter().map(Value::as_str).collect::<Option<_>>()?;
    (unique.len() == ids.len()).then_some(unique)
}

/// Widen only verified generated tracks identified together in a native
/// width finding. Returns the exact IDs and requested new widths.
pub fn widen_recorded_neck_tracks(
    pcb: &mut Expr,
    manifest: &Value,
    native: &Value,
) -> BTreeMap<String, f64> {
    let Some(ids) = recorded_tracks(manifest) else {
        return BTreeMap::new();
    };
    let plan = widening_plan(pcb, &ids, native);
    apply_plan(pcb, &plan);
    plan
}

/// Rechecks a routed project and tries one rule-based enlargement of the
/// previously generated tracks that KiCad says meet too narrowly. The
/// input stays untouched. The published copy holds the original whenever
/// the trial fails to reduce the neck count or introduces any native finding.
pub fn widen_kicad_neck_tracks(
    source: &Path,
    board_id: &str,
    output: &Path,
) -> Result<SupplementalNeckRepair, String> {
    check_board_id(board_id).map_err(exit::input_error)?;
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let source_path =
        fs::canonicalize(source).map_err(|error| exit::input_error(error.to_string()))?;
    let parent_path =
        fs::canonicalize(parent).map_err(|error| exit::input_error(error.to_string()))?;
    if parent_path.starts_with(source_path) {
        return Err(exit::input_error(
            "place the repair output outside the source project",
        ));
    }
    if fs::symlink_metadata(output).is_ok() {
        return Err(exit::input_error(format!(
            "output directory {} already exists",
            output.display()
        )));
    }
    let manifest = read_json(&source.join("necks.json")).map_err(exit::input_error)?;
    if !manifest["track_ids"]
        .as_array()
        .is_some_and(|ids| !ids.is_empty() && manifest["kept"] == true)
    {
        return Err(exit::input_error(
            "necks.json has no recorded repair tracks; run the normal neck repair on a copy of the original routed project first",
        ));
    };
    let ids = recorded_tracks(&manifest).ok_or_else(|| {
        exit::input_error("recorded repair track IDs do not match the repair track count")
    })?;

    let stage = temporary_next_to(output);
    let result = (|| {
        copy_directory_tree(source, &stage)?;
        let repair = widen_in_copy(&stage, board_id, &ids)?;
        write_typed_json(&stage.join("supplemental-neck-repair.json"), &repair)?;
        fs::create_dir(output)
            .map_err(|error| format!("failed to reserve {}: {error}", output.display()))?;
        if let Err(error) = fs::rename(&stage, output) {
            let _ = fs::remove_dir(output);
            return Err(format!("failed to publish {}: {error}", output.display()));
        }
        Ok(repair)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&stage);
    }
    result
}

fn require_unchanged(path: &Path, expected: &str) -> Result<(), String> {
    if fs::read_to_string(path).map_err(|error| error.to_string())? == expected {
        Ok(())
    } else {
        Err("native check modified the board file".into())
    }
}

fn widen_in_copy(
    directory: &Path,
    board_id: &str,
    manifest: &BTreeSet<&str>,
) -> Result<SupplementalNeckRepair, String> {
    let board = directory.join(format!("{board_id}.kicad_pcb"));
    let original =
        fs::read_to_string(&board).map_err(|error| exit::input_error(error.to_string()))?;
    let mut pcb = parse(&original).map_err(exit::input_error)?;
    if pcb.head() != Some("kicad_pcb") {
        return Err(exit::input_error("input is not a KiCad board"));
    }
    let parity = directory.join(format!("{board_id}.kicad_sch")).is_file();
    let refill = pcb.child("zone").is_some()
        || pcb
            .children()
            .iter()
            .any(|item| item.head() == Some("footprint") && item.child("zone").is_some());
    native_report::remove_stale_board_run_reports(directory)?;
    let baseline_path = directory.join("drc-before-supplemental-necks.json");
    let baseline = native_check(&board, &baseline_path, parity, refill)?;
    require_unchanged(&board, &original)?;
    let before = findings(&baseline).len();
    let mut report = SupplementalNeckRepair {
        before,
        after: before,
        proposed_tracks: Vec::new(),
        widened_tracks: 0,
        note: "no native neck names two unchanged tracks from the recorded neck repair",
    };
    let mut kept_report = baseline_path;
    if report_capped(&baseline) {
        report.note = "the baseline DRC report is capped";
    } else {
        let plan = widening_plan(&pcb, manifest, &baseline);
        report.proposed_tracks = plan.keys().cloned().collect();
        if !plan.is_empty() {
            apply_plan(&mut pcb, &plan);
            let written = format!("{}\n", encode(&pcb));
            fs::write(&board, &written).map_err(|error| error.to_string())?;
            let trial_path = directory.join("drc-supplemental-neck-trial.json");
            let trial = native_check(&board, &trial_path, parity, refill)?;
            require_unchanged(&board, &written)?;
            let after = findings(&trial).len();
            if after < before && preserved(&baseline, &trial)? {
                report.after = after;
                report.widened_tracks = plan.len();
                report.note = "fewer connection-width errors with no new native finding";
                kept_report = trial_path;
            } else {
                report.note =
                    "widening did not reduce the necks without new or incomparable native findings";
                fs::write(&board, &original).map_err(|error| error.to_string())?;
            }
        }
    }
    fs::copy(kept_report, directory.join("drc.json")).map_err(|error| error.to_string())?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn widens_only_the_two_intact_recorded_segments_named_by_drc() {
        note_net_names(&parse("(kicad_pcb)").unwrap());
        let segment = |start| {
            supplemental_segment_expr(&SupplementalSegment {
                connection: "GND".into(),
                start: [start, 1.0],
                end: [2.0, 2.0],
                width: 0.127,
                layer: "F.Cu".into(),
            })
        };
        let first = segment(1.0);
        let second = segment(1.1);
        let unrecorded = segment(1.2);
        let id = |item: &Expr| form_atom(item, "uuid", 1).unwrap().to_owned();
        let a = id(&first);
        let b = id(&second);
        let c = id(&unrecorded);
        let manifest = BTreeSet::from([a.as_str(), b.as_str()]);
        let finding = |other: &str| {
            json!({
                "type": "connection_width", "severity": "error", "items": [{"uuid": a}, {"uuid": other}],
                "description": "Copper connection too narrow (board setup constraints minimum connection width 0.1270 mm; actual 0.0868 mm) (F.Cu)"
            })
        };
        let report = json!({"violations": [finding(&b), finding(&c)]});
        let mut pcb = Expr::List(vec![
            Expr::Atom("kicad_pcb".into()),
            first,
            second,
            unrecorded.clone(),
        ]);
        let plan = widening_plan(&pcb, &manifest, &report);
        assert_eq!(
            plan,
            BTreeMap::from([(a.clone(), 0.168), (b.clone(), 0.168)])
        );
        let mut locked = pcb.clone();
        if let Expr::List(items) = &mut locked
            && let Expr::List(segment) = &mut items[1]
        {
            segment.insert(1, Expr::Atom("locked".into()));
        }
        assert!(widening_plan(&locked, &manifest, &report).is_empty());
        apply_plan(&mut pcb, &plan);
        assert_eq!(form_f64(&pcb.children()[1], "width", 1).unwrap(), 0.168);
        assert_eq!(form_atom(&pcb.children()[1], "uuid", 1), Some(a.as_str()));
        assert_eq!(pcb.children()[3], unrecorded);
        assert!(widening_plan(&pcb, &manifest, &report).is_empty());
    }
}
