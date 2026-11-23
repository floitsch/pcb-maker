// Copyright (C) 2026 Florian Loitsch.

//! Bounded three-track bend variants around a native open.

use super::*;
use pcb_kicad_file::copper_emit::stored_supplemental_segment;
use pcb_kicad_file::kicad_nets::note_net_names;
use pcb_kicad_route::open_repair::{GeneratedBranchWork, propose_generated_branch};
use std::collections::BTreeSet;

mod pad_via;
pub use pad_via::GeneratedPadViaRetry;

const SEARCH_TIME: Duration = Duration::from_secs(24);
const MAX_SEARCHES: usize = 12;
const MAX_PROPOSALS: usize = 2;
const NEAR_PAD_MM: f64 = 1.25;
const SHIFT_MM: f64 = 0.05;

#[derive(Default, Serialize)]
pub(super) struct AutomaticSearch {
    pub(super) variants_searched: usize,
    pub(super) candidates: usize,
    pub(super) work_wall_seconds: f64,
    pub(super) preset_via_mm: Option<[f64; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) filled_pad_via: Option<pad_via::PadViaMove>,
}

/// Keep the original non-routing objects and source copper exactly. An ID
/// alone is insufficient: moved items must also match the emitted geometry.
pub(crate) fn editable(source: &Expr, placed: &Expr) -> Result<BTreeSet<String>, String> {
    if placed.head() != Some("kicad_pcb") || source.head() != Some("kicad_pcb") {
        return Err(invalid(
            "generated repair needs a routed PCB and its original placed PCB",
        ));
    }
    let tuning = |item: &Expr| {
        item.head() == Some("generated")
            && form_atom(item, "type", 1) == Some("tuning_pattern")
            && !placed.children().contains(item)
    };
    let design = |pcb: &Expr| {
        pcb.children()
            .iter()
            .filter(|item| !route(item) && !tuning(item))
            .map(|item| {
                if item.head() == Some("zone") && item.child("filled_polygon").is_some() {
                    let fields = item
                        .children()
                        .iter()
                        .filter(|field| field.head() != Some("filled_polygon"))
                        .cloned()
                        .collect();
                    encode(&Expr::List(fields))
                } else {
                    encode(item)
                }
            })
            .collect::<Vec<_>>()
    };
    if design(source) != design(placed) {
        return Err(invalid(
            "the placed input differs from the routed board outside its tracks and vias",
        ));
    }
    let before = index(placed)?;
    let after = index(source)?;
    if before
        .iter()
        .any(|(id, placed)| after.get(id) != Some(placed))
    {
        return Err(invalid(
            "the routed board changed copper supplied by the placed input",
        ));
    }
    let names = note_net_names(source);
    let codes = net_codes(source);
    Ok(after
        .into_iter()
        .filter_map(|(id, item)| {
            (!before.contains_key(&id)
                && stored_supplemental_segment(item, &codes, &names).is_some())
            .then_some(id)
        })
        .collect())
}

