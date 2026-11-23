// Copyright (C) 2026 Florian Loitsch.

//! Remove one native dangling leaf from copper generated after placement.

use crate::fill_handoff::connectivity::verify_open_witnesses;
use crate::fill_handoff::keys;
use crate::fill_handoff::layout::{copy_fill_payload, has_fillable_zones, project_inputs};
use crate::relocated_open_repair::width_witness;
use crate::relocated_open_repair::{
    editable_generated_tracks, native_evidence_limit, same_routing_rules,
};
use pcb_kicad_check::native_report;
use pcb_kicad_file::copper_emit::stored_supplemental_segment;
use pcb_kicad_file::fs_util::{
    check_board_id, copy_directory_tree, temporary_next_to, write_atomically, write_typed_json,
};
use pcb_kicad_file::kicad_nets::{NetNames, footprint_uuid, net_codes};
use pcb_kicad_file::kicad_sexpr::{Expr, encode, parse};
use pcb_kicad_route::open_repair::{
    eligible_open_partition, finding_types_at_cap, in_repair_copy, preserved, report_capped,
};
use serde::Serialize;
use serde_json::Value;
use std::cell::Cell;
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

type Partition = Vec<(String, Vec<String>)>;

#[derive(Debug, Serialize)]
pub struct GeneratedDanglingRepair {
    pub before: usize,
    pub after: usize,
    pub tried_track: Option<String>,
    pub removed_track: Option<String>,
    pub removed_mm: f64,
    pub native_checks: usize,
    pub check_wall_seconds: f64,
    pub note: String,
    pub schematic_parity_checked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stored_connectivity: Option<DanglingConnectivity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refilled_connectivity: Option<DanglingConnectivity>,
}

#[derive(Debug, Serialize)]
pub struct DanglingConnectivity {
    pub preserved_components: usize,
    pub preserved_copper_objects: usize,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub open_witnesses_changed: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub width_witness_replayed: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct GeneratedDanglingLeaf {
    pub track: String,
    pub net: String,
    pub layer: String,
    pub length_mm: f64,
    pub retry_argv: Vec<String>,
}

fn dangling_errors(report: &Value) -> impl Iterator<Item = &Value> {
    report["violations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|finding| {
            finding["type"] == "track_dangling"
                && finding["severity"] == "error"
                && finding["excluded"] != true
        })
}

fn sole_leaf(report: &Value) -> Option<&str> {
    if report_capped(report) || !finding_types_at_cap(report, "schematic_parity").is_empty() {
        return None;
    }
    let mut errors = dangling_errors(report);
    let witness = errors.next()?;
    let [item] = witness["items"].as_array()?.as_slice() else {
        return None;
    };
    let id = item["uuid"].as_str()?;
    errors.next().is_none().then_some(id)
}

fn references_id(item: &Expr, id: &str) -> bool {
    item.atom()
        .is_some_and(|atom| atom.trim_matches('"').eq_ignore_ascii_case(id))
        || item.children().iter().any(|item| references_id(item, id))
}

fn leaf(pcb: &Expr, placed: &Expr, id: &str) -> Option<GeneratedDanglingLeaf> {
    if !editable_generated_tracks(pcb, placed)
        .ok()?
        .contains(&id.to_ascii_lowercase())
    {
        return None;
    }
    let references = |item: &Expr| {
        // Groups and tuning records can reference native IDs without
        // contributing an electrical contact in KiCad's partition.
        matches!(item.head(), Some("group" | "generated")) && references_id(item, id)
    };
    if pcb.children().iter().any(references) {
        return None;
    }
    let segment = pcb.children().iter().find(|item| {
        item.head() == Some("segment")
            && footprint_uuid(item).is_some_and(|uuid| uuid.eq_ignore_ascii_case(id))
    })?;
    let names = NetNames::of(pcb);
    let copper = stored_supplemental_segment(segment, &net_codes(pcb), &names)?;
    Some(GeneratedDanglingLeaf {
        track: id.to_owned(),
        net: copper.connection,
        layer: copper.layer,
        length_mm: pcb_router::geometry::distance(copper.start, copper.end),
        retry_argv: Vec::new(),
    })
}

fn check_partition(
    mut before: Partition,
    after: Partition,
    id: &str,
) -> Result<DanglingConnectivity, String> {
    let mut removed = 0;
    for (_, items) in &mut before {
        let len = items.len();
        items.retain(|item| !item.eq_ignore_ascii_case(id));
        removed += len - items.len();
    }
    // This repair accepts a leaf of an existing group. An isolated track
    // is not evidence of a generated branch still attached to the net.
    before.sort();
    if removed != 1 || before.iter().any(|(_, items)| items.is_empty()) || before != after {
        return Err("removing the track changes the surviving native copper partition".into());
    }
    Ok(DanglingConnectivity {
        preserved_components: after.len(),
        preserved_copper_objects: after.iter().map(|(_, items)| items.len()).sum(),
        open_witnesses_changed: false,
        width_witness_replayed: false,
    })
}

