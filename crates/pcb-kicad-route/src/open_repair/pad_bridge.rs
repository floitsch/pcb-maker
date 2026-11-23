// Copyright (C) 2026 Florian Loitsch.

//! A short, native-verified join between two pads of one footprint.

use super::{plane_escape::geometry, *};
use pcb_kicad_file::{
    copper_emit::{SupplementalSegment, supplemental_segment_expr},
    copper_geometry::lower_pad,
    kicad_nets::footprint_reference,
};
use pcb_router::{
    Board, ObstacleKind, Point, Shape,
    geometry::{distance, point_in_polygon, polygon_edges},
    rules::{ClassRules, NetRules, TRACK, obstacle_kind},
    verify::shape_distance,
};

const MAX_PAD_DISTANCE_MM: f64 = 1.5;
const NM: f64 = 1e-6;

#[derive(Serialize)]
pub struct SameFootprintPadBridge {
    pub footprint: String,
    pub pads: [String; 2],
    pub pad_uuids: [String; 2],
    pub native_counterpart_uuid: String,
    pub layer: String,
    pub width_mm: f64,
    pub start_mm: Point,
    pub end_mm: Point,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stored_reports: Option<[String; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stored_connectivity_proof: Option<OpenConnectivityProof>,
}

pub(super) fn checks_needed(refill: bool) -> usize {
    // Trial DRC and two exact component scans. With zones, reserve another
    // report and component scan before and after for the stored copper.
    3 + 4 * usize::from(refill)
}

pub(super) fn has_fills(pcb: &Expr) -> bool {
    pcb.children().iter().any(|item| {
        item.head() == Some("zone")
            || (item.head() == Some("footprint") && item.child("zone").is_some())
    })
}

fn eligible_report(native: &Value) -> bool {
    native["coordinate_units"] == "mm"
        && !report_capped(native)
        && finding_types_at_cap(native, "schematic_parity").is_empty()
        && native["ignored_checks"].as_array().is_some_and(|checks| {
            !checks.iter().any(|check| {
                matches!(
                    check["key"].as_str(),
                    Some(
                        "clearance"
                            | "shorting_items"
                            | "track_width"
                            | "connection_width"
                            | "unconnected_items"
                    )
                )
            })
        })
}

pub(super) struct Inputs<'a> {
    pub pcb: &'a Expr,
    pub native: &'a Value,
    pub board: &'a Board,
    pub rules: &'a KiCadBoardRouterConfig,
    pub layers: &'a board_router::LayerTable,
    pub native_net: &'a str,
    pub net: &'a str,
    pub witness: &'a [Value],
}

