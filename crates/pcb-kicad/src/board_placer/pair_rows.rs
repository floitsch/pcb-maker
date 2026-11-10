// Copyright (C) 2026 Toit contributors.

//! A differential pair's two series parts as a row of two (`pair_rows`,
//! docs/placer.md "Pair rows").
//!
//! The router couples a pair only between parts whose pads of the pair lie
//! side by side (router.md, "Pair points and links"); a pair's AC-coupling
//! capacitors placed apart leave the pair uncoupled through them (trial 5:
//! 13 of ducktop2 right_io's 19 fallen-back pairs). Two parts of one
//! footprint, two pads each, whose pads of one number lie on the two nets
//! of a pair KiCad pairs by name, become a `row` of two across their pads
//! (`KiCadRowConstraint::pair`), the positive net's part first, at the
//! pair's own pitch where the footprint allows it (`pair_pitch`: the
//! coupled run's tracks `w + g` apart, at least the courtyards' width and
//! the pads' clearance), and turned after the placement to face the pair's
//! two ends (`pcb_placer::facing`).

use super::PairGeometry;
use crate::placement_constraints::KiCadRowConstraint;
use pcb_placer as core;
use std::collections::BTreeMap;

/// Per positive net of a pair (normalized, as the pads' nets are): its
/// geometry from the router's configuration, as `diff_pairs::find_pairs`
/// takes it (the rules' width and gap on the first layer they allow the
/// pair, else the net's width and its clearance).
pub(crate) fn pair_geometry(
    config: &crate::board_router::KiCadBoardRouterConfig,
) -> BTreeMap<String, PairGeometry> {
    let mut out = BTreeMap::new();
    for (name, rules) in &config.connection_rules {
        let Some((partner, _, true)) = crate::project_rules::diff_pair_of(name) else {
            continue;
        };
        let Some(other) = config.connection_rules.get(&partner) else {
            continue;
        };
        let clearance = rules.clearance_mm.max(other.clearance_mm);
        let (mut width, mut gap) = (rules.trace_width_mm, clearance);
        if let Some(rule) = config.diff_pair_rules.get(name)
            && let Some(&(w, g)) = (0..rule.layers.len())
                .find(|layer| !rule.disallowed.get(*layer).copied().unwrap_or(false))
                .and_then(|layer| rule.layers.get(layer))
        {
            (width, gap) = (w, g);
        }
        out.insert(
            name.clone(),
            PairGeometry {
                width,
                gap,
                clearance,
            },
        );
    }
    out
}

/// A pair row: the row, the nets of its two sides (the pair's pads' nets,
/// positive first, then the nets of the parts' other pads) and what its
/// pitch is (`constraints[]`'s detail).
#[derive(Clone, Debug)]
pub(crate) struct PairRow {
    pub row: KiCadRowConstraint,
    pub nets: [[String; 2]; 2],
    pub detail: String,
}

/// A pair row as the lowered placement keeps it: its parts (first,
/// other), its leader's facing (`pcb_placer::facing`) and the pitch's
/// detail.
#[derive(Clone, Debug)]
pub(crate) struct LoweredPairRow {
    pub first: usize,
    pub other: usize,
    pub facing: Option<core::facing::Facing>,
    pub detail: String,
}

/// A footprint as the pair rows see it.
#[derive(Clone, Debug)]
pub(crate) struct PairCandidate {
    pub reference: String,
    /// The footprint's library name (`Capacitor_SMD:C_0402_1005Metric`).
    pub footprint: String,
    /// Its pads with a net: number, net, centre and copper's box ([min x,
    /// min y, max x, max y]) in the footprint's frame.
    pub pads: Vec<PadNet>,
    /// The body's size in the footprint's frame (its courtyard).
    pub body_size: [f64; 2],
    /// The body without the courtyard's margin, where the project lets
    /// courtyards overlap (`Component::tight`).
    pub tight: Option<[f64; 4]>,
    pub angle: f64,
    pub side: core::Side,
    /// Not fixed, named by no constraint, in no row.
    pub free: bool,
}

/// A footprint's pad with a net: number, net, centre and copper's box in
/// the footprint's frame.
pub(crate) type PadNet = (String, String, [f64; 2], [f64; 4]);

