// Copyright (C) 2026 Florian Loitsch.

//! Compare two routes of the same placed board under their common native inputs.

use crate::fill_handoff::{
    counts, keys,
    layout::{native_pad_via_teardrop, project_inputs},
    same_metadata,
};
use crate::{FillDrcCounts, check_work, fs_util, native_report, verdict::item_net};
use pcb_kicad_file::kicad_sexpr::{Expr, encode, form_atom, parse};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Serialize)]
pub struct KiCadFixedRouteComparison {
    pub status: &'static str,
    pub reason: String,
    pub incumbent_board: PathBuf,
    pub incumbent_board_sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate_board_sha256: Option<String>,
    pub schematic_parity_checked: bool,
    pub native_checks: usize,
    pub verification_work_seconds: f64,
    pub verification_wall_seconds: f64,
    pub required_verification_work_seconds: f64,
    /// Stored teardrop forms independently reproduced by KiCad on each side.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub verified_regenerated_teardrops: BTreeMap<&'static str, usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as_stored: Option<NativeComparison>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refilled: Option<NativeComparison>,
}

#[derive(Clone, Debug, Serialize)]
pub struct NativeComparison {
    before: FillDrcCounts,
    after: FillDrcCounts,
    new_native_findings: BTreeMap<String, BTreeMap<String, usize>>,
    open_nets: Vec<OpenNetChange>,
}

#[derive(Clone, Debug, Serialize)]
struct OpenNetChange {
    net: String,
    before: usize,
    after: usize,
    status: &'static str,
}

struct BoardState {
    path: PathBuf,
    sha256: String,
    design: Vec<String>,
    teardrops: Vec<String>,
    project_inputs: BTreeMap<PathBuf, String>,
    check_work_seconds: f64,
}

impl BoardState {
    fn read(project: &Path, board_id: &str) -> Result<Self, String> {
        let path = project.join(format!("{board_id}.kicad_pcb"));
        let sha256 = fs_util::file_sha256(&path)?;
        let pcb = parse(&fs::read_to_string(&path).map_err(|error| error.to_string())?)?;
        if pcb.head() != Some("kicad_pcb") {
            return Err(format!("{} is not a KiCad board", path.display()));
        }
        Ok(Self {
            path,
            sha256,
            design: design(&pcb),
            teardrops: stored_teardrops(&pcb),
            project_inputs: project_inputs(project)?,
            check_work_seconds: check_work::check_seconds(check_work::check_inputs(&pcb)),
        })
    }

    fn compare_design(&self, other: &Self) -> Result<(), String> {
        if self.project_inputs != other.project_inputs {
            return Err(format!(
                "{} and {} have different project rules, schematics or library inputs",
                self.path.display(),
                other.path.display()
            ));
        }
        if self.design != other.design {
            return Err(format!(
                "{} and {} differ outside freely routable copper or derived zone fill",
                self.path.display(),
                other.path.display()
            ));
        }
        Ok(())
    }

    fn unchanged(&self) -> Result<bool, String> {
        Ok(fs_util::file_sha256(&self.path)? == self.sha256
            && project_inputs(self.path.parent().expect("project for board"))?
                == self.project_inputs)
    }
}

pub(crate) fn design(pcb: &Expr) -> Vec<String> {
    let grouped: BTreeSet<_> = pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("group"))
        .filter_map(|group| group.child("members"))
        .flat_map(|members| members.children().iter().skip(1).filter_map(Expr::atom))
        .collect();
    let locked_or_grouped = |form: &Expr| {
        locked(form)
            || ["uuid", "tstamp"]
                .iter()
                .any(|name| form_atom(form, name, 1).is_some_and(|id| grouped.contains(id)))
    };
    let mut forms: Vec<_> = pcb
        .children()
        .iter()
        .filter(|form| {
            form.head().is_some()
                && !possibly_generated_teardrop(form)
                && (!matches!(form.head(), Some("segment" | "via" | "arc"))
                    || locked_or_grouped(form))
        })
        .map(|form| {
            let mut form = form.clone();
            remove_derived_fill(&mut form);
            encode(&form)
        })
        .collect();
    forms.sort_unstable();
    forms
}

