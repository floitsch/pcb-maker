// Copyright (C) 2026 Florian Loitsch.

//! A short additive escape to confirmed same-net plane copper.

use super::plane_escape::geometry::{Pocket, compact, contact_on_net, pads};
use super::*;
use crate::board_router::LayerTable;
use pcb_router::{Board, Point, geometry};

const MAX_PADS: usize = 4;
const MAX_LENGTH_MM: f64 = 2.0;

/// Shortlist physical open pads that could reach an authored same-net
/// plane. Actual refill copper is checked later by `propose`.
pub(super) fn candidate_pads(
    pcb: &Expr,
    board: &Board,
    rules: &KiCadBoardRouterConfig,
    native: &Value,
    candidates: &[(&str, &[Value])],
    pieces: Option<&Pieces>,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut result = BTreeMap::<String, BTreeSet<String>>::new();
    if board.layer_count <= 2
        || board.planes.iter().all(|plane| plane.hatch.is_some())
        || native["coordinate_units"] != "mm"
        || report_capped(native)
        || !finding_types_at_cap(native, "schematic_parity").is_empty()
        || crate::board_router::vias_tented(pcb) != [true, true]
    {
        return result;
    }
    let mut panel = pieces.map(|pieces| NativePanelScope::new(pcb, pieces));
    let witnesses: BTreeSet<_> = candidates
        .iter()
        .filter(|(_, items)| {
            panel.as_mut().is_none_or(|scope| {
                matches!(
                    scope.location(items),
                    pcb_kicad_file::outline_pieces::NativeFindingLocation::Piece(_)
                )
            })
        })
        .flat_map(|(net, items)| {
            items
                .iter()
                .filter_map(move |item| Some((*net, item["uuid"].as_str()?)))
        })
        .collect();
    let names = NetNames::of(pcb);
    for pad in pads(pcb)
        .into_iter()
        .filter(|pad| witnesses.contains(&(pad.net.as_str(), pad.id.as_str())))
    {
        let name = names.normalize(&pad.net);
        if rules.length_rules.iter().any(|rule| {
            rule.nets
                .iter()
                .any(|member| project_rules::wildcard_match(&member.net, name))
        }) || project_rules::diff_pair_of(name)
            .is_some_and(|(mate, _, _)| board.nets.iter().any(|net| net.name == mate))
        {
            continue;
        }
        let Some((net, terminal)) = contact_on_net(board, &pad.label, name) else {
            continue;
        };
        if !board.planes.iter().any(|plane| {
            plane.net as usize == net
                && terminal.layers != (1 << plane.layer)
                && plane.hatch.is_none()
                && (geometry::point_in_polygon(terminal.anchor, &plane.polygon)
                    || geometry::polygon_edges(&plane.polygon).any(|(a, b)| {
                        geometry::point_segment_distance(terminal.anchor, a, b) <= MAX_LENGTH_MM
                    }))
        }) {
            continue;
        }
        let land = &board.obstacles[terminal.pad].shape;
        let extent = land.aabb();
        let connected = board.obstacles.iter().enumerate().any(|(index, obstacle)| {
            index != terminal.pad
                && obstacle.net == Some(net as pcb_router::NetId)
                && obstacle.layers & terminal.layers != 0
                && obstacle.kind == pcb_router::ObstacleKind::Copper
                && extent.intersects(obstacle.shape.aabb())
                && pcb_router::verify::shape_distance(land, &obstacle.shape) <= 1e-9
        });
        if !connected {
            result.entry(pad.net).or_default().insert(pad.id);
        }
    }
    result
}

#[derive(Serialize)]
pub struct PlaneLanding {
    pub pad: String,
    pub pad_uuid: String,
    pub net: String,
    pub surface: String,
    pub plane_layer: String,
    pub zone_uuid: String,
    pub path_mm: Vec<Point>,
    pub track_width_mm: f64,
    pub via_mm: [f64; 2],
    pub searched_steps: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verified_filled_islands: Option<usize>,
}

pub(super) struct Inputs<'a> {
    pub pcb: &'a Expr,
    pub filled: &'a Expr,
    pub model: &'a Board,
    pub layers: &'a LayerTable,
    pub config: &'a KiCadBoardRouterConfig,
    pub native_net: &'a str,
    pub net: &'a str,
    pub witnesses: &'a BTreeSet<String>,
}

