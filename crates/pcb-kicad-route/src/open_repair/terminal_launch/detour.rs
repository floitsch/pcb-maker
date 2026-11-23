// Copyright (C) 2026 Florian Loitsch.

//! Make room for a via by moving one generated track clear of a pad row.

use super::*;

#[derive(Clone, Debug, Serialize)]
pub struct BlockingTrackDetour {
    pub net: String,
    pub blocker_net: String,
    pub blocker_uuid: String,
    pub original_mm: [Point; 2],
    pub detour_mm: [Point; 4],
    pub channel_mm: f64,
    pub via_channel_mm: f64,
    pub decision: &'static str,
    pub native_partition_preserved: bool,
}

struct Track {
    item: Expr,
    id: String,
    net: String,
    points: [Point; 2],
    span: [f64; 2],
    offset: f64,
    width: f64,
    clearance: f64,
}

struct Detour {
    record: BlockingTrackDetour,
    pieces: Vec<Expr>,
}

fn field(point: Point, name: &str) -> Expr {
    let [x, y] = point;
    Expr::List(vec![
        Expr::Atom(name.into()),
        Expr::Atom(format!("{x:.6}")),
        Expr::Atom(format!("{y:.6}")),
    ])
}

fn plan(
    pcb: &Expr,
    generated: &Expr,
    before: &Value,
    launch: &TerminalLaunch,
    config: &KiCadBoardRouterConfig,
) -> Result<Option<Detour>, String> {
    let axis = if (launch.end_mm[0] - launch.start_mm[0]).abs() > 1e-6 {
        0
    } else {
        1
    };
    let other = 1 - axis;
    let outward = (launch.end_mm[axis] - launch.start_mm[axis]).signum();
    let pads = pads(pcb);
    let mut ids = before["unconnected_items"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|finding| finding["items"].as_array().into_iter().flatten())
        .filter_map(|item| item["uuid"].as_str());
    let (Some(first), Some(second), None) = (ids.next(), ids.next(), ids.next()) else {
        return Ok(None);
    };
    let pad = |id: &str| pads.iter().find(|pad| pad.id.eq_ignore_ascii_case(id));
    let (Some(first), Some(second)) = (pad(first), pad(second)) else {
        return Ok(None);
    };
    if first.net != launch.net || second.net != launch.net || first.reference != second.reference {
        return Ok(None);
    }
    let other_pad = if first.label == launch.pad {
        second
    } else if second.label == launch.pad {
        first
    } else {
        return Ok(None);
    };
    let lower = board_router::lower(pcb, config, true)?;
    let board = &lower.board;
    let Some((index, terminal)) = contact(board, other_pad) else {
        return Ok(None);
    };
    let surface = if terminal.layers == 1 { "F.Cu" } else { "B.Cu" };
    if surface != launch.layer || (terminal.anchor[axis] - launch.start_mm[axis]).abs() > 1e-6 {
        return Ok(None);
    }
    let away = (terminal.anchor[other] - launch.start_mm[other]).signum();
    if (terminal.anchor[other] - launch.start_mm[other]).abs() < 1e-6 {
        return Ok(None);
    }
    let class = board.classes[board.nets[index].class];
    let by_net: BTreeMap<_, _> = board
        .nets
        .iter()
        .map(|net| (net.name.as_str(), board.classes[net.class].clearance))
        .collect();
    let names = note_net_names(pcb);
    let codes = net_codes(pcb);
    let track = |item: &Expr| -> Option<Track> {
        if item.head() != Some("segment") || form_atom(item, "layer", 1)? != surface {
            return None;
        }
        let net = item_net(item, &codes)?;
        let clearance = by_net
            .get(names.normalize(net))
            .copied()?
            .max(class.clearance);
        if names.normalize(net) == names.normalize(&launch.net) {
            return None;
        }
        let points = [form_xy(item, "start").ok()?, form_xy(item, "end").ok()?];
        if (points[0][other] - points[1][other]).abs() > 1e-6 {
            return None;
        }
        let mut span = points.map(|at| (at[axis] - terminal.anchor[axis]) * outward);
        span.sort_by(f64::total_cmp);
        let offset = (points[0][other] - terminal.anchor[other]) * away;
        let width = form_f64(item, "width", 1).ok()?;
        if offset.abs() >= 1.0 || span[0] > 0.65 || span[1] <= 0.35 || width <= 0.0 {
            return None;
        }
        Some(Track {
            id: item_id(item)?,
            item: item.clone(),
            net: net.to_owned(),
            points,
            span,
            offset,
            width,
            clearance,
        })
    };
    let tracks: Vec<_> = pcb.children().iter().filter_map(track).collect();
    let route_ids: BTreeMap<_, _> = generated
        .children()
        .iter()
        .filter(|item| item.head() == Some("segment"))
        .filter_map(|item| Some((item_id(item)?, item)))
        .collect();
    let ordinary = |track: &Track| {
        const FIELDS: &[&str] = &["start", "end", "width", "layer", "net", "uuid"];
        track.item.children().len() == FIELDS.len() + 1
            && FIELDS.iter().all(|field| track.item.child(field).is_some())
            && route_ids
                .get(&track.id)
                .is_some_and(|item| **item == track.item)
            && before["violations"]
                .as_array()
                .into_iter()
                .flatten()
                .all(|finding| {
                    finding["items"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .all(|item| {
                            item["uuid"]
                                .as_str()
                                .is_none_or(|id| !track.id.eq_ignore_ascii_case(id))
                        })
                })
    };
    let mut best: Option<(f64, &Track, f64, f64, f64)> = None;
    for inside in tracks.iter().filter(|track| track.offset < 0.0) {
        for outside in tracks.iter().filter(|track| track.offset > 0.0) {
            let overlap_start = inside.span[0].max(outside.span[0]).max(0.15);
            let overlap_end = inside.span[1].min(outside.span[1]);
            let channel = outside.offset - inside.offset;
            let needed = class.via_diameter
                + inside.clearance
                + outside.clearance
                + (inside.width + outside.width) / 2.0;
            let shortage = needed - channel;
            let shift = ((shortage + 0.075) * 1000.0).round() / 1000.0;
            if !(0.001..=0.10).contains(&shortage)
                || overlap_end - overlap_start < class.via_diameter
                || outside.span[1] - outside.span[0] > 2.0
                || outside.span[1] - outside.span[0] - 2.0 * shift < class.via_diameter
                || inside.net == outside.net
                || !ordinary(outside)
            {
                continue;
            }
            if best
                .as_ref()
                .is_none_or(|(distance, ..)| outside.offset < *distance)
            {
                best = Some((outside.offset, outside, shift, channel, needed));
            }
        }
    }
    let Some((_, chosen, shift, channel, needed)) = best else {
        return Ok(None);
    };
    let [a, b] = chosen.points;
    let along = (b[axis] - a[axis]).signum();
    let (mut left, mut right) = (a, b);
    left[axis] += along * shift;
    right[axis] -= along * shift;
    left[other] += away * shift;
    right[other] += away * shift;
    let points = [a, left, right, b];
    let mut pieces = Vec::new();
    for (index, edge) in points.windows(2).enumerate() {
        let Expr::List(mut items) = chosen.item.clone() else {
            return Ok(None);
        };
        for item in &mut items {
            match item.head() {
                Some("start") => *item = field(edge[0], "start"),
                Some("end") => *item = field(edge[1], "end"),
                Some("uuid") if index != 0 => {
                    let id = deterministic_uuid_for(&format!(
                        "local track detour {} {index}",
                        chosen.id
                    ));
                    *item = parse(&format!(r#"(uuid "{id}")"#))?;
                }
                _ => {}
            }
        }
        pieces.push(Expr::List(items));
    }
    if pieces[1..]
        .iter()
        .filter_map(item_id)
        .any(|id| item_ids(pcb).contains(&id))
    {
        return Ok(None);
    }
    Ok(Some(Detour {
        record: BlockingTrackDetour {
            net: launch.net.clone(),
            blocker_net: chosen.net.clone(),
            blocker_uuid: chosen.id.clone(),
            original_mm: chosen.points,
            detour_mm: points,
            channel_mm: channel,
            via_channel_mm: needed,
            decision: "planned",
            native_partition_preserved: false,
        },
        pieces,
    }))
}

type Partition = Vec<(String, Vec<String>)>;

fn preserves_partition(mut before: Partition, mut after: Partition, detour: &Detour) -> bool {
    let Some((_, members)) = after
        .iter_mut()
        .find(|(_, members)| members.contains(&detour.record.blocker_uuid))
    else {
        return false;
    };
    for id in detour.pieces[1..].iter().filter_map(item_id) {
        let Some(at) = members.iter().position(|member| *member == id) else {
            return false;
        };
        members.remove(at);
    }
    for partition in [&mut before, &mut after] {
        for (_, members) in partition.iter_mut() {
            members.sort_unstable();
        }
        partition.sort_unstable();
    }
    before == after
}

pub(super) fn try_one(
    trial: &Trial<'_>,
    source: &Path,
    launch: &TerminalLaunch,
    config: &KiCadBoardRouterConfig,
    report: &mut TerminalLaunchRetry,
) -> Result<Option<PathBuf>, String> {
    // The two comparison checks, the two partition scans, a local
    // trial with contact repair, then the final check if it succeeds.
    if report.budget_seconds - report.work_seconds < 8.0 * trial.check_seconds + 1.0
        || config.deadline.is_some_and(|limit| Instant::now() >= limit)
    {
        return Ok(None);
    }
    let filename = format!("{}.kicad_pcb", trial.board_id);
    let (original_text, original) = read_board(&source.join(&filename))?;
    let (_, mut generated) = read_board(&trial.output.join("route").join(&filename))?;
    let (_, seed) = read_board(&trial.output.join("seed").join(&filename))?;
    let seeded_ids: BTreeSet<_> = seed
        .children()
        .iter()
        .filter(|item| route_item(item))
        .filter_map(item_id)
        .collect();
    if let Expr::List(items) = &mut generated {
        items.retain(|item| item_id(item).is_none_or(|id| !seeded_ids.contains(&id)));
    }
    let evidence = native_report::read_kicad_report(&source.join("drc.json"))?;
    if report_capped(&evidence) || open_count(&evidence) != 1 {
        return Ok(None);
    }
    report.work_seconds += board_router::lowering_work(&original);
    let Some(mut detour) = plan(&original, &generated, &evidence, launch, config)? else {
        return Ok(None);
    };
    report.track_detour = Some(detour.record.clone());
    let candidate_directory = trial.output.join("detour");
    copy_directory_tree(source, &candidate_directory)?;
    let board = candidate_directory.join(filename);
    let before = report.check(
        &board,
        &candidate_directory.join("drc-before-detour.json"),
        trial.check_seconds,
    )?;
    if fs::read_to_string(&board).map_err(|error| error.to_string())? != original_text
        || report_capped(&before)
        || finding_counts(&evidence, "unconnected_items")?
            != finding_counts(&before, "unconnected_items")?
    {
        detour.record.decision = "baseline_native_attestation_failed";
        report.track_detour = Some(detour.record);
        return Ok(None);
    }
    let mut modified = original;
    if let Expr::List(items) = &mut modified {
        let Some(index) = items
            .iter()
            .position(|item| item_id(item).as_ref() == Some(&detour.record.blocker_uuid))
        else {
            return Err("detour blocker disappeared from the staged board".into());
        };
        items.splice(index..=index, detour.pieces.iter().cloned());
    }
    let new_text = encode(&modified);
    fs::write(&board, &new_text).map_err(|error| error.to_string())?;
    let after = report.check(
        &board,
        &candidate_directory.join("drc-after-detour.json"),
        trial.check_seconds,
    )?;
    if fs::read_to_string(&board).map_err(|error| error.to_string())? != new_text
        || open_count(&before) != 1
        || !preserved(&before, &after)?
        || !preserved(&after, &before)?
    {
        detour.record.decision = "native_findings_changed";
        report.track_detour = Some(detour.record);
        return Ok(None);
    }
    let version = before["kicad_version"]
        .as_str()
        .ok_or("missing native version")?;
    let limit = Instant::now() + std::time::Duration::from_secs(40);
    let limit = config.deadline.map_or(limit, |wall| limit.min(wall));
    let expired = || Instant::now() >= limit;
    let partition = |text: &str, charged: &mut TerminalLaunchRetry| -> Result<Partition, String> {
        if expired() || charged.work_seconds + trial.check_seconds > charged.budget_seconds {
            return Err("not enough time for native detour connectivity verification".into());
        }
        fs::write(&board, text).map_err(|error| error.to_string())?;
        charged.native_checks += 1;
        charged.work_seconds += trial.check_seconds;
        let result = native_report::connectivity_partition(&board, version, true, expired)?;
        if fs::read_to_string(&board).map_err(|error| error.to_string())? != text {
            return Err("KiCad rewrote the board during detour connectivity verification".into());
        }
        Ok(result)
    };
    let before = partition(&original_text, report)?;
    let after = partition(&new_text, report)?;
    let preserved = preserves_partition(before, after, &detour);
    detour.record.native_partition_preserved = preserved;
    detour.record.decision = if preserved {
        "verified"
    } else {
        "native_partition_changed"
    };
    report.track_detour = Some(detour.record);
    Ok(preserved.then_some(candidate_directory))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn matching_geometry_does_not_make_a_locked_or_authored_track_eligible() {
        let text = r#"(kicad_pcb
            (version 20260206)
            (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
            (gr_rect (start 0 0) (end 12 12) (layer "Edge.Cuts"))
            (footprint "footprint" (at 4 4) (layer "F.Cu")
                (property "Reference" "U1")
                (pad "1" smd rect (at 0 0) (size 0.45 0.2) (layers "F.Cu") (net "SIG") (uuid "pad1"))
                (pad "3" smd rect (at 0 0.7) (size 0.45 0.2) (layers "F.Cu") (net "SIG") (uuid "pad3")))
            (footprint "other" (at 8 8) (layer "F.Cu")
                (property "Reference" "U2")
                (pad "1" smd rect (at 0 0) (size 0.45 0.2) (layers "F.Cu") (net "GUARD"))
                (pad "2" smd rect (at 0 0.7) (size 0.45 0.2) (layers "F.Cu") (net "BIAS")))
            (segment (start 4.125 4.35) (end 4.925 4.35) (width 0.15) (layer "F.Cu") (net "BIAS") (uuid "inner"))
            (segment (start 3.725 5.205) (end 5.125 5.205) (width 0.15) (layer "F.Cu") (net "GUARD") (uuid "outer")))"#;
        let open = json!({"unconnected_items":[{"items":[{"uuid":"pad1"},{"uuid":"pad3"}]}]});
        let launch = TerminalLaunch {
            pad: "U1.1".into(),
            net: "SIG".into(),
            layer: "F.Cu".into(),
            start_mm: [4.0, 4.0],
            end_mm: [4.85, 4.0],
            width_mm: 0.15,
        };
        let config = KiCadBoardRouterConfig {
            default_rules: Some(
                pcb_kicad_file::connection_rules::KiCadConnectionRoutingRules {
                    trace_width_mm: 0.15,
                    clearance_mm: 0.15,
                    via_size_mm: 0.45,
                    via_drill_mm: 0.25,
                },
            ),
            ..Default::default()
        };
        let ordinary = parse(text).unwrap();
        let detour = plan(&ordinary, &ordinary, &open, &launch, &config)
            .unwrap()
            .unwrap();
        assert_eq!(detour.record.blocker_uuid, "outer");
        assert_eq!(form_xy(&detour.pieces[1], "start").unwrap(), [3.845, 5.325]);
        assert_eq!(form_xy(&detour.pieces[1], "end").unwrap(), [5.005, 5.325]);
        let original = vec![("GUARD".to_string(), vec!["outer".into(), "via".into()])];
        let mut intact = original.clone();
        intact[0]
            .1
            .extend(detour.pieces[1..].iter().filter_map(item_id));
        assert!(preserves_partition(
            original.clone(),
            intact.clone(),
            &detour
        ));
        intact[0].1.retain(|id| id != "via");
        intact.push(("GUARD".into(), vec!["via".into()]));
        assert!(!preserves_partition(original, intact, &detour));
        let locked =
            parse(&text.replace("(uuid \"outer\")", "(uuid \"outer\") (locked yes)")).unwrap();
        assert!(
            plan(&locked, &locked, &open, &launch, &config)
                .unwrap()
                .is_none()
        );
        let authored = parse("(kicad_pcb)").unwrap();
        assert!(
            plan(&ordinary, &authored, &open, &launch, &config)
                .unwrap()
                .is_none()
        );
    }
}