#[derive(Debug, PartialEq)]
enum OpenWitnesses {
    Unchanged,
    NeedsProof,
}

fn without_leaf(before: &Value) -> Option<Value> {
    let id = sole_leaf(before)?;
    let mut expected = before.clone();
    expected["violations"].as_array_mut()?.retain(|finding| {
        !(finding["type"] == "track_dangling"
            && finding["severity"] == "error"
            && finding["excluded"] != true
            && finding["items"][0]["uuid"] == id)
    });
    Some(expected)
}

fn compare_findings(before: &Value, after: &Value) -> Result<OpenWitnesses, String> {
    let incompatible =
        || "the native trial changed findings beyond its dangling-track error".to_owned();
    let expected = without_leaf(before).ok_or_else(incompatible)?;
    if report_capped(after) || !finding_types_at_cap(after, "schematic_parity").is_empty() {
        return Err(incompatible());
    }
    if preserved(&expected, after)? && preserved(after, &expected)? {
        return Ok(OpenWitnesses::Unchanged);
    }
    if eligible_open_partition(&expected, after)? {
        return Ok(OpenWitnesses::NeedsProof);
    }
    Err(incompatible())
}

fn read(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))
}

/// Suggest the explicit repair only for an identified generated leaf on the
/// selected board. The repair command repeats all native checks.
pub fn suggest_generated_dangling_leaf(
    source: &Path,
    board_id: &str,
    placed: &Path,
    output: &Path,
) -> Option<GeneratedDanglingLeaf> {
    check_board_id(board_id).ok()?;
    let report = native_report::read_kicad_report(&source.join("drc.json")).ok()?;
    if !crate::run_diagnosis::native_report_matches_board(&report, board_id) {
        return None;
    }
    let track = sole_leaf(&report)?;
    same_routing_rules(source, placed, board_id).ok()?;
    let filename = format!("{board_id}.kicad_pcb");
    let source_pcb = parse(&read(&source.join(&filename)).ok()?).ok()?;
    let placed_pcb = parse(&read(&placed.join(filename)).ok()?).ok()?;
    let mut candidate = leaf(&source_pcb, &placed_pcb, track)?;
    candidate.retry_argv = vec![
        "pcb-maker".into(),
        "repair-kicad-dangling".into(),
        source.display().to_string(),
        board_id.into(),
        output.display().to_string(),
        "--generated-from".into(),
        placed.display().to_string(),
    ];
    Some(candidate)
}

