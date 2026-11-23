// Copyright (C) 2026 Florian Loitsch.

//! Recognize when to replay a native width report on unchanged copper.

use pcb_kicad_file::kicad_nets::{footprint_uuid, item_net, net_codes};
use pcb_kicad_file::kicad_sexpr::{Expr, form_atom, form_xy};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Serialize)]
pub struct WidthWitnessChange {
    pub common_copper_uuid: String,
    pub original_other_uuid: String,
    pub alternate_other_uuid: String,
    pub layer: String,
}

fn unique_tracks(board: &Expr) -> Option<BTreeMap<String, (usize, &Expr)>> {
    let mut tracks = BTreeMap::new();
    for (index, item) in board.children().iter().enumerate() {
        if matches!(item.head(), Some("segment" | "via" | "arc")) {
            let uuid = footprint_uuid(item)?.to_ascii_lowercase();
            if uuid.is_empty() || tracks.insert(uuid, (index, item)).is_some() {
                return None;
            }
        }
    }
    Some(tracks)
}

fn counted(report: &Value, field: &str) -> Option<BTreeMap<String, usize>> {
    let mut counts = BTreeMap::new();
    for finding in report[field].as_array()? {
        let mut finding = finding.clone();
        finding["items"]
            .as_array_mut()?
            .sort_by_cached_key(Value::to_string);
        *counts.entry(finding.to_string()).or_default() += 1;
    }
    Some(counts)
}

fn only_unmatched(
    ours: &BTreeMap<String, usize>,
    theirs: &BTreeMap<String, usize>,
) -> Option<Value> {
    let mut unmatched = ours.iter().filter_map(|(finding, count)| {
        let excess = count.saturating_sub(theirs.get(finding).copied().unwrap_or_default());
        (excess != 0).then_some((finding, excess))
    });
    let (finding, 1) = unmatched.next()? else {
        return None;
    };
    if unmatched.next().is_some() {
        return None;
    }
    serde_json::from_str(finding).ok()
}

fn report_position(witness: &Value) -> Option<[f64; 2]> {
    Some([witness["pos"]["x"].as_f64()?, witness["pos"]["y"].as_f64()?])
}

fn same_position(a: [f64; 2], b: [f64; 2]) -> bool {
    a.into_iter()
        .zip(b)
        .all(|(a, b)| (a * 1e6).round() == (b * 1e6).round())
}

struct Segment {
    endpoints: [[f64; 2]; 2],
    width: f64,
}

impl Segment {
    fn read(track: &Expr, witness: &Value, layer: &str) -> Option<Self> {
        if track.head() != Some("segment") || form_atom(track, "layer", 1)? != layer {
            return None;
        }
        let endpoints = [form_xy(track, "start").ok()?, form_xy(track, "end").ok()?];
        let witness = report_position(witness)?;
        let width: f64 = form_atom(track, "width", 1)?.parse().ok()?;
        (width.is_finite()
            && width > 0.0
            && endpoints.into_iter().any(|p| same_position(p, witness)))
        .then_some(Self { endpoints, width })
    }

    fn touches_via(&self, via: &Expr, witness: &Value, layer: &str) -> Option<()> {
        if via.head() != Some("via")
            || ![1, 2]
                .into_iter()
                .any(|field| form_atom(via, "layers", field) == Some(layer))
        {
            return None;
        }
        let center = form_xy(via, "at").ok()?;
        let diameter: f64 = form_atom(via, "size", 1)?.parse().ok()?;
        if !diameter.is_finite()
            || diameter <= 0.0
            || !same_position(center, report_position(witness)?)
        {
            return None;
        }
        let [start, end] = self.endpoints;
        let [dx, dy] = [end[0] - start[0], end[1] - start[1]];
        let length_squared = dx * dx + dy * dy;
        if !length_squared.is_finite() || length_squared <= 0.0 {
            return None;
        }
        let [x, y] = [center[0] - start[0], center[1] - start[1]];
        let t = ((x * dx + y * dy) / length_squared).clamp(0.0, 1.0);
        ((x - t * dx).hypot(y - t * dy) <= (self.width + diameter) / 2.0 + 1e-6).then_some(())
    }
}

