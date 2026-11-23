// Copyright (C) 2026 Florian Loitsch.

//! Evidence of pad copper in a DRC-reported footprint mask opening.

use std::collections::{BTreeMap, BTreeSet};

use crate::geometry::{point_in_polygon, point_segment_distance_squared};
use crate::kicad_nets::{NetNames, footprint_reference, footprint_uuid, node_net};
use crate::kicad_sexpr::{Expr, form_atom, form_xy};

/// Match KiCad's specific graphic by UUID and prove pad exposure in the
/// footprint's local frame. Pad centres must lie inside the filled shape.
pub(super) fn shared_pad_openings(
    board: &Expr,
    names: &NetNames,
    reported: &BTreeSet<(&str, &str, &str)>,
) -> Option<String> {
    if reported.is_empty() {
        return None;
    }
    let mut proven = BTreeSet::new();
    for part in board
        .children()
        .iter()
        .filter(|item| matches!(item.head(), Some("footprint" | "module")))
    {
        let Some(reference) = footprint_reference(part) else {
            continue;
        };
        for opening in part
            .children()
            .iter()
            .filter(|item| matches!(item.head(), Some("fp_poly" | "fp_rect")))
        {
            let Some(mask @ ("F.Mask" | "B.Mask")) = form_atom(opening, "layer", 1) else {
                continue;
            };
            let copper = if mask == "F.Mask" { "F.Cu" } else { "B.Cu" };
            if !footprint_uuid(opening)
                .is_some_and(|uuid| reported.contains(&(reference.as_str(), copper, uuid.as_str())))
            {
                continue;
            }
            let Ok(shapes) = crate::board_router::copper_graphic_shapes(opening) else {
                continue;
            };
            let Some(points) = shapes.iter().find_map(|shape| match shape {
                pcb_router::Shape::Polygon { points } if points.len() >= 3 => Some(points),
                _ => None,
            }) else {
                continue;
            };
            let mut by_net = BTreeMap::new();
            for pad in part
                .children()
                .iter()
                .filter(|item| item.head() == Some("pad"))
            {
                if let Some((number, net)) = exposed_pad(pad, copper, points) {
                    by_net.entry(names.normalize(net)).or_insert((number, net));
                }
            }
            if by_net.len() >= 2 {
                let pads = by_net
                    .into_values()
                    .take(4)
                    .map(|(number, net)| format!("pad {number} [{net}]"))
                    .collect::<Vec<_>>()
                    .join(", ");
                proven.insert(format!("{reference} on {mask} ({pads})"));
            }
        }
    }
    if proven.is_empty() {
        return None;
    }
    let mut examples: Vec<_> = proven.iter().take(4).cloned().collect();
    if proven.len() > examples.len() {
        examples.push(format!("{} more", proven.len() - examples.len()));
    }
    Some(format!(
        "The footprint geometry already exposes its own pads on different nets in one opening ({}) before accounting for tracks, vias or fills. Check the schematic and assembly instructions to see which pads may be soldered together. If the pads must stay separate, use separate openings with a manufacturable mask web. For an intentional shared opening, check that it exposes only the intended jumper copper.",
        examples.join("; ")
    ))
}

fn exposed_pad<'a>(pad: &'a Expr, copper: &str, points: &[[f64; 2]]) -> Option<(&'a str, &'a str)> {
    let [_, number, kind, shape, ..] = pad.children() else {
        return None;
    };
    let number = number.atom().filter(|number| !number.is_empty())?;
    let net = node_net(pad).filter(|net| {
        !net.is_empty()
            && *net != "<no net>"
            && !net
                .rsplit('/')
                .next()
                .unwrap_or(net)
                .starts_with("unconnected-(")
    })?;
    // These ordinary SMD shapes contain copper at their centres. Custom
    // and per-layer padstacks need their own shape proof.
    if kind.atom() != Some("smd")
        || !matches!(shape.atom(), Some("rect" | "roundrect" | "circle" | "oval"))
        || pad.child("padstack").is_some()
        || !form_xy(pad, "size").is_ok_and(|[x, y]| x > 0.0 && y > 0.0)
        || !pad
            .child("layers")?
            .children()
            .iter()
            .skip(1)
            .any(|layer| layer.atom() == Some(copper))
    {
        return None;
    }
    let point = form_xy(pad, "at").ok()?;
    (point_in_polygon(point, points)
        && points
            .iter()
            .zip(points.iter().cycle().skip(1))
            .all(|(&start, &end)| point_segment_distance_squared(point, start, end) > 1.0e-12))
    .then_some((number, net))
}
