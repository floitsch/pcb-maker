// Copyright (C) 2026 Toit contributors.

//! A differential pair's two series parts as a row of two (`pair_rows`,
//! docs/placer.md "Pair rows (2026-11-04)").
//!
//! The router couples a pair only between parts whose pads of the pair lie
//! side by side (router.md, "Pair points and links"); a pair's AC-coupling
//! capacitors placed apart leave the pair uncoupled through them (trial 5:
//! 13 of ducktop2 right_io's 19 fallen-back pairs). Two parts of one
//! footprint, two pads each, whose pads of one number lie on the two nets
//! of a pair KiCad pairs by name, become a `row` of two across their pads
//! (`KiCadRowConstraint::pair`), the positive net's part first.

use crate::placement_constraints::KiCadRowConstraint;
use pcb_placer as core;

/// A footprint as the pair rows see it.
#[derive(Clone, Debug)]
pub(crate) struct PairCandidate {
    pub reference: String,
    /// The footprint's library name (`Capacitor_SMD:C_0402_1005Metric`).
    pub footprint: String,
    /// Its pads with a net: number, net and centre in the footprint's
    /// frame.
    pub pads: Vec<(String, String, [f64; 2])>,
    /// The body's size in the footprint's frame.
    pub body_size: [f64; 2],
    pub angle: f64,
    pub side: core::Side,
    /// Not fixed, named by no constraint, in no row.
    pub free: bool,
}

