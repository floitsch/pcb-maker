// Copyright (C) 2026 Florian Loitsch.

//! Treat KiCad's stored teardrops as local copper in the inspection model.

use super::{LayerTable, NetNames, core, lower};
use crate::board_router::KiCadBoardRouterConfig;
use crate::kicad_nets::{item_net, net_codes};
use crate::{Expr, form_atom};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(super) struct Inventory {
    pub layers: BTreeMap<core::NetId, core::LayerMask>,
    pub unmodelled: BTreeSet<core::NetId>,
}

fn is_teardrop(item: &Expr) -> bool {
    item.head() == Some("zone")
        && item
            .child("attr")
            .is_some_and(|attributes| attributes.child("teardrop").is_some())
}

pub(super) fn inspection_board(
    pcb: &Expr,
    config: &KiCadBoardRouterConfig,
    layers: &LayerTable,
    names: &NetNames,
) -> Result<(core::Board, Inventory), String> {
    let teardrops: Vec<_> = pcb
        .children()
        .iter()
        .filter(|zone| is_teardrop(zone))
        .collect();
    if teardrops.is_empty() {
        return Ok((lower(pcb, config, true)?.board, Inventory::default()));
    }
    let mut projected = pcb.clone();
    let Expr::List(items) = &mut projected else {
        return Err("PCB root is not a list".into());
    };
    items.retain(|item| !is_teardrop(item));
    let mut board = lower(&projected, config, true)?.board;
    let code_names = net_codes(pcb);
    let mut found = Inventory::default();
    for zone in teardrops {
        let declared = form_atom(zone, "net_name", 1).map(|name| names.normalize(name));
        let stored = item_net(zone, &code_names).map(|name| names.normalize(name));
        let Some(name) = stored.or(declared) else {
            continue;
        };
        let net_of = |name| board.nets.iter().position(|net| net.name == name);
        let Some(net) = net_of(name) else {
            continue;
        };
        let net = net as core::NetId;
        if stored.is_some() && declared.is_some_and(|name| Some(name) != stored) {
            found.unmodelled.insert(net);
            if let Some(other) = declared.and_then(net_of) {
                found.unmodelled.insert(other as core::NetId);
            }
            continue;
        }
        let allowed = layers.mask_of_item(zone).unwrap_or(0);
        for fill in zone
            .children()
            .iter()
            .filter(|item| item.head() == Some("filled_polygon"))
        {
            let mask = layers.mask_of_item(fill).unwrap_or(0);
            if mask.count_ones() != 1 || mask & allowed == 0 {
                found.unmodelled.insert(net);
                continue;
            }
            let Some(shape) = polygon(fill) else {
                found.unmodelled.insert(net);
                continue;
            };
            board.obstacles.push(core::Obstacle {
                shape,
                layers: mask,
                kind: core::ObstacleKind::Copper,
                net: Some(net),
                clearance: 0.0,
                clearance_override: None,
                plated: false,
                blocks_tracks: true,
                blocks_vias: true,
                label: "stored teardrop".to_string(),
            });
            *found.layers.entry(net).or_default() |= mask;
        }
    }
    Ok((board, found))
}

fn polygon(fill: &Expr) -> Option<core::Shape> {
    let mut points = crate::outline::pts_points(fill).ok()?;
    if points.first() == points.last() {
        points.pop();
    }
    // The connectivity model supports a simple contour. It has no hole
    // representation, so a bridged hole or self-crossing is ambiguous.
    if !(3..=256).contains(&points.len()) || points.iter().flatten().any(|v| !v.is_finite()) {
        return None;
    }
    let [x, y] = points[0];
    let area: f64 = points[1..]
        .windows(2)
        .map(|pair| (pair[0][0] - x) * (pair[1][1] - y) - (pair[0][1] - y) * (pair[1][0] - x))
        .sum();
    if !area.is_finite() || area == 0.0 {
        return None;
    }
    let edges: Vec<_> = core::geometry::polygon_edges(&points).collect();
    for (index, &(a, b)) in edges.iter().enumerate() {
        if a == b
            || edges
                .iter()
                .enumerate()
                .skip(index + 2)
                .any(|(other, &(c, d))| {
                    (index > 0 || other + 1 != edges.len())
                        && core::geometry::segment_segment_distance(a, b, c, d) < 1.0e-9
                })
        {
            return None;
        }
    }
    Some(core::Shape::Polygon { points })
}