/// Remove at most one router-owned leaf on a fresh sibling project. The
/// original design and all copper supplied by placement must match.
pub fn repair_kicad_generated_dangling(
    source: &Path,
    board_id: &str,
    placed: &Path,
    output: &Path,
) -> Result<GeneratedDanglingRepair, String> {
    check_board_id(board_id).map_err(crate::input_error)?;
    same_routing_rules(source, placed, board_id)?;
    let filename = format!("{board_id}.kicad_pcb");
    let placed = parse(&read(&placed.join(&filename)).map_err(crate::input_error)?)
        .map_err(crate::input_error)?;
    let started = Instant::now();
    let deadline = started + Duration::from_secs(150);
    let expired = || Instant::now() >= deadline;
    in_repair_copy(source, output, |stage| {
        let pcb_path = stage.join(&filename);
        let original = read(&pcb_path)?;
        let pcb = parse(&original)?;
        let project = project_inputs(stage)?;
        let parity = pcb_path.with_extension("kicad_sch").is_file()
            || pcb_path.with_extension("sch").is_file();
        native_report::remove_stale_board_run_reports(stage)?;
        let calls = Cell::new(0);
        let charge = || calls.set(calls.get() + 1);
        let check = |path: &Path, label: &str, refill| {
            charge();
            native_report::check_kept_route_input(path, &stage.join(label), parity, refill, expired)
        };
        let before = [
            check(&pcb_path, "drc-before-dangling-stored.json", false)?,
            check(&pcb_path, "drc-before-dangling-refilled.json", true)?,
        ];
        let count = dangling_errors(&before[1]).count();
        let mut report = GeneratedDanglingRepair {
            before: count,
            after: count,
            tried_track: None,
            removed_track: None,
            removed_mm: 0.0,
            native_checks: 0,
            check_wall_seconds: 0.0,
            note: native_evidence_limit(&before.each_ref()).unwrap_or_else(|| {
                "stored and refilled KiCad must name the same sole generated dangling track".into()
            }),
            schematic_parity_checked: parity,
            stored_connectivity: None,
            refilled_connectivity: None,
        };
        let leaves = before.each_ref().map(sole_leaf);
        if let [Some(a), Some(b)] = leaves
            && a.eq_ignore_ascii_case(b)
            && let Some(leaf) = leaf(&pcb, &placed, a)
        {
            report.tried_track = Some(leaf.track.clone());
            let trial = temporary_next_to(stage);
            let width_source = temporary_next_to(&trial);
            let attempt = (|| {
                copy_directory_tree(stage, &trial)?;
                let path = trial.join(&filename);
                let mut candidate = pcb.clone();
                let Expr::List(items) = &mut candidate else {
                    return Err("the routed project does not contain a PCB".to_string());
                };
                items.retain(|item| {
                    !(item.head() == Some("segment")
                        && footprint_uuid(item).is_some_and(|id| id.eq_ignore_ascii_case(a)))
                });
                write_atomically(&path, encode(&candidate).as_bytes())?;
                if has_fillable_zones(&candidate) {
                    charge();
                    native_report::save_kept_route_fill(
                        &path,
                        &trial.join("fill.json"),
                        parity,
                        expired,
                    )?;
                    copy_fill_payload(&mut candidate, &parse(&read(&path)?)?)?;
                }
                let saved = encode(&candidate);
                write_atomically(&path, saved.as_bytes())?;
                let after = [
                    check(&path, "drc-dangling-trial-stored.json", false)?,
                    check(&path, "drc-dangling-trial-refilled.json", true)?,
                ];
                let mut proofs = Vec::new();
                for (index, refill) in [false, true].into_iter().enumerate() {
                    let mut replayed = false;
                    let changed = match compare_findings(&before[index], &after[index]) {
                        Ok(witnesses) => witnesses == OpenWitnesses::NeedsProof,
                        Err(reason) => {
                            let comparison =
                                without_leaf(&before[index]).ok_or_else(|| reason.clone())?;
                            replayed = width_witness::Proof {
                                source: &pcb,
                                source_project: stage,
                                filename: &filename,
                                scratch: &width_source,
                                target: &leaf.net,
                                moved: a,
                                try_third_order: true,
                            }
                            .replay(
                                &candidate,
                                &before[index],
                                &comparison,
                                &after[index],
                                refill,
                                check,
                            )?;
                            if !replayed {
                                return Err(reason);
                            }
                            keys(&before[index], "unconnected_items")
                                != keys(&after[index], "unconnected_items")
                        }
                    };
                    let version = before[index]["kicad_version"]
                        .as_str()
                        .ok_or("missing KiCad version")?;
                    charge();
                    let original =
                        native_report::connectivity_partition(&pcb_path, version, refill, expired)?;
                    charge();
                    let candidate =
                        native_report::connectivity_partition(&path, version, refill, expired)?;
                    if changed {
                        verify_open_witnesses(
                            &original,
                            &candidate,
                            &before[index],
                            &after[index],
                        )?;
                    }
                    let mut proof = check_partition(original, candidate, a)?;
                    proof.open_witnesses_changed = changed;
                    proof.width_witness_replayed = replayed;
                    proofs.push(proof);
                }
                if expired() || read(&path)? != saved || project_inputs(&trial)? != project {
                    return Err("native verification changed the candidate project or exceeded the deadline".into());
                }
                Ok((saved, after, proofs))
            })();
            let _ = fs::remove_dir_all(&trial);
            let _ = fs::remove_dir_all(&width_source);
            match attempt {
                Ok((saved, after, proofs)) => {
                    if read(&pcb_path)? != original || project_inputs(stage)? != project {
                        return Err("native verification changed the source project copy".into());
                    }
                    write_atomically(&pcb_path, saved.as_bytes())?;
                    write_typed_json(&stage.join("drc.json"), &after[0])?;
                    write_typed_json(&stage.join("drc-refilled.json"), &after[1])?;
                    let [stored, refilled]: [_; 2] = proofs.try_into().expect("both modes checked");
                    report.stored_connectivity = Some(stored);
                    report.refilled_connectivity = Some(refilled);
                    report.removed_track = Some(leaf.track);
                    report.removed_mm = leaf.length_mm;
                    report.after = 0;
                    report.note = "removed one generated leaf with identical surviving copper connectivity in both KiCad check modes".into();
                }
                Err(reason) => report.note = reason,
            }
        }
        if report.removed_track.is_none() {
            if read(&pcb_path)? != original || project_inputs(stage)? != project {
                return Err("native verification changed the source project copy".into());
            }
            write_typed_json(&stage.join("drc.json"), &before[0])?;
            write_typed_json(&stage.join("drc-refilled.json"), &before[1])?;
        }
        report.native_checks = calls.get();
        report.check_wall_seconds = started.elapsed().as_secs_f64();
        write_typed_json(&stage.join("dangling-repair.json"), &report)?;
        Ok(report)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pcb_kicad_file::copper_emit::{SupplementalSegment, supplemental_segment_expr};
    use serde_json::json;

    #[test]
    fn requires_identical_surviving_groups_and_an_attached_leaf() {
        let partition = |groups: &[&[&str]]| -> Partition {
            groups
                .iter()
                .map(|items| {
                    (
                        "GND".into(),
                        items.iter().map(|id| id.to_string()).collect(),
                    )
                })
                .collect()
        };
        let before = partition(&[&["a", "leaf"], &["b", "c"]]);
        let after = partition(&[&["a"], &["b", "c"]]);
        assert!(check_partition(before.clone(), after.clone(), "leaf").is_ok());
        assert!(check_partition(before, partition(&[&["a", "b", "c"]]), "leaf").is_err());
        let detached = partition(&[&["a"], &["b", "c"], &["leaf"]]);
        assert!(check_partition(detached, after, "leaf").is_err());
    }

    #[test]
    fn provenance_protects_held_and_changed_tracks() {
        let design = r#"(net 1 "GND") (footprint "R" (at 1 1))"#;
        let placed = parse(&format!("(kicad_pcb {design})")).unwrap();
        pcb_kicad_file::kicad_nets::note_net_names(&placed);
        let track = supplemental_segment_expr(&SupplementalSegment {
            connection: "GND".into(),
            start: [2.0, 3.0],
            end: [2.8, 3.0],
            width: 0.1,
            layer: "In2.Cu".into(),
        });
        let id = footprint_uuid(&track).unwrap();
        let board = |extra: &str| {
            parse(&format!("(kicad_pcb {design} {} {extra})", encode(&track))).unwrap()
        };
        assert!(leaf(&board(""), &placed, &id).is_some());
        assert!(leaf(&board(""), &board(""), &id).is_none());
        let moved = parse(&encode(&board("")).replace("(at 1 1)", "(at 1 2)")).unwrap();
        assert!(leaf(&moved, &placed, &id).is_none());
        let changed = parse(&encode(&board("")).replace("2.8", "2.9")).unwrap();
        assert!(leaf(&changed, &placed, &id).is_none());
        // The router can add a tuning object beyond the placed input.
        // Preserve its track membership even if native connectivity agrees.
        assert!(
            leaf(
                &board(&format!(
                    r#"(generated (type tuning_pattern) (members "{id}"))"#
                )),
                &placed,
                &id
            )
            .is_none()
        );
    }

    #[test]
    fn a_new_finding_or_multiple_native_leaves_declines_repair() {
        let first = json!({"severity":"error", "type":"track_dangling", "items":[{"uuid":"a"}]});
        let second = json!({"severity":"error", "type":"track_dangling", "items":[{"uuid":"b"}]});
        let before = json!({"violations":[first], "unconnected_items":[]});
        let after = json!({"violations":[], "unconnected_items":[]});
        assert_eq!(
            compare_findings(&before, &after).unwrap(),
            OpenWitnesses::Unchanged
        );
        let mut different = after;
        different["unconnected_items"] = json!([{"type":"unconnected_items"}]);
        assert!(compare_findings(&before, &different).is_err());
        let mut ambiguous = before;
        ambiguous["violations"].as_array_mut().unwrap().push(second);
        assert!(sole_leaf(&ambiguous).is_none());
    }

    #[test]
    fn alternate_open_witnesses_must_prove_the_same_counts_by_net() {
        let dangling =
            json!({"severity":"error", "type":"track_dangling", "items":[{"uuid":"leaf"}]});
        let open = |a, b| json!({"severity":"error", "type":"unconnected_items", "items":[{"uuid":a},{"uuid":b}]});
        let before = json!({"violations":[dangling], "unconnected_items":[open("leaf","b")]});
        let after = json!({"violations":[], "unconnected_items":[open("a","b")]});
        let wrong = json!({"violations":[], "unconnected_items":[open("c","d")]});
        let original: Partition = vec![
            ("GND".into(), vec!["a".into(), "leaf".into()]),
            ("GND".into(), vec!["b".into()]),
            ("VCC".into(), vec!["c".into()]),
            ("VCC".into(), vec!["d".into()]),
        ];
        let mut surviving = original.clone();
        surviving[0].1.pop();
        assert_eq!(
            compare_findings(&before, &after).unwrap(),
            OpenWitnesses::NeedsProof
        );
        assert!(verify_open_witnesses(&original, &surviving, &before, &after).is_ok());
        assert!(verify_open_witnesses(&original, &surviving, &before, &wrong).is_err());
    }
}
