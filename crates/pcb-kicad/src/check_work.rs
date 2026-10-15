// Copyright (C) 2026 Toit contributors.

//! The work a KiCad check inside a layout counts.
//!
//! A layout checks boards with KiCad (`kicad-cli pcb drc --refill-zones`,
//! with ERC and schematic parity where there is a schematic): the board it
//! kept, the ladder's attempts that could win, the neck repair's trials.
//! Uncached, a check takes 3 to 50 s of wall clock; answered by the DRC
//! cache, under one. The layout's budget is work (seconds of an idle
//! machine), so that the same input gives the same board whatever the
//! load; a check's wall time follows the load and the cache, so counting
//! it would make the layout follow both. Instead every check counts a
//! cost modelled from the board alone, the same cold or warm: a constant,
//! and a term for the holes of hatched copper zones, which KiCad's zone
//! fill cuts one by one (the boards whose checks take 20 to 50 s).
//! Measured on the v16 layout sweep's board checks and on the routed v16
//! boards checked again uncached (docs/handover.md, "Committed on
//! 2026-10-15 (KiCad check work)").

use super::*;
use crate::board_router::{LayerTable, pours};

/// The modelled seconds of one check of a board without hatched zones:
/// the median uncached check on such boards, measured with x1093's
/// kicad-cli settings (a library-free configuration, about 0.69 of the
/// time before it per board): 3.6 s over 58 boards (the v16 sweep's
/// routed boards and the short tier's, checked again one at a time at a
/// load of 14 to 50; quartiles 2.8 and 7.2 s, the slowest 28 s). Before
/// that configuration: 8.0 s over 51 boards in the v16 sweep's logs
/// (load 16 to 35), 5.6 s over 8 in the short tier's cold run, 5.3 s
/// checked again. The spread follows nothing the board file tells: pads,
/// tracks, vias and file size correlate at 0.02 (the v16 triage); ULK,
/// ghoul and koeg (complex outlines, 18 to 33 s) are the slow plain ones.
pub(crate) const CHECK_SECONDS: f64 = 3.6;

/// The seconds a thousand holes of hatched copper add to a check (the
/// holes of a hatched zone: its area on each of its copper layers over
/// the square of the hatch pitch, thickness plus gap). Least squares over
/// the nine hatched boards' checks beyond `CHECK_SECONDS` with x1093:
/// 1.73, correlation 0.90 with the measured times (Batch_1's 30500 holes
/// 60 s, qfwfq's 14500 22 s, capybully's 10600 25 s, the 0xCB panel's
/// 8100 in four zones 29 s, WLED_Matrix's 7200 14 to 34 s with the load,
/// USB_Keypad's 6600 9 s, the stm32 chiffre boards' 6800 on one layer
/// 4 s, 0xCB-1337's 2000 2 s). Before x1093: 1.57 over the v16 logs.
pub(crate) const SECONDS_PER_THOUSAND_HATCH_HOLES: f64 = 1.7;

/// At most this much a check counts, whatever the hatch (the slowest
/// checks measured took 50 to 60 s, under load).
pub(crate) const MAX_CHECK_SECONDS: f64 = 50.0;

/// What the model reads from a board: its hatched copper zones and their
/// holes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct CheckInputs {
    pub hatched_zones: usize,
    pub hatch_holes: f64,
}

/// The model's inputs from the board as given (before routing: the
/// layout's zones are the designer's, its added planes are solid).
pub(crate) fn check_inputs(pcb: &Expr) -> CheckInputs {
    let Ok(layers) = LayerTable::from_pcb(pcb) else {
        return CheckInputs::default();
    };
    let Ok(pours) = pours(pcb, &layers) else {
        return CheckInputs::default();
    };
    let mut inputs = CheckInputs::default();
    for pour in &pours {
        let Some(hatch) = &pour.hatch else { continue };
        let pitch = hatch.thickness + hatch.gap;
        if pitch <= 0.0 {
            continue;
        }
        inputs.hatched_zones += 1;
        inputs.hatch_holes += polygon_area(&pour.polygon) * pour.layers.count_ones() as f64 / (pitch * pitch);
    }
    inputs
}

/// The modelled seconds of one check from the model's inputs.
pub(crate) fn check_seconds(inputs: CheckInputs) -> f64 {
    (CHECK_SECONDS + SECONDS_PER_THOUSAND_HATCH_HOLES * inputs.hatch_holes / 1000.0).min(MAX_CHECK_SECONDS)
}

/// KiCad checks counted as work: each counts `charge`, whether the DRC
/// cache answered it or KiCad ran, so the tally (and every decision taken
/// from the work it adds) is the same warm and cold.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct CheckTally {
    pub charge: f64,
    pub checks: usize,
}

