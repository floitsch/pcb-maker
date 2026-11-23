// Copyright (C) 2026 Florian Loitsch.

//! One bounded, KiCad-verified pour between an open SMD pad and retained copper.

use crate::fill_handoff::layout::{copy_fill_with_native_teardrops, project_inputs};
use crate::rule_area_open_repair::{opened, opens};
use pcb_kicad_check::native_report;
use pcb_kicad_file::copper_geometry::{lower_pad, rule_area_polygon_points};
use pcb_kicad_file::fs_util::{
    check_board_id, copy_directory_tree, temporary_next_to, write_atomically, write_typed_json,
};
use pcb_kicad_file::geometry::{point_in_polygon, point_segment_distance_squared};
use pcb_kicad_file::kicad_nets::{footprint_reference, footprint_uuid, item_net, net_codes};
use pcb_kicad_file::kicad_sexpr::{
    Expr, encode, form_at, form_atom, form_f64, form_xy, parse, unescape,
};
use pcb_kicad_route::board_router::{LayerTable, local_pour_zone};
use pcb_kicad_route::open_repair::{
    OpenConnectivityProof, eligible_open_partition, in_repair_copy, report_capped,
    verify_native_partition_merges,
};
use pcb_router::geometry::Shape;
use serde::Serialize;
use serde_json::Value;
use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};

const MARGIN_MM: f64 = 0.30;
const DEADLINE: Duration = Duration::from_secs(60);

type Partition = Vec<(String, Vec<String>)>;

#[derive(Serialize)]
pub struct LocalPourRepair {
    pub status: &'static str,
    pub pad: String,
    pub pad_uuid: Option<String>,
    pub net: Option<String>,
    pub reason: Option<String>,
    pub stored_opens_before: usize,
    pub stored_opens_after: usize,
    pub refilled_opens_before: usize,
    pub refilled_opens_after: usize,
    pub schematic_parity_checked: bool,
    pub wall_seconds: f64,
    pub proposal: Option<LocalPourProposal>,
    pub stored_connectivity: Option<OpenConnectivityProof>,
    pub refilled_connectivity: Option<OpenConnectivityProof>,
}

#[derive(Serialize)]
pub struct LocalPourProposal {
    pub via_uuid: String,
    pub zone_uuid: String,
    pub layer: String,
    pub minimum_mm: [f64; 2],
    pub maximum_mm: [f64; 2],
}

fn component<'a>(partition: &'a Partition, id: &str, net: &str) -> Option<&'a [String]> {
    partition
        .iter()
        .find(|(name, members)| name == net && members.iter().any(|member| member == id))
        .map(|(_, members)| members.as_slice())
}

fn target_merge(
    before: Partition,
    after: Partition,
    net: &str,
    pad: &str,
    via: &str,
) -> Result<OpenConnectivityProof, String> {
    let contains_via = |group: &[String]| group.iter().any(|id| id == via);
    if component(&before, pad, net).is_none_or(contains_via)
        || component(&before, via, net).is_none()
        || !component(&after, pad, net).is_some_and(contains_via)
    {
        return Err("native connectivity did not show the proposed pad-to-via join".into());
    }
    verify_native_partition_merges(before, after, &[net.to_owned()], 1, &[])
}

/// Repeated pad numbers occur on real components. Admit only a single
/// physical land independently identified by both native reports.
fn select_pad<'a>(
    owner: &'a Expr,
    number: &str,
    stored: &Value,
    refilled: &Value,
) -> Result<Option<(&'a Expr, String)>, String> {
    let mut selected = None;
    for pad in owner
        .children()
        .iter()
        .filter(|item| item.head() == Some("pad"))
    {
        if pad.children().get(1).and_then(Expr::atom) != Some(number) {
            continue;
        }
        let Some(uuid) = footprint_uuid(pad).map(|id| id.to_ascii_lowercase()) else {
            continue;
        };
        if opened(stored, &uuid)? && opened(refilled, &uuid)? {
            if selected.is_some() {
                return Err(
                    "more than one physical land of the selected pad is open in both checks".into(),
                );
            }
            selected = Some((pad, uuid));
        }
    }
    Ok(selected)
}

pub(crate) struct LocalPourRuleArea<'a> {
    pub(crate) zone: &'a Expr,
    pub(crate) owner: Option<&'a Expr>,
}

