// Copyright (C) 2026 Toit contributors.

//! The gaps between adjacent SMD pads of a footprint's pin rows
//! (`core::Board::pin_row_gaps`).

use pcb_router as core;

/// Pads of a pin row lie at most this many times the closer one's nearest
/// neighbour apart (score-kicad-board's pin-row crossings, aesthetics.rs).
const PIN_ROW_ADJACENT: f64 = 1.25;

/// Adjacent pads farther apart than this (centre to centre, mm) are no pin
/// row to the negotiation's pin-row cost: a track between them is
/// ordinary there.
const PIN_ROW_PITCH_MM: f64 = 1.5;

/// Adjacent pads farther apart than `PIN_ROW_PITCH_MM` and at most this
/// apart are a pin row to the clean-up's searches and comparisons only
/// (`PinRowGap::cleanup_only`): an SOT-23's 1.9 mm, a 2.54 mm header.
/// Charged in the negotiation they cost 46 unconnected on the expansion
/// tier (2026-10-23); the clean-up never leaves a net open.
const PIN_ROW_CLEANUP_PITCH_MM: f64 = 2.6;

/// The gaps between adjacent SMD pads of one footprint (`pads`: centre,
/// copper layers, net), per copper layer with three pads or more: the
/// pairs whose centres lie at most `PIN_ROW_ADJACENT` times the closer
/// pad's nearest-neighbour distance and `PIN_ROW_CLEANUP_PITCH_MM` apart,
/// as score-kicad-board's crossing measure pairs them, those over
/// `PIN_ROW_PITCH_MM` apart `cleanup_only`.
pub(crate) fn footprint_pin_row_gaps(
    pads: &[([f64; 2], core::LayerMask, Option<core::NetId>)],
) -> Vec<core::PinRowGap> {
    let mut gaps = Vec::new();
    let layers = pads.iter().fold(0, |union, pad| union | pad.1);
    let distance = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).hypot(a[1] - b[1]);
    for layer in 0..core::LayerMask::BITS as usize {
        if layers & (1 << layer) == 0 {
            continue;
        }
        let members: Vec<_> = pads
            .iter()
            .filter(|pad| pad.1 & (1 << layer) != 0)
            .collect();
        if members.len() < 3 {
            continue;
        }
        let nearest: Vec<f64> = members
            .iter()
            .map(|a| {
                members
                    .iter()
                    .map(|b| distance(a.0, b.0))
                    .filter(|d| *d > 1.0e-6)
                    .fold(f64::INFINITY, f64::min)
            })
            .collect();
        for (i, a) in members.iter().enumerate() {
            for (j, b) in members.iter().enumerate().skip(i + 1) {
                let gap = distance(a.0, b.0);
                if gap < 1.0e-6
                    || gap > PIN_ROW_CLEANUP_PITCH_MM
                    || gap > PIN_ROW_ADJACENT * nearest[i].min(nearest[j])
                {
                    continue;
                }
                gaps.push(core::PinRowGap {
                    layer,
                    a: a.0,
                    b: b.0,
                    nets: [a.2, b.2],
                    cleanup_only: gap > PIN_ROW_PITCH_MM,
                });
            }
        }
    }
    gaps
}