pub(super) fn propose(input: Inputs<'_>) -> Option<(Vec<Expr>, PlaneLanding)> {
    if crate::board_router::vias_tented(input.pcb) != [true, true] {
        return None;
    }
    let mask_margins = same_net_mask_margins(input.pcb, input.native_net);
    let codes = net_codes(input.pcb);
    let filled_codes = net_codes(input.filled);
    let zones: BTreeSet<_> = input
        .pcb
        .children()
        .iter()
        .filter_map(|item| {
            (item.head() == Some("zone")
                && item.child("keepout").is_none()
                && item
                    .child("fill")
                    .and_then(|fill| form_atom(fill, "mode", 1))
                    != Some("hatch")
                && item_net(item, &codes) == Some(input.native_net))
            .then(|| item_id(item))
            .flatten()
        })
        .collect();
    let mut fills = Vec::new();
    for zone in input
        .filled
        .children()
        .iter()
        .filter(|item| item.head() == Some("zone"))
    {
        let Some(id) = item_id(zone).filter(|id| zones.contains(id)) else {
            continue;
        };
        if item_net(zone, &filled_codes) != Some(input.native_net) {
            continue;
        }
        for polygon in zone
            .children()
            .iter()
            .filter(|item| item.head() == Some("filled_polygon") && item.child("island").is_none())
        {
            if let Some((layer, points)) = fill_polygon(polygon, input.layers) {
                fills.push((layer, id.clone(), points));
            }
        }
    }
    if fills.is_empty() {
        return None;
    }
    let mut probes = 0;
    let mut best = None::<(usize, f64, PlaneLanding)>;
    for pad in pads(input.pcb)
        .into_iter()
        .filter(|pad| pad.net == input.native_net && input.witnesses.contains(&pad.id))
        .take(MAX_PADS)
    {
        let Some((net, terminal)) = contact_on_net(input.model, &pad.label, input.net) else {
            continue;
        };
        let class = input.model.classes[input.model.nets[net].class];
        if [
            class.trace_width,
            class.via_diameter,
            class.via_drill,
            input.config.edge_clearance_mm,
        ]
        .into_iter()
        .any(|value| !value.is_finite() || value < 0.0)
            || input
                .config
                .track_disallowed
                .get(input.net)
                .is_some_and(|mask| mask & terminal.layers != 0)
            || class.layers != 0 && class.layers & terminal.layers == 0
        {
            continue;
        }
        let placed_fill = |at| {
            fills.iter().find(|(layer, _, shape)| {
                terminal.layers != 1 << layer && geometry::point_in_polygon(at, shape)
            })
        };
        let nominal = [class.via_diameter, class.via_drill];
        let via_sizes = std::iter::once(nominal).chain(
            super::local::compact_vias(input.config, class)
                .into_iter()
                .take(3),
        );
        for (rank, via) in via_sizes.enumerate() {
            if best.as_ref().is_some_and(|(prior, _, _)| rank > *prior) {
                break;
            }
            let finder = Pocket::on_routed_board(
                input.model,
                net,
                terminal,
                input.config.edge_clearance_mm,
                via,
                &mask_margins,
            );
            let Some(path) = finder.find(&mut probes, |at| placed_fill(at).is_some()) else {
                continue;
            };
            let length: f64 = path
                .windows(2)
                .map(|pair| geometry::distance(pair[0], pair[1]))
                .sum();
            if path.len() < 2
                || length > MAX_LENGTH_MM
                || best
                    .as_ref()
                    .is_some_and(|(prior, best, _)| rank == *prior && length >= *best)
            {
                continue;
            }
            let (landing_layer, zone, _) = placed_fill(*path.last()?)?;
            let surface = terminal.layers.trailing_zeros() as usize;
            best = Some((
                rank,
                length,
                PlaneLanding {
                    pad: pad.label.clone(),
                    pad_uuid: pad.id.clone(),
                    net: unescape(input.native_net).into_owned(),
                    surface: input.layers.names.get(surface)?.clone(),
                    plane_layer: input.layers.names.get(*landing_layer)?.clone(),
                    zone_uuid: zone.clone(),
                    path_mm: compact(&path),
                    track_width_mm: class.trace_width,
                    via_mm: via,
                    searched_steps: 0,
                    verified_filled_islands: None,
                },
            ));
        }
    }
    let (_, _, mut landing) = best?;
    landing.searched_steps = probes;
    let via = *landing.path_mm.last()?;
    let objects = landing
        .path_mm
        .windows(2)
        .map(|points| {
            supplemental_segment_expr(&SupplementalSegment {
                connection: input.native_net.to_owned(),
                start: points[0],
                end: points[1],
                width: landing.track_width_mm,
                layer: landing.surface.clone(),
            })
        })
        .chain(std::iter::once(supplemental_via_expr(&SupplementalVia {
            connection: input.native_net.to_owned(),
            at: via,
            size: landing.via_mm[0],
            drill: landing.via_mm[1],
            layers: ["F.Cu".into(), "B.Cu".into()],
            kind: None,
        })))
        .map(|mut object| {
            if let Expr::List(parts) = &mut object {
                for field in parts.iter_mut().filter(|item| item.head() == Some("net")) {
                    *field = source_net_field(&codes, input.native_net);
                }
            }
            object
        })
        .collect();
    Some((objects, landing))
}