pub(super) fn propose(input: Inputs<'_>) -> Option<(Expr, SameFootprintPadBridge)> {
    let Inputs {
        pcb,
        native,
        board,
        rules,
        layers,
        native_net,
        net,
        witness,
    } = input;
    let [a, b] = witness else { return None };
    if !eligible_report(native)
        || rules.diff_pair_rules.contains_key(net)
        || rules.length_rules.iter().any(|rule| {
            rule.nets
                .iter()
                .any(|member| project_rules::wildcard_match(&member.net, net))
        })
    {
        return None;
    }
    let pads = geometry::pads(pcb);
    let end_points = [(a, b), (b, a)];
    let selection = end_points.into_iter().find_map(|(open, mate)| {
        let id = open["uuid"].as_str()?.to_ascii_lowercase();
        let mut matching = pads
            .iter()
            .filter(|pad| pad.id == id && pad.net == native_net);
        let pad = matching.next()?;
        if matching.next().is_some() {
            return None;
        }
        let (index, terminal) = geometry::contact_on_net(board, &pad.label, net)?;
        let witness_at = [open["pos"]["x"].as_f64()?, open["pos"]["y"].as_f64()?];
        (distance(witness_at, terminal.anchor) <= 1e-5).then_some((
            pad,
            index,
            terminal,
            mate["uuid"].as_str()?.to_ascii_lowercase(),
        ))
    })?;
    let (source, net_id, start, counterpart) = selection;
    let owner = pcb
        .children()
        .iter()
        .filter(|part| part.head() == Some("footprint"))
        .filter(|part| footprint_reference(part).as_deref() == Some(source.reference.as_str()))
        .collect::<Vec<_>>();
    let [owner] = owner.as_slice() else {
        return None;
    };
    if owner.child("net_tie_pad_groups").is_some()
        || start.contact.is_some()
        || rules
            .track_disallowed
            .get(net)
            .is_some_and(|mask| mask & start.layers != 0)
    {
        return None;
    }
    let pose = form_at(owner).ok()?;
    let layer = start.layers.trailing_zeros() as usize;
    let mut neighbors: Vec<_> = pads
        .iter()
        .filter(|pad| {
            pad.id != source.id && pad.reference == source.reference && pad.net == native_net
        })
        .filter_map(|pad| {
            // Kept-copper lowering may collapse a connected pad into
            // another terminal. Its original land is still a valid target.
            let mut forms = owner.children().iter().filter(|item| {
                item.head() == Some("pad") && item_id(item).as_ref() == Some(&pad.id)
            });
            let form = forms.next()?;
            if forms.next().is_some()
                || layers.mask_of_item(form).ok()? != start.layers
                || !matches!(
                    form.children().get(3).and_then(Expr::atom),
                    Some("rect" | "roundrect" | "circle" | "oval")
                )
            {
                return None;
            }
            let land = lower_pad(form, pose).ok()?;
            let shape = land.geometry.router_shape();
            let span = distance(start.anchor, land.center);
            ((NM..=MAX_PAD_DISTANCE_MM).contains(&span)
                && shape_distance(&board.obstacles[start.pad].shape, &shape) > NM)
                .then_some((span, pad, land.center, shape))
        })
        .collect();
    neighbors.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.id.cmp(&b.1.id)));
    if neighbors.is_empty() {
        return None;
    }
    let fallback = NetRules::default();
    let (board, classes) =
        ClassRules::split(board, rules.net_rules.as_deref().unwrap_or(&fallback));
    let class = board.nets[net_id].class;
    let floor = board
        .neck_width
        .max(rules.width_floor())
        .max(classes.floor_on(class, layer));
    let width = (floor / NM).ceil() * NM;
    if !width.is_finite()
        || width <= 0.0
        || width > classes.width_on(&board, class, layer) + NM
        || !board.obstacles[start.pad]
            .shape
            .contains_disc(start.anchor, width / 2.0 + NM)
    {
        return None;
    }
    for (_, pad, center, shape) in neighbors.into_iter().take(4) {
        let [x, y] = start.anchor;
        let [tx, ty] = center;
        // The straight projection stays between the target's pad row
        // neighbors. Try to land well inside the recipient land.
        let ends = [[tx, y], [x, ty], [tx, ty]];
        for end in ends.map(|p| p.map(|v| (v / NM).round() * NM)) {
            if !(NM..=MAX_PAD_DISTANCE_MM).contains(&distance(start.anchor, end))
                || !shape.contains_disc(end, width / 2.0 + NM)
                || !clear(&board, &classes, net_id, layer, [start.anchor, end], width)
            {
                continue;
            }
            let layer = layers.names[layer].clone();
            let mut trace = supplemental_segment_expr(&SupplementalSegment {
                connection: unescape(native_net).into_owned(),
                start: start.anchor,
                end,
                width,
                layer: layer.clone(),
            });
            let Expr::List(fields) = &mut trace else {
                return None;
            };
            let net_field = fields
                .iter_mut()
                .find(|field| field.head() == Some("net"))?;
            *net_field = source_net_field(&net_codes(pcb), native_net);
            return Some((
                trace,
                SameFootprintPadBridge {
                    footprint: source.reference.clone(),
                    pads: [source.label.clone(), pad.label.clone()],
                    pad_uuids: [source.id.clone(), pad.id.clone()],
                    native_counterpart_uuid: counterpart,
                    layer,
                    width_mm: width,
                    start_mm: start.anchor,
                    end_mm: end,
                    stored_reports: None,
                    stored_connectivity_proof: None,
                },
            ));
        }
    }
    None
}

fn clear(
    board: &Board,
    rules: &ClassRules,
    net: usize,
    layer: usize,
    [start, end]: [Point; 2],
    width: f64,
) -> bool {
    let trace = Shape::Capsule {
        start,
        end,
        radius: width / 2.0,
    };
    let class = board.nets[net].class;
    if !point_in_polygon(start, &board.outline)
        || !point_in_polygon(end, &board.outline)
        || polygon_edges(&board.outline).any(|(start, end)| {
            shape_distance(
                &trace,
                &Shape::Capsule {
                    start,
                    end,
                    radius: 0.0,
                },
            ) + NM
                < board.edge_clearance
        })
    {
        return false;
    }
    board.obstacles.iter().all(|obstacle| {
        if obstacle.net == Some(net as pcb_router::NetId)
            || !obstacle.blocks_tracks
            || obstacle.layers & (1 << layer) == 0
        {
            return true;
        }
        let gap = match obstacle.kind {
            ObstacleKind::Copper => {
                let custom = obstacle_kind(obstacle).map_or(0.0, |kind| {
                    rules.resolved.extra_on(
                        rules.class_group[class],
                        TRACK,
                        rules.resolved.obstacle_group(obstacle),
                        kind,
                        layer,
                    )
                });
                board
                    .copper_clearance(&board.classes[class], obstacle)
                    .max(custom)
            }
            ObstacleKind::Hole => board.hole_clearance,
            ObstacleKind::Keepout => obstacle.clearance,
        };
        let distance = shape_distance(&trace, &obstacle.shape);
        distance > 0.0 && distance + NM >= gap
    })
}

