// Copyright (C) 2026 Toit contributors.

//! A footprint's exposed pad, which keeps the vias its designer puts in
//! it (`exposed_pad`).

use super::*;

/// A footprint's pad with copper, for `exposed_pad` (the lowering's and
/// score-kicad-board's).
pub(crate) struct PadBox {
    /// Its number (empty for none), without quotes.
    pub number: String,
    pub center: [f64; 2],
    pub smd: bool,
    /// Its copper's box (the pad's own shape): minimum and maximum.
    pub bounds: [[f64; 2]; 2],
}

/// SMD pads of one number whose boxes lie within this of each other are
/// one pad to `exposed_pad`: an exposed pad drawn on both sides (F.Cu and
/// B.Cu copies at one place, thermal vias between) or split into pieces.
const SAME_PAD_GAP: f64 = 0.5;

/// The footprint's exposed pad (a QFN's, a DFN's, an SOIC's thermal pad),
/// if it has one, as the indices of its pads: the largest SMD pad, at
/// least twice any other SMD pad's area, its centre strictly inside the
/// box of the centres of at least three other pads. SMD pads of one
/// number that touch or nearly do (`SAME_PAD_GAP`) count as one pad of
/// their boxes' box: a two-sided exposed pad (Sisu's U2.11, two 1.2 x 2
/// mm copies at one place) and one split into pieces are each one
/// candidate, its copies not "another pad" of its size. A via there is
/// the designer's way out of a pad the pins enclose (thermal vias, as
/// KiCad's own footprints have them, and a large pad keeps its solder);
/// a tab beside a row of pins (SOT-223, D-PAK) is not one: a track
/// reaches it from outside.
pub(crate) fn exposed_pad(pads: &[PadBox]) -> Vec<usize> {
    if pads.len() < 4 {
        return Vec::new();
    }
    // Union-find over the SMD pads of one number within the gap.
    let mut parent: Vec<usize> = (0..pads.len()).collect();
    fn root(parent: &mut [usize], mut index: usize) -> usize {
        while parent[index] != index {
            parent[index] = parent[parent[index]];
            index = parent[index];
        }
        index
    }
    let mut by_number: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (index, pad) in pads.iter().enumerate() {
        if pad.smd && !pad.number.is_empty() {
            by_number.entry(&pad.number).or_default().push(index);
        }
    }
    for members in by_number.values() {
        for (position, &a) in members.iter().enumerate() {
            for &b in &members[position + 1..] {
                let near = (0..2).all(|axis| {
                    pads[a].bounds[0][axis] <= pads[b].bounds[1][axis] + SAME_PAD_GAP
                        && pads[b].bounds[0][axis] <= pads[a].bounds[1][axis] + SAME_PAD_GAP
                });
                if near {
                    let (ra, rb) = (root(&mut parent, a), root(&mut parent, b));
                    parent[ra.max(rb)] = ra.min(rb);
                }
            }
        }
    }
    // Each SMD group's box, by its root.
    let mut groups: BTreeMap<usize, [[f64; 2]; 2]> = BTreeMap::new();
    for (index, pad) in pads.iter().enumerate().filter(|(_, pad)| pad.smd) {
        let group = root(&mut parent, index);
        let bounds = groups.entry(group).or_insert(pad.bounds);
        *bounds = [
            [
                bounds[0][0].min(pad.bounds[0][0]),
                bounds[0][1].min(pad.bounds[0][1]),
            ],
            [
                bounds[1][0].max(pad.bounds[1][0]),
                bounds[1][1].max(pad.bounds[1][1]),
            ],
        ];
    }
    let area =
        |bounds: &[[f64; 2]; 2]| (bounds[1][0] - bounds[0][0]) * (bounds[1][1] - bounds[0][1]);
    let Some((&largest, bounds)) = groups.iter().max_by(|a, b| area(a.1).total_cmp(&area(b.1)))
    else {
        return Vec::new();
    };
    let largest_area = area(bounds);
    if groups
        .iter()
        .any(|(group, other)| *group != largest && 2.0 * area(other) > largest_area)
    {
        return Vec::new();
    }
    let members: Vec<usize> = (0..pads.len())
        .filter(|index| pads[*index].smd && root(&mut parent, *index) == largest)
        .collect();
    let others: Vec<&PadBox> = pads
        .iter()
        .enumerate()
        .filter(|(index, _)| !members.contains(index))
        .map(|(_, other)| other)
        .collect();
    if others.len() < 3 {
        return Vec::new();
    }
    let center = [
        (bounds[0][0] + bounds[1][0]) / 2.0,
        (bounds[0][1] + bounds[1][1]) / 2.0,
    ];
    let inside = (0..2).all(|axis| {
        let (low, high) = others
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), other| {
                (low.min(other.center[axis]), high.max(other.center[axis]))
            });
        low < center[axis] && center[axis] < high
    });
    if inside { members } else { Vec::new() }
}