struct Witness<'a> {
    ids: [&'a str; 2],
    pad: Point,
    pad_id: &'a str,
    other_pad: Option<(Point, &'a str)>,
    distance: f64,
}

fn witness<'a>(
    source: &Expr,
    native: &'a Value,
    target: &str,
    pad_pairs: bool,
) -> Option<Witness<'a>> {
    let codes = net_codes(source);
    let ids = source
        .children()
        .iter()
        .flat_map(|item| std::iter::once(item).chain(item.children()))
        .filter(|item| route(item) || item.head() == Some("pad"))
        .filter_map(|item| Some((uuid(item).ok()?, item)))
        .collect::<BTreeMap<_, _>>();
    native["unconnected_items"]
        .as_array()?
        .iter()
        .filter(|finding| {
            finding["type"] == "unconnected_items"
                && finding["severity"] == "error"
                && finding["excluded"] != true
        })
        .filter_map(|finding| {
            let [a, b] = finding["items"].as_array()?.as_slice() else {
                return None;
            };
            let ids_native = [a["uuid"].as_str()?, b["uuid"].as_str()?];
            let [a_form, b_form] = [
                ids.get(&ids_native[0].to_ascii_lowercase())?,
                ids.get(&ids_native[1].to_ascii_lowercase())?,
            ];
            if [a_form, b_form]
                .iter()
                .any(|form| item_net(form, &codes) != Some(target))
            {
                return None;
            }
            let position =
                |item: &Value| Some([item["pos"]["x"].as_f64()?, item["pos"]["y"].as_f64()?]);
            let [a_pos, b_pos] = [position(a)?, position(b)?];
            let (pad, pad_id, other_pad) = match (a_form.head(), b_form.head()) {
                (Some("pad"), Some("segment" | "arc" | "via")) => (a_pos, ids_native[0], None),
                (Some("segment" | "arc" | "via"), Some("pad")) => (b_pos, ids_native[1], None),
                (Some("pad"), Some("pad")) if pad_pairs => {
                    (a_pos, ids_native[0], Some((b_pos, ids_native[1])))
                }
                _ => return None,
            };
            Some(Witness {
                ids: ids_native,
                pad,
                pad_id,
                other_pad,
                distance: point_segment_distance_squared(a_pos, b_pos, b_pos),
            })
        })
        .min_by(|a, b| a.distance.total_cmp(&b.distance))
}

pub(super) fn pad_via_retries(
    source: &Expr,
    placed: &Expr,
    native: &Value,
    directory: &Path,
    board_id: &str,
    placed_directory: &Path,
    output: &Path,
) -> Vec<GeneratedPadViaRetry> {
    if !source.children().iter().any(|via| {
        via.head() == Some("via")
            && form_atom(via, "filling", 1) == Some("yes")
            && form_atom(via, "capping", 1) == Some("yes")
    }) || editable(source, placed).is_err()
    {
        return Vec::new();
    }
    let codes = net_codes(source);
    let nets: BTreeMap<_, _> = source
        .children()
        .iter()
        .filter(|owner| owner.head() == Some("footprint"))
        .flat_map(Expr::children)
        .filter(|item| item.head() == Some("pad"))
        .filter_map(|pad| Some((uuid(pad).ok()?, item_net(pad, &codes)?)))
        .collect();
    let candidates: BTreeSet<_> = native["unconnected_items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|finding| finding["items"].as_array())
        .flatten()
        .filter_map(|item| nets.get(&item["uuid"].as_str()?.to_ascii_lowercase()))
        .collect();
    candidates
        .into_iter()
        .filter_map(|net| {
            let witness = witness(source, native, net, true)?;
            let path = |dir: &Path| {
                std::path::absolute(dir)
                    .unwrap_or_else(|_| dir.to_path_buf())
                    .display()
                    .to_string()
            };
            let command = vec![
                "pcb-maker".into(),
                "repair-kicad-opens".into(),
                path(directory),
                board_id.into(),
                "--relocate".into(),
                path(output),
                "--generated-from".into(),
                path(placed_directory),
                "--net".into(),
                (*net).into(),
            ];
            pad_via::suggestion(source, placed, net, &witness, command)
        })
        .take(4)
        .collect()
}

#[derive(Clone, Copy)]
struct Track<'a> {
    slot: usize,
    net: &'a str,
    layer: &'a str,
    width: f64,
    ends: [Point; 2],
    editable: bool,
}

