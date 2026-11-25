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
    /// Those items, each as its two ends' uuids as KiCad's report names
    /// them (empty for an end without one), in the report's order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unconnected_across_items: Vec<[String; 2]>,
}

/// The board's outlines when it has two or more, with KiCad's items
/// across them (`report`: its drc.json, when it checked).
pub(super) fn of_board(board: &Expr, report: Option<&Value>) -> Option<KiCadBoardOutlines> {
    let loops = crate::outline::board_loops(board).ok()?;
    if loops.piece_loops.len() < 2 {
        return None;
    }
    let items = report.map_or_else(Vec::new, |report| across(report, &loops.piece_loops));
    Some(KiCadBoardOutlines {
        outlines: loops.piece_loops.len(),
        boxes: loops.pieces.clone(),
        unconnected_across: items.len(),
        unconnected_across_items: items,
    })
}

/// Narrow necks of the outline (under 2 mm, no mouse bites or V-cut at
/// them) whose two sides hold no copies of one board, so that the board
/// is one there (`outline_pieces::Pieces::outline_waists`; RS485M's
/// necks between its LEGO holes): KiCad's unconnected items across them
/// are open connections like any other.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct KiCadOutlineWaists {
    pub necks: Vec<KiCadOutlineNeck>,
    /// KiCad's unconnected items whose two ends lie on two sides of a
    /// neck (and on one board).
    pub unconnected_across: usize,
}

/// A neck of `KiCadOutlineWaists`.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct KiCadOutlineNeck {
    /// Its box (`[min x, min y, max x, max y]`, mm).
    pub neck: [f64; 4],
    /// The parts (annotated footprints with a pad on a net) on its two
    /// sides.
    pub parts: [usize; 2],
    /// Of the parts on the side with fewer, those whose reference the
    /// other side has too: more than half would make the sides copies of
    /// one board and the neck a tab between them.
    pub repeated: usize,
}

/// The outline's waists, with KiCad's items across them (`report`: its
/// drc.json, when it checked).
pub(super) fn waists_of_board(board: &Expr, report: Option<&Value>) -> Option<KiCadOutlineWaists> {
    let waists = crate::outline_pieces::Pieces::outline_waists(board)
        .ok()
        .flatten()?;
    let boards = crate::outline_pieces::Pieces::of_board(board)
        .ok()
        .flatten();
    let unconnected_across = report.map_or(0, |report| {
        report["unconnected_items"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|item| item["excluded"] != true)
            .filter(|item| {
                let ends: Vec<[f64; 2]> = item["items"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|end| Some([end["pos"]["x"].as_f64()?, end["pos"]["y"].as_f64()?]))
                    .collect();
                let [a, b] = ends[..] else { return false };
                let sides = (waists.parts.piece_at(a), waists.parts.piece_at(b));
                matches!(sides, (Some(x), Some(y)) if x != y)
                    && boards.as_ref().is_none_or(|boards| {
                        boards.piece_at(a).is_some() && boards.piece_at(a) == boards.piece_at(b)
                    })
            })
            .count()
    });
    let mm = |value: f64| (value * 100.0).round() / 100.0;
    Some(KiCadOutlineWaists {
        necks: waists
            .necks
            .iter()
            .map(|neck| KiCadOutlineNeck {
                neck: neck.neck.map(mm),
                parts: neck.parts,
                repeated: neck.repeated,
            })
            .collect(),
        unconnected_across,
    })
}

impl KiCadOutlineWaists {
    /// The open connections' line for items across the necks, when any.
    pub(super) fn across_action(&self) -> Option<String> {
        (self.unconnected_across > 0).then(|| {
            format!(
                "the outline narrows to under 2 mm at {} neck{} (outline_waists in verdict.json) with no mouse bites or V-cut at {}, and the parts on the two sides are no copies of one board (no reference repeats across), so it is checked and routed as one board: {} of KiCad's unconnected items join copper across the necks and are open connections like any other. If the board file is a panel to break apart there, mark each tab as one (a row of three or more non-plated holes of at most 1 mm drill, at most 1.25 mm apart, or a line on a user layer with a V-CUT text at one end): then each board's nets are its own and the items between boards are not counted (sub_boards)",
                self.necks.len(),
                if self.necks.len() == 1 { "" } else { "s" },
                if self.necks.len() == 1 { "it" } else { "them" },
                self.unconnected_across
            )
        })
    }
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
/// ends lie on two different loops, as their ends' uuids.
fn across(report: &Value, loops: &[Vec<[f64; 2]>]) -> Vec<[String; 2]> {
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
        .map(|item| {
            let uuid = |index: usize| {
                item["items"][index]["uuid"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string()
            };
            [uuid(0), uuid(1)]
        })
        .collect()
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
                {"description": "Zone [GND] on F.Cu", "pos": {"x": a[0], "y": a[1]},
                 "uuid": format!("z{}", a[0])},
                {"description": "Zone [GND] on F.Cu", "pos": {"x": b[0], "y": b[1]},
                 "uuid": format!("z{}", b[0])},
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
        // The items counted, by their ends' uuids.
        assert_eq!(
            outlines.unconnected_across_items,
            vec![
                ["z0.5".to_string(), "z30.5".to_string()],
                ["z30.5".to_string(), "z79.9".to_string()]
            ]
        );
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

    /// Two 20 x 20 squares joined by a 1.5 mm neck (y 20 to 25), a part of
    /// reference `upper` above it and one of `lower` below, both on SIG.
    fn necked(upper: &str, lower: &str) -> Expr {
        let part = |reference: &str, y: f64| {
            format!(
                r#"(footprint "R" (at 5 {y}) (property "Reference" "{reference}") (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "SIG")))"#
            )
        };
        crate::parse(&format!(
            r#"(kicad_pcb (version 20240108) (gr_poly (pts (xy 0 0) (xy 20 0) (xy 20 20) (xy 10.75 20) (xy 10.75 25) (xy 20 25) (xy 20 45) (xy 0 45) (xy 0 25) (xy 9.25 25) (xy 9.25 20) (xy 0 20)) (layer "Edge.Cuts")) {} {})"#,
            part(upper, 5.0),
            part(lower, 35.0)
        ))
        .unwrap()
    }

    #[test]
    fn a_waist_and_the_items_across_it_are_said() {
        // R1 above the neck, R2 below, SIG between them open: one board,
        // and KiCad's item across the neck is an open connection.
        let report = serde_json::json!({"unconnected_items": [
            {"items": [
                {"description": "Pad 1 [SIG] of R1", "pos": {"x": 5.0, "y": 5.0}, "uuid": "a"},
                {"description": "Pad 1 [SIG] of R2", "pos": {"x": 5.0, "y": 35.0}, "uuid": "b"},
            ]},
        ]});
        let waists = waists_of_board(&necked("R1", "R2"), Some(&report)).expect("waists");
        assert_eq!(waists.necks.len(), 1);
        assert_eq!(
            (waists.necks[0].parts, waists.necks[0].repeated),
            ([1, 1], 0)
        );
        assert_eq!(waists.unconnected_across, 1);
        let action = waists.across_action().unwrap();
        assert!(action.contains("1 neck (outline_waists"), "{action}");
        assert!(action.contains("mark each tab"), "{action}");
        // Copies of one board on the two sides: a tab, no waist.
        assert!(waists_of_board(&necked("R1", "R1"), Some(&report)).is_none());
    }
}
