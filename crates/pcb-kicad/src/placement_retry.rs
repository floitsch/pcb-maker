// Copyright (C) 2026 Florian Loitsch.

//! Evidence from two attempts of the same mechanical layout brief.

use crate::{check_work, fill_handoff::has_fillable_zones, fs_util, native_report};
use pcb_kicad_file::kicad_sexpr::{Expr, encode, form_atom, parse};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Serialize)]
pub struct KiCadPlacementRetryEvidence {
    pub board_sha256: String,
    pub refilled: RetryNativeCheck,
    pub as_stored: Option<RetryNativeCheck>,
    pub minimum_check_work_seconds: f64,
    #[serde(skip)]
    project_inputs: BTreeMap<PathBuf, String>,
    #[serde(skip)]
    mechanics: Vec<String>,
}

#[derive(Clone, Serialize)]
pub struct RetryNativeCheck {
    pub opens: usize,
    pub errors: usize,
    pub warnings: usize,
    pub parity: usize,
    pub exclusions: usize,
    kicad_version: String,
    #[serde(skip)]
    errors_by_identity: BTreeMap<String, usize>,
    #[serde(skip)]
    warnings_by_identity: BTreeMap<String, usize>,
    #[serde(skip)]
    parity_by_identity: BTreeMap<String, usize>,
}

impl RetryNativeCheck {
    pub(crate) fn measure(report: &Value) -> Self {
        // The native report has already passed structural validation.
        let active = |field: &str| {
            report[field]
                .as_array()
                .unwrap()
                .iter()
                .filter(|item| item["excluded"] != true && item["severity"] != "exclusion")
        };
        let errors_by_identity = identities(
            active("violations").filter(|item| item["severity"] == "error"),
            |_| false,
        );
        let warnings_by_identity = identities(
            active("violations").filter(|item| item["severity"] == "warning"),
            // Intrinsic checks give the item's board position for navigation.
            // Preserve item ID, severity, rule text and multiplicity.
            |item| {
                item["type"] == "text_height"
                    || item["type"] == "lib_footprint_mismatch"
                        && item["items"].as_array().is_some_and(|items| {
                            matches!(items.as_slice(), [subject]
                                if subject["uuid"].as_str().is_some()
                                    && subject["description"].as_str()
                                        .is_some_and(|text| text.starts_with("Footprint ")))
                        })
            },
        );
        let parity_by_identity = identities(active("schematic_parity"), |_| true);
        Self {
            opens: active("unconnected_items").count(),
            errors: errors_by_identity.values().sum(),
            warnings: warnings_by_identity.values().sum(),
            parity: parity_by_identity.values().sum(),
            exclusions: ["violations", "unconnected_items", "schematic_parity"]
                .iter()
                .map(|field| report[*field].as_array().unwrap().len() - active(field).count())
                .sum(),
            kicad_version: report["kicad_version"].as_str().unwrap().into(),
            errors_by_identity,
            warnings_by_identity,
            parity_by_identity,
        }
    }

    fn retains_quality(&self, before: &Self) -> bool {
        self.kicad_version == before.kicad_version
            && [
                (&self.errors_by_identity, &before.errors_by_identity),
                (&self.parity_by_identity, &before.parity_by_identity),
            ]
            .into_iter()
            .all(|(after, before)| contains_findings(before, after))
    }

    pub(crate) fn qualifies_body_near_against(&self, before: &Self) -> Result<(), &'static str> {
        if self.opens != 0 || before.opens != 0 {
            return Err("native_opens_remain");
        }
        if !self.retains_quality(before) {
            return Err("new_native_errors_or_parity");
        }
        if !contains_findings(&before.warnings_by_identity, &self.warnings_by_identity) {
            return Err("additional_native_warnings");
        }
        Ok(())
    }
}

fn contains_findings(before: &BTreeMap<String, usize>, after: &BTreeMap<String, usize>) -> bool {
    after
        .iter()
        .all(|(item, count)| before.get(item).is_some_and(|n| n >= count))
}

fn identities<'a>(
    findings: impl Iterator<Item = &'a Value>,
    ignores_pose: impl Fn(&Value) -> bool,
) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for finding in findings {
        let mut items = finding["items"].as_array().unwrap().clone();
        if ignores_pose(finding) {
            for item in &mut items {
                item.as_object_mut().unwrap().remove("pos");
            }
        }
        items.sort_by_cached_key(Value::to_string);
        let key = json!([
            finding["type"],
            finding["severity"],
            finding["description"],
            items
        ]);
        *counts.entry(key.to_string()).or_default() += 1;
    }
    counts
}

