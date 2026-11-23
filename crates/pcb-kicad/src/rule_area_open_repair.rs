// Copyright (C) 2026 Florian Loitsch.

//! Tries a local, native-verified move for a routed board's trapped pad.

use crate::fill_handoff::layout::{copy_fill_payload, has_fillable_zone, project_inputs};
use crate::{KiCadBoardRouterConfig, KiCadExitMove};
use pcb_kicad_check::native_report;
use pcb_kicad_file::footprint_bodies::write_footprint_pose;
use pcb_kicad_file::fs_util::{
    check_board_id, copy_directory_tree, temporary_next_to, write_atomically, write_typed_json,
};
use pcb_kicad_file::kicad_nets::{footprint_reference, footprint_uuid, item_net, net_codes};
use pcb_kicad_file::kicad_sexpr::{Expr, encode, form_at, parse};
use pcb_kicad_route::open_repair::{
    eligible_open_partition, in_repair_copy, preserved, repair_kicad_opens_locally, report_capped,
    verify_native_partition_merges,
};
use pcb_placer::Pose;
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    time::{Duration, Instant},
};

#[derive(Serialize)]
pub struct RuleAreaOpenRepair {
    pub status: &'static str,
    pub pad: String,
    pub note: &'static str,
    pub stored_opens_before: usize,
    pub stored_opens_after: usize,
    pub refilled_opens_before: usize,
    pub refilled_opens_after: usize,
    pub schematic_parity_checked: bool,
    pub placement_work_seconds: f64,
    pub search_work_seconds: f64,
    pub primary_native_checks: usize,
    pub connectivity_checks: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connectivity_note: Option<String>,
    pub wall_seconds: f64,
    pub added_copper: usize,
    pub moves: Vec<KiCadExitMove>,
}

/// Only a known footprint library warning can follow a moved footprint
/// without becoming a new finding. Check its UUID and the exact move first.
fn with_library_moves(
    report: &Value,
    moves: &BTreeMap<String, &KiCadExitMove>,
    trial: bool,
) -> Value {
    let mut adjusted = report.clone();
    for warning in adjusted["violations"].as_array_mut().into_iter().flatten() {
        if warning["type"] != "lib_footprint_mismatch" || warning["severity"] != "warning" {
            continue;
        }
        let Some([item]) = warning["items"].as_array_mut().map(Vec::as_mut_slice) else {
            continue;
        };
        let Some(moved) = item["uuid"].as_str().and_then(|uuid| moves.get(uuid)) else {
            continue;
        };
        let pose = if trial { moved.to } else { moved.from };
        if ["x", "y"].into_iter().zip(pose).all(|(axis, expected)| {
            item["pos"][axis]
                .as_f64()
                .is_some_and(|position| (position - expected).abs() <= 1e-6)
        }) {
            item.as_object_mut().expect("native DRC item").remove("pos");
        }
    }
    adjusted
}

struct NativeRepair<'a> {
    original: &'a Path,
    candidate: &'a Path,
    moved: BTreeMap<String, &'a KiCadExitMove>,
    nets: &'a [String],
    additions: &'a [Expr],
}

impl NativeRepair<'_> {
    fn preserves_connection(
        &self,
        before: &Value,
        after: &Value,
        refilled: bool,
        report: &mut RuleAreaOpenRepair,
    ) -> Result<bool, String> {
        let before = &with_library_moves(before, &self.moved, false);
        let after = &with_library_moves(after, &self.moved, true);
        if preserved(before, after)? {
            return Ok(true);
        }
        if !eligible_open_partition(before, after)? {
            return Ok(false);
        }
        let mut check = || {
            let deadline = Instant::now() + Duration::from_secs(20);
            let expired = || Instant::now() >= deadline;
            let version = before["kicad_version"]
                .as_str()
                .ok_or("missing native KiCad version")?;
            report.connectivity_checks += 1;
            let old =
                native_report::connectivity_partition(self.original, version, refilled, expired)?;
            report.connectivity_checks += 1;
            let new =
                native_report::connectivity_partition(self.candidate, version, refilled, expired)?;
            verify_native_partition_merges(
                old,
                new,
                self.nets,
                opens(before)?.len() - opens(after)?.len(),
                self.additions,
            )
        };
        match check() {
            Ok(_) => Ok(true),
            Err(error) => {
                report.connectivity_note = Some(error);
                Ok(false)
            }
        }
    }
}