fn locked(form: &Expr) -> bool {
    form.children()
        .iter()
        .any(|part| part.atom() == Some("locked"))
        || form
            .child("locked")
            .is_some_and(|lock| lock.children().get(1).and_then(Expr::atom) != Some("no"))
}

fn possibly_generated_teardrop(form: &Expr) -> bool {
    native_pad_via_teardrop(form) && !locked(form) && form.child("filled_polygon").is_some()
}

fn stored_teardrops(pcb: &Expr) -> Vec<String> {
    let mut forms: Vec<_> = pcb
        .children()
        .iter()
        .filter(|form| possibly_generated_teardrop(form))
        .map(encode)
        .collect();
    forms.sort_unstable();
    forms
}

fn remove_derived_fill(form: &mut Expr) {
    let parent = match form.head() {
        Some("zone") => Some("zone"),
        Some("pad") => Some("pad"),
        _ => None,
    };
    let Expr::List(parts) = form else { return };
    parts.retain(|part| match parent {
        Some("zone") => !matches!(
            part.head(),
            Some("filled_polygon" | "filled_segments" | "fill_segments")
        ),
        Some("pad") => part.head() != Some("zone_layer_connections"),
        _ => true,
    });
    parts.iter_mut().for_each(remove_derived_fill);
}

pub struct FixedRouteIncumbent {
    board: BoardState,
    board_id: String,
}

impl FixedRouteIncumbent {
    /// Check the fixed geometry and authored electrical inputs before routing.
    pub fn read(placed: &Path, incumbent: &Path, board_id: &str) -> Result<Self, String> {
        fs_util::check_board_id(board_id)?;
        let board = BoardState::read(incumbent, board_id)?;
        board.compare_design(&BoardState::read(placed, board_id)?)?;
        Ok(Self {
            board,
            board_id: board_id.into(),
        })
    }

    pub fn required_verification_work(&self) -> f64 {
        4.0 * self.board.check_work_seconds
    }

    /// Run four private native checks only when their modeled work fits.
    /// The input boards are both preserved, including when the trial loses.
    pub fn compare(
        &self,
        candidate: &Path,
        work_remaining: f64,
        wall_deadline: Option<Instant>,
    ) -> KiCadFixedRouteComparison {
        let started = Instant::now();
        let mut result = KiCadFixedRouteComparison {
            status: "inconclusive",
            reason: String::new(),
            incumbent_board: self.board.path.clone(),
            incumbent_board_sha256: self.board.sha256.clone(),
            candidate_board_sha256: None,
            schematic_parity_checked: false,
            native_checks: 0,
            verification_work_seconds: 0.0,
            verification_wall_seconds: 0.0,
            required_verification_work_seconds: self.required_verification_work(),
            verified_regenerated_teardrops: BTreeMap::new(),
            as_stored: None,
            refilled: None,
        };
        if let Err(error) = self.check(candidate, work_remaining, wall_deadline, &mut result) {
            result.reason = error;
        }
        result.verification_wall_seconds = started.elapsed().as_secs_f64();
        result
    }

    fn check(
        &self,
        candidate: &Path,
        work_remaining: f64,
        wall_deadline: Option<Instant>,
        result: &mut KiCadFixedRouteComparison,
    ) -> Result<(), String> {
        let output = BoardState::read(candidate, &self.board_id)?;
        result.candidate_board_sha256 = Some(output.sha256.clone());
        self.board.compare_design(&output)?;
        if !self.board.unchanged()? {
            return Err(
                "the incumbent board or its project inputs changed during the retry".into(),
            );
        }
        let required = 2.0 * (self.board.check_work_seconds + output.check_work_seconds);
        result.required_verification_work_seconds = required;
        if work_remaining < required {
            return Err(format!(
                "native comparison needs {required:.1} modeled seconds and {work_remaining:.1} remain in the retry budget"
            ));
        }
        let estimated_wall = Instant::now() + Duration::from_secs_f64(4.0 * required);
        let deadline =
            wall_deadline.map_or(estimated_wall, |deadline| deadline.min(estimated_wall));
        if Instant::now() >= deadline {
            return Err("the retry wall-clock guard expired before native comparison".into());
        }
        let stage = fs_util::temporary_next_to(candidate);
        fs::create_dir(&stage).map_err(|error| error.to_string())?;
        let checked = self.native_comparison(candidate, &output, &stage, deadline, result);
        let _ = fs::remove_dir_all(&stage);
        checked
    }