fn mechanical_forms(pcb: &Expr) -> Vec<String> {
    fn edge_cut(item: &Expr) -> bool {
        form_atom(item, "layer", 1) == Some("Edge.Cuts")
    }
    fn locked(part: &Expr) -> bool {
        part.children()
            .iter()
            .any(|value| value.atom() == Some("locked"))
            || part
                .child("locked")
                .is_some_and(|lock| lock.children().get(1).and_then(Expr::atom) != Some("no"))
    }
    let mut forms = Vec::new();
    for item in pcb.children() {
        if matches!(item.head(), Some("footprint" | "module")) {
            let local_edges: Vec<_> = item
                .children()
                .iter()
                .filter(|item| edge_cut(item))
                .collect();
            if locked(item) || !local_edges.is_empty() {
                forms.push(
                    json!([
                        item.child("uuid")
                            .or_else(|| item.child("tstamp"))
                            .map(encode),
                        item.child("at").map(encode),
                        item.child("layer").map(encode),
                        locked(item),
                        local_edges.into_iter().map(encode).collect::<Vec<_>>()
                    ])
                    .to_string(),
                );
            }
        } else if matches!(item.head(), Some("layers" | "setup"))
            || edge_cut(item)
            || (item.head() == Some("zone") && item.child("keepout").is_some())
        {
            forms.push(encode(item));
        }
    }
    forms.sort_unstable();
    forms
}

impl KiCadPlacementRetryEvidence {
    /// Use the ordinary layout's checks, including its byte-verified stored
    /// proof when a refill could change the copper. This performs no native work.
    pub fn read(result: &Path, board_id: &str, verdict: &Value) -> Result<Self, String> {
        Self::read_with_fill(result, board_id, &verdict["fill_publication"])
    }

    pub(crate) fn read_with_fill(
        result: &Path,
        board_id: &str,
        stored_receipt: &Value,
    ) -> Result<Self, String> {
        let board = result.join(format!("{board_id}.kicad_pcb"));
        let board_sha256 = fs_util::file_sha256(&board)?;
        let text = fs::read_to_string(&board).map_err(|error| error.to_string())?;
        let pcb = parse(&text)?;
        if pcb.head() != Some("kicad_pcb") {
            return Err("the result is missing its KiCad PCB data".into());
        }
        if fs_util::file_sha256(&board)? != board_sha256 {
            return Err("the result PCB changed while reading its evidence".into());
        }
        let read = |file| {
            let report = native_report::read_kicad_report(&result.join(file))?;
            native_report::validate_drc_report(&report, &board)?;
            if report["ignored_checks"]
                .as_array()
                .is_some_and(|checks| checks.iter().any(|item| item["key"] == "unconnected_items"))
                || report["unconnected_across_sub_boards"]
                    .as_array()
                    .is_some_and(|items| !items.is_empty())
            {
                return Err("native connectivity evidence omits some connections".into());
            }
            Ok::<_, String>(RetryNativeCheck::measure(&report))
        };
        let refilled = read("drc.json")?;
        let as_stored = if stored_receipt["status"] == "saved"
            && stored_receipt["saved_board_sha256"] == board_sha256
        {
            Some(read("drc-saved-fill.json")?)
        } else if !has_fillable_zones(&pcb) {
            Some(refilled.clone())
        } else {
            None
        };
        let mut project_inputs = fs_util::project_companion_hashes(result)?;
        project_inputs.insert(
            "constraints.json".into(),
            fs_util::file_sha256(&result.join("constraints.json"))?,
        );
        Ok(Self {
            board_sha256,
            refilled,
            as_stored,
            minimum_check_work_seconds: check_work::check_seconds(check_work::check_inputs(&pcb)),
            project_inputs,
            mechanics: mechanical_forms(&pcb),
        })
    }