/// The pair rows of a lowered board's footprints (`references`, with
/// their library names and pads with nets): its free parts are those not
/// fixed and named by no entry of `constraints` (a reference, a glob, a
/// `pin_of`'s part), so that no constraint lands on a row's second part.
/// `geometry`: the pairs' widths and gaps (`pair_geometry`; empty: the
/// rows at the bodies' width).
pub(crate) fn implicit_pair_rows(
    problem: &core::Problem,
    references: &[String],
    library_names: &[String],
    pad_nets: &[Vec<PadNet>],
    constraints: Option<&crate::placement_constraints::KiCadPlacementConstraints>,
    geometry: &BTreeMap<String, PairGeometry>,
) -> Vec<PairRow> {
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
            tight: problem.components[index].tight,
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
    let rows = pair_rows(&candidates, geometry);
    if !rows.is_empty() {
        eprintln!(
            "placer: pair rows: {} pairs' series parts as rows of two ({})",
            rows.len(),
            rows.iter()
                .map(|row| format!("{} at {} mm", row.row.parts.join(" and "), row.row.pitch_mm))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    rows
}

/// The pair rows of a lowered placement: their parts' indices and their
/// leaders' facings (by the nets' ids of `net_ids`).
pub(crate) fn lowered_pair_rows(
    rows: &[PairRow],
    references: &[String],
    net_ids: &BTreeMap<String, usize>,
) -> Vec<LoweredPairRow> {
    rows.iter()
        .filter_map(|pair_row| {
            let index = |reference: &String| references.iter().position(|r| r == reference);
            let (first, other) = (
                index(&pair_row.row.parts[0])?,
                index(&pair_row.row.parts[1])?,
            );
            let side = |nets: &[String; 2]| -> Option<Vec<usize>> {
                nets.iter().map(|net| net_ids.get(net).copied()).collect()
            };
            let facing = side(&pair_row.nets[0])
                .zip(side(&pair_row.nets[1]))
                .map(|(near, far)| core::facing::Facing {
                    part: first,
                    nets: [near, far],
                });
            Some(LoweredPairRow {
                first,
                other,
                facing,
                detail: pair_row.detail.clone(),
            })
        })
        .collect()
}

/// The pair rows as `constraints[]` reports them: `pair_row`, the first
/// part and the other, kept (a row is one part), with the pitch's detail
/// and how the row faces the pair's ends at `poses`.
pub(crate) fn pair_row_statuses(
    pair_rows: &[LoweredPairRow],
    problem: &core::Problem,
    poses: &[core::Pose],
    references: &[String],
) -> Vec<crate::placement_constraints::KiCadConstraintStatus> {
    pair_rows
        .iter()
        .map(|row| {
            let faced = row
                .facing
                .as_ref()
                .filter(|facing| facing.part < problem.components.len())
                .map(|facing| core::facing::faced(problem, poses, facing));
            let facing = match faced {
                Some(core::facing::Faced::Faces) => "; it faces the pair's two ends",
                Some(core::facing::Faced::Ambiguous) | None => {
                    "; the pair's two ends lie to one side of it: turned as the placement gave it"
                }
                Some(core::facing::Faced::Away(_)) => {
                    "; it does not face the pair's two ends: no room to turn it"
                }
            };
            crate::placement_constraints::KiCadConstraintStatus {
                kind: "pair_row".into(),
                part: references[row.first].clone(),
                other: Some(references[row.other].clone()),
                satisfied: true,
                violation_mm: 0.0,
                detail: Some(format!("{}{facing}", row.detail)),
            }
        })
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
/// of their first parts' references, at their pairs' pitch (`geometry` by
/// positive net).
pub(crate) fn pair_rows(
    candidates: &[PairCandidate],
    geometry: &BTreeMap<String, PairGeometry>,
) -> Vec<PairRow> {
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
        for (number, net, at, _) in &part.pads {
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
            let (Some(far_positive), Some(far_negative)) =
                (other_net(part), other_net(&candidates[negative]))
            else {
                continue;
            };
            if far_positive == far_negative {
                continue;
            }
            let Some((row, detail)) = pair_row(part, &candidates[negative], *at, geometry.get(net))
            else {
                continue;
            };
            used[positive] = true;
            used[negative] = true;
            rows.push(PairRow {
                row,
                nets: [
                    [net.clone(), complement.clone()],
                    [far_positive, far_negative],
                ],
                detail,
            });
            break;
        }
    }
    rows
}

/// The row of `positive` and `negative` across their pads (`at`: the
/// pair's pad of `positive` in its frame) at the pair's pitch
/// (`pair_pitch`), with what the pitch is; `None` where `positive` is not
/// at a quarter turn.
fn pair_row(
    positive: &PairCandidate,
    negative: &PairCandidate,
    at: [f64; 2],
    geometry: Option<&PairGeometry>,
) -> Option<(KiCadRowConstraint, String)> {
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
    let (pitch, detail) = pair_pitch(positive, negative, size, geometry);
    Some((
        KiCadRowConstraint {
            parts: vec![positive.reference.clone(), negative.reference.clone()],
            pitch_mm: pitch,
            axis: Some(axis.into()),
            columns: None,
            row_pitch_mm: None,
            pair: true,
        },
        detail,
    ))
}

/// Rounds up to the hundredth of a millimetre (a hair below it stays).
fn hundredths_up(value: f64) -> f64 {
    ((value - 1.0e-6) * 100.0).ceil() / 100.0
}

/// The pitch of a pair row across its parts' frame axis `size` (0: x, 1:
/// y) and what it is: the pair's track width plus gap (the coupled run's
/// tracks' centres apart, so that the stubs run straight from the pads to
/// it), at least the courtyards' width (touching courtyards are legal;
/// the tight bodies where the project lets courtyards overlap) and the
/// pads' extent plus the pair's clearance; without the pair's geometry the
/// bodies' width.
fn pair_pitch(
    positive: &PairCandidate,
    negative: &PairCandidate,
    size: usize,
    geometry: Option<&PairGeometry>,
) -> (f64, String) {
    let body = |part: &PairCandidate| match part.tight {
        Some(tight) => tight[size + 2] - tight[size],
        None => part.body_size[size],
    };
    let courtyards = body(positive).max(body(negative));
    let courtyard_bound = hundredths_up(courtyards + PAIR_ROW_MARGIN_MM);
    let what = if positive.tight.is_some() {
        "bodies"
    } else {
        "courtyards"
    };
    let Some(geometry) = geometry else {
        return (
            courtyard_bound,
            format!(
                "pitch {courtyard_bound} mm, the {what}' {courtyards:.2} mm across (the pair's track width and gap unknown)"
            ),
        );
    };
    let extent = |part: &PairCandidate| {
        let low = part
            .pads
            .iter()
            .map(|pad| pad.3[size])
            .fold(f64::INFINITY, f64::min);
        let high = part
            .pads
            .iter()
            .map(|pad| pad.3[size + 2])
            .fold(f64::NEG_INFINITY, f64::max);
        (high - low).max(0.0)
    };
    let pads = extent(positive).max(extent(negative));
    let copper_bound = hundredths_up(pads + geometry.clearance);
    let wanted = geometry.width + geometry.gap;
    let pitch = hundredths_up(wanted).max(courtyard_bound).max(copper_bound);
    let pair = format!(
        "the pair's track width and gap ({:.3} + {:.3} = {:.2} mm)",
        geometry.width, geometry.gap, wanted
    );
    let detail = if hundredths_up(wanted) >= pitch {
        format!("pitch {pitch} mm, {pair}: the pads in line with the coupled run's tracks")
    } else if courtyard_bound >= copper_bound {
        format!(
            "pitch {pitch} mm, the {what}' {courtyards:.2} mm across: {pair} cannot be met by the footprint, the stubs narrow to the run"
        )
    } else {
        format!(
            "pitch {pitch} mm, the pads' {pads:.2} mm across and the pair's clearance {:.2} mm: {pair} cannot be met by the footprint, the stubs narrow to the run",
            geometry.clearance
        )
    };
    (pitch, detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An 0402 capacitor: pads 0.56 x 0.62 mm at x = -0.48 and 0.48, its
    /// courtyard 1.82 x 0.92 mm.
    fn capacitor(reference: &str, nets: [&str; 2], angle: f64) -> PairCandidate {
        let pad = |x: f64| [x - 0.28, -0.31, x + 0.28, 0.31];
        PairCandidate {
            reference: reference.into(),
            footprint: "Capacitor_SMD:C_0402_1005Metric".into(),
            pads: vec![
                ("1".into(), nets[0].into(), [-0.48, 0.0], pad(-0.48)),
                ("2".into(), nets[1].into(), [0.48, 0.0], pad(0.48)),
            ],
            body_size: [1.82, 0.92],
            tight: None,
            angle,
            side: core::Side::Front,
            free: true,
        }
    }

    fn rows_of(parts: &[PairCandidate]) -> Vec<KiCadRowConstraint> {
        pair_rows(parts, &BTreeMap::new())
            .into_iter()
            .map(|row| row.row)
            .collect()
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
        let pair_rows = pair_rows(&parts, &BTreeMap::new());
        let rows: Vec<_> = pair_rows.iter().map(|row| row.row.clone()).collect();
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert_eq!(rows[0].parts, vec!["C150", "C151"]);
        // Pads along x: across is y, at the bodies' 0.92 mm and the margin.
        assert_eq!(rows[0].axis.as_deref(), Some("y"));
        assert_eq!(rows[0].pitch_mm, 0.93);
        assert!(rows[0].pair);
        // The sides: the pair's pads' nets, then the far side's.
        assert_eq!(
            pair_rows[0].nets,
            [
                ["/TCP2_TX0_P".to_string(), "/TCP2_TX0_N".to_string()],
                ["/HDMI/D2_P".to_string(), "/HDMI/D2_N".to_string()]
            ]
        );
        // The positive part leads; turned a quarter, across is x.
        assert_eq!(rows[1].parts, vec!["C152", "C153"]);
        assert_eq!(rows[1].axis.as_deref(), Some("x"));
    }

    #[test]
    fn a_pair_rows_pitch_is_the_pairs_width_and_gap_where_the_footprint_allows() {
        let pair = |a: &str, b: &str, part: fn(&str, [&str; 2], f64) -> PairCandidate| {
            vec![
                part("C1", [&format!("{a}_P"), &format!("{b}_P")], 0.0),
                part("C2", [&format!("{a}_N"), &format!("{b}_N")], 0.0),
            ]
        };
        let geometry = |width: f64, gap: f64, clearance: f64| {
            BTreeMap::from([(
                "TX_P".to_string(),
                PairGeometry {
                    width,
                    gap,
                    clearance,
                },
            )])
        };
        // Right_io's DIFF_100 on the outer layers: 0.152 + 0.254 mm, far
        // below two 0402s' courtyards: the courtyards' pitch, said so.
        let rows = pair_rows(
            &pair("TX", "HDMI", capacitor),
            &geometry(0.1521, 0.254, 0.15),
        );
        assert_eq!(rows[0].row.pitch_mm, 0.93);
        assert!(
            rows[0].detail.contains("courtyards' 0.92 mm across")
                && rows[0].detail.contains("0.152 + 0.254 = 0.41 mm")
                && rows[0].detail.contains("cannot be met by the footprint"),
            "{}",
            rows[0].detail
        );
        // Two 0201s (courtyards 1.4 x 0.7 mm, pads 0.3 mm across) on a wide
        // pair (0.3 + 0.5 mm): the pair's pitch, the pads in line with the
        // run's tracks.
        fn small(reference: &str, nets: [&str; 2], angle: f64) -> PairCandidate {
            let mut part = capacitor(reference, nets, angle);
            part.footprint = "Capacitor_SMD:C_0201_0603Metric".into();
            part.body_size = [1.4, 0.7];
            for pad in &mut part.pads {
                let x = pad.2[0] * 0.65;
                pad.2 = [x, 0.0];
                pad.3 = [x - 0.15, -0.15, x + 0.15, 0.15];
            }
            part
        }
        let rows = pair_rows(&pair("TX", "Y", small), &geometry(0.3, 0.5, 0.2));
        assert_eq!(rows[0].row.pitch_mm, 0.8);
        assert!(
            rows[0]
                .detail
                .contains("in line with the coupled run's tracks"),
            "{}",
            rows[0].detail
        );
        // The pads' clearance bounds it where the courtyards would allow
        // less (a project that lets courtyards overlap: the tight bodies,
        // 0.62 mm across, and a clearance of 0.25 mm).
        fn tight(reference: &str, nets: [&str; 2], angle: f64) -> PairCandidate {
            let mut part = capacitor(reference, nets, angle);
            part.tight = Some([-0.76, -0.31, 0.76, 0.31]);
            part
        }
        let rows = pair_rows(&pair("TX", "Y", tight), &geometry(0.15, 0.2, 0.25));
        assert_eq!(rows[0].row.pitch_mm, 0.87);
        assert!(
            rows[0].detail.contains("the pads' 0.62 mm across"),
            "{}",
            rows[0].detail
        );
        // Without the pair's geometry: the courtyards.
        let rows = pair_rows(&pair("TX", "Y", capacitor), &BTreeMap::new());
        assert_eq!(rows[0].row.pitch_mm, 0.93);
        assert!(rows[0].detail.contains("unknown"), "{}", rows[0].detail);
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
        assert!(rows_of(&parts).is_empty());
        // Series resistors whose far side is no pair (the IC's own nets)
        // pair by the near side.
        let parts = vec![
            capacitor("R5", ["Net-(U1-DP)", "USB_DP"], 0.0),
            capacitor("R6", ["Net-(U1-DN)", "USB_DN"], 0.0),
        ];
        let rows = rows_of(&parts);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].parts, vec!["R5", "R6"]);
    }
}