fn same_joint(
    common: (&Expr, &Value),
    original: (&Expr, &Value),
    alternate: (&Expr, &Value),
    layer: &str,
) -> Option<()> {
    if common.0.head() == Some("via") {
        for (track, witness) in [original, alternate] {
            Segment::read(track, witness, layer)?.touches_via(common.0, common.1, layer)?;
        }
        return Some(());
    }
    let trunk = Segment::read(common.0, common.1, layer)?;
    let (via, track) = match (original.0.head()?, alternate.0.head()?) {
        ("via", "segment") => (original, alternate),
        ("segment", "via") => (alternate, original),
        _ => return None,
    };
    trunk.touches_via(via.0, via.1, layer)?;
    let track = Segment::read(track.0, track.1, layer)?;
    // Require the second track to meet the common track at a real endpoint.
    // The report may identify the other end of the second track.
    trunk
        .endpoints
        .into_iter()
        .any(|a| track.endpoints.into_iter().any(|b| same_position(a, b)))
        .then_some(())
}

/// Admit only one replaced active width warning, with the full description
/// and identical copper anchor at the same unchanged via joint. This is only
/// an eligibility test. A fresh native replay must reproduce the entire
/// alternate finding, including the changed item UUID and reported position.
pub fn local_width_change(
    source: &Expr,
    candidate: &Expr,
    before: &Value,
    after: &Value,
    repaired_net: Option<&str>,
) -> Option<WidthWitnessChange> {
    if before["coordinate_units"] != "mm" || after["coordinate_units"] != "mm" {
        return None;
    }
    let original = counted(before, "violations")?;
    let alternate = counted(after, "violations")?;
    let mut prior = only_unmatched(&original, &alternate)?;
    let mut next = only_unmatched(&alternate, &original)?;
    if prior["type"] != "connection_width"
        || prior["severity"] != "warning"
        || prior["excluded"] == true
    {
        return None;
    }
    let prior_items = prior.as_object_mut()?.remove("items")?;
    let next_items = next.as_object_mut()?.remove("items")?;
    let [first, second] = prior_items.as_array()?.as_slice() else {
        return None;
    };
    let [new_first, new_second] = next_items.as_array()?.as_slice() else {
        return None;
    };
    if prior != next {
        return None;
    }
    let (common, old, new) = match (
        [new_first, new_second].contains(&first),
        [new_first, new_second].contains(&second),
    ) {
        (true, false) => (
            first,
            second,
            [new_first, new_second].into_iter().find(|p| *p != first)?,
        ),
        (false, true) => (
            second,
            first,
            [new_first, new_second].into_iter().find(|p| *p != second)?,
        ),
        _ => return None,
    };
    let layer = prior["description"]
        .as_str()?
        .strip_suffix(')')?
        .rsplit_once(" (")?
        .1;
    let [common, old, new] = [common, old, new].map(|witness| {
        (
            witness["uuid"].as_str().map(str::to_ascii_lowercase),
            witness,
        )
    });
    let (common_id, original_id, alternate_id) = (common.0?, old.0?, new.0?);
    if original_id == alternate_id || [&original_id, &alternate_id].contains(&&common_id) {
        return None;
    }
    let source_tracks = unique_tracks(source)?;
    let candidate_tracks = unique_tracks(candidate)?;
    let source_codes = net_codes(source);
    let candidate_codes = net_codes(candidate);
    let anchor = source_tracks.get(&common_id)?.1;
    let net = item_net(anchor, &source_codes)?;
    if net.is_empty()
        || repaired_net
            .is_some_and(|repaired| repaired == pcb_kicad_file::kicad_sexpr::unescape(net))
    {
        return None;
    }
    let copper_on_net = |tracks: &BTreeMap<String, (usize, &Expr)>, codes| {
        tracks
            .iter()
            .filter(|(_, (_, track))| item_net(track, codes) == Some(net))
            .map(|(id, (_, track))| (id.clone(), (*track).clone()))
            .collect::<BTreeMap<_, _>>()
    };
    if copper_on_net(&source_tracks, &source_codes)
        != copper_on_net(&candidate_tracks, &candidate_codes)
    {
        return None;
    }
    let original = source_tracks.get(&original_id)?.1;
    let alternate = source_tracks.get(&alternate_id)?.1;
    if [original, alternate]
        .into_iter()
        .any(|track| item_net(track, &source_codes) != Some(net))
    {
        return None;
    }
    same_joint(
        (anchor, common.1),
        (original, old.1),
        (alternate, new.1),
        layer,
    )?;
    Some(WidthWitnessChange {
        common_copper_uuid: common_id,
        original_other_uuid: original_id,
        alternate_other_uuid: alternate_id,
        layer: layer.into(),
    })
}