impl SameFootprintPadBridge {
    pub(super) fn can_compare_partitions(
        &self,
        before: &Value,
        after: &Value,
    ) -> Result<bool, String> {
        Ok(eligible_report(before)
            && eligible_report(after)
            && eligible_open_partition(before, after)?)
    }

    pub(super) fn closes(&self, before: &Value, after: &Value) -> Result<bool, String> {
        if !eligible_report(before) || !eligible_report(after) || !same_check_policy(before, after)
        {
            return Ok(false);
        }
        let mut before = before.clone();
        let Some(opens) = before["unconnected_items"].as_array_mut() else {
            return Ok(false);
        };
        let old = opens.len();
        opens.retain(|finding| {
            let has = |id| {
                finding["items"].as_array().is_some_and(|items| {
                    items.len() == 2
                        && items.iter().any(|item| {
                            item["uuid"]
                                .as_str()
                                .is_some_and(|uuid| uuid.eq_ignore_ascii_case(id))
                        })
                })
            };
            !(finding["type"] == "unconnected_items"
                && finding["severity"] == "error"
                && finding["excluded"] != true
                && has(&self.pad_uuids[0])
                && has(&self.native_counterpart_uuid))
        });
        if old.checked_sub(opens.len()) != Some(1) {
            return Ok(false);
        }
        for key in FINDING_KEYS {
            if finding_counts(&before, key)? != finding_counts(after, key)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub(super) fn proves_partitions(
        &self,
        before: &[(String, Vec<String>)],
        after: &[(String, Vec<String>)],
    ) -> bool {
        let [open, sink] = &self.pad_uuids;
        let mate = &self.native_counterpart_uuid;
        let group = |partition: &[(String, Vec<String>)], id: &String| {
            partition.iter().position(|(_, items)| items.contains(id))
        };
        let (Some(a), Some(b)) = (group(before, open), group(before, sink)) else {
            return false;
        };
        a != b
            && group(before, mate) == Some(b)
            && group(after, open).is_some_and(|after_group| {
                group(after, sink) == Some(after_group) && group(after, mate) == Some(after_group)
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn bridge() -> SameFootprintPadBridge {
        SameFootprintPadBridge {
            footprint: "U1".into(),
            pads: ["U1.1".into(), "U1.2".into()],
            pad_uuids: ["open".into(), "sink".into()],
            native_counterpart_uuid: "witness".into(),
            layer: "F.Cu".into(),
            width_mm: 0.15,
            start_mm: [0.0, 0.0],
            end_mm: [1.0, 0.0],
            stored_reports: None,
            stored_connectivity_proof: None,
        }
    }

    #[test]
    fn the_added_bridge_must_merge_the_two_reported_components() {
        let groups = |components: &[&[&str]]| {
            components
                .iter()
                .map(|ids| {
                    (
                        "GND".to_string(),
                        ids.iter().map(|id| id.to_string()).collect(),
                    )
                })
                .collect::<Vec<_>>()
        };
        let after = groups(&[&["open", "sink", "witness", "bridge"]]);
        let bridge = bridge();
        assert!(bridge.proves_partitions(&groups(&[&["open"], &["sink", "witness"]]), &after));
        assert!(!bridge.proves_partitions(&groups(&[&["open", "sink"], &["witness"]]), &after));
        assert!(!bridge.proves_partitions(&groups(&[&["open"], &["sink"], &["witness"]]), &after));
    }

    #[test]
    fn the_other_native_witnesses_and_findings_must_remain_exact() {
        let open = |a, b| {
            json!({
                "type":"unconnected_items", "severity":"error", "items":[{"uuid":a},{"uuid":b}]
            })
        };
        let before = json!({
            "coordinate_units":"mm", "ignored_checks":[], "violations":[], "schematic_parity":[],
            "unconnected_items":[open("open","witness"), open("other","mate")]
        });
        let mut after = before.clone();
        after["unconnected_items"] = json!([open("mate", "other")]);
        let bridge = bridge();
        assert!(bridge.closes(&before, &after).unwrap());
        after["unconnected_items"] = json!([open("mate", "substitute")]);
        assert!(!bridge.closes(&before, &after).unwrap());
        after["unconnected_items"] = json!([open("mate", "other")]);
        after["violations"] = json!([{"type":"shorting_items","severity":"error"}]);
        assert!(!bridge.closes(&before, &after).unwrap());
    }
}
