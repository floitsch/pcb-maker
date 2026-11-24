// Copyright (C) 2026 Florian Loitsch.

//! A board file that holds several boards: Edge.Cuts loops that do not
//! touch (`outline::BoardLoops::piece_loops`). No copper joins them, so
//! KiCad's unconnected items between two of them are no routing failure,
//! and a pour over several of them is in as many pieces whatever
//! `keep_whole` does (the eval study of 2026-11-24: Hub's three boards,
//! each with a GND zone, drew `keep_whole` advice for the pour).

use super::*;
use crate::kicad_sexpr::Expr;

/// The board's separate outlines and KiCad's unconnected items between two
/// of them.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct KiCadBoardOutlines {
    /// Edge.Cuts loops that do not touch: boards in one file.
    pub outlines: usize,
    /// Each outline's box (`[min x, min y, max x, max y]`, mm), the
    /// largest first.
    pub boxes: Vec<[f64; 4]>,
    /// KiCad's unconnected items whose two ends lie on two different
    /// outlines (each end on the outline that holds its place, else the
    /// nearest): no track or pour can join them.
    pub unconnected_across: usize,
}

/// The board's outlines when it has two or more, with KiCad's items
/// across them (`report`: its drc.json, when it checked).
pub(super) fn of_board(board: &Expr, report: Option<&Value>) -> Option<KiCadBoardOutlines> {
    let loops = crate::outline::board_loops(board).ok()?;
    if loops.piece_loops.len() < 2 {
        return None;
    }
    let unconnected_across = report.map_or(0, |report| across(report, &loops.piece_loops));
    Some(KiCadBoardOutlines {
        outlines: loops.piece_loops.len(),
        boxes: loops.pieces.clone(),
        unconnected_across,
    })
}

/// The loop that holds `point`, else the nearest one.
fn outline_of(loops: &[Vec<[f64; 2]>], point: [f64; 2]) -> Option<usize> {
    if let Some(index) = loops.iter().position(|ring| contains(ring, point)) {
        return Some(index);
    }
    loops
        .iter()
        .enumerate()
        .map(|(index, ring)| (distance_to_ring(ring, point), index))
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, index)| index)
}

fn contains(ring: &[[f64; 2]], point: [f64; 2]) -> bool {
    let mut inside = false;
    let mut j = ring.len().wrapping_sub(1);
    for i in 0..ring.len() {
        let (a, b) = (ring[i], ring[j]);
        if (a[1] > point[1]) != (b[1] > point[1])
            && point[0] < (b[0] - a[0]) * (point[1] - a[1]) / (b[1] - a[1]) + a[0]
        {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn distance_to_ring(ring: &[[f64; 2]], point: [f64; 2]) -> f64 {
    let mut best = f64::INFINITY;
    for i in 0..ring.len() {
        let a = ring[i];
        let b = ring[(i + 1) % ring.len()];
        let d = [b[0] - a[0], b[1] - a[1]];
        let length = d[0] * d[0] + d[1] * d[1];
        let t = if length > 0.0 {
            (((point[0] - a[0]) * d[0] + (point[1] - a[1]) * d[1]) / length).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let nearest = [a[0] + t * d[0], a[1] + t * d[1]];
        best = best.min((point[0] - nearest[0]).hypot(point[1] - nearest[1]));
    }
    best
}

/// KiCad's unconnected items (errors and warnings it counts) whose two
/// ends lie on two different loops.
fn across(report: &Value, loops: &[Vec<[f64; 2]>]) -> usize {
    report["unconnected_items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|item| item["excluded"] != true)
        .filter(|item| {
            let ends: Vec<Option<usize>> = item["items"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|end| {
                    let at = [end["pos"]["x"].as_f64()?, end["pos"]["y"].as_f64()?];
                    outline_of(loops, at)
                })
                .collect();
            matches!(ends[..], [Some(a), Some(b)] if a != b)
        })
        .count()
}

impl KiCadBoardOutlines {
    /// The open connections' line for items across outlines, when any.
    pub(super) fn across_action(&self) -> Option<String> {
        (self.unconnected_across > 0).then(|| {
            format!(
                "the board file holds {} separate board outlines (Edge.Cuts loops that do not touch: board_outlines in verdict.json), and {} of KiCad's unconnected items join copper on two of them, which no track or pour can join: they are no routing failure (each board's copper is its own; the boards join by their connectors or cables), and no change of room, layers or rules closes them",
                self.outlines, self.unconnected_across
            )
        })
    }

    /// What a pour's action says first on such a board.
    pub(super) fn pour_note(&self) -> String {
        format!(
            "the board file holds {} separate board outlines, so a pour over several of them is in at least as many pieces that no copper joins, whatever keep_whole does (keep_whole keeps a piece whole within one outline only); ",
            self.outlines
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board(rects: &[[f64; 4]]) -> Expr {
        let mut text = String::from("(kicad_pcb (version 20240108)");
        for r in rects {
            text.push_str(&format!(
                "(gr_rect (start {} {}) (end {} {}) (layer \"Edge.Cuts\") (width 0.1))",
                r[0], r[1], r[2], r[3]
            ));
        }
        text.push(')');
        crate::parse(&text).unwrap()
    }

    #[test]
    fn three_outlines_and_the_items_between_them_are_counted() {
        // Hub: three boards in one file, a GND zone on each; KiCad names
        // a zone's first corner as its item's place.
        let pcb = board(&[
            [0.0, 0.0, 20.0, 20.0],
            [30.0, 0.0, 50.0, 20.0],
            [60.0, 0.0, 80.0, 20.0],
        ]);
        let item = |a: [f64; 2], b: [f64; 2]| {
            serde_json::json!({"items": [
                {"description": "Zone [GND] on F.Cu", "pos": {"x": a[0], "y": a[1]}},
                {"description": "Zone [GND] on F.Cu", "pos": {"x": b[0], "y": b[1]}},
            ]})
        };
        let report = serde_json::json!({"unconnected_items": [
            item([0.5, 0.5], [30.5, 0.5]),
            item([30.5, 0.5], [79.9, 20.0]),
            item([5.0, 5.0], [15.0, 15.0]),
        ]});
        let outlines = of_board(&pcb, Some(&report)).unwrap();
        assert_eq!(outlines.outlines, 3);
        assert_eq!(outlines.unconnected_across, 2);
        let action = outlines.across_action().unwrap();
        assert!(action.contains("3 separate board outlines"), "{action}");
        assert!(
            action.contains("2 of KiCad's unconnected items"),
            "{action}"
        );
        assert!(
            outlines
                .pour_note()
                .contains("keep_whole keeps a piece whole within one outline only")
        );
        // One board: nothing to say.
        assert!(of_board(&board(&[[0.0, 0.0, 20.0, 20.0]]), Some(&report)).is_none());
    }
}
