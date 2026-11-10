// Copyright (C) 2026 Toit contributors.

//! After the route: each coupled pair's skew and uncoupled length, from
//! the written board.

use super::*;
use pcb_router as core;

/// A net's copper: tracks (layer, start, end) and vias.
type NetCopper = (Vec<(usize, Point, Point)>, Vec<Point>);

/// After the route: each coupled pair's uncoupled length (the longer net's
/// tracks less the coupled run), from the routed board.
pub(crate) fn measure_uncoupled(report: &mut KiCadDiffPairs, pcb: &Expr) {
    let mut lengths: BTreeMap<String, f64> = BTreeMap::new();
    // Per net: its tracks (layer as an index into `names`) and vias, for
    // the skew.
    let by_name = copper_heights(pcb);
    let names: Vec<&String> = by_name.keys().collect();
    let heights: Vec<f64> = by_name.values().copied().collect();
    let mut copper: BTreeMap<String, NetCopper> = BTreeMap::new();
    for item in pcb.children() {
        let kind = item.head();
        if kind != Some("segment") && kind != Some("via") {
            continue;
        }
        let Some(net) = node_net(item) else {
            continue;
        };
        // The board's spelling, as the pairs name their nets.
        let net = crate::kicad_nets::normalize_net(net).to_string();
        if kind == Some("via") {
            if let Ok(at) = form_xy(item, "at") {
                copper.entry(net).or_default().1.push(at);
            }
            continue;
        }
        let (Ok(start), Ok(end)) = (form_xy(item, "start"), form_xy(item, "end")) else {
            continue;
        };
        *lengths.entry(net.clone()).or_default() += core::geometry::distance(start, end);
        let layer = form_atom(item, "layer", 1)
            .and_then(|layer| names.iter().position(|name| name.as_str() == layer))
            .unwrap_or(usize::MAX);
        copper.entry(net).or_default().0.push((layer, start, end));
    }
    // The skew: per pair with a coupled link its nets' lengths apart, the
    // vias at the stackup's heights.
    report.skew_mm = 0.0;
    report.skews.clear();
    for pair in &mut report.pairs {
        if pair.links_coupled == 0 {
            continue;
        }
        let length = |net: &str| {
            copper
                .get(net)
                .map_or(0.0, |(tracks, vias)| copper_length(tracks, vias, &heights))
        };
        let (p, n) = (length(&pair.positive), length(&pair.negative));
        let skew = (p - n).abs();
        pair.skew_mm = Some(skew);
        report.skew_mm = report.skew_mm.max(skew);
        report.skews.push(KiCadPairSkew {
            positive: pair.positive.clone(),
            negative: pair.negative.clone(),
            lengths_mm: [p, n],
            skew_mm: skew,
            target_mm: pair.skew_target_mm,
            meander_mm: pair.meander_mm,
            pair_vias: pair.pair_vias,
            return_vias: pair.return_vias,
            uncoupled_mm: 0.0,
        });
    }
    report.uncoupled_mm = 0.0;
    for pair in &mut report.pairs {
        if pair.links_coupled == 0 {
            continue;
        }
        let longer = lengths
            .get(&pair.positive)
            .copied()
            .unwrap_or(0.0)
            .max(lengths.get(&pair.negative).copied().unwrap_or(0.0));
        let uncoupled = (longer - pair.coupled_mm).max(0.0);
        pair.uncoupled_mm = Some(uncoupled);
        report.uncoupled_mm += uncoupled;
        if let Some(skew) = report
            .skews
            .iter_mut()
            .find(|skew| skew.positive == pair.positive && skew.negative == pair.negative)
        {
            skew.uncoupled_mm = uncoupled;
        }
    }
}