fn chains<'a>(
    source: &'a Expr,
    editable: &BTreeSet<String>,
    pad: Point,
    target: &str,
) -> Vec<[Track<'a>; 3]> {
    let codes = net_codes(source);
    let tracks = source
        .children()
        .iter()
        .enumerate()
        .filter_map(|(slot, item)| {
            if item.head() != Some("segment") {
                return None;
            }
            let id = uuid(item).ok()?;
            if item_net(item, &codes)? == target {
                return None;
            }
            Some(Track {
                slot,
                net: form_atom(item, "net", 1)?,
                layer: form_atom(item, "layer", 1)?,
                width: form_f64(item, "width", 1).ok()?,
                ends: [form_xy(item, "start").ok()?, form_xy(item, "end").ok()?],
                editable: editable.contains(&id),
            })
        })
        .collect::<Vec<_>>();
    let distance =
        |track: Track<'_>| point_segment_distance_squared(pad, track.ends[0], track.ends[1]);
    let mut options = Vec::new();
    for &middle in &tracks {
        if !middle.editable || distance(middle) > NEAR_PAD_MM.powi(2) {
            continue;
        }
        let neighbor = |point| {
            let mut neighbors = tracks.iter().copied().filter(|track| {
                track.slot != middle.slot
                    && track.net == middle.net
                    && track.layer == middle.layer
                    && track.ends.contains(&point)
            });
            let first = neighbors.next()?;
            (first.editable && first.width == middle.width && neighbors.next().is_none())
                .then_some(first)
        };
        if let [Some(a), Some(b)] = middle.ends.map(neighbor)
            && a.slot != b.slot
        {
            options.push([a, middle, b]);
        }
    }
    options.sort_by(|a, b| {
        distance(a[1])
            .total_cmp(&distance(b[1]))
            .then(a[1].slot.cmp(&b[1].slot))
    });
    options
}

fn moved(source: &Expr, [a, middle, b]: [Track<'_>; 3], shift: Point) -> Option<Expr> {
    let mut candidate = source.clone();
    let Expr::List(items) = &mut candidate else {
        return None;
    };
    for track in [a, middle, b] {
        let Expr::List(fields) = &mut items[track.slot] else {
            return None;
        };
        for (end, name) in track.ends.into_iter().zip(["start", "end"]) {
            if middle.ends.contains(&end) {
                let field = fields.iter_mut().find(|field| field.head() == Some(name))?;
                *field = parse(&format!(
                    "({name} {:.6} {:.6})",
                    end[0] + shift[0],
                    end[1] + shift[1]
                ))
                .ok()?;
            }
        }
    }
    Some(candidate)
}

pub(super) fn propose(
    source: &Expr,
    placed: &Expr,
    path: &Path,
    native: &Value,
    target: &str,
    mode: SearchMode<'_>,
) -> Result<(Vec<Relocation>, AutomaticSearch), String> {
    let started = Instant::now();
    let deadline = (started + SEARCH_TIME).min(mode.deadline);
    let mut report = AutomaticSearch::default();
    let mut candidates = Vec::new();
    let editable = editable(source, placed)?;
    if let Some(witness) = witness(source, native, target, true)
        && let Some(move_) = pad_via::propose(
            source,
            placed,
            path,
            native,
            target,
            &witness,
            deadline,
            &mut report,
            mode.work,
        )?
    {
        candidates.push(move_);
        report.candidates = candidates.len();
        report.work_wall_seconds = started.elapsed().as_secs_f64();
        return Ok((candidates, report));
    }
    if mode.pad_via_only {
        report.work_wall_seconds = started.elapsed().as_secs_f64();
        return Ok((candidates, report));
    }
    let Some(witness) = witness(source, native, target, false) else {
        return Ok((candidates, report));
    };
    let options: Vec<_> = chains(source, &editable, witness.pad, target)
        .into_iter()
        .map(|chain| {
            let middle = chain[1];
            let mut shifts = [
                [SHIFT_MM, 0.0],
                [-SHIFT_MM, 0.0],
                [0.0, SHIFT_MM],
                [0.0, -SHIFT_MM],
            ];
            let clearance = |shift: Point| {
                let [a, b] = middle.ends.map(|[x, y]| [x + shift[0], y + shift[1]]);
                point_segment_distance_squared(witness.pad, a, b)
            };
            shifts.sort_by(|&a, &b| clearance(b).total_cmp(&clearance(a)));
            (chain, shifts)
        })
        .collect();
    // Try one direction on each nearby chain before spending the cap on
    // several almost-identical shifts of the first chain.
    let variants = (0..4).flat_map(|direction| {
        options
            .iter()
            .map(move |(chain, shifts)| (*chain, shifts[direction]))
    });
    let codes = net_codes(source);
    for (chain, shift) in variants.take(MAX_SEARCHES) {
        if Instant::now() >= deadline || candidates.len() >= MAX_PROPOSALS {
            break;
        }
        let Some(mut candidate) = moved(source, chain, shift) else {
            continue;
        };
        report.variants_searched += 1;
        let Some(blocker) = item_net(&source.children()[chain[1].slot], &codes) else {
            continue;
        };
        let Some(branch) = propose_generated_branch(
            &candidate,
            path,
            native,
            target,
            blocker,
            witness.ids,
            deadline,
            true,
            mode.work,
        )?
        else {
            continue;
        };
        let Expr::List(items) = &mut candidate else {
            continue;
        };
        items.extend(branch.copper);
        if let Ok(plan) = plan(source, &candidate, target) {
            candidates.push(plan);
            report.preset_via_mm = Some(branch.via_mm);
        }
    }
    report.candidates = candidates.len();
    report.work_wall_seconds = started.elapsed().as_secs_f64();
    Ok((candidates, report))
}

pub(super) struct SearchMode<'a> {
    pub deadline: Instant,
    pub pad_via_only: bool,
    pub work: &'a mut GeneratedBranchWork,
}

