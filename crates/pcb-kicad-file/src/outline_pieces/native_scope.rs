// Copyright (C) 2026 Florian Loitsch.

//! Physical ownership of native KiCad report findings on a panel.

use super::{ACROSS_KEY, CAPPED_KEY, KICAD_REPORT_CAP, Pieces};
use crate::kicad_sexpr::{Expr, form_f64, form_xy};
use pcb_router::geometry::point_segment_distance;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum NativeFindingLocation {
    /// Zero-based index in the source outline's pieces.
    Piece(usize),
    AcrossPieces,
    Unknown,
}

/// Resolves native positions. Zone positions refer to an outline vertex, not
/// the island in the finding. Only a zone confined to one piece has an owner.
pub struct NativePanelScope<'a> {
    pcb: &'a Expr,
    pieces: &'a Pieces,
    zone_pieces: BTreeMap<String, Option<usize>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct NativePanelCounts {
    pub raw_opens: usize,
    pub excluded_across: usize,
    pub retained_across: usize,
    pub unresolved_opens: usize,
    pub opens_may_be_capped: bool,
    pub pieces: Vec<NativePieceCounts>,
    pub errors_across: BTreeMap<String, usize>,
    pub errors_without_piece: BTreeMap<String, usize>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct NativePieceCounts {
    /// One-based physical piece number, as used in the CLI reports.
    pub piece: usize,
    pub opens: usize,
    pub errors: BTreeMap<String, usize>,
}

impl NativePanelCounts {
    /// An additive repair should close a physical open without increasing
    /// opens on any other piece or changing the cross-gap/unresolved totals.
    /// Finding identities and cap checks for violations are separate gates.
    pub fn reduces_within_piece_opens(&self, after: &Self) -> bool {
        !self.opens_may_be_capped
            && !after.opens_may_be_capped
            && self.raw_opens > after.raw_opens
            && self.excluded_across == after.excluded_across
            && self.retained_across == after.retained_across
            && self.unresolved_opens == after.unresolved_opens
            && self.pieces.len() == after.pieces.len()
            && self
                .pieces
                .iter()
                .zip(&after.pieces)
                .all(|(a, b)| a.piece == b.piece && a.opens >= b.opens)
    }

    /// The changed copper lies on one piece. Require every measured
    /// connectivity improvement to be on that piece as well.
    pub fn reduces_opens_on_piece(&self, after: &Self, piece: usize) -> bool {
        self.reduces_within_piece_opens(after)
            && self
                .pieces
                .iter()
                .zip(&after.pieces)
                .enumerate()
                .all(|(index, (a, b))| {
                    if index == piece {
                        a.opens > b.opens
                    } else {
                        a.opens == b.opens
                    }
                })
    }
}

impl Pieces {
    /// The zero-based owner of every added track and via. Includes their
    /// copper widths. A candidate reaching a tab or a different piece is
    /// ineligible for piece-local repair.
    pub fn added_routing_piece(&self, objects: &[Expr]) -> Option<usize> {
        let mut owners = objects.iter().map(|item| {
            let (start, end, width) = match item.head()? {
                "segment" => (
                    form_xy(item, "start").ok()?,
                    form_xy(item, "end").ok()?,
                    form_f64(item, "width", 1).ok()?,
                ),
                "via" => {
                    let at = form_xy(item, "at").ok()?;
                    (at, at, form_f64(item, "size", 1).ok()?)
                }
                _ => return None,
            };
            if !width.is_finite() || width <= 0.0 {
                return None;
            }
            self.capsule_piece(start, end, width / 2.0)
        });
        let piece = owners.next()??;
        owners.all(|owner| owner == Some(piece)).then_some(piece)
    }

    fn capsule_piece(&self, start: [f64; 2], end: [f64; 2], radius: f64) -> Option<usize> {
        let piece = self.piece_at(start)?;
        let low = [start[0].min(end[0]) - radius, start[1].min(end[1]) - radius];
        let high = [start[0].max(end[0]) + radius, start[1].max(end[1]) + radius];
        if self.boxed() {
            let [left, top, right, bottom] = self.boxes[piece];
            return (left <= low[0] && top <= low[1] && high[0] <= right && high[1] <= bottom)
                .then_some(piece);
        }
        let first = self.cell_of(low)?;
        let last = self.cell_of(high)?;
        let reach = radius + self.cell * std::f64::consts::FRAC_1_SQRT_2;
        for y in first / self.columns..=last / self.columns {
            for x in first % self.columns..=last % self.columns {
                if self.labels[y * self.columns + x] as usize == piece {
                    continue;
                }
                let center = [
                    self.origin[0] + (x as f64 + 0.5) * self.cell,
                    self.origin[1] + (y as f64 + 0.5) * self.cell,
                ];
                if point_segment_distance(center, start, end) <= reach {
                    return None;
                }
            }
        }
        Some(piece)
    }
}

impl<'a> NativePanelScope<'a> {
    pub fn new(pcb: &'a Expr, pieces: &'a Pieces) -> Self {
        Self {
            pcb,
            pieces,
            zone_pieces: BTreeMap::new(),
        }
    }

    pub fn item_piece(&mut self, item: &Value) -> Option<usize> {
        let description = item["description"].as_str()?;
        if description.starts_with("Zone ") {
            let uuid = item["uuid"].as_str()?;
            return *self
                .zone_pieces
                .entry(uuid.to_owned())
                .or_insert_with(|| self.pieces.single_zone_piece(self.pcb, uuid));
        }
        let pos = &item["pos"];
        self.pieces
            .piece_at([pos["x"].as_f64()?, pos["y"].as_f64()?])
    }

    pub fn location(&mut self, items: &[Value]) -> NativeFindingLocation {
        let mut first = None;
        let mut unknown = false;
        for item in items {
            let Some(piece) = self.item_piece(item) else {
                unknown = true;
                continue;
            };
            if first.is_some_and(|other| other != piece) {
                return NativeFindingLocation::AcrossPieces;
            }
            first = Some(piece);
        }
        if unknown {
            return NativeFindingLocation::Unknown;
        }
        first.map_or(NativeFindingLocation::Unknown, NativeFindingLocation::Piece)
    }

    pub fn counts(&mut self, report: &Value) -> Option<NativePanelCounts> {
        if report["coordinate_units"] != "mm" {
            return None;
        }
        let opens = report["unconnected_items"].as_array()?;
        let excluded_across = report[ACROSS_KEY].as_array().map_or(0, Vec::len);
        let raw_opens = opens.len() + excluded_across;
        let mut counts = NativePanelCounts {
            raw_opens,
            excluded_across,
            retained_across: 0,
            unresolved_opens: 0,
            opens_may_be_capped: report[CAPPED_KEY] == true || raw_opens >= KICAD_REPORT_CAP,
            pieces: (1..=self.pieces.count())
                .map(|piece| NativePieceCounts {
                    piece,
                    ..Default::default()
                })
                .collect(),
            errors_across: BTreeMap::new(),
            errors_without_piece: BTreeMap::new(),
        };
        for open in opens {
            match open["items"].as_array().map(Vec::as_slice) {
                Some(items @ [_, _]) => match self.location(items) {
                    NativeFindingLocation::Piece(piece) => counts.pieces[piece].opens += 1,
                    NativeFindingLocation::AcrossPieces => counts.retained_across += 1,
                    NativeFindingLocation::Unknown => counts.unresolved_opens += 1,
                },
                _ => counts.unresolved_opens += 1,
            }
        }
        for finding in report["violations"].as_array()? {
            if finding["severity"] != "error" {
                continue;
            }
            let kind = finding["type"].as_str()?;
            let errors = match self.location(finding["items"].as_array()?) {
                NativeFindingLocation::Piece(piece) => &mut counts.pieces[piece].errors,
                NativeFindingLocation::AcrossPieces => &mut counts.errors_across,
                NativeFindingLocation::Unknown => &mut counts.errors_without_piece,
            };
            *errors.entry(kind.to_owned()).or_default() += 1;
        }
        Some(counts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kicad_sexpr::parse;
    use serde_json::json;

    #[test]
    fn additive_copper_keeps_its_width_and_every_object_on_one_piece() {
        let pieces = Pieces::of_loops(
            &[vec![
                [0.0, 0.0],
                [12.0, 0.0],
                [12.0, 4.5],
                [15.0, 4.5],
                [15.0, 0.0],
                [27.0, 0.0],
                [27.0, 10.0],
                [15.0, 10.0],
                [15.0, 5.5],
                [12.0, 5.5],
                [12.0, 10.0],
                [0.0, 10.0],
            ]],
            &[],
            &Default::default(),
        )
        .unwrap();
        let route =
            parse("(route (segment (start 2 2) (end 7 8) (width 0.2)) (via (at 7 8) (size 0.6)))")
                .unwrap();
        assert_eq!(
            pieces.added_routing_piece(&route.children()[1..]),
            pieces.piece_at([7.0, 8.0])
        );
        let spread = parse("(route (via (at 7 8) (size 0.6)) (via (at 17 8) (size 0.6)))").unwrap();
        assert_eq!(pieces.added_routing_piece(&spread.children()[1..]), None);
        let overhang = parse("(route (via (at 11.8 8) (size 0.6)))").unwrap();
        assert_eq!(pieces.added_routing_piece(&overhang.children()[1..]), None);
    }

    #[test]
    fn repair_only_counts_an_open_removed_on_its_piece() {
        let pcb = parse("(kicad_pcb)").unwrap();
        let pieces = Pieces::of_boxes(vec![[0.0, 0.0, 10.0, 10.0], [20.0, 0.0, 30.0, 10.0]]);
        let mut scope = NativePanelScope::new(&pcb, &pieces);
        let item = |x: f64| json!({"description": "Pad", "pos": {"x": x, "y": 5.0}});
        let open = |a, b| json!({"items": [item(a), item(b)]});
        let report = |opens, across| {
            json!({
                "coordinate_units": "mm",
                "violations": [],
                "unconnected_items": opens,
                "unconnected_across_sub_boards": across
            })
        };
        let across = vec![open(3.0, 23.0)];
        let first = open(2.0, 3.0);
        let second = open(22.0, 23.0);
        let before = scope
            .counts(&report(vec![first, second.clone()], across.clone()))
            .unwrap();
        let improved = scope.counts(&report(vec![second.clone()], across)).unwrap();
        let bridged = scope.counts(&report(vec![second], vec![])).unwrap();
        assert!(before.reduces_opens_on_piece(&improved, 0));
        assert!(!before.reduces_opens_on_piece(&improved, 1));
        assert!(!before.reduces_opens_on_piece(&bridged, 0));
    }
}