    fn native_comparison(
        &self,
        candidate: &Path,
        output: &BoardState,
        stage: &Path,
        deadline: Instant,
        result: &mut KiCadFixedRouteComparison,
    ) -> Result<(), String> {
        let report_directory = candidate.join("fixed-route-comparison");
        fs::create_dir(&report_directory).map_err(|error| error.to_string())?;
        let parity = self.board.path.with_extension("kicad_sch").is_file();
        let check_teardrops = self.board.teardrops != output.teardrops;
        let mut checks = Vec::new();
        for (label, state) in [("incumbent", &self.board), ("candidate", output)] {
            let staged = stage.join(label);
            fs_util::copy_directory_tree(state.path.parent().expect("project for board"), &staged)?;
            let path = staged.join(state.path.file_name().expect("board filename"));
            if fs_util::file_sha256(&path)? != state.sha256
                || project_inputs(&staged)? != state.project_inputs
            {
                return Err(format!("the {label} changed before its native checks"));
            }
            for (mode, refill) in [("stored", false), ("refilled", true)] {
                if Instant::now() >= deadline {
                    return Err(
                        "the retry wall-clock guard expired during native comparison".into(),
                    );
                }
                let report = report_directory.join(format!("{label}-{mode}.json"));
                result.native_checks += 1;
                result.verification_work_seconds += state.check_work_seconds;
                let expired = || Instant::now() >= deadline;
                let checked = if check_teardrops && refill && !state.teardrops.is_empty() {
                    // KiCad must reproduce the complete set from the current
                    // copper and settings before it can be treated as fill.
                    let checked =
                        native_report::save_kept_route_fill(&path, &report, parity, expired)?;
                    let regenerated =
                        parse(&fs::read_to_string(&path).map_err(|error| error.to_string())?)?;
                    if stored_teardrops(&regenerated) != state.teardrops
                        || project_inputs(&staged)? != state.project_inputs
                    {
                        return Err(format!(
                            "the {label}'s stored teardrops did not exactly reproduce on native refill with unchanged project inputs"
                        ));
                    }
                    result
                        .verified_regenerated_teardrops
                        .insert(label, state.teardrops.len());
                    checked
                } else {
                    native_report::check_kept_route_input(&path, &report, parity, refill, expired)?
                };
                native_report::validate_drc_report(&checked, &path)?;
                checks.push(checked);
            }
        }
        if !self.board.unchanged()? || !output.unchanged()? {
            return Err("a board or its project inputs changed during native comparison".into());
        }
        let [before_stored, before_refill, after_stored, after_refill] = checks.as_slice() else {
            unreachable!()
        };
        same_metadata(before_stored, before_refill, after_stored)?;
        same_metadata(before_stored, before_refill, after_refill)?;
        let stored = summarize(before_stored, after_stored)?;
        let refilled = summarize(before_refill, after_refill)?;
        result.schematic_parity_checked = parity;
        let (status, reason) = decision(&stored, &refilled);
        result.status = status;
        result.reason = reason.into();
        result.as_stored = Some(stored);
        result.refilled = Some(refilled);
        Ok(())
    }
}