impl WidthWitnessChange {
    /// The replay may alter only the enumeration order of these two original
    /// forms. Every UUID, copper shape, and nonrouting form stays intact.
    pub fn swap_tracks(&self, mut source: Expr) -> Option<Expr> {
        let tracks = unique_tracks(&source)?;
        let a = tracks.get(&self.original_other_uuid)?.0;
        let b = tracks.get(&self.alternate_other_uuid)?.0;
        let Expr::List(items) = &mut source else {
            return None;
        };
        items.swap(a, b);
        Some(source)
    }

    /// Replay may also rotate the order of complete copper forms. All
    /// nonrouting forms keep their original slots.
    pub fn rotate_tracks(mut source: Expr) -> Option<Expr> {
        let Expr::List(items) = &mut source else {
            return None;
        };
        let (slots, mut copper): (Vec<_>, Vec<_>) = items
            .iter()
            .enumerate()
            .filter(|(_, item)| matches!(item.head(), Some("segment" | "arc" | "via")))
            .map(|(slot, item)| (slot, item.clone()))
            .unzip();
        let shift = slots.len() / 2;
        if shift == 0 {
            return None;
        }
        copper.rotate_left(shift);
        for (slot, form) in slots.into_iter().zip(copper) {
            items[slot] = form;
        }
        Some(source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pcb_kicad_file::kicad_sexpr::parse;
    use serde_json::json;

    const VIA: &str = r#"(via (at 10 20) (size 0.6) (drill 0.3)
                          (layers "F.Cu" "B.Cu") (net "untouched") (uuid "anchor"))"#;
    const FIRST: &str = r#"(segment (start 10 20) (end 9.7 20.3) (width 0.2)
                            (layer "F.Cu") (net "untouched") (uuid "first"))"#;
    const SECOND: &str = r#"(segment (start 9.8 20.4) (end 11.8 20.4) (width 0.2)
                             (layer "F.Cu") (net "untouched") (uuid "second"))"#;

    fn board(via: &str, first: &str, second: &str, extra: &str) -> Expr {
        parse(&format!(
            "(kicad_pcb {via} {first} (general (thickness 1.6)) {second} {extra})"
        ))
        .unwrap()
    }

    fn report(track: &str, pos: [f64; 2]) -> Value {
        json!({
            "coordinate_units":"mm",
            "violations":[{
                "type":"connection_width", "severity":"warning",
                "description":"Copper connection too narrow (board setup constraints minimum connection width 0.1500 mm; actual 0.0790 mm) (F.Cu)",
                "items":[
                   {"uuid":"anchor","pos":{"x":10.0,"y":20.0}},
                   {"uuid":track,"pos":{"x":pos[0],"y":pos[1]}}
                ]
            }]
        })
    }

    #[test]
    fn alternate_contacts_of_an_unchanged_via_request_an_original_form_swap() {
        let source = board(VIA, FIRST, SECOND, "");
        let trial = board(
            VIA,
            FIRST,
            SECOND,
            &FIRST
                .replace("untouched", "target")
                .replace("first", "added"),
        );
        let prior = report("first", [10.0, 20.0]);
        let alternate = report("second", [9.8, 20.4]);
        let change =
            local_width_change(&source, &trial, &prior, &alternate, Some("target")).unwrap();
        assert_eq!(change.common_copper_uuid, "anchor");
        assert_eq!(
            change.swap_tracks(source),
            Some(board(VIA, SECOND, FIRST, ""))
        );
    }

    #[test]
    fn a_connected_segment_can_replace_a_via_next_to_the_same_track() {
        let branch = r#"(segment (start 9.6 20.6) (end 9.8 20.4) (width 0.2)
                                (layer "F.Cu") (net "untouched") (uuid "branch"))"#;
        let source = board(VIA, FIRST, SECOND, branch);
        let prior = report("second", [9.8, 20.4]);
        let mut alternate = prior.clone();
        alternate["violations"][0]["items"][0] =
            json!({"uuid":"branch", "pos":{"x":9.6, "y":20.6}});
        let change = local_width_change(&source, &source, &prior, &alternate, None).unwrap();
        assert_eq!(change.common_copper_uuid, "second");
        assert_eq!(change.alternate_other_uuid, "branch");
        let rotated = WidthWitnessChange::rotate_tracks(source.clone()).unwrap();
        assert_eq!(source.children()[3], rotated.children()[3]);
        let original = unique_tracks(&source)
            .unwrap()
            .into_iter()
            .map(|(id, (_, item))| (id, item))
            .collect::<BTreeMap<_, _>>();
        let rotated = unique_tracks(&rotated)
            .unwrap()
            .into_iter()
            .map(|(id, (_, item))| (id, item))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(original, rotated);

        let detached = board(
            VIA,
            FIRST,
            SECOND,
            &branch.replace("(end 9.8 20.4)", "(end 9.8 21)"),
        );
        assert!(local_width_change(&detached, &detached, &prior, &alternate, None).is_none());
    }

    #[test]
    fn changed_copper_rule_position_severity_and_multiplicity_do_not_request_replay() {
        let source = board(VIA, FIRST, SECOND, "");
        let prior = report("first", [10.0, 20.0]);
        let alternate = report("second", [9.8, 20.4]);
        for trial in [
            board(VIA, FIRST, &SECOND.replace("0.2", "0.19"), ""),
            board(VIA, FIRST, &SECOND.replace("20.4", "20.401"), ""),
            board(VIA, FIRST, &SECOND.replace("F.Cu", "B.Cu"), ""),
            board(&VIA.replace("0.6", "0.7"), FIRST, SECOND, ""),
            board(VIA, FIRST, SECOND, &FIRST.replace("first", "added")),
            board(VIA, FIRST, SECOND, FIRST),
        ] {
            assert!(local_width_change(&source, &trial, &prior, &alternate, None).is_none());
        }
        let newly_added = board(VIA, FIRST, "", "");
        assert!(local_width_change(&newly_added, &source, &prior, &alternate, None).is_none());
        let separated = board(VIA, FIRST, &SECOND.replace("20.4", "21.0"), "");
        let elsewhere = report("second", [9.8, 21.0]);
        assert!(local_width_change(&separated, &separated, &prior, &elsewhere, None).is_none());
        assert!(
            local_width_change(&source, &source, &prior, &alternate, Some("untouched")).is_none()
        );
        for (key, value) in [
            (
                "description",
                json!(
                    "Copper connection too narrow (board setup constraints minimum connection width 0.1500 mm; actual 0.0789 mm) (F.Cu)"
                ),
            ),
            ("severity", json!("error")),
            ("excluded", json!(true)),
            (
                "items",
                json!([
                    {"uuid":"anchor","pos":{"x":10.0,"y":20.001}},
                    {"uuid":"second","pos":{"x":9.8,"y":20.4}}
                ]),
            ),
        ] {
            let mut different = alternate.clone();
            different["violations"][0][key] = value;
            assert!(local_width_change(&source, &source, &prior, &different, None).is_none());
        }
        let mut additional = alternate.clone();
        additional["violations"]
            .as_array_mut()
            .unwrap()
            .push(alternate["violations"][0].clone());
        assert!(local_width_change(&source, &source, &prior, &additional, None).is_none());
    }
}
