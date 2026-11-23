// Copyright (C) 2026 Florian Loitsch.

//! A new track may replace a native cross-gap witness at an existing contact.

use super::*;
use crate::board_router::LayerTable;
use pcb_router::{Point, geometry::point_segment_distance};

#[derive(Serialize)]
pub struct PanelWitnessReplacement {
    pub net: String,
    pub piece: usize,
    pub at: Point,
    pub previous_uuid: String,
    pub track_uuid: String,
}

pub(super) struct PanelTrial<'a> {
    pub pcb: &'a Expr,
    pub pieces: &'a Pieces,
    pub piece: usize,
    pub previous: &'a [Expr],
    pub added: &'a [Expr],
}

impl PanelTrial<'_> {
    pub fn prove(&self, before: &Value, after: &Value) -> Option<Vec<PanelWitnessReplacement>> {
        if before["coordinate_units"] != "mm" || after["coordinate_units"] != "mm" {
            return None;
        }
        let a = before[ACROSS_KEY].as_array()?;
        let b = after[ACROSS_KEY].as_array()?;
        if a.len() != b.len() {
            return None;
        }
        let new_piece = self.pieces.added_routing_piece(self.added)?;
        if new_piece != self.piece {
            return None;
        }
        let layers = LayerTable::from_pcb(self.pcb).ok()?;
        let codes = net_codes(self.pcb);
        let original = self
            .pcb
            .children()
            .iter()
            .chain(self.previous)
            .flat_map(|item| {
                let footprint = matches!(item.head(), Some("footprint" | "module"));
                std::iter::once((None, item)).chain(
                    item.children()
                        .iter()
                        .filter(move |child| {
                            footprint && matches!(child.head(), Some("pad" | "zone"))
                        })
                        .map(move |child| (Some(item), child)),
                )
            });
        let mut original_ids = BTreeMap::new();
        for (footprint, object) in original {
            if let Some(id) = item_id(object)
                && original_ids.insert(id, (footprint, object)).is_some()
            {
                return None;
            }
        }
        let added: BTreeMap<_, _> = self
            .added
            .iter()
            .filter_map(|item| Some((item_id(item)?, item)))
            .collect();
        if added.len() != self.added.len() || added.keys().any(|id| original_ids.contains_key(id)) {
            return None;
        }
        let mut scope = NativePanelScope::new(self.pcb, self.pieces);
        let mut replace = |old: &Value, new: &Value, peer: &Value| {
            let previous_uuid = witness_id(old)?;
            let track_uuid = witness_id(new)?;
            let peer_uuid = witness_id(peer)?;
            let at = witness_position(old)?;
            if witness_position(new).map(joint)? != joint(at)
                || scope.item_piece(old)? != self.piece
                || scope.item_piece(new)? != self.piece
                || scope.item_piece(peer)? == self.piece
            {
                return None;
            }
            let &(footprint, old) = original_ids.get(&previous_uuid)?;
            let &(_, peer) = original_ids.get(&peer_uuid)?;
            let new = *added.get(&track_uuid)?;
            let net = item_net(old, &codes)?;
            if net.is_empty()
                || item_net(new, &codes) != Some(net)
                || item_net(peer, &codes) != Some(net)
            {
                return None;
            }
            let bit = joined_segment_layer(new, at, &layers)?;
            if !same_copper_contact(old, footprint, at, bit, &layers) {
                return None;
            }
            Some(PanelWitnessReplacement {
                net: unescape(net).into_owned(),
                piece: self.piece + 1,
                at,
                previous_uuid,
                track_uuid,
            })
        };
        let mut remaining: Vec<_> = b.iter().collect();
        let mut replacements = Vec::new();
        for finding in a {
            let ([left, right], header) = witness_pair(finding)?;
            let mut matched = None;
            for (index, candidate) in remaining.iter().enumerate() {
                let ([a, b], other_header) = witness_pair(candidate)?;
                if header != other_header {
                    continue;
                }
                for [new_left, new_right] in [[a, b], [b, a]] {
                    if left == new_left && right == new_right {
                        matched = Some((index, None));
                    } else if left == new_left {
                        if let Some(proof) = replace(right, new_right, left) {
                            matched = Some((index, Some(proof)));
                        }
                    } else if right == new_right
                        && let Some(proof) = replace(left, new_left, right)
                    {
                        matched = Some((index, Some(proof)));
                    }
                    if matched.is_some() {
                        break;
                    }
                }
                if matched.is_some() {
                    break;
                }
            }
            let (index, replacement) = matched?;
            remaining.swap_remove(index);
            replacements.extend(replacement);
        }
        (!replacements.is_empty()).then_some(replacements)
    }
}

fn witness_pair(finding: &Value) -> Option<([&Value; 2], serde_json::Map<String, Value>)> {
    if finding["type"] != "unconnected_items"
        || finding["severity"] != "error"
        || finding["excluded"] == true
    {
        return None;
    }
    let [left, right] = finding["items"].as_array()?.as_slice() else {
        return None;
    };
    let mut header = finding.as_object()?.clone();
    header.remove("items");
    Some(([left, right], header))
}