/// The pair rows of a lowered board's footprints (`references`, with
/// their library names and pads with nets): its free parts are those not
/// fixed and named by no entry of `constraints` (a reference, a glob, a
/// `pin_of`'s part), so that no constraint lands on a row's second part.
pub(crate) fn implicit_pair_rows(
    problem: &core::Problem,
    references: &[String],
    library_names: &[String],
    pad_nets: &[Vec<(String, String, [f64; 2])>],
    constraints: Option<&crate::placement_constraints::KiCadPlacementConstraints>,
) -> Vec<KiCadRowConstraint> {
    let mut named: Vec<String> = Vec::new();
    if let Some(value) = constraints.and_then(|constraints| serde_json::to_value(constraints).ok())
    {
        strings_of(&value, &mut named);
    }
    let candidates: Vec<PairCandidate> = references
        .iter()
        .enumerate()
        .map(|(index, reference)| PairCandidate {
            reference: reference.clone(),
            footprint: library_names.get(index).cloned().unwrap_or_default(),
            pads: pad_nets.get(index).cloned().unwrap_or_default(),
            body_size: problem.components[index].body_size,
            angle: problem.poses[index].angle,
            side: problem.components[index].side,
            free: !problem.components[index].fixed
                && !named
                    .iter()
                    .any(|name| crate::board_placer::glob_matches(name, reference)),
        })
        .collect();
    // A reference several footprints share (a panel's copies) is no
    // pair's: which is meant is not told.
    let mut counts: std::collections::BTreeMap<&str, usize> = Default::default();
    for reference in references {
        *counts.entry(reference.as_str()).or_default() += 1;
    }
    let candidates: Vec<PairCandidate> = candidates
        .into_iter()
        .map(|mut candidate| {
            candidate.free &= counts[candidate.reference.as_str()] == 1;
            candidate
        })
        .collect();
    let rows = pair_rows(&candidates);
    if !rows.is_empty() {
        eprintln!(
            "placer: pair rows: {} pairs' series parts as rows of two ({})",
            rows.len(),
            rows.iter()
                .map(|row| row.parts.join(" and "))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    rows
}

/// The pair rows as `constraints[]` reports them: `pair_row`, the first
/// part and the other, kept (a row is one part).
pub(crate) fn pair_row_statuses(
    pair_rows: &[(usize, usize)],
    references: &[String],
) -> Vec<crate::placement_constraints::KiCadConstraintStatus> {
    pair_rows
        .iter()
        .map(
            |(first, other)| crate::placement_constraints::KiCadConstraintStatus {
                kind: "pair_row".into(),
                part: references[*first].clone(),
                other: Some(references[*other].clone()),
                satisfied: true,
                violation_mm: 0.0,
            },
        )
        .collect()
}

/// Every string in `value` (a `pin_of`'s `J1:11` as its part `J1`).
fn strings_of(value: &serde_json::Value, into: &mut Vec<String>) {
    match value {
        serde_json::Value::String(text) => {
            into.push(text.split(':').next().unwrap_or(text).to_string());
        }
        serde_json::Value::Array(items) => {
            for item in items {
                strings_of(item, into);
            }
        }
        serde_json::Value::Object(map) => {
            for item in map.values() {
                strings_of(item, into);
            }
        }
        _ => {}
    }
}

/// What lies between two pair parts' bodies across the row: courtyards
/// that touch are legal in KiCad, the margin keeps rounding from making
/// them overlap.
const PAIR_ROW_MARGIN_MM: f64 = 0.01;

/// The pair rows of `candidates` (each part in one at most), in the order
/// of their first parts' references.
pub(crate) fn pair_rows(candidates: &[PairCandidate]) -> Vec<KiCadRowConstraint> {
    let two_pads = |candidate: &PairCandidate| {
        candidate.free
            && candidate.pads.len() == 2
            && candidate.pads[0].0 != candidate.pads[1].0
            && candidate.pads[0].1 != candidate.pads[1].1
    };
    let mut order: Vec<usize> = (0..candidates.len())
        .filter(|index| two_pads(&candidates[*index]))
        .collect();
    order.sort_by(|a, b| {
        crate::canonical_order::natural_cmp(&candidates[*a].reference, &candidates[*b].reference)
    });
    let mut used = vec![false; candidates.len()];
    let mut rows = Vec::new();
    for &positive in &order {
        if used[positive] {
            continue;
        }
        let part = &candidates[positive];
        for (number, net, at) in &part.pads {
            let Some((complement, _, true)) = crate::project_rules::diff_pair_of(net) else {
                continue;
            };
            // The one other part of the footprint with the same pad on the
            // pair's other net; two such parts: no row (which is meant is
            // not told).
            let partners: Vec<usize> = order
                .iter()
                .copied()
                .filter(|other| {
                    let candidate = &candidates[*other];
                    *other != positive
                        && !used[*other]
                        && candidate.footprint == part.footprint
                        && candidate.side == part.side
                        && candidate
                            .pads
                            .iter()
                            .any(|pad| &pad.0 == number && pad.1 == complement)
                })
                .collect();
            let [negative] = partners[..] else {
                continue;
            };
            // Series parts: their other pads lie on two nets (not both on
            // one rail, as a pair's ESD diodes or terminations do).
            let other_net = |candidate: &PairCandidate| {
                candidate
                    .pads
                    .iter()
                    .find(|pad| &pad.0 != number)
                    .map(|pad| pad.1.clone())
            };
            if other_net(part) == other_net(&candidates[negative]) {
                continue;
            }
            let Some(row) = pair_row(part, &candidates[negative], *at) else {
                continue;
            };
            used[positive] = true;
            used[negative] = true;
            rows.push(row);
            break;
        }
    }
    rows
}

/// The row of `positive` and `negative` across their pads (`at`: the
/// pair's pad of `positive` in its frame), at their bodies' width across;
/// `None` where `positive` is not at a quarter turn.
fn pair_row(
    positive: &PairCandidate,
    negative: &PairCandidate,
    at: [f64; 2],
) -> Option<KiCadRowConstraint> {
    let other = positive.pads.iter().find(|pad| pad.2 != at)?.2;
    let along = [other[0] - at[0], other[1] - at[1]];
    // Across the pads, in the part's frame.
    let (across, size) = if along[0].abs() >= along[1].abs() {
        ([0.0, 1.0], 1)
    } else {
        ([1.0, 0.0], 0)
    };
    let turns = positive.angle.rem_euclid(360.0) / 90.0;
    if (turns - turns.round()).abs() > 1.0e-6 {
        return None;
    }
    // A row's axis is the board's (`apply_rows` turns it into the first
    // part's frame by its angle).
    let board = core::problem::rotate(across, -positive.angle);
    let axis = if board[0].abs() > 0.5 { "x" } else { "y" };
    let width = positive.body_size[size].max(negative.body_size[size]);
    let pitch = ((width + PAIR_ROW_MARGIN_MM) * 100.0).ceil() / 100.0;
    Some(KiCadRowConstraint {
        parts: vec![positive.reference.clone(), negative.reference.clone()],
        pitch_mm: pitch,
        axis: Some(axis.into()),
        columns: None,
        row_pitch_mm: None,
        pair: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capacitor(reference: &str, nets: [&str; 2], angle: f64) -> PairCandidate {
        PairCandidate {
            reference: reference.into(),
            footprint: "Capacitor_SMD:C_0402_1005Metric".into(),
            pads: vec![
                ("1".into(), nets[0].into(), [-0.48, 0.0]),
                ("2".into(), nets[1].into(), [0.48, 0.0]),
            ],
            body_size: [1.82, 0.92],
            angle,
            side: core::Side::Front,
            free: true,
        }
    }

    #[test]
    fn a_pairs_series_capacitors_become_a_row_across_their_pads() {
        // Trial 5's right_io: C150 and C151 on TCP2_TX0_P and _N, the
        // HDMI side behind them; C152 and C153 the next pair, listed N
        // first.
        let parts = vec![
            capacitor("C150", ["/TCP2_TX0_P", "/HDMI/D2_P"], 0.0),
            capacitor("C151", ["/TCP2_TX0_N", "/HDMI/D2_N"], 180.0),
            capacitor("C153", ["/TCP2_TX1_N", "/HDMI/D1_N"], 90.0),
            capacitor("C152", ["/TCP2_TX1_P", "/HDMI/D1_P"], 90.0),
        ];
        let rows = pair_rows(&parts);
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert_eq!(rows[0].parts, vec!["C150", "C151"]);
        // Pads along x: across is y, at the bodies' 0.92 mm and the margin.
        assert_eq!(rows[0].axis.as_deref(), Some("y"));
        assert_eq!(rows[0].pitch_mm, 0.93);
        assert!(rows[0].pair);
        // The positive part leads; turned a quarter, across is x.
        assert_eq!(rows[1].parts, vec!["C152", "C153"]);
        assert_eq!(rows[1].axis.as_deref(), Some("x"));
    }

    #[test]
    fn only_free_series_parts_of_one_footprint_pair() {
        let mut fixed = capacitor("C2", ["D_N", "X_N"], 0.0);
        fixed.free = false;
        let mut other_footprint = capacitor("C4", ["E_N", "Y_N"], 0.0);
        other_footprint.footprint = "Capacitor_SMD:C_0603_1608Metric".into();
        let parts = vec![
            capacitor("C1", ["D_P", "X_P"], 0.0),
            fixed,
            capacitor("C3", ["E_P", "Y_P"], 0.0),
            other_footprint,
            // Both to one rail: a pair's ESD diodes, no series parts.
            capacitor("D1", ["USB_D+", "GND"], 0.0),
            capacitor("D2", ["USB_D-", "GND"], 0.0),
            // The pair's pads of different numbers (one part turned in
            // the schematic's pinout): not told which way to row them.
            capacitor("R1", ["F_P", "Z_P"], 0.0),
            capacitor("R2", ["Z_N", "F_N"], 0.0),
            // A pair net on one part alone.
            capacitor("R3", ["G_P", "W"], 0.0),
        ];
        assert!(pair_rows(&parts).is_empty());
        // Series resistors whose far side is no pair (the IC's own nets)
        // pair by the near side.
        let parts = vec![
            capacitor("R5", ["Net-(U1-DP)", "USB_DP"], 0.0),
            capacitor("R6", ["Net-(U1-DN)", "USB_DN"], 0.0),
        ];
        let rows = pair_rows(&parts);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].parts, vec!["R5", "R6"]);
    }
}