    pub fn qualifies_against(&self, before: &Self) -> Result<(), &'static str> {
        let (stored, original) = self.preserves_project_and_findings(before)?;
        if stored.opens != 0
            || self.refilled.opens != 0
            || original.opens == 0
            || before.refilled.opens == 0
        {
            return Err("native_opens_remain");
        }
        Ok(())
    }

    pub fn qualifies_body_near_against(&self, before: &Self) -> Result<(), &'static str> {
        let (stored, original) = self.preserves_project_and_findings(before)?;
        self.refilled
            .qualifies_body_near_against(&before.refilled)?;
        stored.qualifies_body_near_against(original)
    }

    fn preserves_project_and_findings<'a>(
        &'a self,
        before: &'a Self,
    ) -> Result<(&'a RetryNativeCheck, &'a RetryNativeCheck), &'static str> {
        let (Some(stored), Some(original)) = (&self.as_stored, &before.as_stored) else {
            return Err("missing_native_stored_proof");
        };
        if self.project_inputs != before.project_inputs || self.mechanics != before.mechanics {
            return Err("authored_project_or_mechanics_changed");
        }
        if !stored.retains_quality(original) || !self.refilled.retains_quality(&before.refilled) {
            return Err("new_native_errors_or_parity");
        }
        Ok((stored, original))
    }
}

/// Prove the alternate kept every other physical footprint and fitted the
/// selected footprint at the model's exact candidate pose.
pub fn verify_body_near_reseat(
    old: &Path,
    new: &Path,
    reseat: &crate::BodyNearReseat,
) -> Result<(), String> {
    verify_body_near_poses(old, new, &reseat.part, reseat.before_at, reseat.at)
}