fn witness_id(item: &Value) -> Option<String> {
    Some(item["uuid"].as_str()?.to_ascii_lowercase())
}

fn witness_position(item: &Value) -> Option<Point> {
    let at = [item["pos"]["x"].as_f64()?, item["pos"]["y"].as_f64()?];
    at.iter().all(|v| v.is_finite()).then_some(at)
}

fn joint(at: Point) -> [i64; 2] {
    at.map(|value| (value * 1e6).round() as i64)
}

fn joined_segment_layer(new: &Expr, at: Point, layers: &LayerTable) -> Option<u32> {
    if new.head() != Some("segment") {
        return None;
    }
    let [start, end] = [form_xy(new, "start").ok()?, form_xy(new, "end").ok()?];
    let mask = layers.mask_of_item(new).ok()?;
    let width = form_f64(new, "width", 1).ok()?;
    (width.is_finite()
        && width > 0.0
        && start != end
        && [start, end].map(joint).contains(&joint(at))
        && mask.count_ones() == 1)
        .then_some(mask)
}

fn same_copper_contact(
    old: &Expr,
    footprint: Option<&Expr>,
    at: Point,
    bit: u32,
    layers: &LayerTable,
) -> bool {
    match old.head() {
        Some("segment") => {
            let Ok(start) = form_xy(old, "start") else {
                return false;
            };
            let Ok(end) = form_xy(old, "end") else {
                return false;
            };
            layers.mask_of_item(old) == Ok(bit)
                && form_f64(old, "width", 1).is_ok_and(|width| width.is_finite() && width > 0.0)
                && point_segment_distance(at, start, end) <= 0.000_000_5
        }
        Some("pad")
            if old.children().get(2).and_then(Expr::atom) == Some("smd")
                && matches!(
                    old.children().get(3).and_then(Expr::atom),
                    Some("rect" | "circle" | "oval" | "roundrect")
                )
                && old.child("padstack").is_none()
                && old.child("drill").is_none()
                && layers.pad_mask(old) == Ok(bit) =>
        {
            footprint
                .and_then(|fp| lower_pad(old, form_at(fp).ok()?).ok())
                .is_some_and(|pad| {
                    joint(at) == joint(pad.center)
                        && pad.geometry.router_shape().contains_disc(at, 0.000_002)
                })
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_same_cross_gap_can_end_on_a_pad_or_a_joined_track() {
        let pcb = parse(
            r#"(kicad_pcb
          (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
          (footprint "local" (layer "F.Cu") (at 8 5)
            (pad "1" smd circle (at 0 0) (size 1 1) (layers "F.Cu") (net "/A") (uuid pad)))
          (footprint "peer" (layer "F.Cu") (at 22 5)
            (pad "1" smd circle (at 0 0) (size 1 1) (layers "F.Cu") (net "/A") (uuid peer)))
          (segment (start 8 5) (end 8 7) (width 0.2) (layer "F.Cu") (net "/A") (uuid kept)))"#,
        )
        .unwrap();
        let pieces = Pieces::of_boxes(vec![[0.0, 0.0, 12.0, 10.0], [15.0, 0.0, 27.0, 10.0]]);
        let piece = pieces.piece_at([8.0, 5.0]).unwrap();
        let item = |uuid, description, x| json!({"uuid": uuid, "description": description, "pos": {"x": x, "y": 5.0}});
        let report = |items| {
            json!({"coordinate_units": "mm", ACROSS_KEY: [{
                "type": "unconnected_items", "severity": "error", "items": items
            }]})
        };
        let peer = item("peer", "Pad", 22.0);
        let after = report([peer.clone(), item("new", "Track", 8.0)]);
        let new_segment = |layer, net, x| {
            parse(&format!(
            r#"(segment (start {x} 5) (end 6 5) (width 0.2) (layer "{layer}") (net "{net}") (uuid new))"#
        )).unwrap()
        };
        for (source, description) in [("pad", "Pad"), ("kept", "Track")] {
            let before = report([item(source, description, 8.0), peer.clone()]);
            let check = |added: Expr, candidate: &Value| {
                PanelTrial {
                    pcb: &pcb,
                    pieces: &pieces,
                    piece,
                    previous: &[],
                    added: &[added],
                }
                .prove(&before, candidate)
            };
            let proof = check(new_segment("F.Cu", "/A", 8.0), &after).unwrap();
            assert_eq!(proof.len(), 1);
            assert_eq!(proof[0].previous_uuid, source);
            assert_eq!(proof[0].track_uuid, "new");
            assert_eq!(proof[0].piece, piece + 1);
            assert!(check(new_segment("B.Cu", "/A", 8.0), &after).is_none());
            assert!(check(new_segment("F.Cu", "/other", 8.0), &after).is_none());
            assert!(check(new_segment("F.Cu", "/A", 7.99), &after).is_none());
            let changed_peer = report([item("another", "Pad", 22.0), item("new", "Track", 8.0)]);
            assert!(check(new_segment("F.Cu", "/A", 8.0), &changed_peer).is_none());
        }
    }
}