fn same_net_mask_margins(pcb: &Expr, net: &str) -> BTreeMap<String, f64> {
    let default = pcb
        .child("setup")
        .and_then(|setup| form_f64(setup, "pad_to_mask_clearance", 1).ok())
        .unwrap_or(0.0);
    let codes = net_codes(pcb);
    let mut margins = BTreeMap::<String, f64>::new();
    for footprint in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
    {
        let Some(reference) = pcb_kicad_file::kicad_nets::footprint_reference(footprint) else {
            continue;
        };
        let base = form_f64(footprint, "solder_mask_margin", 1).unwrap_or(default);
        for pad in footprint
            .children()
            .iter()
            .filter(|item| item.head() == Some("pad"))
        {
            let Some(number) = pad.children().get(1).and_then(Expr::atom) else {
                continue;
            };
            if item_net(pad, &codes) == Some(net) {
                let margin = form_f64(pad, "solder_mask_margin", 1)
                    .unwrap_or(base)
                    .max(0.0);
                let entry = margins.entry(format!("{reference}.{number}")).or_default();
                *entry = entry.max(margin);
            }
        }
    }
    margins
}

fn fill_polygon(polygon: &Expr, layers: &LayerTable) -> Option<(usize, Vec<Point>)> {
    let layer = layers.index(form_atom(polygon, "layer", 1)?)?;
    let points = polygon
        .child("pts")?
        .children()
        .iter()
        .skip(1)
        .map(|point| {
            let x = point.children().get(1)?.atom()?.parse::<f64>().ok()?;
            let y = point.children().get(2)?.atom()?.parse::<f64>().ok()?;
            (x.is_finite() && y.is_finite()).then_some([x, y])
        })
        .collect::<Option<Vec<_>>>()?;
    (points.len() >= 3).then_some((layer, points))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn shortlist_uses_local_isolated_pad_geometry_and_requires_complete_native_evidence() {
        let pcb = parse(
            r#"(kicad_pcb (version 20260206)
              (layers (0 "F.Cu" signal) (4 "In1.Cu" signal) (6 "In2.Cu" signal)
                      (2 "B.Cu" signal) (25 "Edge.Cuts" user))
              (gr_rect (start 0 0) (end 30 20) (layer "Edge.Cuts"))
              (footprint "test" (layer "F.Cu") (at 3 5) (property "Reference" "J1")
                (pad "1" smd circle (at 0 0) (size 1 1) (layers "F.Cu") (net "/V") (uuid "ready")))
              (footprint "test" (layer "F.Cu") (at 7 5) (property "Reference" "J2")
                (pad "1" smd circle (at 0 0) (size 1 1) (layers "F.Cu") (net "/V") (uuid "wired")))
              (footprint "test" (layer "F.Cu") (at 23 5) (property "Reference" "J3")
                (pad "1" smd circle (at 0 0) (size 1 1) (layers "F.Cu") (net "/V") (uuid "far")))
              (segment (start 7 5) (end 9 5) (width 0.2) (layer "F.Cu") (net "/V"))
              (zone (net "/V") (layer "In1.Cu")
                  (polygon (pts (xy 2 2) (xy 11 2) (xy 11 10) (xy 2 10)))))"#,
        )
        .unwrap();
        let config = KiCadBoardRouterConfig {
            default_rules: Some(KiCadConnectionRoutingRules {
                trace_width_mm: 0.25,
                clearance_mm: 0.2,
                via_size_mm: 0.7,
                via_drill_mm: 0.35,
            }),
            ..Default::default()
        };
        let (rules, board) = local::lower_for_local(&pcb, config).unwrap();
        let nearby = [json!({"uuid":"ready"}), json!({"uuid":"wired"})];
        let distant = [json!({"uuid":"far"}), json!({"uuid":"wired"})];
        let candidates = [("/V", &nearby[..]), ("/V", &distant[..])];
        let report = json!({"coordinate_units":"mm", "violations":[], "unconnected_items":[]});
        assert_eq!(
            candidate_pads(&pcb, &board, &rules, &report, &candidates, None),
            BTreeMap::from([("/V".into(), BTreeSet::from(["ready".into()]))])
        );
        let mut capped = report;
        capped[CAPPED_KEY] = json!(true);
        assert!(candidate_pads(&pcb, &board, &rules, &capped, &candidates, None).is_empty());
    }
}