pub(crate) fn verify_body_near_poses(
    old: &Path,
    new: &Path,
    part: &str,
    before_at: [f64; 3],
    after_at: [f64; 3],
) -> Result<(), String> {
    use pcb_kicad_file::{
        footprint_bodies::write_footprint_pose, kicad_nets::footprint_reference,
        kicad_sexpr::form_at,
    };
    let read = |file: &Path| -> Result<_, String> {
        let pcb = parse(&fs::read_to_string(file).map_err(|error| error.to_string())?)?;
        let mut result = BTreeMap::new();
        for item in pcb
            .children()
            .iter()
            .filter(|item| item.head() == Some("footprint"))
        {
            let id = form_atom(item, "uuid", 1).ok_or("footprint has no UUID")?;
            if result.insert(id.to_owned(), item.clone()).is_some() {
                return Err("repeated footprint UUID".into());
            }
        }
        Ok(result)
    };
    let source = read(old)?;
    let result = read(new)?;
    if source.len() != result.len() || source.is_empty() {
        return Err("footprint count changed".into());
    }
    let geometry = |item: &Expr| {
        item.children()
            .iter()
            .filter(|field| !matches!(field.head(), Some("fp_text" | "property" | "at")))
            .cloned()
            .collect::<Vec<_>>()
    };
    let same_pose = |a: [f64; 3], b: [f64; 3]| {
        (a[0] - b[0]).abs() <= 1.0e-6
            && (a[1] - b[1]).abs() <= 1.0e-6
            && ((a[2] - b[2] + 180.0).rem_euclid(360.0) - 180.0).abs() <= 1.0e-6
    };
    let mut moved = 0;
    for (id, mut item) in source {
        let candidate = result.get(&id).ok_or("footprint identity changed")?;
        let before = form_at(&item)?;
        let after = form_at(candidate)?;
        if footprint_reference(&item).as_deref() == Some(part) {
            if !same_pose(before, before_at) || !same_pose(after, after_at) {
                return Err("the selected footprint's before or after pose differs".into());
            }
            write_footprint_pose(
                &mut item,
                pcb_placer::Pose {
                    position: [after_at[0], after_at[1]],
                    angle: after_at[2],
                },
            )?;
            moved += 1;
        } else if !same_pose(before, after) {
            return Err(format!("retained footprint moved: {id}"));
        }
        if geometry(&item) != geometry(candidate) {
            return Err(format!("footprint geometry changed: {id}"));
        }
    }
    (moved == 1)
        .then_some(())
        .ok_or_else(|| "the retry did not select exactly one footprint".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finding(kind: &str, uuid: &str, x: i64) -> Value {
        json!({
            "type": kind, "description": "native finding", "severity": "error",
            "items": [{"uuid": uuid, "description": "Pad J1:1", "pos": {"x": x, "y": 10}}],
        })
    }

    #[test]
    fn placement_retry_uses_active_errors_and_preserves_inherited_parity_identity() {
        let mut excluded = finding("shorting_items", "J1", 10);
        excluded["excluded"] = json!(true);
        let native = |error: Value, parity: Value| {
            RetryNativeCheck::measure(&json!({
                "kicad_version": "10.0.8", "violations": [error],
                "schematic_parity": [parity], "unconnected_items": [],
            }))
        };
        let before = native(excluded.clone(), finding("net_conflict", "J1", 10));
        let moved = native(excluded.clone(), finding("net_conflict", "J1", 20));
        assert_eq!((moved.errors, moved.exclusions, moved.parity), (0, 1, 1));
        assert!(moved.retains_quality(&before));
        assert!(!native(excluded, finding("net_conflict", "J2", 20)).retains_quality(&before));
        assert!(
            !native(
                finding("shorting_items", "J1", 10),
                finding("net_conflict", "J1", 20)
            )
            .retains_quality(&before)
        );
    }

    #[test]
    fn near_retry_keeps_spatial_and_intrinsic_warning_identities() {
        let warnings = |kind: &str, id: &str, x| {
            let mut issue = finding(kind, id, x);
            issue["severity"] = json!("warning");
            if kind == "lib_footprint_mismatch" {
                issue["items"][0]["description"] = json!("Footprint C14");
            }
            RetryNativeCheck::measure(&json!({
                "kicad_version": "10.0.8", "violations": [issue],
                "schematic_parity": [], "unconnected_items": [],
            }))
            .warnings_by_identity
        };
        let before = warnings("text_height", "a", 10);
        assert!(contains_findings(
            &before,
            &warnings("text_height", "a", 20)
        ));
        assert!(!contains_findings(
            &before,
            &warnings("text_height", "b", 20)
        ));
        let library = warnings("lib_footprint_mismatch", "cap", 10);
        assert!(contains_findings(
            &library,
            &warnings("lib_footprint_mismatch", "cap", 20)
        ));
        assert!(!contains_findings(
            &library,
            &warnings("lib_footprint_mismatch", "other_cap", 20)
        ));
        assert!(!contains_findings(
            &warnings("connection_width", "a", 10),
            &warnings("connection_width", "a", 20)
        ));
    }

    #[test]
    fn a_body_near_reseat_keeps_each_other_pose_and_the_original_pad_shapes() {
        let source = r#"(kicad_pcb
          (footprint "C" (uuid a) (property "Reference" "C1") (layer "F.Cu") (at 1 1 0)
              (pad "1" smd rect (at 1 0) (size 1 1) (layers "F.Cu")))
          (footprint "U" (uuid b) (property "Reference" "U1") (layer "F.Cu") (at 2 2 0)))"#;
        let stage = fs_util::temporary_next_to(&std::env::temp_dir().join("body-near-reseat"));
        fs::create_dir(&stage).unwrap();
        let old = stage.join("old.kicad_pcb");
        let new = stage.join("new.kicad_pcb");
        fs::write(&old, source).unwrap();
        let reseat = crate::BodyNearReseat {
            part: "C1".into(),
            pin_of: "U1:3".into(),
            before_at: [1.0, 1.0, 0.0],
            at: [1.2, 1.0, 0.0],
            before_mm: 1.7,
            distance_mm: 1.3,
            max_mm: 1.5,
            local_pitch_mm: 0.1,
            relaxed_halos: ["C1".into(), "U1".into()],
        };
        let candidate = source.replace("(at 1 1 0)", "(at 1.2 1 0)");
        fs::write(&new, &candidate).unwrap();
        assert!(verify_body_near_reseat(&old, &new, &reseat).is_ok());
        fs::write(&new, candidate.replace("(at 2 2 0)", "(at 2 3 0)")).unwrap();
        assert!(verify_body_near_reseat(&old, &new, &reseat).is_err());
        fs::write(&new, candidate.replace("(size 1 1)", "(size 1 1.2)")).unwrap();
        assert!(verify_body_near_reseat(&old, &new, &reseat).is_err());
        fs::remove_dir_all(stage).unwrap();
    }

    #[test]
    fn placement_retry_keeps_native_locked_poses_and_board_outline() {
        let board = r#"(kicad_pcb
            (layers (0 "F.Cu" signal))
            (footprint "fixed" (locked yes) (uuid fixed) (layer "F.Cu") (at 1 1 0))
            (footprint "free" (uuid free) (layer "F.Cu") (at 2 2 0))
            (gr_line (start 0 0) (end 10 0) (layer "Edge.Cuts")))"#;
        let forms = |board: &str| mechanical_forms(&parse(board).unwrap());
        let original = forms(board);
        assert_eq!(forms(&board.replace("2 2 0", "3 3 90")), original);
        assert_ne!(forms(&board.replace("1 1 0", "1 2 0")), original);
        assert_ne!(forms(&board.replace("end 10 0", "end 11 0")), original);
    }
}