fn summarize(before: &Value, after: &Value) -> Result<NativeComparison, String> {
    for report in [before, after] {
        let ignored = report["ignored_checks"]
            .as_array()
            .ok_or("KiCad did not list its ignored checks")?;
        if ignored.iter().any(|check| {
            matches!(
                check["key"].as_str(),
                Some("unconnected_items" | "missing_footprint" | "net_conflict")
            )
        }) {
            return Err("KiCad did not run a required connectivity or schematic check".into());
        }
    }
    let before_counts = counts(before);
    let after_counts = counts(after);
    if !before_counts.possibly_capped_types.is_empty()
        || !after_counts.possibly_capped_types.is_empty()
    {
        return Err("KiCad may have capped a native report. Inspect the four raw reports before selecting a route".into());
    }
    let mut new_native_findings = BTreeMap::new();
    for field in ["violations", "schematic_parity"] {
        let before_findings = keys(before, field);
        for (key, quantity) in keys(after, field) {
            let added = quantity.saturating_sub(before_findings.get(&key).copied().unwrap_or(0));
            if added != 0 {
                *new_native_findings
                    .entry(field.to_owned())
                    .or_insert_with(BTreeMap::new)
                    .entry(key.0)
                    .or_default() += added;
            }
        }
    }
    let old = open_nets(before)?;
    let new = open_nets(after)?;
    let open_nets = old
        .keys()
        .chain(new.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|net| {
            let before = old.get(net).copied().unwrap_or(0);
            let after = new.get(net).copied().unwrap_or(0);
            let status = match (before, after) {
                (0, _) => "newly_open",
                (_, 0) => "closed",
                (before, after) if before < after => "worsened",
                (before, after) if before > after => "improved",
                _ => "unchanged",
            };
            OpenNetChange {
                net: net.clone(),
                before,
                after,
                status,
            }
        })
        .collect();
    Ok(NativeComparison {
        before: before_counts,
        after: after_counts,
        new_native_findings,
        open_nets,
    })
}

fn open_nets(report: &Value) -> Result<BTreeMap<String, usize>, String> {
    let mut counts = BTreeMap::new();
    for finding in report["unconnected_items"]
        .as_array()
        .expect("validated native opens")
    {
        let mut net = None;
        for item in finding["items"].as_array().expect("validated native items") {
            let resolved = item["description"].as_str().and_then(item_net);
            if resolved.is_none()
                || net
                    .as_ref()
                    .is_some_and(|first| Some(first) != resolved.as_ref())
            {
                return Err("KiCad's open endpoints did not identify the same net".into());
            }
            net = resolved;
        }
        *counts
            .entry(net.ok_or("a native open has no net identity")?)
            .or_default() += 1;
    }
    Ok(counts)
}