impl CheckTally {
    pub(crate) fn new(charge: f64) -> Self {
        Self { charge, checks: 0 }
    }

    /// Counts `checks` more checks and returns the work they add.
    pub(crate) fn add(&mut self, checks: usize) -> f64 {
        self.checks += checks;
        checks as f64 * self.charge
    }

    /// The work of the checks counted so far.
    pub(crate) fn work(&self) -> f64 {
        self.checks as f64 * self.charge
    }
}

fn polygon_area(points: &[[f64; 2]]) -> f64 {
    let n = points.len();
    if n < 3 {
        return 0.0;
    }
    (0..n)
        .map(|index| {
            let [x0, y0] = points[index];
            let [x1, y1] = points[(index + 1) % n];
            x0 * y1 - x1 * y0
        })
        .sum::<f64>()
        .abs()
        / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board(zones: &str) -> Expr {
        parse(&format!(
            "(kicad_pcb (version 20240108) (layers (0 \"F.Cu\" signal) (31 \"B.Cu\" signal) (44 \"Edge.Cuts\" user)) \
             (net 0 \"\") (net 1 \"GND\") {zones})"
        ))
        .unwrap()
    }

    fn zone(layers: &str, fill: &str) -> String {
        format!(
            "(zone (net 1) (net_name \"GND\") (layers {layers}) (hatch edge 0.5) (connect_pads (clearance 0.2)) \
             (min_thickness 0.2) (fill yes {fill}) (polygon (pts (xy 0 0) (xy 100 0) (xy 100 50) (xy 0 50))))"
        )
    }

    #[test]
    fn checks_count_the_same_work_cached_or_not() {
        // The wall seconds of a cold run's checks and a warm run's (the
        // cache answers in under a second): the tally never sees them.
        let cold = [31.2, 33.5, 50.4, 60.3];
        let warm = [0.5, 0.4, 0.6, 0.5];
        let mut tallies = [CheckTally::new(9.7), CheckTally::new(9.7)];
        let mut work = [0.0, 0.0];
        for side in 0..2 {
            for _seconds in if side == 0 { cold } else { warm } {
                work[side] += tallies[side].add(1);
            }
        }
        assert_eq!(tallies[0], tallies[1]);
        assert_eq!(work[0], work[1]);
        assert!((tallies[0].work() - 4.0 * 9.7).abs() < 1.0e-9);
        assert_eq!(CheckTally::new(9.7).add(0), 0.0);
    }

    #[test]
    fn a_board_without_hatched_zones_counts_the_constant() {
        assert_eq!(check_seconds(check_inputs(&board(""))), CHECK_SECONDS);
        let solid = board(&zone("\"F.Cu\" \"B.Cu\"", "(thermal_gap 0.5)"));
        assert_eq!(check_inputs(&solid), CheckInputs::default());
        assert_eq!(check_seconds(check_inputs(&solid)), CHECK_SECONDS);
    }

    #[test]
    fn hatch_holes_are_the_area_per_layer_over_the_square_pitch() {
        // 5000 mm2 on two layers at a 0.8 mm pitch: 15625 holes.
        let hatched = board(&zone("\"F.Cu\" \"B.Cu\"", "(mode hatch) (hatch_thickness 0.2) (hatch_gap 0.6)"));
        let inputs = check_inputs(&hatched);
        assert_eq!(inputs.hatched_zones, 1);
        assert!((inputs.hatch_holes - 15625.0).abs() < 1.0e-6, "{inputs:?}");
        // One layer, half the holes; a coarser pitch, fewer.
        let one = check_inputs(&board(&zone("\"F.Cu\"", "(mode hatch) (hatch_thickness 0.2) (hatch_gap 0.6)")));
        assert!((one.hatch_holes - 7812.5).abs() < 1.0e-6);
        let coarse = check_inputs(&board(&zone("\"F.Cu\" \"B.Cu\"", "(mode hatch) (hatch_thickness 0.2) (hatch_gap 1.8)")));
        assert!((coarse.hatch_holes - 2500.0).abs() < 1.0e-6);
    }

    #[test]
    fn the_charge_grows_with_the_holes_up_to_its_cap() {
        let few = check_seconds(CheckInputs { hatched_zones: 1, hatch_holes: 1000.0 });
        let many = check_seconds(CheckInputs { hatched_zones: 1, hatch_holes: 5000.0 });
        assert!(CHECK_SECONDS < few && few < many, "{few} {many}");
        assert_eq!(check_seconds(CheckInputs { hatched_zones: 1, hatch_holes: 1.0e9 }), MAX_CHECK_SECONDS);
    }
}