#[cfg(test)]
mod tests {
    use super::*;
    use pcb_kicad_file::copper_emit::{SupplementalSegment, supplemental_segment_expr};

    #[test]
    fn only_unchanged_router_output_absent_from_the_placed_input_can_move() {
        let mut placed = parse(
            r#"(kicad_pcb (segment (start 0 1) (end 1 1) (net "GND")
                (width 0.127) (layer "F.Cu") (uuid "authored")))"#,
        )
        .unwrap();
        note_net_names(&placed);
        let segment = supplemental_segment_expr(&SupplementalSegment {
            connection: "GND".into(),
            start: [2.0, 1.0],
            end: [3.0, 1.0],
            width: 0.127,
            layer: "F.Cu".into(),
        });
        let generated = uuid(&segment).unwrap();
        let mut route = placed.clone();
        let Expr::List(items) = &mut route else {
            unreachable!()
        };
        items.push(segment.clone());
        assert_eq!(
            editable(&route, &placed).unwrap(),
            BTreeSet::from([generated])
        );
        let Expr::List(items) = &mut placed else {
            unreachable!()
        };
        items.push(segment);
        assert!(editable(&route, &placed).unwrap().is_empty());
    }

    #[test]
    fn derived_polygons_do_not_obscure_authored_zone_differences() {
        let board = |outline, fill| {
            parse(&format!(
                "(kicad_pcb (zone (uuid z) (net \"GND\") (layer \"F.Cu\")\n+                    (polygon (pts (xy 0 0) (xy {outline} 0) (xy 1 1))) {fill}))"
            ))
            .unwrap()
        };
        let filled = "(filled_polygon (layer \"F.Cu\") (pts (xy 0 0) (xy 1 0) (xy 1 1)))";
        let placed = board(1, "");
        assert!(editable(&board(1, filled), &placed).is_ok());
        assert!(editable(&board(2, filled), &placed).is_err());
    }

    #[test]
    fn a_nearer_pad_pair_does_not_displace_the_track_bend_witness() {
        let source = parse(
            r#"(kicad_pcb
                (segment (uuid track) (net "GND"))
                (footprint (pad (uuid a) (net "GND")) (pad (uuid b) (net "GND"))))"#,
        )
        .unwrap();
        let item = |id, x| serde_json::json!({"uuid": id, "pos": {"x": x, "y": 1}});
        let open = |a, b| serde_json::json!({"type": "unconnected_items", "severity": "error", "items": [a, b]});
        let native = serde_json::json!({"unconnected_items": [
            open(item("a", 1), item("b", 2)),
            open(item("a", 1), item("track", 3))
        ]});
        assert_eq!(
            witness(&source, &native, "GND", true).unwrap().ids,
            ["a", "b"]
        );
        assert_eq!(
            witness(&source, &native, "GND", false).unwrap().ids,
            ["a", "track"]
        );
    }
}