pub(crate) fn check_rule_area<'a>(
    pcb: &'a Expr,
    layers: &LayerTable,
    mask: u32,
    center: [f64; 2],
) -> Result<LocalPourRuleArea<'a>, String> {
    let mut area = None;
    for (zone, owner) in pcb
        .children()
        .iter()
        .flat_map(|item| {
            let owner = matches!(item.head(), Some("footprint" | "module")).then_some(item);
            std::iter::once((item, None)).chain(owner.into_iter().flat_map(|footprint| {
                footprint
                    .children()
                    .iter()
                    .map(move |zone| (zone, Some(footprint)))
            }))
        })
        .filter(|(item, _)| item.head() == Some("zone"))
    {
        let Some(keepout) = zone.child("keepout") else {
            continue;
        };
        if layers.mask_of_item(zone)? & mask == 0 {
            continue;
        }
        let track_rule = form_atom(keepout, "tracks", 1);
        let fill_rule = form_atom(keepout, "copperpour", 1);
        if (track_rule == Some("not_allowed") || fill_rule == Some("not_allowed"))
            && point_in_polygon(center, &rule_area_polygon_points(zone)?)
        {
            if fill_rule == Some("not_allowed") {
                return Err("an authored rule area at the selected pad prohibits zone fill".into());
            }
            if area.is_none() && track_rule == Some("not_allowed") && fill_rule == Some("allowed") {
                area = Some(LocalPourRuleArea { zone, owner });
            }
        }
    }
    area.ok_or_else(|| {
        "no rule area at the selected pad's center prohibits tracks while allowing zone fill".into()
    })
}

fn propose(
    pcb: &Expr,
    owner: &Expr,
    pad: &Expr,
    pad_uuid: &str,
    net: &str,
    partition: &Partition,
) -> Result<Option<(LocalPourProposal, Expr)>, String> {
    let target =
        component(partition, pad_uuid, net).ok_or("native connectivity omitted the open pad")?;
    let layers = LayerTable::from_pcb(pcb)?;
    let mask = layers.pad_mask(pad)?;
    if pad.children().get(2).and_then(Expr::atom) != Some("smd") || mask.count_ones() != 1 {
        return Err("local pour needs a surface-mount pad with one copper layer".into());
    }
    let layer = &layers.names[mask.trailing_zeros() as usize];
    let land = lower_pad(pad, form_at(owner)?)?;
    check_rule_area(pcb, &layers, mask, land.center)?;
    let codes = net_codes(pcb);
    let nearest = pcb.children().iter().filter_map(|via| {
        if via.head() != Some("via")
            || form_atom(via, "layers", 1) != Some("F.Cu")
            || form_atom(via, "layers", 2) != Some("B.Cu")
            || via.child("padstack").is_some()
            || item_net(via, &codes).is_none_or(|raw| unescape(raw) != net)
        {
            return None;
        }
        let id = footprint_uuid(via)?.to_ascii_lowercase();
        if target.contains(&id) || component(partition, &id, net).is_none() {
            return None;
        }
        let center = form_xy(via, "at").ok()?;
        let radius = form_f64(via, "size", 1).ok()? * 0.5;
        (radius > 0.0).then_some((id, center, radius))
    });
    let distance = |p| point_segment_distance_squared(land.center, p, p);
    let Some((via_uuid, center, radius)) =
        nearest.min_by(|a, b| distance(a.1).total_cmp(&distance(b.1)).then(a.0.cmp(&b.0)))
    else {
        return Ok(None);
    };
    let via = Shape::Circle { center, radius }.aabb();
    let bounds = land
        .geometry
        .router_shape()
        .aabb()
        .union(via)
        .inflated(MARGIN_MM);
    let [min_x, min_y] = bounds.minimum;
    let [max_x, max_y] = bounds.maximum;
    let raw_net = item_net(pad, &codes).ok_or("open pad has no named net")?;
    let (zone_uuid, zone) = local_pour_zone(
        raw_net,
        layer,
        [pad_uuid, &via_uuid],
        [min_x, min_y, max_x, max_y],
    )?;
    if pcb
        .children()
        .iter()
        .any(|item| footprint_uuid(item).as_deref() == Some(&zone_uuid))
    {
        return Err("the local repair zone identity already exists on the board".into());
    }
    Ok(Some((
        LocalPourProposal {
            via_uuid,
            zone_uuid,
            layer: layer.clone(),
            minimum_mm: bounds.minimum,
            maximum_mm: bounds.maximum,
        },
        zone,
    )))
}