fn decision(
    stored: &NativeComparison,
    refilled: &NativeComparison,
) -> (&'static str, &'static str) {
    let modes = [stored, refilled];
    if modes
        .iter()
        .any(|mode| !mode.new_native_findings.is_empty())
    {
        return (
            "new_native_findings",
            "the candidate has new native DRC or schematic findings",
        );
    }
    if modes
        .iter()
        .any(|mode| mode.after.opens >= mode.before.opens)
    {
        return (
            "no_improvement",
            "the candidate needs fewer native opens in both saved and refilled checks",
        );
    }
    if modes
        .iter()
        .any(|mode| mode.open_nets.iter().any(|net| net.after > net.before))
    {
        return (
            "improved_with_tradeoffs",
            "the candidate has fewer native opens overall and leaves more opens on some nets. Review the per-net changes before selecting a route",
        );
    }
    (
        "improved",
        "the candidate has fewer native opens in both checks with no new native findings or additional per-net opens",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_ordinary_unlocked_copper_and_derived_fill_may_change() {
        let board = r#"(kicad_pcb
            (footprint "U" (at 1 2) (pad "1" (zone_layer_connections "F.Cu")))
            (zone (polygon (pts (xy 0 1))) (filled_polygon (pts (xy 0 1))))
            (segment (uuid "movable"))
            (via (uuid "authored") (locked yes))
            (arc (uuid "grouped"))
            (group "nested" (uuid "inner") (members "grouped"))
            (group "authored" (uuid "outer") (locked yes) (members "inner")))"#;
        let state = |source: &str| design(&parse(source).unwrap());
        let original = state(board);
        for (a, b) in [
            (r#"(segment (uuid "movable"))"#, ""),
            (r#"(zone_layer_connections "F.Cu")"#, ""),
            (
                "(filled_polygon (pts (xy 0 1)))",
                "(filled_polygon (pts (xy 1 2)))",
            ),
        ] {
            assert_eq!(original, state(&board.replace(a, b)));
        }
        for changed in [
            board.replace("(at 1 2)", "(at 1 3)"),
            board.replace(r#"(via (uuid "authored") (locked yes))"#, ""),
            board.replace(r#"(arc (uuid "grouped"))"#, ""),
            board.replace("(polygon (pts (xy 0 1)))", "(polygon (pts (xy 0 2)))"),
        ] {
            assert_ne!(original, state(&changed));
        }
    }

    #[test]
    fn generated_teardrop_candidates_keep_exact_fill_and_authored_zone_identity() {
        let raw = parse(r#"(kicad_pcb (layers (0 "F.Cu" signal) (2 "B.Cu" signal)))"#).unwrap();
        let make_board = |form: &str| {
            parse(&format!(
                r#"(kicad_pcb (layers (0 "F.Cu" signal) (2 "B.Cu" signal)) {form})"#
            ))
            .unwrap()
        };
        let drop = r#"(zone (net "GND") (layer "F.Cu") (attr (teardrop (type padvia)))
            (fill yes) (polygon (pts (xy 0 0) (xy 1 0) (xy 0 1)))
            (filled_polygon (layer "F.Cu") (pts (xy 0 0) (xy 1 0) (xy 0 1))))"#;
        let native = make_board(drop);
        assert_eq!(design(&native), design(&raw));
        assert_eq!(stored_teardrops(&native).len(), 1);
        assert_ne!(
            stored_teardrops(&native),
            stored_teardrops(&make_board(&drop.replace("(xy 1 0)", "(xy 2 0)")))
        );
        for authored in [
            drop.replace("(fill yes)", "(fill yes) (uuid \"authored\")"),
            drop.replace("(fill yes)", "(fill yes) (locked yes)"),
            drop.replace("(fill yes)", "(fill no)"),
            drop.replace(
                "(attr (teardrop (type padvia)))",
                "(keepout (tracks not_allowed))",
            ),
            drop.replace("(type padvia)", "(type track_end)"),
        ] {
            let authored = make_board(&authored);
            assert_ne!(design(&authored), design(&raw));
            assert!(stored_teardrops(&authored).is_empty());
        }
    }

    #[test]
    fn both_native_modes_must_improve_and_changed_findings_are_visible() {
        let open = |net| {
            json!({
                "type":"unconnected_items", "severity":"error",
                "items":[{"description":format!("Pad 1 [{net}] of U1")}, {"description":format!("Pad 2 [{net}] of U2")}],
            })
        };
        let report = |violations, opens| {
            json!({
                "ignored_checks": [], "violations": violations,
                "unconnected_items": opens, "schematic_parity": [],
            })
        };
        let inherited =
            json!([{"type":"connection_width", "severity":"warning", "items":[{"uuid":"a"}]}]);
        let before = report(inherited.clone(), vec![open("A"), open("B"), open("C")]);
        let after = report(inherited, vec![open("A"), open("D")]);
        let mode = summarize(&before, &after).unwrap();
        assert_eq!(decision(&mode, &mode).0, "improved_with_tradeoffs");
        assert_eq!(
            mode.open_nets
                .iter()
                .map(|net| (net.net.as_str(), net.status))
                .collect::<Vec<_>>(),
            [
                ("A", "unchanged"),
                ("B", "closed"),
                ("C", "closed"),
                ("D", "newly_open")
            ]
        );
        let alternate = report(
            json!([{"type":"connection_width", "severity":"warning", "items":[{"uuid":"b"}]}]),
            vec![open("A")],
        );
        let findings = summarize(&before, &alternate).unwrap();
        assert_eq!(
            findings.new_native_findings["violations"]["connection_width"],
            1
        );
        assert_eq!(decision(&mode, &findings).0, "new_native_findings");
        let no_change = summarize(&before, &before).unwrap();
        assert_eq!(decision(&mode, &no_change).0, "no_improvement");
    }
}