pub(crate) fn opens(report: &Value) -> Result<&[Value], String> {
    report["unconnected_items"]
        .as_array()
        .map(Vec::as_slice)
        .ok_or("native report did not contain unconnected items".into())
}

pub(crate) fn opened(report: &Value, uuid: &str) -> Result<bool, String> {
    Ok(opens(report)?.iter().any(|item| {
        item["items"].as_array().into_iter().flatten().any(|end| {
            end["uuid"]
                .as_str()
                .is_some_and(|id| id.eq_ignore_ascii_case(uuid))
        })
    }))
}

fn unique_pad(pcb: &Expr, reference: &str, number: &str) -> Option<String> {
    let mut footprints = pcb.children().iter().filter(|f| {
        f.head() == Some("footprint") && footprint_reference(f).as_deref() == Some(reference)
    });
    let footprint = footprints.next()?;
    if footprints.next().is_some() {
        return None;
    }
    let mut pads = footprint.children().iter().filter(|pad| {
        pad.head() == Some("pad") && pad.children().get(1).and_then(Expr::atom) == Some(number)
    });
    let uuid = footprint_uuid(pads.next()?)?;
    pads.next().is_none().then_some(uuid)
}

fn apply_moves(pcb: &mut Expr, moves: &[KiCadExitMove]) -> Result<Option<Vec<String>>, String> {
    let codes = net_codes(pcb);
    let Expr::List(children) = pcb else {
        return Ok(None);
    };
    let mut nets = BTreeSet::new();
    for moved in moves {
        let mut footprints = children.iter_mut().filter(|f| {
            f.head() == Some("footprint") && footprint_reference(f).as_deref() == Some(&moved.part)
        });
        let Some(footprint) = footprints.next() else {
            return Ok(None);
        };
        if footprints.next().is_some() || footprint.child("zone").is_some() {
            return Ok(None);
        }
        let at = form_at(footprint)?;
        let angle = (at[2] - moved.from[2] + 180.0).rem_euclid(360.0) - 180.0;
        if (at[0] - moved.from[0]).abs() > 1e-6
            || (at[1] - moved.from[1]).abs() > 1e-6
            || angle.abs() > 1e-6
            || footprint
                .children()
                .iter()
                .any(|field| field.atom() == Some("locked") || field.head() == Some("locked"))
        {
            return Ok(None);
        }
        nets.extend(
            footprint
                .children()
                .iter()
                .filter(|pad| pad.head() == Some("pad"))
                .filter_map(|pad| item_net(pad, &codes))
                .filter(|name| !name.is_empty())
                .map(str::to_owned),
        );
        write_footprint_pose(
            footprint,
            Pose {
                position: [moved.to[0], moved.to[1]],
                angle: moved.to[2],
            },
        )?;
    }
    Ok(Some(nets.into_iter().collect()))
}