/// Copies the project and publishes one new local zone if both native modes
/// verify the requested merge and preserve the other electrical findings.
pub fn repair_kicad_local_pour(
    source: &Path,
    board_id: &str,
    output: &Path,
    selection: &str,
) -> Result<LocalPourRepair, String> {
    check_board_id(board_id)?;
    let (reference, number) = selection
        .split_once(':')
        .filter(|(reference, number)| !reference.is_empty() && !number.is_empty())
        .ok_or_else(|| crate::input_error("expected a pad as REF:PAD"))?;
    let started = Instant::now();
    let expired = || started.elapsed() >= DEADLINE;
    in_repair_copy(source, output, |stage| {
        let filename = format!("{board_id}.kicad_pcb");
        let board = stage.join(&filename);
        let read = |path: &Path| fs::read_to_string(path).map_err(|error| error.to_string());
        let original = read(&board)?;
        let pcb = parse(&original)?;
        let mut owners = pcb.children().iter().filter(|item| {
            matches!(item.head(), Some("footprint" | "module"))
                && footprint_reference(item).as_deref() == Some(reference)
        });
        let owner = owners
            .next()
            .filter(|_| owners.next().is_none())
            .ok_or_else(|| {
                crate::input_error(format!("footprint {reference} is missing or ambiguous"))
            })?;
        if !owner.children().iter().any(|item| {
            item.head() == Some("pad")
                && item.children().get(1).and_then(Expr::atom) == Some(number)
        }) {
            return Err(crate::input_error(format!("pad {selection} is missing")));
        }
        let inputs = project_inputs(stage)?;
        let parity =
            board.with_extension("kicad_sch").is_file() || board.with_extension("sch").is_file();
        let check = |path: &Path, name: &str, refill| {
            native_report::check_kept_route_input(path, &stage.join(name), parity, refill, expired)
        };
        let before_stored = check(&board, "local-pour-before-stored.json", false)?;
        let before_refilled = check(&board, "local-pour-before-refilled.json", true)?;
        let stored_opens = opens(&before_stored)?.len();
        let refilled_opens = opens(&before_refilled)?.len();
        let mut report = LocalPourRepair {
            status: "unchanged",
            pad: selection.into(),
            pad_uuid: None,
            net: None,
            reason: None,
            stored_opens_before: stored_opens,
            stored_opens_after: stored_opens,
            refilled_opens_before: refilled_opens,
            refilled_opens_after: refilled_opens,
            schematic_parity_checked: parity,
            wall_seconds: 0.0,
            proposal: None,
            stored_connectivity: None,
            refilled_connectivity: None,
        };
        let work = temporary_next_to(stage);
        let attempt = (|| -> Result<(), String> {
            if report_capped(&before_stored) || report_capped(&before_refilled) {
                return Err("native baseline reached a finding cap".into());
            }
            let (pad, pad_uuid) = select_pad(owner, number, &before_stored, &before_refilled)?
                .ok_or("the selected pad is not a native open endpoint in both checks")?;
            report.pad_uuid = Some(pad_uuid.clone());
            let codes = net_codes(&pcb);
            let net = item_net(pad, &codes)
                .filter(|net| !net.is_empty())
                .map(|net| unescape(net).into_owned())
                .ok_or("the selected pad has no net")?;
            report.net = Some(net.clone());
            let version = before_refilled["kicad_version"]
                .as_str()
                .ok_or("missing native KiCad version")?;
            let partition = |path: &Path, refill| {
                native_report::connectivity_partition(path, version, refill, expired)
            };
            let refilled_partition = partition(&board, true)?;
            let (proposal, zone) = propose(&pcb, owner, pad, &pad_uuid, &net, &refilled_partition)?
                .ok_or("no retained same-net through via in another native component")?;
            report.proposal = Some(proposal);
            let proposal = report.proposal.as_ref().unwrap();
            let mut candidate_board = pcb.clone();
            let Expr::List(items) = &mut candidate_board else {
                unreachable!()
            };
            items.push(zone);
            copy_directory_tree(stage, &work)?;
            let trial = work.join(&filename);
            write_atomically(&trial, encode(&candidate_board).as_bytes())?;
            native_report::save_kept_route_fill(
                &trial,
                &stage.join("local-pour-fill.json"),
                parity,
                expired,
            )?;
            copy_fill_with_native_teardrops(&mut candidate_board, parse(&read(&trial)?)?)?;
            if !candidate_board.children().iter().any(|item| {
                footprint_uuid(item).as_deref() == Some(&proposal.zone_uuid)
                    && item.child("filled_polygon").is_some()
            }) {
                return Err("the proposed region did not yield native filled copper".into());
            }
            let candidate = encode(&candidate_board);
            write_atomically(&trial, candidate.as_bytes())?;
            let after_stored = check(&trial, "local-pour-trial-stored.json", false)?;
            let after_refilled = check(&trial, "local-pour-trial-refilled.json", true)?;
            for (before, after) in [
                (&before_stored, &after_stored),
                (&before_refilled, &after_refilled),
            ] {
                if opens(before)?.len().checked_sub(opens(after)?.len()) != Some(1)
                    || opened(after, &pad_uuid)?
                    || !eligible_open_partition(before, after)?
                {
                    return Err("the local pour did not close exactly the selected open with identical other native findings".into());
                }
            }
            let prove =
                |before, after| target_merge(before, after, &net, &pad_uuid, &proposal.via_uuid);
            let stored = prove(partition(&board, false)?, partition(&trial, false)?)?;
            let refilled = prove(refilled_partition, partition(&trial, true)?)?;
            if expired()
                || read(&board)? != original
                || read(&trial)? != candidate
                || project_inputs(stage)? != inputs
                || project_inputs(&work)? != inputs
            {
                return Err(
                    "the repair deadline expired or a native check changed the project".into(),
                );
            }
            native_report::remove_stale_board_run_reports(stage)?;
            write_typed_json(&stage.join("drc.json"), &after_stored)?;
            write_typed_json(&stage.join("drc-refilled.json"), &after_refilled)?;
            write_atomically(&board, candidate.as_bytes())?;
            report.status = "improved";
            report.stored_opens_after -= 1;
            report.refilled_opens_after -= 1;
            report.stored_connectivity = Some(stored);
            report.refilled_connectivity = Some(refilled);
            Ok(())
        })();
        if work.is_dir() {
            fs::remove_dir_all(&work).map_err(|error| error.to_string())?;
        }
        if let Err(reason) = attempt {
            if read(&board)? != original || project_inputs(stage)? != inputs {
                return Err("the local pour trial changed the incumbent PCB or project".into());
            }
            report.reason = Some(reason);
        }
        report.wall_seconds = started.elapsed().as_secs_f64();
        write_typed_json(&stage.join("local-pour-repair.json"), &report)?;
        Ok(report)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn pad_numbers_can_repeat_when_both_native_reports_identify_one_land() {
        let owner =
            parse(r#"(footprint X (pad "MP" smd rect (uuid a)) (pad "MP" smd rect (uuid b)))"#)
                .unwrap();
        let report = |ids: &[&str]| json!({"unconnected_items": ids.iter().map(|id| json!({"items": [{"uuid": id}]})).collect::<Vec<_>>()});
        let select =
            |stored, refilled| select_pad(&owner, "MP", &report(stored), &report(refilled));
        assert_eq!(select(&["A", "b"], &["a"]).unwrap().unwrap().1, "a");
        assert!(select(&["a"], &["b"]).unwrap().is_none());
        assert!(select(&["a", "b"], &["a", "b"]).is_err());
    }

    #[test]
    fn local_pour_requires_a_track_keepout_on_the_pad_layer_that_allows_fill() {
        let board = |fill| {
            parse(&format!(
                "(kicad_pcb (layers (0 F.Cu signal) (31 B.Cu signal))
                    (footprint C (zone (layer F.Cu) (keepout (tracks not_allowed) (copperpour {fill}))
                        (polygon (pts (xy -1 -1) (xy 1 -1) (xy 1 1) (xy -1 1))))))"
            )).unwrap()
        };
        let allowed = board("allowed");
        let layers = LayerTable::from_pcb(&allowed).unwrap();
        assert!(check_rule_area(&allowed, &layers, 1, [0.0, 0.0]).is_ok());
        assert!(check_rule_area(&allowed, &layers, 1, [2.0, 0.0]).is_err());
        assert!(check_rule_area(&allowed, &layers, 2, [0.0, 0.0]).is_err());
        assert!(check_rule_area(&board("not_allowed"), &layers, 1, [0.0, 0.0]).is_err());
    }

    #[test]
    fn closing_another_component_does_not_certify_the_proposed_pour() {
        let native = |groups: &[&[&str]]| -> Partition {
            groups
                .iter()
                .map(|group| {
                    (
                        "GND".into(),
                        group.iter().map(|id| id.to_string()).collect(),
                    )
                })
                .collect()
        };
        let before = native(&[&["pad"], &["via", "main"], &["other"]]);
        let after = native(&[&["pad", "via", "main"], &["other"]]);
        assert!(target_merge(before.clone(), after, "GND", "pad", "via").is_ok());
        for wrong in [
            native(&[&["pad", "other"], &["via", "main"]]),
            native(&[&["pad", "via", "main", "other"]]),
            native(&[&["pad", "via"], &["main", "other"]]),
        ] {
            assert!(target_merge(before.clone(), wrong, "GND", "pad", "via").is_err());
        }
    }
}