/// Repairs one native-open pad on a private copy. The published board keeps
/// its original placement and copper when a move or its route fails verification.
pub fn repair_kicad_rule_area_open(
    source: &Path,
    board_id: &str,
    pad: &str,
    output: &Path,
    router: &KiCadBoardRouterConfig,
) -> Result<RuleAreaOpenRepair, String> {
    let started = Instant::now();
    check_board_id(board_id)?;
    let (reference, number) = pad
        .split_once(':')
        .filter(|(a, b)| !a.is_empty() && !b.is_empty())
        .ok_or_else(|| crate::input_error("expected a pad as REF:PAD"))?;
    in_repair_copy(source, output, |stage| {
        let filename = format!("{board_id}.kicad_pcb");
        let board = stage.join(&filename);
        let original = fs::read_to_string(&board).map_err(|error| error.to_string())?;
        let mut pcb = parse(&original)?;
        let uuid = unique_pad(&pcb, reference, number)
            .ok_or_else(|| crate::input_error(format!("pad {pad} is missing or ambiguous")))?;
        let parity =
            board.with_extension("kicad_sch").is_file() || board.with_extension("sch").is_file();
        let check = |board: &Path, path: &Path, refill| {
            native_report::check_kept_route_input(board, path, parity, refill, || false)
        };
        let before_stored = check(&board, &stage.join("rule-area-before-stored.json"), false)?;
        let before_refilled = check(&board, &stage.join("rule-area-before-refilled.json"), true)?;
        if fs::read_to_string(&board).map_err(|error| error.to_string())? != original {
            return Err("native validation changed the incumbent PCB".into());
        }
        let mut report = RuleAreaOpenRepair {
            status: "unchanged",
            pad: pad.into(),
            note: "the target pad is not an open endpoint after native refill",
            stored_opens_before: opens(&before_stored)?.len(),
            stored_opens_after: opens(&before_stored)?.len(),
            refilled_opens_before: opens(&before_refilled)?.len(),
            refilled_opens_after: opens(&before_refilled)?.len(),
            schematic_parity_checked: parity,
            placement_work_seconds: 0.0,
            search_work_seconds: 0.0,
            primary_native_checks: 2,
            connectivity_checks: 0,
            connectivity_note: None,
            wall_seconds: 0.0,
            added_copper: 0,
            moves: Vec::new(),
        };
        let work = temporary_next_to(stage);
        let trial = (|| -> Result<(), String> {
            if report_capped(&before_stored) || report_capped(&before_refilled) {
                report.note = "native baseline reached a finding cap";
                return Ok(());
            }
            if !opened(&before_refilled, &uuid)? {
                return Ok(());
            }
            let target = format!("{reference}.{number}");
            let proposal = pcb_kicad_place::board_placer::propose_rule_area_pad_move(
                stage, board_id, router, &target,
            )?;
            report.placement_work_seconds = proposal.work_seconds;
            report.note = "no legal move clears the target pad from an authored rule area";
            if !proposal
                .moves
                .iter()
                .any(|moved| moved.opened.contains(&target))
            {
                return Ok(());
            }
            let Some(nets) = apply_moves(&mut pcb, &proposal.moves)? else {
                report.note = "a proposed footprint is locked, ambiguous or carries a rule area";
                return Ok(());
            };
            fs::create_dir(&work).map_err(|error| error.to_string())?;
            let moved = work.join("moved");
            copy_directory_tree(stage, &moved)?;
            write_atomically(&moved.join(&filename), encode(&pcb).as_bytes())?;
            let routed = work.join("routed");
            let repair = repair_kicad_opens_locally(&moved, board_id, &routed, &nets)?;
            report.primary_native_checks += 1 + repair
                .trials
                .iter()
                .map(|trial| {
                    usize::from(trial.native_report.is_some())
                        + usize::from(trial.contact_refinement.is_some())
                })
                .sum::<usize>();
            report.search_work_seconds = repair
                .trials
                .iter()
                .filter_map(|trial| trial.local_search.as_ref())
                .map(|search| {
                    search.expanded_nodes as f64 / pcb_router::router::expansions_per_second()
                })
                .sum();
            report.note = "local routing could not connect the moved pads";
            if repair.added_copper == 0 {
                return Ok(());
            }
            let verified = routed.join(&filename);
            let project = project_inputs(&routed)?;
            let mut final_pcb = parse(&fs::read_to_string(&verified).map_err(|e| e.to_string())?)?;
            let is_track = |item: &&Expr| matches!(item.head(), Some("segment" | "arc" | "via"));
            let old_tracks: BTreeSet<_> = pcb
                .children()
                .iter()
                .filter(is_track)
                .filter_map(footprint_uuid)
                .collect();
            let additions: Vec<Expr> = final_pcb
                .children()
                .iter()
                .filter(is_track)
                .filter(|item| footprint_uuid(item).is_some_and(|uuid| !old_tracks.contains(&uuid)))
                .cloned()
                .collect();
            if has_fillable_zone(&final_pcb) {
                native_report::run_fill_handoff_drc(
                    &verified,
                    &work.join("fill.json"),
                    parity,
                    true,
                )?;
                report.primary_native_checks += 1;
                let native_pcb = parse(&fs::read_to_string(&verified).map_err(|e| e.to_string())?)?;
                // Keep the routed PCB's form of every non-fill object, including
                // all original tracks, vias, pads, board edges and rule areas.
                copy_fill_payload(&mut final_pcb, &native_pcb)?;
            }
            let materialized = encode(&final_pcb);
            write_atomically(&verified, materialized.as_bytes())?;
            let after_stored = check(&verified, &stage.join("rule-area-trial-stored.json"), false)?;
            let after_refilled = check(
                &verified,
                &stage.join("rule-area-trial-refilled.json"),
                true,
            )?;
            report.primary_native_checks += 2;
            if fs::read_to_string(&verified).map_err(|error| error.to_string())? != materialized
                || project_inputs(&routed)? != project
            {
                return Err("native validation changed the trial PCB or project".into());
            }
            report.note = "the saved trial added a native finding or did not improve both checks";
            let evidence = NativeRepair {
                original: &board,
                candidate: &verified,
                moved: pcb
                    .children()
                    .iter()
                    .filter_map(|item| {
                        let reference = footprint_reference(item)?;
                        let moved = proposal
                            .moves
                            .iter()
                            .find(|moved| moved.part == reference)?;
                        Some((footprint_uuid(item)?, moved))
                    })
                    .collect(),
                nets: &nets,
                additions: &additions,
            };
            if opens(&after_stored)?.len() >= report.stored_opens_before
                || opens(&after_refilled)?.len() >= report.refilled_opens_before
                || opened(&after_stored, &uuid)?
                || opened(&after_refilled, &uuid)?
                || !evidence.preserves_connection(
                    &before_stored,
                    &after_stored,
                    false,
                    &mut report,
                )?
                || !evidence.preserves_connection(
                    &before_refilled,
                    &after_refilled,
                    true,
                    &mut report,
                )?
            {
                return Ok(());
            }
            if fs::read_to_string(&board).map_err(|error| error.to_string())? != original {
                return Err("native validation changed the incumbent PCB".into());
            }
            native_report::remove_stale_board_run_reports(stage)?;
            write_typed_json(&stage.join("drc.json"), &after_stored)?;
            let copper_report =
                fs::read(routed.join("open-repair.json")).map_err(|error| error.to_string())?;
            write_atomically(&stage.join("open-repair.json"), &copper_report)?;
            write_atomically(&board, materialized.as_bytes())?;
            report.status = "improved";
            report.note = "the target closed without new native findings in either check";
            report.stored_opens_after = opens(&after_stored)?.len();
            report.refilled_opens_after = opens(&after_refilled)?.len();
            report.added_copper = repair.added_copper;
            report.moves = proposal.moves;
            Ok(())
        })();
        if work.is_dir() {
            fs::remove_dir_all(&work).map_err(|error| error.to_string())?;
        }
        trial?;
        report.wall_seconds = started.elapsed().as_secs_f64();
        write_typed_json(&stage.join("rule-area-open-repair.json"), &report)?;
        Ok(report)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_a_library_warning_can_follow_a_known_footprint_move() {
        let moved = KiCadExitMove {
            part: "C1".into(),
            from: [1.0, 2.0, 0.0],
            to: [3.0, 2.0, 0.0],
            opened: Vec::new(),
        };
        let moves = BTreeMap::from([("c1".to_string(), &moved)]);
        let report = |kind, severity, id, x| {
            json!({"violations": [{
                "type": kind, "severity": severity,
                "items": [{"uuid": id, "pos": {"x": x, "y": 2.0}}],
            }]})
        };
        let old = report("lib_footprint_mismatch", "warning", "c1", 1.0);
        let new = report("lib_footprint_mismatch", "warning", "c1", 3.0);
        assert_eq!(
            with_library_moves(&old, &moves, false),
            with_library_moves(&new, &moves, true)
        );
        for unchanged in [
            report("clearance", "warning", "c1", 3.0),
            report("lib_footprint_mismatch", "error", "c1", 3.0),
            report("lib_footprint_mismatch", "warning", "other", 3.0),
            report("lib_footprint_mismatch", "warning", "c1", 3.1),
        ] {
            assert_eq!(with_library_moves(&unchanged, &moves, true), unchanged);
        }
    }
}
