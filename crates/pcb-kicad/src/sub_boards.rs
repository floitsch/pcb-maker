// Copyright (C) 2026 Toit contributors.

//! Sub-boards of a panel: a panel of different boards (the OpenESC 4in1
//! panel: a 30 x 30 and a 20 x 20 ESC joined by tabs with mouse bites)
//! shares net names between its boards (GND, +BATT, the ESC sheets' nets),
//! but no copper joins them: KiCad reports one unconnected item per such
//! net on the designer's own board (130 on the 4in1 panel), and after the
//! tabs are broken they are different nets. Placed and routed as one
//! board, the parts interleave and the router tries to join the same-named
//! nets across the tabs (docs/placer.md, "Sub-boards in a panel
//! (2026-10-27)").
//!
//! - `Pieces`: the outline's pieces without its tabs: the board area
//!   rasterised, opened by `NECK_RADIUS_MM` (eroded and grown back), so
//!   that necks narrower than twice that (mouse bites, thin tabs) fall
//!   away; each part of the opened area large enough is a piece.
//! - `split_router_nets`: the router's net split: a net whose pads lie on
//!   two or more pieces, with none of its copper (pads, tracks, vias) on a
//!   tab or spanning two pieces, is one net per piece (its pours clipped
//!   to each piece).
//! - `excuse_across`: KiCad's check judged per sub-board: an unconnected
//!   item whose two items lie on two pieces of a net split so is moved out
//!   of the counted list (`unconnected_across_sub_boards`).
//! - `detect`: which sub-board each part belongs to, for the placer: from
//!   the parts that stay where they are, the reference board's places
//!   (`--reference`), and the nets no evidence splits.

use super::*;
use crate::kicad_sexpr::{Expr, form_at};
use pcb_router as core;

/// Necks narrower than twice this are no part of a piece (mm): a mouse
/// bite's bridges (0.3 mm between 0.6 mm holes) and tabs narrower than
/// 2 mm fall away; boards are far wider.
pub(crate) const NECK_RADIUS_MM: f64 = 1.0;

/// The least area of a piece's core (mm², the area left after the
/// erosion): what remains of a tab between two rows of mouse bites (1 x
/// 5 mm on the 4in1 panel) is no piece.
const MIN_CORE_MM2: f64 = 20.0;

/// The least core of a piece as a share of the largest one's.
const MIN_PIECE_SHARE: f64 = 0.05;

/// The keepouts' blocks (mm, `Pieces::keepouts`).
const KEEPOUT_BLOCK_MM: f64 = 0.5;

/// How far parts keep from a tab beyond the board the opening took from
/// it (mm): a row of mouse bites breaks along its holes, its edge rough
/// (the keepout then reaches about 1.6 mm from the row's centre line,
/// 1.4 mm from 0.5 mm holes' edges).
pub(crate) const TAB_MARGIN_MM: f64 = 1.5;

/// Frames and rails together take at most this share of the area of the
/// pieces parts lie on (the ULK panels' frames: about a sixth).
const MOST_FRAME_SHARE: f64 = 0.5;

/// No piece: the cell's label where the opened area does not reach.
const NONE: u16 = u16::MAX;

/// A mouse bite's hole: a non-plated drill of at most this diameter (mm;
/// KiKit's and the Panelization library's 0.5 mm, 0.4 to 0.6 mm
/// usually).
const MOUSE_BITE_DRILL_MM: f64 = 1.0;

/// Two holes of a row of mouse bites lie at most this far apart, centre
/// to centre (mm; 0.6 to 1.19 mm on the sweep's panels, 0.75 mm on the
/// ULK's). Wider gaps carry tracks: protoesp's breakaway modules keep
/// 1.5 mm between some holes for signals that cross the tab, and stay
/// one board.
const MOUSE_BITE_PITCH_MM: f64 = 1.25;

/// A row of mouse bites has at least this many holes.
const MOUSE_BITE_HOLES: usize = 3;

/// A V-cut's label lies at most this far from its line's extension and
/// this far beyond one of its ends (mm; KiKit writes "V-CUT" 3 mm before
/// the line's start, on its axis).
const V_CUT_LABEL_OFF_AXIS_MM: f64 = 2.0;
const V_CUT_LABEL_REACH_MM: f64 = 10.0;

/// What cuts a board's outline into pieces besides its own narrowing:
/// rows of mouse bites drilled as separate holes (non-plated pads of
/// footprints with no other pad), and V-cut lines (a graphic line on a
/// user layer with a "V-CUT" or "V-SCORE" label at one end).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Cuts {
    /// The mouse bites' holes: centre and radius.
    pub holes: Vec<([f64; 2], f64)>,
    /// The V-cut lines.
    pub lines: Vec<[[f64; 2]; 2]>,
    /// How many rows of mouse bites the holes make.
    pub rows: usize,
}

impl Cuts {
    /// The mouse bites and V-cuts of a KiCad board (`Cuts`).
    pub(crate) fn of_board(pcb: &Expr) -> Cuts {
        let mut holes: Vec<([f64; 2], f64)> = Vec::new();
        for footprint in pcb
            .children()
            .iter()
            .filter(|item| item.head() == Some("footprint"))
        {
            let pads: Vec<&Expr> = footprint
                .children()
                .iter()
                .filter(|child| child.head() == Some("pad"))
                .collect();
            if pads.is_empty()
                || pads
                    .iter()
                    .any(|pad| pad.children().get(2).and_then(Expr::atom) != Some("np_thru_hole"))
            {
                continue;
            }
            let Ok(at) = form_at(footprint) else { continue };
            for pad in pads {
                // A round drill (`(drill 0.5)`, not `(drill oval ...)`).
                let Some(drill) =
                    form_atom(pad, "drill", 1).and_then(|text| text.parse::<f64>().ok())
                else {
                    continue;
                };
                if !(drill > 0.0 && drill <= MOUSE_BITE_DRILL_MM) {
                    continue;
                }
                let Ok(local) = form_at(pad) else { continue };
                let offset = crate::geometry::rotate_vector([local[0], local[1]], -at[2]);
                holes.push(([at[0] + offset[0], at[1] + offset[1]], drill / 2.0));
            }
        }
        // Rows: holes chained by the pitch, three or more.
        let mut parent: Vec<usize> = (0..holes.len()).collect();
        fn root(parent: &mut [usize], mut at: usize) -> usize {
            while parent[at] != at {
                parent[at] = parent[parent[at]];
                at = parent[at];
            }
            at
        }
        let mut order: Vec<usize> = (0..holes.len()).collect();
        order.sort_by(|a, b| holes[*a].0[0].total_cmp(&holes[*b].0[0]));
        for (index, &a) in order.iter().enumerate() {
            for &b in &order[index + 1..] {
                if holes[b].0[0] - holes[a].0[0] > MOUSE_BITE_PITCH_MM {
                    break;
                }
                let (pa, pb) = (holes[a].0, holes[b].0);
                if (pa[0] - pb[0]).hypot(pa[1] - pb[1]) <= MOUSE_BITE_PITCH_MM {
                    let (ra, rb) = (root(&mut parent, a), root(&mut parent, b));
                    parent[ra] = rb;
                }
            }
        }
        let mut sizes: BTreeMap<usize, usize> = BTreeMap::new();
        for index in 0..holes.len() {
            *sizes.entry(root(&mut parent, index)).or_default() += 1;
        }
        let in_row: Vec<bool> = (0..holes.len())
            .map(|index| sizes[&root(&mut parent, index)] >= MOUSE_BITE_HOLES)
            .collect();
        let rows = sizes
            .values()
            .filter(|count| **count >= MOUSE_BITE_HOLES)
            .count();
        let holes: Vec<([f64; 2], f64)> = holes
            .into_iter()
            .zip(in_row)
            .filter_map(|(hole, keep)| keep.then_some(hole))
            .collect();
        Cuts {
            holes,
            lines: v_cut_lines(pcb),
            rows,
        }
    }
}

/// Whether a layer may carry a V-cut's drawing: a user layer (not copper,
/// not the outline, the mask, the paste or the silk).
fn v_cut_layer(layer: &str) -> bool {
    layer.ends_with(".User") || layer.starts_with("User.")
}

/// Whether a text labels a V-cut.
fn v_cut_label(text: &str) -> bool {
    let plain: String = text
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_uppercase();
    matches!(plain.as_str(), "VCUT" | "VSCORE" | "VGROOVE" | "VSCORING")
}

/// The V-cut lines of a board: graphic lines on a user layer with a V-cut
/// label on the same layer on their axis near one of their ends.
fn v_cut_lines(pcb: &Expr) -> Vec<[[f64; 2]; 2]> {
    let mut labels: Vec<(&str, [f64; 2])> = Vec::new();
    for item in pcb.children() {
        if item.head() == Some("gr_text")
            && let Some(text) = item.children().get(1).and_then(Expr::atom)
            && v_cut_label(text)
            && let Some(layer) = form_atom(item, "layer", 1)
            && let Ok(at) = form_at(item)
        {
            labels.push((layer, [at[0], at[1]]));
        }
    }
    if labels.is_empty() {
        return Vec::new();
    }
    let mut lines = Vec::new();
    for item in pcb.children() {
        if item.head() != Some("gr_line") {
            continue;
        }
        let Some(layer) = form_atom(item, "layer", 1) else {
            continue;
        };
        if !v_cut_layer(layer) {
            continue;
        }
        let (Ok(a), Ok(b)) = (
            crate::kicad_sexpr::form_xy(item, "start"),
            crate::kicad_sexpr::form_xy(item, "end"),
        ) else {
            continue;
        };
        let length = (b[0] - a[0]).hypot(b[1] - a[1]);
        if length <= 0.0 {
            continue;
        }
        let direction = [(b[0] - a[0]) / length, (b[1] - a[1]) / length];
        let labelled = labels.iter().any(|(label_layer, at)| {
            let relative = [at[0] - a[0], at[1] - a[1]];
            let along = relative[0] * direction[0] + relative[1] * direction[1];
            let across = (relative[0] * direction[1] - relative[1] * direction[0]).abs();
            *label_layer == layer
                && across <= V_CUT_LABEL_OFF_AXIS_MM
                && along >= -V_CUT_LABEL_REACH_MM
                && along <= length + V_CUT_LABEL_REACH_MM
                && (along <= V_CUT_LABEL_REACH_MM.min(length / 2.0)
                    || along >= length - V_CUT_LABEL_REACH_MM.min(length / 2.0))
        });
        if labelled {
            lines.push([a, b]);
        }
    }
    lines
}

/// The outline's pieces without its tabs (module comment): a raster of
/// the board, each cell labelled with its piece.
#[derive(Clone, Debug)]
pub(crate) struct Pieces {
    origin: [f64; 2],
    cell: f64,
    columns: usize,
    rows: usize,
    /// Per cell (row-major): the piece, `NONE` off every piece.
    labels: Vec<u16>,
    /// Per cell: inside the board.
    inside: Vec<bool>,
    /// Per piece, the box of its cells.
    pub boxes: Vec<[f64; 4]>,
    /// The boxes of the board's area on no piece that touches two pieces
    /// or more (the tabs).
    pub tabs: Vec<[f64; 4]>,
    /// Per piece, its area (mm²).
    pub areas: Vec<f64>,
    /// What cut the outline into its pieces: `narrow outline` (a neck of
    /// the outline under 2 mm), `mouse bites` (a row of holes across a
    /// wider bridge), `V-cuts`.
    pub cut_by: Vec<&'static str>,
}

impl Pieces {
    /// The pieces of a board's outline, or None where it has fewer than
    /// two.
    pub(crate) fn of_board(pcb: &Expr) -> Result<Option<Pieces>, String> {
        let loops = outline::board_loops(pcb)?;
        Ok(Pieces::of_loops(
            &loops.piece_loops,
            &loops.cutouts,
            &Cuts::of_board(pcb),
        ))
    }

    /// The pieces of the area inside `loops` (even-odd with `cutouts`),
    /// less the mouse bites' holes and the V-cut lines of `cuts`.
    pub(crate) fn of_loops(
        loops: &[Vec<[f64; 2]>],
        cutouts: &[Vec<[f64; 2]>],
        cuts: &Cuts,
    ) -> Option<Pieces> {
        let mut all = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for point in loops.iter().flatten() {
            all = [
                all[0].min(point[0]),
                all[1].min(point[1]),
                all[2].max(point[0]),
                all[3].max(point[1]),
            ];
        }
        let extent = (all[2] - all[0]).max(all[3] - all[1]);
        if !extent.is_finite() || extent <= 0.0 {
            return None;
        }
        // About 1500 cells along the longer side, 0.05 to 0.25 mm each.
        let cell = (extent / 1500.0).clamp(0.05, 0.25);
        let margin = 2.0 * cell;
        let origin = [all[0] - margin, all[1] - margin];
        let columns = ((all[2] - all[0] + 2.0 * margin) / cell).ceil() as usize + 1;
        let rows = ((all[3] - all[1] + 2.0 * margin) / cell).ceil() as usize + 1;
        // Inside: even-odd over every loop, row by row.
        let mut inside = vec![false; columns * rows];
        let mut crossings: Vec<f64> = Vec::new();
        for row in 0..rows {
            let y = origin[1] + (row as f64 + 0.5) * cell;
            crossings.clear();
            for polygon in loops.iter().chain(cutouts) {
                for index in 0..polygon.len() {
                    let (a, b) = (polygon[index], polygon[(index + 1) % polygon.len()]);
                    if (a[1] <= y) != (b[1] <= y) {
                        crossings.push(a[0] + (y - a[1]) / (b[1] - a[1]) * (b[0] - a[0]));
                    }
                }
            }
            crossings.sort_by(f64::total_cmp);
            for pair in crossings.chunks(2) {
                let [start, end] = pair else { break };
                let first = ((start - origin[0]) / cell - 0.5).ceil().max(0.0) as usize;
                let last = ((end - origin[0]) / cell - 0.5).floor();
                if last < 0.0 {
                    continue;
                }
                for column in first..=(last as usize).min(columns - 1) {
                    inside[row * columns + column] = true;
                }
            }
        }
        // The cuts: a mouse bite's hole and a V-cut line are no board.
        let mut cut = vec![false; columns * rows];
        let cell_range = |low: f64, high: f64, origin: f64, count: usize| {
            let first = ((low - origin) / cell - 0.5).ceil().max(0.0) as usize;
            let last = ((high - origin) / cell - 0.5).floor();
            (
                first,
                if last < 0.0 {
                    None
                } else {
                    Some((last as usize).min(count - 1))
                },
            )
        };
        for (centre, radius) in &cuts.holes {
            let (first_row, last_row) =
                cell_range(centre[1] - radius, centre[1] + radius, origin[1], rows);
            let (first_column, last_column) =
                cell_range(centre[0] - radius, centre[0] + radius, origin[0], columns);
            let (Some(last_row), Some(last_column)) = (last_row, last_column) else {
                continue;
            };
            for row in first_row..=last_row {
                let y = origin[1] + (row as f64 + 0.5) * cell;
                for column in first_column..=last_column {
                    let x = origin[0] + (column as f64 + 0.5) * cell;
                    if (x - centre[0]).hypot(y - centre[1]) <= *radius {
                        cut[row * columns + column] = true;
                    }
                }
            }
        }
        // A V-cut line as the cells its centre line passes: board on no
        // piece (a tab along the whole line), which the erosion starts
        // from as from the outside.
        let mut scored = vec![false; columns * rows];
        for [a, b] in &cuts.lines {
            let length = (b[0] - a[0]).hypot(b[1] - a[1]);
            let steps = (length / (cell / 4.0)).ceil().max(1.0) as usize;
            for step in 0..=steps {
                let t = step as f64 / steps as f64;
                let point = [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])];
                let column = ((point[0] - origin[0]) / cell).floor();
                let row = ((point[1] - origin[1]) / cell).floor();
                if column < 0.0 || row < 0.0 {
                    continue;
                }
                let (column, row) = (column as usize, row as usize);
                if column < columns && row < rows {
                    scored[row * columns + column] = true;
                }
            }
        }
        for (inside, cut) in inside.iter_mut().zip(&cut) {
            *inside &= !cut;
        }
        // Distance to the outside or a V-cut (chamfer 3-4, in thirds of a
        // cell).
        let far = u32::MAX / 2;
        let mut distance: Vec<u32> = inside
            .iter()
            .zip(&scored)
            .map(|(&i, &s)| if i && !s { far } else { 0 })
            .collect();
        chamfer(&mut distance, columns, rows);
        let radius = (NECK_RADIUS_MM / cell * 3.0).round() as u32;
        // The cores: what the erosion leaves, by 4-connected component.
        let mut core_label = vec![NONE; columns * rows];
        let mut cores: Vec<usize> = Vec::new();
        let mut stack = Vec::new();
        for start in 0..columns * rows {
            if distance[start] <= radius || core_label[start] != NONE {
                continue;
            }
            let label = cores.len() as u16;
            let mut count = 0usize;
            core_label[start] = label;
            stack.push(start);
            while let Some(at) = stack.pop() {
                count += 1;
                let (row, column) = (at / columns, at % columns);
                let mut visit = |next: usize| {
                    if distance[next] > radius && core_label[next] == NONE {
                        core_label[next] = label;
                        stack.push(next);
                    }
                };
                if column > 0 {
                    visit(at - 1);
                }
                if column + 1 < columns {
                    visit(at + 1);
                }
                if row > 0 {
                    visit(at - columns);
                }
                if row + 1 < rows {
                    visit(at + columns);
                }
            }
            cores.push(count);
            if cores.len() >= NONE as usize - 1 {
                return None;
            }
        }
        // Cores large enough are pieces, renumbered: a twentieth of the
        // largest at least (a strip a row of slots sets off, SNSP-CPU-01's
        // 15 x 16 mm beside its 216 x 159 mm, is part of its board).
        let largest = cores.iter().copied().max().unwrap_or(0) as f64 * cell * cell;
        let mut piece_of_core = vec![NONE; cores.len()];
        let mut pieces = 0u16;
        for (core, count) in cores.iter().enumerate() {
            let area = *count as f64 * cell * cell;
            if area >= MIN_CORE_MM2 && area >= MIN_PIECE_SHARE * largest {
                piece_of_core[core] = pieces;
                pieces += 1;
            }
        }
        if pieces < 2 {
            return None;
        }
        // Grown back: each inside cell within the radius of a piece's core
        // takes that core's label (the nearest one's).
        let mut grown: Vec<u32> = vec![far; columns * rows];
        let mut labels = vec![NONE; columns * rows];
        for index in 0..columns * rows {
            if core_label[index] != NONE && piece_of_core[core_label[index] as usize] != NONE {
                grown[index] = 0;
                labels[index] = piece_of_core[core_label[index] as usize];
            }
        }
        chamfer_labels(&mut grown, &mut labels, columns, rows);
        for index in 0..columns * rows {
            if !inside[index] || grown[index] > radius + 3 || scored[index] {
                labels[index] = NONE;
            }
        }
        let mut boxes = vec![
            [
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ];
            pieces as usize
        ];
        for (index, label) in labels.iter().enumerate() {
            if *label != NONE {
                let (row, column) = (index / columns, index % columns);
                let (x, y) = (
                    origin[0] + column as f64 * cell,
                    origin[1] + row as f64 * cell,
                );
                let b = &mut boxes[*label as usize];
                *b = [
                    b[0].min(x),
                    b[1].min(y),
                    b[2].max(x + cell),
                    b[3].max(y + cell),
                ];
            }
        }
        // The tabs: components of the inside cells on no piece that touch
        // two pieces.
        let mut seen = vec![false; columns * rows];
        let mut tabs = Vec::new();
        for start in 0..columns * rows {
            if !inside[start] || labels[start] != NONE || seen[start] {
                continue;
            }
            let mut touching: BTreeSet<u16> = BTreeSet::new();
            let mut b = [usize::MAX, usize::MAX, 0, 0];
            let mut members = Vec::new();
            seen[start] = true;
            stack.push(start);
            while let Some(at) = stack.pop() {
                members.push(at);
                let (row, column) = (at / columns, at % columns);
                b = [
                    b[0].min(column),
                    b[1].min(row),
                    b[2].max(column),
                    b[3].max(row),
                ];
                let mut neighbours = Vec::with_capacity(4);
                if column > 0 {
                    neighbours.push(at - 1);
                }
                if column + 1 < columns {
                    neighbours.push(at + 1);
                }
                if row > 0 {
                    neighbours.push(at - columns);
                }
                if row + 1 < rows {
                    neighbours.push(at + columns);
                }
                for next in neighbours {
                    if labels[next] != NONE {
                        touching.insert(labels[next]);
                    } else if inside[next] && !seen[next] {
                        seen[next] = true;
                        stack.push(next);
                    }
                }
            }
            // What the opening took from one piece alone (a corner, a
            // strip a row of slots sets off) stays that piece's.
            if touching.len() == 1 {
                let piece = *touching.iter().next().expect("one piece");
                for at in members {
                    labels[at] = piece;
                }
            } else if touching.len() >= 2 {
                tabs.push([
                    origin[0] + b[0] as f64 * cell,
                    origin[1] + b[1] as f64 * cell,
                    origin[0] + (b[2] + 1) as f64 * cell,
                    origin[1] + (b[3] + 1) as f64 * cell,
                ]);
            }
        }
        // The pieces of a row of mouse bites (between the holes, at its
        // ends) are one tab: boxes within the neck radius merge.
        let near = |a: &[f64; 4], b: &[f64; 4]| {
            let gap = NECK_RADIUS_MM;
            a[0] <= b[2] + gap && b[0] <= a[2] + gap && a[1] <= b[3] + gap && b[1] <= a[3] + gap
        };
        let mut merged: Vec<[f64; 4]> = Vec::new();
        for tab in tabs {
            let mut tab = tab;
            while let Some(index) = merged.iter().position(|other| near(other, &tab)) {
                let other = merged.remove(index);
                tab = [
                    tab[0].min(other[0]),
                    tab[1].min(other[1]),
                    tab[2].max(other[2]),
                    tab[3].max(other[3]),
                ];
            }
            merged.push(tab);
        }
        let tabs = merged;
        // What cut each tab: a hole or a V-cut within the neck radius of
        // its box, else the outline's own narrowing.
        let mut kinds: BTreeSet<usize> = BTreeSet::new();
        for tab in &tabs {
            let grown = NECK_RADIUS_MM;
            let (first_row, last_row) = cell_range(tab[1] - grown, tab[3] + grown, origin[1], rows);
            let (first_column, last_column) =
                cell_range(tab[0] - grown, tab[2] + grown, origin[0], columns);
            let mut kind = 0;
            if let (Some(last_row), Some(last_column)) = (last_row, last_column) {
                'scan: for row in first_row..=last_row {
                    for column in first_column..=last_column {
                        let at = row * columns + column;
                        if cut[at] {
                            kind = 1;
                            break 'scan;
                        }
                        if scored[at] {
                            kind = 2;
                        }
                    }
                }
            }
            kinds.insert(kind);
        }
        let cut_by: Vec<&'static str> = kinds
            .iter()
            .map(|kind| ["narrow outline", "mouse bites", "V-cuts"][*kind])
            .collect();
        let mut areas = vec![0.0; pieces as usize];
        for label in &labels {
            if *label != NONE {
                areas[*label as usize] += cell * cell;
            }
        }
        // Pieces only tabs join: a board drawn as separate loops (a module
        // beside its main board, zpn_devboard; the Modular Hub's three
        // boards) is left to `keep_on_pieces` (the Hub with its GND split
        // three ways and its two items across them excused: 19 to 22
        // KiCad unconnected at seed 1, its placement moved by the measured
        // escape correction's route).
        if tabs.is_empty() {
            return None;
        }
        Some(Pieces {
            origin,
            cell,
            columns,
            rows,
            labels,
            inside,
            boxes,
            tabs,
            areas,
            cut_by,
        })
    }

    pub(crate) fn count(&self) -> usize {
        self.boxes.len()
    }

    /// Pieces that are boxes on a board of one outline piece: a panel's
    /// cells the placement found (`panel_copies` told apart by repeated
    /// references: the PolyKybd molecules' cells side by side, cut apart
    /// along straight lines, no neck between them). A point lies on the
    /// first box that holds it; there are no tabs.
    pub(crate) fn of_boxes(boxes: Vec<[f64; 4]>) -> Pieces {
        Pieces {
            origin: [0.0, 0.0],
            cell: 1.0,
            columns: 0,
            rows: 0,
            labels: Vec::new(),
            inside: Vec::new(),
            boxes,
            tabs: Vec::new(),
            areas: Vec::new(),
            cut_by: Vec::new(),
        }
    }

    /// Whether the pieces are boxes (`of_boxes`).
    pub(crate) fn boxed(&self) -> bool {
        self.columns == 0
    }

    fn cell_of(&self, point: [f64; 2]) -> Option<usize> {
        let column = ((point[0] - self.origin[0]) / self.cell).floor();
        let row = ((point[1] - self.origin[1]) / self.cell).floor();
        if column < 0.0 || row < 0.0 {
            return None;
        }
        let (column, row) = (column as usize, row as usize);
        (column < self.columns && row < self.rows).then_some(row * self.columns + column)
    }

    /// The piece `point` lies on; None off the board, on a tab or a neck.
    pub(crate) fn piece_at(&self, point: [f64; 2]) -> Option<usize> {
        if self.boxed() {
            return self.boxes.iter().position(|b| {
                point[0] >= b[0] && point[0] <= b[2] && point[1] >= b[1] && point[1] <= b[3]
            });
        }
        let cell = self.cell_of(point)?;
        (self.labels[cell] != NONE).then_some(self.labels[cell] as usize)
    }

    /// The piece `point` lies on or, off the board, the piece whose box is
    /// nearest within `reach` mm and half a millimetre nearer than any
    /// other's (a zone's corner beyond the edge, where KiCad places a
    /// zone's unconnected item); None on a tab.
    pub(crate) fn piece_near(&self, point: [f64; 2], reach: f64) -> Option<usize> {
        if let Some(piece) = self.piece_at(point) {
            return Some(piece);
        }
        if self.between(point) {
            return None;
        }
        // The nearest piece's box, where no other's is about as near.
        let mut distances: Vec<(f64, usize)> = self
            .boxes
            .iter()
            .enumerate()
            .map(|(piece, b)| {
                let dx = (b[0] - point[0]).max(point[0] - b[2]).max(0.0);
                let dy = (b[1] - point[1]).max(point[1] - b[3]).max(0.0);
                ((dx * dx + dy * dy).sqrt(), piece)
            })
            .collect();
        distances.sort_by(|a, b| a.0.total_cmp(&b.0));
        match distances.as_slice() {
            [(nearest, piece), rest @ ..]
                if *nearest <= reach
                    && rest.first().is_none_or(|(next, _)| next - nearest > 0.5) =>
            {
                Some(*piece)
            }
            _ => None,
        }
    }

    /// Whether `point` lies on the board but on no piece (a tab).
    pub(crate) fn between(&self, point: [f64; 2]) -> bool {
        self.cell_of(point)
            .is_some_and(|cell| self.inside[cell] && self.labels[cell] == NONE)
    }

    /// Where a box lies: `Some(Some(piece))` wholly on one piece (its
    /// corners and centre), `Some(None)` off the board, None on a tab or
    /// over two pieces.
    pub(crate) fn piece_of_box(&self, b: [f64; 4]) -> Option<Option<usize>> {
        let points = [
            [(b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0],
            [b[0], b[1]],
            [b[2], b[1]],
            [b[2], b[3]],
            [b[0], b[3]],
        ];
        let mut piece = None;
        let mut on_board = false;
        for point in points {
            if self.between(point) {
                return None;
            }
            if let Some(here) = self.piece_at(point) {
                on_board = true;
                match piece {
                    None => piece = Some(here),
                    Some(other) if other != here => return None,
                    _ => {}
                }
            }
        }
        Some(if on_board { piece } else { None })
    }

    /// The area no part may take, as boxes: the tabs (the board on no
    /// piece) and the pieces `frames` marks, grown by `margin` into the
    /// pieces beside them, in blocks of `KEEPOUT_BLOCK_MM` (a block any
    /// such cell of the board reaches is kept out whole), the blocks of
    /// one row joined into runs and equal runs of consecutive rows into
    /// one box. None on pieces that are boxes (`of_boxes`).
    pub(crate) fn keepouts(&self, frames: &[bool], margin: f64) -> Vec<[f64; 4]> {
        if self.boxed() {
            return Vec::new();
        }
        let (columns, rows, cell) = (self.columns, self.rows, self.cell);
        let far = u32::MAX / 2;
        let mut distance: Vec<u32> = (0..columns * rows)
            .map(|at| {
                let label = self.labels[at];
                let kept = self.inside[at]
                    && (label == NONE || frames.get(label as usize).copied().unwrap_or(false));
                if kept { 0 } else { far }
            })
            .collect();
        chamfer(&mut distance, columns, rows);
        let reach = (margin.max(0.0) / cell * 3.0).round() as u32;
        let per = ((KEEPOUT_BLOCK_MM / cell).round() as usize).max(1);
        let (block_columns, block_rows) = (columns.div_ceil(per), rows.div_ceil(per));
        let mut blocked = vec![false; block_columns * block_rows];
        for (at, near) in distance.iter().enumerate() {
            if self.inside[at] && *near <= reach {
                let (row, column) = (at / columns, at % columns);
                blocked[(row / per) * block_columns + column / per] = true;
            }
        }
        let size = per as f64 * cell;
        let mut boxes = Vec::new();
        // Open boxes: (first block column, last, first block row).
        let mut open: Vec<(usize, usize, usize)> = Vec::new();
        for row in 0..=block_rows {
            let mut runs: Vec<(usize, usize)> = Vec::new();
            if row < block_rows {
                let mut column = 0;
                while column < block_columns {
                    if blocked[row * block_columns + column] {
                        let first = column;
                        while column + 1 < block_columns
                            && blocked[row * block_columns + column + 1]
                        {
                            column += 1;
                        }
                        runs.push((first, column));
                    }
                    column += 1;
                }
            }
            let mut still = Vec::new();
            for (first, last, top) in open.drain(..) {
                if let Some(index) = runs.iter().position(|run| *run == (first, last)) {
                    runs.remove(index);
                    still.push((first, last, top));
                } else {
                    boxes.push([
                        self.origin[0] + first as f64 * size,
                        self.origin[1] + top as f64 * size,
                        self.origin[0] + (last + 1) as f64 * size,
                        self.origin[1] + row as f64 * size,
                    ]);
                }
            }
            still.extend(runs.into_iter().map(|(first, last)| (first, last, row)));
            open = still;
        }
        boxes
    }

    /// The pieces that are a panel's frame or rails: none of `occupied`
    /// (the pieces parts lie on) where at least one is, and together at
    /// most `MOST_FRAME_SHARE` of the occupied pieces' area (else a
    /// board whose parts are all movable would lose its room: nothing is
    /// a frame).
    pub(crate) fn frames(&self, occupied: &BTreeSet<usize>) -> Vec<bool> {
        let none = vec![false; self.count()];
        if occupied.is_empty() || self.boxed() {
            return none;
        }
        let frames: Vec<bool> = (0..self.count())
            .map(|piece| !occupied.contains(&piece))
            .collect();
        let area = |frame: bool| -> f64 {
            frames
                .iter()
                .zip(&self.areas)
                .filter(|(is, _)| **is == frame)
                .map(|(_, area)| area)
                .sum()
        };
        if area(true) > MOST_FRAME_SHARE * area(false) {
            return none;
        }
        frames
    }

    /// Whether two pieces' boxes overlap (an L-shaped board beside
    /// another): a pour cut to one box would reach into the other.
    pub(crate) fn boxes_overlap(&self) -> bool {
        for (index, a) in self.boxes.iter().enumerate() {
            for b in &self.boxes[index + 1..] {
                if a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3] {
                    return true;
                }
            }
        }
        false
    }
}

/// The chamfer distance (3 along a side, 4 across a corner) from the
/// zero cells, in place.
fn chamfer(distance: &mut [u32], columns: usize, rows: usize) {
    let mut labels = vec![0u16; distance.len()];
    chamfer_labels(distance, &mut labels, columns, rows);
}

/// `chamfer` that carries each source's label along to the cells nearest
/// it.
fn chamfer_labels(distance: &mut [u32], labels: &mut [u16], columns: usize, rows: usize) {
    let relax = |distance: &mut [u32], labels: &mut [u16], at: usize, from: usize, step: u32| {
        let through = distance[from].saturating_add(step);
        if through < distance[at] {
            distance[at] = through;
            labels[at] = labels[from];
        }
    };
    for row in 0..rows {
        for column in 0..columns {
            let at = row * columns + column;
            if column > 0 {
                relax(distance, labels, at, at - 1, 3);
            }
            if row > 0 {
                relax(distance, labels, at, at - columns, 3);
                if column > 0 {
                    relax(distance, labels, at, at - columns - 1, 4);
                }
                if column + 1 < columns {
                    relax(distance, labels, at, at - columns + 1, 4);
                }
            }
        }
    }
    for row in (0..rows).rev() {
        for column in (0..columns).rev() {
            let at = row * columns + column;
            if column + 1 < columns {
                relax(distance, labels, at, at + 1, 3);
            }
            if row + 1 < rows {
                relax(distance, labels, at, at + columns, 3);
                if column + 1 < columns {
                    relax(distance, labels, at, at + columns + 1, 4);
                }
                if column > 0 {
                    relax(distance, labels, at, at + columns - 1, 4);
                }
            }
        }
    }
}

/// Where a terminal of the router's board lies: its anchor's piece, else
/// (an anchor off the board: a pad overhanging the edge) its pad's box's.
/// `Err(())` on a tab or over two pieces.
fn terminal_piece(
    board: &core::Board,
    terminal: &core::Terminal,
    pieces: &Pieces,
) -> Result<Option<usize>, ()> {
    if pieces.boxed() {
        // A panel's cells: a part outside every cell (a test point of an
        // outer row) belongs to the nearest.
        return Ok(pieces.piece_near(terminal.anchor, ZONE_REACH_MM));
    }
    if let Some(piece) = pieces.piece_at(terminal.anchor) {
        return Ok(Some(piece));
    }
    if pieces.between(terminal.anchor) {
        return Err(());
    }
    match board.obstacles.get(terminal.pad) {
        Some(obstacle) => pieces.piece_of_box(aabb_box(&obstacle.shape)).ok_or(()),
        None => Ok(None),
    }
}

fn aabb_box(shape: &core::Shape) -> [f64; 4] {
    let b = shape.aabb();
    [b.minimum[0], b.minimum[1], b.maximum[0], b.maximum[1]]
}

fn polygon_box(points: &[[f64; 2]]) -> [f64; 4] {
    points.iter().fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |b, p| {
            [
                b[0].min(p[0]),
                b[1].min(p[1]),
                b[2].max(p[0]),
                b[3].max(p[1]),
            ]
        },
    )
}

/// `polygon` clipped to `region` (Sutherland and Hodgman).
fn clip(polygon: &[[f64; 2]], region: [f64; 4]) -> Vec<[f64; 2]> {
    let mut points = polygon.to_vec();
    for (axis, bound, keep_below) in [
        (0, region[0], false),
        (0, region[2], true),
        (1, region[1], false),
        (1, region[3], true),
    ] {
        if points.is_empty() {
            break;
        }
        let inside = |point: &[f64; 2]| {
            if keep_below {
                point[axis] <= bound
            } else {
                point[axis] >= bound
            }
        };
        let crossing = |a: [f64; 2], b: [f64; 2]| {
            let t = (bound - a[axis]) / (b[axis] - a[axis]);
            let mut point = [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])];
            point[axis] = bound;
            point
        };
        let mut clipped = Vec::new();
        for index in 0..points.len() {
            let (current, next) = (points[index], points[(index + 1) % points.len()]);
            match (inside(&current), inside(&next)) {
                (true, true) => clipped.push(next),
                (true, false) => clipped.push(crossing(current, next)),
                (false, true) => {
                    clipped.push(crossing(current, next));
                    clipped.push(next);
                }
                (false, false) => {}
            }
        }
        points = clipped;
    }
    points
}

/// The router's refusal of an incremental update to a board of other nets
/// (`Router::update`): a part moved to another sub-board splits its nets
/// differently.
pub(crate) const OTHER_NETS: &str = "incremental update needs the same nets and rules";

/// What `split_router_nets` did: the nets split and into how many.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct NetSplit {
    /// Per net split: its name and the pieces it was split into.
    pub nets: Vec<(String, usize)>,
    /// Nets with pads on two pieces that stay one net, by why: a pad on a
    /// tab, copper of theirs on a tab or over two pieces, a pour over two
    /// pieces whose boxes overlap.
    pub joined: Vec<String>,
}

/// The router's net split (module comment): on a board of two pieces or
/// more, each net whose terminals lie on several pieces, none on a tab,
/// and none of whose other copper (tracks, vias, merged pads, graphics)
/// lies on a tab or over two pieces, becomes one net per piece: the piece
/// of its first terminal keeps the net, each other piece gets a new net
/// of the same name and class (the writer names copper by the net's name,
/// so KiCad sees one net again). Its pours are cut to each piece's box.
/// Nothing changes on a board of one piece.
pub(crate) fn split_router_nets(
    board: &mut core::Board,
    pieces: &Pieces,
    only: Option<&BTreeSet<String>>,
) -> NetSplit {
    let mut split = NetSplit::default();
    let overlapping = pieces.boxes_overlap();
    let net_count = board.nets.len();
    // Each net's other copper (obstacles with the net that are no
    // terminal's pad) and pours.
    let mut terminal_pads: BTreeSet<usize> = BTreeSet::new();
    for net in &board.nets {
        for terminal in &net.terminals {
            terminal_pads.insert(terminal.pad);
        }
    }
    let mut copper: Vec<Vec<usize>> = vec![Vec::new(); net_count];
    for (index, obstacle) in board.obstacles.iter().enumerate() {
        if let Some(net) = obstacle.net
            && (net as usize) < net_count
            && !terminal_pads.contains(&index)
        {
            copper[net as usize].push(index);
        }
    }
    let mut pours: Vec<Vec<usize>> = vec![Vec::new(); net_count];
    for (index, plane) in board.planes.iter().enumerate() {
        if (plane.net as usize) < net_count {
            pours[plane.net as usize].push(index);
        }
    }
    for net in 0..net_count {
        if board.nets[net].terminals.len() < 2
            || only.is_some_and(|only| !only.contains(&board.nets[net].name))
        {
            continue;
        }
        let places: Result<Vec<Option<usize>>, ()> = board.nets[net]
            .terminals
            .iter()
            .map(|terminal| terminal_piece(board, terminal, pieces))
            .collect();
        let on: BTreeSet<usize> = match &places {
            Ok(places) => places.iter().flatten().copied().collect(),
            Err(()) => {
                // A pad on a tab: KiCad's net may well run over it.
                let some: BTreeSet<usize> = board.nets[net]
                    .terminals
                    .iter()
                    .filter_map(|terminal| pieces.piece_at(terminal.anchor))
                    .collect();
                if some.len() >= 2 {
                    split.joined.push(board.nets[net].name.clone());
                }
                continue;
            }
        };
        if on.len() < 2 {
            continue;
        }
        let places = places.expect("checked");
        let copper_places: Option<Vec<Option<usize>>> = copper[net]
            .iter()
            .map(|index| pieces.piece_of_box(aabb_box(&board.obstacles[*index].shape)))
            .collect();
        let Some(copper_places) = copper_places else {
            split.joined.push(board.nets[net].name.clone());
            continue;
        };
        let pour_places: Vec<Option<usize>> = pours[net]
            .iter()
            .map(|index| {
                pieces
                    .piece_of_box(polygon_box(&board.planes[*index].polygon))
                    .flatten()
            })
            .collect();
        if overlapping && pour_places.iter().any(Option::is_none) {
            split.joined.push(board.nets[net].name.clone());
            continue;
        }
        // The pieces in the order of the net's terminals: the first keeps
        // the net.
        let mut order: Vec<usize> = Vec::new();
        for piece in places.iter().flatten() {
            if !order.contains(piece) {
                order.push(*piece);
            }
        }
        let mut id_of_piece: BTreeMap<usize, core::NetId> = BTreeMap::new();
        id_of_piece.insert(order[0], net as core::NetId);
        for piece in &order[1..] {
            let id = board.nets.len() as core::NetId;
            board.nets.push(core::Net {
                name: board.nets[net].name.clone(),
                class: board.nets[net].class,
                terminals: Vec::new(),
            });
            id_of_piece.insert(*piece, id);
        }
        // The terminals, each to its piece's net (one off every piece
        // stays); old index -> (net, new index).
        let terminals = std::mem::take(&mut board.nets[net].terminals);
        let mut moved: Vec<(core::NetId, usize)> = Vec::with_capacity(terminals.len());
        for (terminal, place) in terminals.into_iter().zip(&places) {
            let id = place.map_or(net as core::NetId, |piece| id_of_piece[&piece]);
            moved.push((id, board.nets[id as usize].terminals.len()));
            board.nets[id as usize].terminals.push(terminal);
        }
        // The pads' obstacles follow their terminals.
        for id in id_of_piece.values().copied() {
            for terminal in &board.nets[id as usize].terminals {
                if let Some(obstacle) = board.obstacles.get_mut(terminal.pad) {
                    obstacle.net = Some(id);
                }
            }
        }
        for (index, place) in copper[net].iter().zip(&copper_places) {
            if let Some(piece) = place {
                board.obstacles[*index].net = Some(
                    id_of_piece
                        .get(piece)
                        .copied()
                        .unwrap_or(net as core::NetId),
                );
            }
        }
        for merged in board
            .merged_pads
            .iter_mut()
            .filter(|m| m.net as usize == net)
        {
            if let Some(&(id, index)) = moved.get(merged.host) {
                merged.net = id;
                merged.host = index;
                if let Some(obstacle) = board.obstacles.get_mut(merged.pad) {
                    obstacle.net = Some(id);
                }
            }
        }
        for copper in board
            .terminal_copper
            .iter_mut()
            .filter(|c| c.net as usize == net)
        {
            if let Some(&(id, index)) = moved.get(copper.terminal) {
                copper.net = id;
                copper.terminal = index;
            }
        }
        for gap in &mut board.pin_row_gaps {
            let ends = [gap.a, gap.b];
            for (side, end) in gap.nets.iter_mut().zip(ends) {
                if *side == Some(net as core::NetId)
                    && let Some(piece) = pieces.piece_at(end)
                    && let Some(id) = id_of_piece.get(&piece)
                {
                    *side = Some(*id);
                }
            }
        }
        // The pours: one on a single piece goes to that piece's net; one
        // over several is cut to each piece's box.
        for (index, place) in pours[net].iter().zip(&pour_places) {
            match place {
                Some(piece) => {
                    if let Some(id) = id_of_piece.get(piece) {
                        board.planes[*index].net = *id;
                    }
                }
                None => {
                    let original = board.planes[*index].clone();
                    let mut first = true;
                    for piece in &order {
                        let polygon = clip(&original.polygon, pieces.boxes[*piece]);
                        if polygon.len() < 3 {
                            continue;
                        }
                        let mut plane = original.clone();
                        plane.polygon = polygon;
                        plane.net = id_of_piece[piece];
                        if first {
                            board.planes[*index] = plane;
                            first = false;
                        } else {
                            board.planes.push(plane);
                        }
                    }
                }
            }
        }
        split.nets.push((board.nets[net].name.clone(), order.len()));
    }
    split
}

/// A pad's or a track's place on a KiCad board, for `board_split_nets`.
enum Place {
    /// On one piece (or off the board: no place).
    Piece(Option<usize>),
    /// On a tab, a neck, or over two pieces.
    Joining,
}

/// The nets of a KiCad board that its sub-boards split, by the router's
/// rule (`split_router_nets`) on the board's own items: pads on two
/// pieces or more, none on a tab, and no track, arc or via of the net on
/// a tab or from one piece to another. Each with the number of pieces
/// its pads lie on. Pours do not join (KiCad's fill decides: where it
/// joins two pieces, KiCad reports nothing across them).
pub(crate) fn board_split_nets(pcb: &Expr, pieces: &Pieces) -> BTreeMap<String, usize> {
    let codes = crate::kicad_nets::net_codes(pcb);
    let mut on: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
    let mut joined: BTreeSet<String> = BTreeSet::new();
    let mut note = |net: &str, place: Place| match place {
        Place::Piece(Some(piece)) => {
            on.entry(net.to_string()).or_default().insert(piece);
        }
        Place::Piece(None) => {}
        Place::Joining => {
            joined.insert(net.to_string());
        }
    };
    let point_place = |point: [f64; 2]| {
        if pieces.between(point) {
            Place::Joining
        } else {
            Place::Piece(pieces.piece_at(point))
        }
    };
    let points_place = |points: &[[f64; 2]]| {
        let mut piece = None;
        for point in points {
            match point_place(*point) {
                Place::Joining => return Place::Joining,
                Place::Piece(Some(here)) => match piece {
                    None => piece = Some(here),
                    Some(other) if other != here => return Place::Joining,
                    _ => {}
                },
                Place::Piece(None) => {}
            }
        }
        Place::Piece(piece)
    };
    for item in pcb.children() {
        match item.head() {
            Some("footprint") => {
                let Ok(at) = form_at(item) else { continue };
                for pad in item
                    .children()
                    .iter()
                    .filter(|child| child.head() == Some("pad"))
                {
                    let Some(net) = crate::kicad_nets::item_net(pad, &codes) else {
                        continue;
                    };
                    let Ok(local) = form_at(pad) else { continue };
                    let offset = crate::geometry::rotate_vector([local[0], local[1]], -at[2]);
                    note(net, point_place([at[0] + offset[0], at[1] + offset[1]]));
                }
            }
            Some("segment" | "arc") => {
                let Some(net) = crate::kicad_nets::item_net(item, &codes) else {
                    continue;
                };
                let points: Vec<[f64; 2]> = ["start", "mid", "end"]
                    .iter()
                    .filter_map(|head| crate::kicad_sexpr::form_xy(item, head).ok())
                    .collect();
                note(net, points_place(&points));
            }
            Some("via") => {
                let Some(net) = crate::kicad_nets::item_net(item, &codes) else {
                    continue;
                };
                if let Ok(at) = crate::kicad_sexpr::form_xy(item, "at") {
                    note(net, point_place(at));
                }
            }
            _ => {}
        }
    }
    on.into_iter()
        .filter(|(net, pieces)| pieces.len() >= 2 && !joined.contains(net))
        .map(|(net, pieces)| (unescaped(&net), pieces.len()))
        .collect()
}

/// How far beyond the board an unconnected item's place may lie and still
/// count on the nearest piece (mm): KiCad places a zone's item at a corner
/// of its outline, which designers draw past the edge (up to 9 mm on the
/// 4in1 panel).
const ZONE_REACH_MM: f64 = 25.0;

/// A net's name as KiCad's reports write it: the board file escapes `/`
/// as `{slash}` and so on (KiCad's EscapeString), the DRC report does not.
fn unescaped(name: &str) -> String {
    if !name.contains('{') {
        return name.to_string();
    }
    [
        ("{slash}", "/"),
        ("{backslash}", "\\"),
        ("{lt}", "<"),
        ("{gt}", ">"),
        ("{colon}", ":"),
        ("{dblquote}", "\""),
        ("{quote}", "'"),
        ("{space}", " "),
        ("{tab}", "\t"),
        ("{bar}", "|"),
        ("{dollar}", "$"),
        ("{comma}", ","),
    ]
    .iter()
    .fold(name.to_string(), |name, (escaped, plain)| {
        name.replace(escaped, plain)
    })
}

/// The nets the placement found on several sub-boards by design
/// (`designed_nets`), for KiCad's check judged per sub-board in this
/// process: None outside a layout or placement of a panel (the checked
/// board's own places decide, `board_split_nets`).
static DESIGNED: std::sync::Mutex<Option<BTreeSet<String>>> = std::sync::Mutex::new(None);

/// Notes the nets of one name that lie on several sub-boards by design
/// (None: no panel of different boards): `excuse_across` excuses the
/// items of these nets alone. A placement whose parts' boards are unknown
/// puts the parts of other nets where it would: that a net lies on two
/// boards in the result says nothing of the design, and its items across
/// them count.
pub(crate) fn note_designed(nets: Option<BTreeSet<String>>) {
    if let Ok(mut designed) = DESIGNED.lock() {
        *designed = nets;
    }
    note_cells(None);
}

/// A panel's cells the placement found where the outline has no pieces
/// (`note_cells`), for the router's net split and KiCad's check judged per
/// cell in this process.
static CELLS: std::sync::Mutex<Option<Vec<[f64; 4]>>> = std::sync::Mutex::new(None);

/// Notes a panel's cells (None: none): the nets of `note_designed` are
/// split per cell and their items across cells excused, as on pieces.
pub(crate) fn note_cells(cells: Option<Vec<[f64; 4]>>) {
    if let Ok(mut noted) = CELLS.lock() {
        *noted = cells;
    }
}

/// The cells and the designed nets noted for this process, where both
/// are.
pub(crate) fn noted_cells() -> Option<(Pieces, BTreeSet<String>)> {
    let cells = CELLS.lock().ok()?.clone()?;
    let designed = DESIGNED.lock().ok()?.clone()?;
    Some((Pieces::of_boxes(cells), designed))
}

/// The nets with pads of two footprints of one reference (the second half
/// of `designed_nets`), by the board's names.
pub(crate) fn repeated_reference_nets(pcb: &Expr) -> BTreeSet<String> {
    let codes = crate::kicad_nets::net_codes(pcb);
    let mut on: BTreeMap<String, BTreeMap<String, BTreeSet<usize>>> = BTreeMap::new();
    for (index, footprint) in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
        .enumerate()
    {
        let reference = crate::kicad_nets::footprint_reference(footprint).unwrap_or_default();
        for pad in footprint
            .children()
            .iter()
            .filter(|child| child.head() == Some("pad"))
        {
            if let Some(net) = crate::kicad_nets::item_net(pad, &codes) {
                on.entry(net.to_string())
                    .or_default()
                    .entry(reference.clone())
                    .or_default()
                    .insert(index);
            }
        }
    }
    on.into_iter()
        .filter(|(_, references)| references.values().any(|parts| parts.len() >= 2))
        .map(|(net, _)| net)
        .collect()
}

/// The nets on several sub-boards by design: those the placement split
/// (pins of parts whose board is known on two), and those with pads of
/// two footprints of one reference (a reference names one part per
/// board: two are two boards' parts, the 4in1 panel's two Q6 on
/// `Net-(Q6-G)`), by KiCad's report names.
pub(crate) fn designed_nets(
    pcb: &Expr,
    sub: &PlacerSubBoards,
    net_names: &[String],
) -> BTreeSet<String> {
    // The placer names a net as a connection (`NetNames::normalize`: the
    // hierarchical slash dropped, `/ESC1/GHA` as `ESC1/GHA`); KiCad's
    // report by the pads' own names.
    let names = crate::kicad_nets::NetNames::of(pcb);
    let codes = crate::kicad_nets::net_codes(pcb);
    let split: BTreeSet<&str> = sub
        .split
        .iter()
        .filter_map(|net| net_names.get(*net))
        .map(String::as_str)
        .collect();
    let mut nets: BTreeSet<String> = BTreeSet::new();
    for footprint in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
    {
        for pad in footprint
            .children()
            .iter()
            .filter(|child| child.head() == Some("pad"))
        {
            if let Some(net) = crate::kicad_nets::item_net(pad, &codes)
                && split.contains(names.normalize(net))
            {
                nets.insert(net.to_string());
            }
        }
    }
    nets.extend(repeated_reference_nets(pcb));
    nets
}

/// The key of a DRC report's unconnected items that KiCad's check judged
/// per sub-board does not count (`excuse_across`).
pub(crate) const ACROSS_KEY: &str = "unconnected_across_sub_boards";

/// KiCad's check judged per sub-board: in a DRC report of `pcb`, the
/// unconnected items of a net the sub-boards split (`board_split_nets`)
/// whose two items lie on two pieces are what joining the sub-boards
/// would take (one less than the pieces the net lies on, per net; KiCad
/// reports them on the designer's own board too): moved from
/// `unconnected_items` to `ACROSS_KEY`. Other items, and as many more as
/// KiCad's ratsnest put across the pieces (an island within a piece left
/// open: its edge may end on the other piece), stay. Returns how many
/// moved.
pub(crate) fn excuse_across(
    report: &mut serde_json::Value,
    pcb: &Expr,
    pieces: &Pieces,
    designed: Option<&BTreeSet<String>>,
) -> usize {
    let Some(items) = report["unconnected_items"].as_array() else {
        return 0;
    };
    if items.is_empty() {
        return 0;
    }
    let mut split = board_split_nets(pcb, pieces);
    if let Some(designed) = designed {
        let designed: BTreeSet<String> = designed.iter().map(|net| unescaped(net)).collect();
        split.retain(|net, _| designed.contains(net));
    }
    if split.is_empty() {
        return 0;
    }
    let mut left: BTreeMap<&str, usize> = split
        .iter()
        .map(|(net, count)| (net.as_str(), count - 1))
        .collect();
    let mut kept = Vec::new();
    let mut across = Vec::new();
    for item in items {
        let ends: Vec<(Option<String>, Option<usize>)> = item["items"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|end| {
                let net = end["description"].as_str().and_then(described_net);
                let at = [end["pos"]["x"].as_f64(), end["pos"]["y"].as_f64()];
                let piece = match at {
                    [Some(x), Some(y)] => pieces.piece_near([x, y], ZONE_REACH_MM),
                    _ => None,
                };
                (net, piece)
            })
            .collect();
        let excused = match ends.as_slice() {
            [(Some(a), Some(pa)), (Some(b), Some(pb))] if a == b && pa != pb => {
                match left.get_mut(a.as_str()) {
                    Some(count) if *count > 0 => {
                        *count -= 1;
                        true
                    }
                    _ => false,
                }
            }
            _ => false,
        };
        if excused {
            across.push(item.clone());
        } else {
            kept.push(item.clone());
        }
    }
    // KiCad lists at most `KICAD_REPORT_CAP` unconnected items: at the cap
    // the items left are a lower bound, and none left would be a pass
    // KiCad's list cannot show; then nothing moves.
    let capped = items.len() >= KICAD_REPORT_CAP;
    if capped && kept.is_empty() {
        return 0;
    }
    let moved = across.len();
    if moved > 0 {
        report["unconnected_items"] = serde_json::Value::Array(kept);
        report[ACROSS_KEY] = serde_json::Value::Array(across);
        if capped {
            report[CAPPED_KEY] = serde_json::Value::Bool(true);
        }
    }
    moved
}

/// KiCad's DRC lists at most this many items of a kind (KiCad 10; as
/// `run_diagnosis`'s).
const KICAD_REPORT_CAP: usize = 499;

/// Set in a DRC report whose unconnected items reached KiCad's cap before
/// `excuse_across` moved some: the items left are at least that many.
pub(crate) const CAPPED_KEY: &str = "unconnected_items_capped";

/// The net a DRC item's description names (`Pad 2 [GND] of C1 on F.Cu`).
fn described_net(description: &str) -> Option<String> {
    let start = description.find('[')? + 1;
    let end = start + description[start..].find(']')?;
    let net = &description[start..end];
    (net != "<no net>" && !net.is_empty()).then(|| net.to_string())
}

/// `excuse_across` on a DRC report file of the board file `board`, in
/// place: read only when the report has unconnected items and the
/// board's outline has two pieces or more. The report keeps KiCad's other
/// fields as they are.
pub(crate) fn excuse_across_file(report: &Path, board: &Path) -> Result<usize, String> {
    let text = fs::read_to_string(report)
        .map_err(|error| format!("failed to read {}: {error}", report.display()))?;
    let mut value: serde_json::Value =
        serde_json::from_str(&text).map_err(|error| format!("{}: {error}", report.display()))?;
    if value["unconnected_items"]
        .as_array()
        .is_none_or(|items| items.is_empty())
    {
        return Ok(0);
    }
    let source = fs::read_to_string(board)
        .map_err(|error| format!("failed to read {}: {error}", board.display()))?;
    let pcb = crate::canonical_order::parse_board(&source)?;
    let (pieces, designed) = match Pieces::of_board(&pcb)? {
        Some(pieces) => (
            pieces,
            DESIGNED.lock().ok().and_then(|designed| designed.clone()),
        ),
        None => match noted_cells() {
            Some((cells, designed)) => (cells, Some(designed)),
            None => return Ok(0),
        },
    };
    let moved = excuse_across(&mut value, &pcb, &pieces, designed.as_ref());
    if moved > 0 {
        let text = serde_json::to_string_pretty(&value).map_err(|error| error.to_string())?;
        crate::drc_cache::write_atomically(report, text.as_bytes())
            .map_err(|error| format!("failed to write {}: {error}", report.display()))?;
    }
    Ok(moved)
}

/// The last outline's pieces (`Pieces::of_loops_cached`): its hash and
/// pieces. Route mode's ladder and the layout's moves lower one outline
/// many times.
static PIECES_CACHE: std::sync::Mutex<Option<(u64, Option<std::sync::Arc<Pieces>>)>> =
    std::sync::Mutex::new(None);

impl Pieces {
    /// `of_loops`, remembered for the last outline.
    pub(crate) fn of_loops_cached(
        loops: &[Vec<[f64; 2]>],
        cutouts: &[Vec<[f64; 2]>],
        cuts: &Cuts,
    ) -> Option<std::sync::Arc<Pieces>> {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for polygon in loops
            .iter()
            .chain(std::iter::once(&Vec::new()))
            .chain(cutouts)
        {
            polygon.len().hash(&mut hasher);
            for point in polygon {
                point[0].to_bits().hash(&mut hasher);
                point[1].to_bits().hash(&mut hasher);
            }
        }
        for (centre, radius) in &cuts.holes {
            centre[0].to_bits().hash(&mut hasher);
            centre[1].to_bits().hash(&mut hasher);
            radius.to_bits().hash(&mut hasher);
        }
        for line in &cuts.lines {
            for point in line {
                point[0].to_bits().hash(&mut hasher);
                point[1].to_bits().hash(&mut hasher);
            }
        }
        let key = hasher.finish();
        if let Ok(cache) = PIECES_CACHE.lock()
            && let Some((cached, pieces)) = cache.as_ref()
            && *cached == key
        {
            return pieces.clone();
        }
        let pieces = Pieces::of_loops(loops, cutouts, cuts).map(std::sync::Arc::new);
        if let Ok(mut cache) = PIECES_CACHE.lock() {
            *cache = Some((key, pieces.clone()));
        }
        pieces
    }
}

/// `pcb-maker sub-boards-kicad-board`: a board's pieces without its tabs
/// and the nets its sub-boards split, as JSON (research; the census of
/// docs/placer.md "Sub-boards in a panel").
pub fn describe_sub_boards(board: &Path) -> Result<serde_json::Value, String> {
    let source = fs::read_to_string(board)
        .map_err(|error| format!("failed to read {}: {error}", board.display()))?;
    let pcb = crate::canonical_order::parse_board(&source)?;
    let loops = outline::board_loops(&pcb)?;
    let round = |b: &[f64; 4]| b.map(|v| (v * 1000.0).round() / 1000.0);
    let cuts = Cuts::of_board(&pcb);
    let Some(pieces) = Pieces::of_loops(&loops.piece_loops, &loops.cutouts, &cuts) else {
        return Ok(serde_json::json!({
            "pieces": 1,
            "outline_loops": loops.piece_loops.len(),
            "mouse_bite_rows": cuts.rows,
            "v_cut_lines": cuts.lines.len(),
        }));
    };
    let split = board_split_nets(&pcb, &pieces);
    // The parts with pins on each piece, by the board's own places: a
    // piece without any is a frame or rail.
    let codes = crate::kicad_nets::net_codes(&pcb);
    let mut parts = vec![0usize; pieces.count()];
    for footprint in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
    {
        let pinned = footprint
            .children()
            .iter()
            .filter(|child| child.head() == Some("pad"))
            .any(|pad| crate::kicad_nets::item_net(pad, &codes).is_some());
        if pinned
            && let Ok(at) = form_at(footprint)
            && let Some(piece) = pieces.piece_at([at[0], at[1]])
        {
            parts[piece] += 1;
        }
    }
    let occupied: BTreeSet<usize> = (0..pieces.count())
        .filter(|piece| parts[*piece] > 0)
        .collect();
    let frames = pieces.frames(&occupied);
    let keepouts = pieces.keepouts(&frames, TAB_MARGIN_MM);
    Ok(serde_json::json!({
        "pieces": pieces.count(),
        "outline_loops": loops.piece_loops.len(),
        "cut_by": pieces.cut_by,
        "mouse_bite_rows": cuts.rows,
        "v_cut_lines": cuts.lines.len(),
        "boxes": pieces.boxes.iter().map(round).collect::<Vec<_>>(),
        "areas_mm2": pieces.areas.iter().map(|area| area.round()).collect::<Vec<_>>(),
        "parts": parts,
        "frames": frames,
        "tabs": pieces.tabs.iter().map(round).collect::<Vec<_>>(),
        "keepouts": keepouts.len(),
        "kept_clear_mm2": keepouts.iter().map(|b| (b[2] - b[0]) * (b[3] - b[1])).sum::<f64>().round(),
        "split_nets": split.len(),
        "split": split,
    }))
}

/// The sub-boards the placer found (`detect_for_placer`).
#[derive(Clone, Debug)]
pub(crate) struct PlacerSubBoards {
    pub pieces: std::sync::Arc<Pieces>,
    /// Per footprint (the problem's first components): its sub-board,
    /// None where nothing says which.
    pub part_piece: Vec<Option<usize>>,
    /// The problem's nets with pins on two sub-boards or more.
    pub split: Vec<usize>,
    /// What placed the parts on their sub-boards: `fixed parts`,
    /// `reference board`, `placed input`, `nets`.
    pub from: Vec<&'static str>,
    /// Groups of parts nets join that the evidence puts on two sub-boards
    /// (left without one).
    pub conflicts: usize,
    /// Per piece: a frame or rail (`Pieces::frames`): no part with pins
    /// lies on it by the fixed parts, the reference board or the placed
    /// input; no part may.
    pub frames: Vec<bool>,
}

/// The share of the movable parts that may be without a sub-board for
/// the placement to hold the others in theirs and split the nets.
pub(crate) const MOST_UNASSIGNED_SHARE: f64 = 0.1;

fn body_box(component: &pcb_placer::Component, pose: pcb_placer::Pose) -> [f64; 4] {
    let (center, half) = (component.center(pose), component.half_extent(pose.angle));
    [
        center[0] - half[0],
        center[1] - half[1],
        center[0] + half[0],
        center[1] + half[1],
    ]
}

/// Which sub-board each part belongs to (module comment), on a board whose
/// outline has two pieces or more without its tabs (else None):
///
/// 1. the parts that stay where they are (fixed, locked, the board's
///    marks among the footprints) lie on their piece;
/// 2. with a reference board (`reference`), each footprint whose uuid it
///    holds lies on the piece of its place there;
/// 3. an input that looks placed (nine in ten movable parts on a piece,
///    nine in ten at a place of their own: not a stack) gives each
///    movable part its piece;
/// 4. a net with pins of such parts on two pieces is split; the others
///    join their parts, and a group so joined takes the piece of its
///    placed parts where they agree (none where they do not: a conflict).
pub(crate) fn detect_for_placer(
    pcb: &Expr,
    problem: &pcb_placer::Problem,
    footprints: usize,
    reference: Option<&Expr>,
) -> Result<Option<PlacerSubBoards>, String> {
    let loops = outline::board_loops(pcb)?;
    let Some(pieces) =
        Pieces::of_loops_cached(&loops.piece_loops, &loops.cutouts, &Cuts::of_board(pcb))
    else {
        return Ok(None);
    };
    let footprints = footprints.min(problem.components.len());
    let uuids: Vec<Option<String>> = pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
        .map(crate::kicad_nets::footprint_uuid)
        .collect();
    let mut part_piece: Vec<Option<usize>> = vec![None; footprints];
    let mut from: BTreeSet<&'static str> = BTreeSet::new();
    let with_pins = |part: usize| !problem.components[part].pins.is_empty();
    for (part, place) in part_piece.iter_mut().enumerate() {
        let component = &problem.components[part];
        if component.fixed
            && let Some(Some(piece)) = pieces.piece_of_box(body_box(component, problem.poses[part]))
        {
            *place = Some(piece);
            if with_pins(part) {
                from.insert("fixed parts");
            }
        }
    }
    if let Some(reference) = reference {
        let places: BTreeMap<String, [f64; 3]> = reference
            .children()
            .iter()
            .filter(|item| item.head() == Some("footprint"))
            .filter_map(|item| {
                Some((
                    crate::kicad_nets::footprint_uuid(item)?,
                    form_at(item).ok()?,
                ))
            })
            .collect();
        for (part, place) in part_piece.iter_mut().enumerate() {
            if place.is_none()
                && let Some(Some(uuid)) = uuids.get(part)
                && let Some(at) = places.get(uuid)
                && let Some(piece) = pieces.piece_at([at[0], at[1]])
            {
                *place = Some(piece);
                from.insert("reference board");
            }
        }
    }
    let movable: Vec<usize> = (0..footprints)
        .filter(|part| !problem.components[*part].fixed && with_pins(*part))
        .collect();
    if !movable.is_empty() {
        let on_piece = movable
            .iter()
            .filter(|part| pieces.piece_at(problem.poses[**part].position).is_some())
            .count();
        let mut seen: BTreeMap<(i64, i64), usize> = BTreeMap::new();
        for part in &movable {
            let at = problem.poses[*part].position;
            *seen
                .entry((
                    (at[0] * 100.0).round() as i64,
                    (at[1] * 100.0).round() as i64,
                ))
                .or_default() += 1;
        }
        let alone = movable
            .iter()
            .filter(|part| {
                let at = problem.poses[**part].position;
                seen[&(
                    (at[0] * 100.0).round() as i64,
                    (at[1] * 100.0).round() as i64,
                )] == 1
            })
            .count();
        if on_piece * 10 >= movable.len() * 9 && alone * 10 >= movable.len() * 9 {
            for part in &movable {
                if part_piece[*part].is_none()
                    && let Some(piece) = pieces.piece_at(problem.poses[*part].position)
                {
                    part_piece[*part] = Some(piece);
                    from.insert("placed input");
                }
            }
        }
    }
    // The frames: pieces no part with pins lies on by that evidence.
    let occupied: BTreeSet<usize> = part_piece
        .iter()
        .enumerate()
        .filter(|(part, _)| with_pins(*part))
        .filter_map(|(_, piece)| *piece)
        .collect();
    let frames = pieces.frames(&occupied);
    // The nets: on two pieces by the parts placed so far, split.
    let net_count = problem.net_weights.len();
    let mut on: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); net_count];
    for (part, place) in part_piece.iter().enumerate() {
        if let Some(piece) = *place {
            for pin in &problem.components[part].pins {
                on[pin.net].insert(piece);
            }
        }
    }
    let split: Vec<usize> = (0..net_count).filter(|net| on[*net].len() >= 2).collect();
    let split_set: BTreeSet<usize> = split.iter().copied().collect();
    // The other nets join their parts.
    let mut parent: Vec<usize> = (0..footprints).collect();
    fn root(parent: &mut [usize], mut part: usize) -> usize {
        while parent[part] != part {
            parent[part] = parent[parent[part]];
            part = parent[part];
        }
        part
    }
    let mut first_on_net: BTreeMap<usize, usize> = BTreeMap::new();
    for part in 0..footprints {
        for pin in &problem.components[part].pins {
            if split_set.contains(&pin.net) {
                continue;
            }
            match first_on_net.get(&pin.net) {
                Some(&other) => {
                    let (a, b) = (root(&mut parent, part), root(&mut parent, other));
                    parent[a] = b;
                }
                None => {
                    first_on_net.insert(pin.net, part);
                }
            }
        }
    }
    let mut group_pieces: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for (part, place) in part_piece.iter().enumerate() {
        if let Some(piece) = *place {
            let group = root(&mut parent, part);
            group_pieces.entry(group).or_default().insert(piece);
        }
    }
    let conflicts = group_pieces
        .values()
        .filter(|pieces| pieces.len() > 1)
        .count();
    let mut by_nets = false;
    for (part, place) in part_piece.iter_mut().enumerate() {
        if place.is_none() && with_pins(part) {
            let group = root(&mut parent, part);
            if let Some(pieces) = group_pieces.get(&group)
                && pieces.len() == 1
            {
                *place = pieces.iter().next().copied();
                by_nets = true;
            }
        }
    }
    if by_nets {
        from.insert("nets");
    }
    let order = ["fixed parts", "reference board", "placed input", "nets"];
    Ok(Some(PlacerSubBoards {
        pieces,
        part_piece,
        split,
        from: order
            .into_iter()
            .filter(|kind| from.contains(kind))
            .collect(),
        conflicts,
        frames,
    }))
}

impl PlacerSubBoards {
    /// Whether some pieces are a frame or rails (`frames`).
    pub(crate) fn has_frames(&self) -> bool {
        self.frames.iter().any(|frame| *frame)
    }

    /// The sub-boards that hold parts with pins: a panel of different
    /// boards has two or more.
    pub(crate) fn count(&self, problem: &pcb_placer::Problem) -> usize {
        self.part_piece
            .iter()
            .enumerate()
            .filter(|(part, _)| !problem.components[*part].pins.is_empty())
            .filter_map(|(_, piece)| *piece)
            .collect::<BTreeSet<usize>>()
            .len()
    }

    /// The movable parts with pins and those of them without a sub-board.
    pub(crate) fn unassigned(&self, problem: &pcb_placer::Problem) -> (usize, Vec<usize>) {
        let movable: Vec<usize> = (0..self.part_piece.len())
            .filter(|part| {
                let component = &problem.components[*part];
                !component.fixed && !component.pins.is_empty()
            })
            .collect();
        let unassigned = movable
            .iter()
            .copied()
            .filter(|part| self.part_piece[*part].is_none())
            .collect();
        (movable.len(), unassigned)
    }
}

/// What `apply_to_placer` changed.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct PlacerApplied {
    /// Parts held in their sub-board's region.
    pub held: usize,
    /// Nets of the problem split, and the nets added for them.
    pub nets_split: usize,
    pub nets_added: usize,
    /// Keepouts added (boxes over the tabs and the frames,
    /// `Pieces::keepouts`), and their area (mm², overlaps counted once).
    pub tabs: usize,
    pub kept_clear_mm2: f64,
}

/// The sub-boards in the placement problem (a panel: two sub-boards or
/// more, or a board beside its frame or rails):
///
/// - no part's body on a tab or a frame: each box of `Pieces::keepouts`
///   (the tabs grown by `TAB_MARGIN_MM`, the frames) a keepout (an
///   obstacle of the board, as a rule area's), where the parts are held
///   (below) or the board has a frame;
/// - where the parts lie on two sub-boards or more and at most
///   `MOST_UNASSIGNED_SHARE` of the movable parts have none, each movable
///   part held in its sub-board's region (unless a constraint holds it
///   in one already) and started at its centre, and each split net one
///   net per sub-board (the parts without one stay on the first's), its
///   weight as the lowering gives a net of that many pins.
pub(crate) fn apply_to_placer(
    sub: &PlacerSubBoards,
    problem: &mut pcb_placer::Problem,
    references: &mut Vec<String>,
    source_at: &mut Vec<[f64; 3]>,
    poured: &mut Vec<bool>,
) -> PlacerApplied {
    let mut applied = PlacerApplied::default();
    // Most parts' boards unknown: no part is held (tab keepouts alone
    // cost the crowded 4in1 panel's front 70 mm², and 8 parts found no
    // place where 0 to 4 had not), unless the panel has a frame.
    let (movable, unassigned) = sub.unassigned(problem);
    // Held also where no net lies on two sub-boards (the ULK's halves:
    // their nets apart, each part's half told by the nets): a part on
    // the other half is no part of its circuit there (no track crosses
    // a row of mouse bites).
    let hold = sub.count(problem) >= 2
        && unassigned.len() as f64 <= MOST_UNASSIGNED_SHARE * movable as f64;
    if !hold && !sub.has_frames() {
        return applied;
    }
    let keepouts = sub.pieces.keepouts(&sub.frames, TAB_MARGIN_MM);
    applied.kept_clear_mm2 = keepouts.iter().map(|b| (b[2] - b[0]) * (b[3] - b[1])).sum();
    for tab in &keepouts {
        let size = [tab[2] - tab[0], tab[3] - tab[1]];
        problem.components.push(pcb_placer::Component {
            name: "tab".into(),
            body_center: [0.0, 0.0],
            body_size: size,
            round: false,
            halo: 0.0,
            pins: Vec::new(),
            side: pcb_placer::Side::Both,
            fixed: true,
            angle_options: vec![0.0],
            far_side: Vec::new(),
            far_copper: Vec::new(),
            far_courtyard: Vec::new(),
            hollow: Vec::new(),
            tight: None,
            edge_inset: 0.0,
            courtyards: Vec::new(),
            holes_inside: false,
            pads: Vec::new(),
            copper_only: false,
            cutout_outline: Vec::new(),
            tight_hollow: Vec::new(),
            hole_rings: Vec::new(),
            copper_graphics: Vec::new(),
        });
        problem.poses.push(pcb_placer::Pose {
            position: [(tab[0] + tab[2]) / 2.0, (tab[1] + tab[3]) / 2.0],
            angle: 0.0,
        });
        references.push("tab".into());
        source_at.push([0.0; 3]);
        applied.tabs += 1;
    }
    if !hold {
        return applied;
    }
    let held: BTreeSet<usize> = problem
        .constraints
        .regions
        .iter()
        .map(|(part, _)| *part)
        .collect();
    let started: BTreeSet<usize> = problem
        .constraints
        .starts
        .iter()
        .map(|(part, _)| *part)
        .collect();
    for part in 0..sub.part_piece.len() {
        let component = &problem.components[part];
        let Some(piece) = sub.part_piece[part] else {
            continue;
        };
        if component.fixed || component.pins.is_empty() || held.contains(&part) {
            continue;
        }
        let region = sub.pieces.boxes[piece];
        problem.constraints.regions.push((part, region));
        if !started.contains(&part) {
            problem.constraints.starts.push((
                part,
                [(region[0] + region[2]) / 2.0, (region[1] + region[3]) / 2.0],
            ));
        }
        applied.held += 1;
    }
    // The split nets: each sub-board's pins on a net of their own.
    let mut pin_counts = vec![0usize; problem.net_weights.len()];
    for component in &problem.components {
        for pin in &component.pins {
            pin_counts[pin.net] += 1;
        }
    }
    let base = |net: usize, weights: &[f64]| {
        let pins = pin_counts[net] as f64;
        weights[net] / (3.0 / (pins - 1.0).max(1.0)).min(1.0)
    };
    for &net in &sub.split {
        let mut ids: BTreeMap<usize, usize> = BTreeMap::new();
        let scale = base(net, &problem.net_weights);
        let mut counts: BTreeMap<usize, usize> = BTreeMap::new();
        for part in 0..sub.part_piece.len() {
            let piece = sub.part_piece[part];
            for pin in problem.components[part].pins.iter_mut() {
                if pin.net != net {
                    continue;
                }
                let Some(piece) = piece else {
                    *counts.entry(net).or_default() += 1;
                    continue;
                };
                let id = match ids.get(&piece) {
                    Some(id) => *id,
                    None => {
                        let id = if ids.is_empty() {
                            net
                        } else {
                            problem.net_weights.push(0.0);
                            poured.push(poured.get(net).copied().unwrap_or(false));
                            applied.nets_added += 1;
                            problem.net_weights.len() - 1
                        };
                        ids.insert(piece, id);
                        id
                    }
                };
                pin.net = id;
                *counts.entry(id).or_default() += 1;
            }
        }
        for (id, count) in counts {
            problem.net_weights[id] = scale * (3.0 / (count as f64 - 1.0).max(1.0)).min(1.0);
        }
        applied.nets_split += 1;
    }
    applied
}

/// The sub-boards as `board-placer.json` and the verdict say them
/// (`sub_boards`): present where the outline has two pieces or more
/// without its tabs and parts lie on two of them.
#[derive(Clone, Debug, Default, Serialize, serde::Deserialize, PartialEq)]
pub struct KiCadSubBoardsReport {
    /// How many sub-boards hold parts.
    pub count: usize,
    /// What told the parts' sub-boards: `fixed parts`, `reference board`,
    /// `placed input`, `nets`.
    pub from: Vec<String>,
    /// Nets (by name) whose parts lie on two sub-boards or more: one net
    /// per sub-board to the placer and the router where no copper joins
    /// them.
    pub split_nets: usize,
    /// Movable parts no evidence puts on a sub-board: placed where the
    /// placement put them, the nets they share are the router's per
    /// sub-board where their pads land. The first 20 by label.
    pub unassigned: usize,
    pub unassigned_parts: Vec<String>,
    /// Groups of parts joined by nets that the evidence put on two
    /// sub-boards (their unplaced parts left without one).
    pub conflicts: usize,
    /// Whether the placement held the parts in their sub-boards (at most
    /// a tenth of the movable parts without one), and the tabs (boxes)
    /// it kept parts off.
    pub held: bool,
    pub tabs: Vec<[f64; 4]>,
    /// What cut the outline into pieces: `narrow outline`, `mouse
    /// bites`, `V-cuts`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cut_by: Vec<String>,
    /// The boxes of the pieces that are a frame or rails (no part with
    /// pins lies on them by the fixed parts, the reference or the placed
    /// input): no part is placed there.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub frames: Vec<[f64; 4]>,
    /// The area kept free of parts over the tabs (grown by 1.5 mm) and the
    /// frames (mm²); 0 where the placement kept nothing free.
    #[serde(default)]
    pub kept_clear_mm2: f64,
    /// Each sub-board: its region and parts (with pins), of them the
    /// movable ones.
    pub sub_boards: Vec<KiCadSubBoard>,
}

#[derive(Clone, Debug, Default, Serialize, serde::Deserialize, PartialEq)]
pub struct KiCadSubBoard {
    pub region: [f64; 4],
    pub parts: usize,
    pub movable: usize,
}

impl KiCadSubBoardsReport {
    pub(crate) fn of(
        sub: &PlacerSubBoards,
        problem: &pcb_placer::Problem,
        references: &[String],
        names: &[String],
        applied: &PlacerApplied,
    ) -> KiCadSubBoardsReport {
        let round = |b: &[f64; 4]| b.map(|v| (v * 1000.0).round() / 1000.0);
        let (_, unassigned) = sub.unassigned(problem);
        let mut sub_boards: Vec<KiCadSubBoard> = sub
            .pieces
            .boxes
            .iter()
            .map(|b| KiCadSubBoard {
                region: round(b),
                parts: 0,
                movable: 0,
            })
            .collect();
        for (part, piece) in sub.part_piece.iter().enumerate() {
            if let Some(piece) = piece
                && !problem.components[part].pins.is_empty()
            {
                sub_boards[*piece].parts += 1;
                if !problem.components[part].fixed {
                    sub_boards[*piece].movable += 1;
                }
            }
        }
        sub_boards.retain(|board| board.parts > 0);
        KiCadSubBoardsReport {
            count: sub.count(problem),
            from: sub.from.iter().map(|kind| kind.to_string()).collect(),
            split_nets: sub.split.len(),
            unassigned: unassigned.len(),
            unassigned_parts: unassigned
                .iter()
                .take(20)
                .map(|part| {
                    names
                        .get(*part)
                        .or(references.get(*part))
                        .cloned()
                        .unwrap_or_default()
                })
                .collect(),
            conflicts: sub.conflicts,
            held: applied.held > 0,
            tabs: sub.pieces.tabs.iter().map(round).collect(),
            cut_by: sub
                .pieces
                .cut_by
                .iter()
                .map(|kind| kind.to_string())
                .collect(),
            frames: sub
                .pieces
                .boxes
                .iter()
                .zip(&sub.frames)
                .filter(|(_, frame)| **frame)
                .map(|(b, _)| round(b))
                .collect(),
            kept_clear_mm2: (applied.kept_clear_mm2 * 10.0).round() / 10.0,
            sub_boards,
        }
    }
}

/// The looser of two relaxations, field by field (as `panel::loosest`).
fn loosest(a: &pcb_placer::Relaxation, b: &pcb_placer::Relaxation) -> pcb_placer::Relaxation {
    let mut on_marks: Vec<usize> = a.on_marks.iter().chain(&b.on_marks).copied().collect();
    on_marks.sort_unstable();
    on_marks.dedup();
    pcb_placer::Relaxation {
        spacing: a.spacing.min(b.spacing),
        grid: a.grid.min(b.grid),
        halo_scale: a.halo_scale.min(b.halo_scale),
        small_halo_scale: a.small_halo_scale.min(b.small_halo_scale),
        edge_inset: a.edge_inset || b.edge_inset,
        tight: a.tight || b.tight,
        edge_copper: a.edge_copper || b.edge_copper,
        edge_rule: a.edge_rule.min(b.edge_rule),
        courtyard_spacing: a.courtyard_spacing || b.courtyard_spacing,
        pad_clearance: a.pad_clearance || b.pad_clearance,
        halos: a.halos.clone().or_else(|| b.halos.clone()),
        on_marks,
    }
}

/// A part that takes no room and pulls no net (as `panel::make_inert`).
fn make_inert(component: &mut pcb_placer::Component) {
    component.fixed = true;
    component.side = pcb_placer::Side::Neither;
    component.pins.clear();
    component.far_side.clear();
    component.far_copper.clear();
    component.far_courtyard.clear();
    component.hollow.clear();
    component.pads.clear();
    component.courtyards.clear();
    component.tight_hollow.clear();
    component.hole_rings.clear();
    component.copper_graphics.clear();
    component.copper_only = false;
    component.cutout_outline.clear();
    component.halo = 0.0;
}

/// One sub-board's problem: the outline cut to its region, its movable
/// parts (`active`) the only ones placed; the other movable parts take
/// no room and pull no net; fixed parts away from the region keep their
/// bodies but pull no net; the constraints of the active parts only.
fn region_problem(
    problem: &pcb_placer::Problem,
    poses: &[pcb_placer::Pose],
    active: &BTreeSet<usize>,
    region: [f64; 4],
) -> pcb_placer::Problem {
    let mut sub = problem.clone();
    sub.poses = poses.to_vec();
    for (index, component) in sub.components.iter_mut().enumerate() {
        if active.contains(&index) {
            continue;
        }
        if !component.fixed {
            make_inert(component);
            continue;
        }
        if !component.pins.is_empty() {
            let b = body_box(component, poses[index]);
            if b[0] > region[2] || b[2] < region[0] || b[1] > region[3] || b[3] < region[1] {
                component.pins.clear();
            }
        }
    }
    let pulling: Vec<bool> = (0..sub.components.len())
        .map(|part| active.contains(&part) || !sub.components[part].pins.is_empty())
        .collect();
    sub.outline = clip(&problem.outline, region);
    sub.pieces.clear();
    let constraints = &mut sub.constraints;
    constraints
        .regions
        .retain(|(part, _)| active.contains(part));
    constraints.starts.retain(|(part, _)| active.contains(part));
    constraints
        .edges
        .retain(|(part, _, _)| active.contains(part));
    constraints
        .overhangs
        .retain(|(part, _, _)| active.contains(part));
    constraints.kept.retain(|kept| active.contains(&kept.part));
    constraints
        .followers
        .retain(|follower| active.contains(&follower.leader));
    let user = constraints.relations.len() - constraints.automatic;
    let mut relations = Vec::new();
    let mut automatic = 0;
    for (index, relation) in constraints.relations.drain(..).enumerate() {
        let relation = match relation {
            pcb_placer::constraints::Relation::NearAny { part, anchors, max } => {
                let anchors: Vec<_> = anchors
                    .into_iter()
                    .filter(|anchor| pulling[anchor.part()])
                    .collect();
                if anchors.is_empty() || !active.contains(&part) {
                    continue;
                }
                pcb_placer::constraints::Relation::NearAny { part, anchors, max }
            }
            other => {
                let parts = other.parts();
                if !parts.iter().flatten().all(|part| pulling[*part])
                    || !parts.iter().flatten().any(|part| active.contains(part))
                {
                    continue;
                }
                other
            }
        };
        if index >= user {
            automatic += 1;
        }
        relations.push(relation);
    }
    constraints.relations = relations;
    constraints.automatic = automatic;
    constraints.linked.clear();
    constraints.linked_partners.clear();
    constraints.link_pairs();
    // The measured escape correction routes the whole board.
    if let Some(escape) = &sub.escape
        && escape.measured.is_some()
    {
        let mut estimate = (**escape).clone();
        estimate.measured = None;
        sub.escape = Some(std::sync::Arc::new(estimate));
    }
    sub
}

/// One seed's placement of a panel whose parts the placement holds in
/// their sub-boards (`PlacerApplied::held`): each sub-board with the
/// ordinary placer in turn, alone in its region (the outline cut to it,
/// so that its own fullness decides its levels: the 20 x 20 4in1-mini is
/// 107 % full at the full spacing and places at the tight bodies, which
/// the whole panel's levels, its area not overfull, reached too late),
/// with a share of the seed's budget by its movable parts; then the
/// movable parts without a sub-board with the whole board, the placed
/// ones fixed (the budget left, a tenth at least). The placement reports
/// the loosest level of the sub-boards', field by field.
pub(crate) fn place_sub_boards(
    problem: &pcb_placer::Problem,
    sub: &PlacerSubBoards,
    config: &pcb_placer::Config,
) -> pcb_placer::Placement {
    let started = pcb_placer::work();
    let debug = std::env::var_os("PCB_PLACER_DEBUG").is_some();
    let mut boards: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for (part, piece) in sub.part_piece.iter().enumerate() {
        let component = &problem.components[part];
        if let Some(piece) = piece
            && !component.fixed
        {
            boards.entry(*piece).or_default().insert(part);
        }
    }
    let total: usize = boards.values().map(BTreeSet::len).sum::<usize>().max(1);
    let mut poses = problem.poses.clone();
    let mut relaxation: Option<pcb_placer::Relaxation> = None;
    let mut unplaced: Vec<usize> = Vec::new();
    let mut overfull = false;
    let mut routing_room_failed = false;
    let mut frames = Vec::new();
    let mut global_iterations = 0;
    let mut global_overflow: f64 = 0.0;
    let count = boards.len();
    for (index, (piece, active)) in boards.iter().enumerate() {
        let region_problem = region_problem(problem, &poses, active, sub.pieces.boxes[*piece]);
        let mut board_config = config.clone();
        board_config.work_limit = config.work_limit.map(|limit| {
            if index + 1 == count {
                limit
                    .saturating_sub(pcb_placer::work() - started)
                    .max(limit / 10)
            } else {
                (limit as f64 * active.len() as f64 / total as f64) as u64
            }
        });
        let placed = pcb_placer::place(&region_problem, &board_config);
        if debug {
            eprintln!(
                "sub-boards: board {piece} placed: {} movable, {} unplaced, {:.1} s of work",
                active.len(),
                placed.unplaced.len(),
                placed.work as f64 / pcb_placer::work_per_second()
            );
        }
        for part in active {
            poses[*part] = placed.poses[*part];
        }
        unplaced.extend(placed.unplaced.iter().copied());
        overfull |= placed.overfull;
        routing_room_failed |= placed.routing_room_failed;
        global_iterations += placed.global_iterations;
        global_overflow = global_overflow.max(placed.global_overflow);
        frames.extend(placed.frames.iter().cloned());
        relaxation = Some(match relaxation {
            Some(before) => loosest(&before, &placed.relaxation),
            None => placed.relaxation.clone(),
        });
    }
    let held = poses.clone();
    let (_, unassigned) = sub.unassigned(problem);
    let relaxation = relaxation.unwrap_or_else(|| pcb_placer::place(problem, config).relaxation);
    let (poses, relaxation, overfull) = if unassigned.is_empty() {
        (poses, relaxation, overfull)
    } else {
        // The parts without a sub-board, with the whole board.
        let mut rest = problem.clone();
        rest.poses = poses.clone();
        for active in boards.values() {
            for part in active {
                rest.components[*part].fixed = true;
            }
        }
        let mut rest_config = config.clone();
        rest_config.work_limit = config.work_limit.map(|limit| {
            limit
                .saturating_sub(pcb_placer::work() - started)
                .max(limit / 10)
        });
        let placed = pcb_placer::place(&rest, &rest_config);
        unplaced.extend(placed.unplaced.iter().copied());
        let relaxation = loosest(&relaxation, &placed.relaxation);
        (placed.poses, relaxation, overfull || placed.overfull)
    };
    let mut poses = poses;
    problem.sync_followers(&mut poses);
    unplaced.sort_unstable();
    unplaced.dedup();
    let mut fin = problem.clone();
    relaxation.apply(&mut fin);
    let illegal = pcb_placer::legal::illegal_components(&fin, &poses);
    let constraints = pcb_placer::constraints::report(&fin, &poses);
    let wirelength_final = problem.wirelength(&poses);
    frames.push(pcb_placer::global::Frame {
        iteration: global_iterations + 1,
        overflow: global_overflow,
        wirelength: wirelength_final,
        poses: poses.clone(),
    });
    pcb_placer::Placement {
        frames,
        global_iterations,
        global_overflow,
        wirelength_initial: problem.wirelength(&problem.poses),
        wirelength_global: problem.wirelength(&held),
        wirelength_legal: problem.wirelength(&held),
        wirelength_final,
        unplaced,
        illegal,
        constraints,
        relaxation,
        work: pcb_placer::work() - started,
        stage_poses: vec![("sub-boards", held), ("final", poses.clone())],
        routing_room_failed,
        overfull,
        poses,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two 20 x 20 boards one above the other, joined by a 4 mm tab (with
    /// a row of mouse bites when `bites`); two parts on each, on GND and
    /// SIG; `extra` more items.
    fn panel(bites: bool, extra: &str) -> Expr {
        panel_with(bites, extra, "SIG")
    }

    /// `panel` with the second net named `signal`.
    fn panel_with(bites: bool, extra: &str, signal: &str) -> Expr {
        let outline = "(gr_poly (pts (xy 0 0) (xy 20 0) (xy 20 20) (xy 12 20) (xy 12 25) (xy 20 25) (xy 20 45) (xy 0 45) (xy 0 25) (xy 8 25) (xy 8 20) (xy 0 20)) (layer \"Edge.Cuts\"))";
        let holes: String = if bites {
            (0..5)
                .map(|index| {
                    let x = 8.4 + 0.8 * index as f64;
                    format!(
                        "(gr_circle (center {x} 22.5) (end {} 22.5) (layer \"Edge.Cuts\"))",
                        x + 0.25
                    )
                })
                .collect()
        } else {
            String::new()
        };
        let part = |name: &str, x: f64, y: f64| {
            format!(
                r#"(footprint "{name}" (at {x} {y}) (property "Reference" "{name}") (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "GND")) (pad "2" smd rect (at 2 0) (size 1 1) (layers "F.Cu") (net "{signal}")))"#
            )
        };
        let parts = [
            part("R1", 3.0, 5.0),
            part("R2", 13.0, 5.0),
            part("R1", 3.0, 35.0),
            part("R2", 13.0, 35.0),
        ]
        .join(" ");
        crate::kicad_sexpr::parse(&format!(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user)) {outline} {holes} {parts} {extra})"#
        ))
        .unwrap()
    }

    /// A 20 x 20 board above a 20 x `lower` piece, joined by a bridge
    /// `bridge` mm wide at x 10 (y 20 to 25); `extra` more items.
    fn two_pieces(bridge: f64, lower: f64, extra: &str) -> Expr {
        let (left, right, bottom) = (10.0 - bridge / 2.0, 10.0 + bridge / 2.0, 25.0 + lower);
        let outline = format!(
            "(gr_poly (pts (xy 0 0) (xy 20 0) (xy 20 20) (xy {right} 20) (xy {right} 25) (xy 20 25) (xy 20 {bottom}) (xy 0 {bottom}) (xy 0 25) (xy {left} 25) (xy {left} 20) (xy 0 20)) (layer \"Edge.Cuts\"))"
        );
        crate::kicad_sexpr::parse(&format!(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user) (41 "Cmts.User" user)) {outline} {extra})"#
        ))
        .unwrap()
    }

    /// A footprint of `count` non-plated holes of `drill` mm at `pitch`
    /// along x from `x`, at y 22.5 (a row of mouse bites across the
    /// bridge).
    fn bites(x: f64, count: usize, pitch: f64, drill: f64) -> String {
        let pads: String = (0..count)
            .map(|index| {
                format!(
                    r#"(pad "" np_thru_hole circle (at {} 0) (size {drill} {drill}) (drill {drill}) (layers "*.Cu" "*.Mask"))"#,
                    index as f64 * pitch
                )
            })
            .collect();
        format!(
            r#"(footprint "Panelization:mouse-bites" (at {x} 22.5) (property "Reference" "MB") {pads})"#
        )
    }

    #[test]
    fn tabs_are_narrow_bridges_rows_of_mouse_bites_and_v_cuts() {
        // A bridge of 1.5 mm: the outline's own neck.
        let narrow = Pieces::of_board(&two_pieces(1.5, 20.0, ""))
            .unwrap()
            .expect("pieces");
        assert_eq!(narrow.count(), 2);
        assert_eq!(narrow.cut_by, vec!["narrow outline"]);
        // A solid bridge of 6 mm is board.
        assert!(
            Pieces::of_board(&two_pieces(6.0, 20.0, ""))
                .unwrap()
                .is_none()
        );
        // Crossed by a row of 0.5 mm holes at 0.75 mm (a footprint of
        // non-plated pads, as the ULK's), from edge to edge.
        let row = bites(7.25, 8, 0.75, 0.5);
        let pcb = two_pieces(6.0, 20.0, &row);
        let cuts = Cuts::of_board(&pcb);
        assert_eq!((cuts.holes.len(), cuts.rows), (8, 1));
        let bitten = Pieces::of_board(&pcb).unwrap().expect("pieces");
        assert_eq!(bitten.count(), 2);
        assert_eq!(bitten.cut_by, vec!["mouse bites"]);
        assert!(bitten.between([10.25 + 0.375, 22.5]));
        assert_ne!(bitten.piece_at([10.0, 10.0]), bitten.piece_at([10.0, 35.0]));
        // Holes 1.5 mm apart leave bridges for tracks (protoesp's signal
        // tabs): no row, one board.
        let sparse = two_pieces(6.0, 20.0, &bites(7.25, 4, 1.5, 0.5));
        assert_eq!(Cuts::of_board(&sparse).rows, 0);
        assert!(Pieces::of_board(&sparse).unwrap().is_none());
        // Two holes are no row.
        assert_eq!(
            Cuts::of_board(&two_pieces(6.0, 20.0, &bites(9.0, 2, 0.75, 0.5))).rows,
            0
        );
        // A V-cut: a line on a user layer across the bridge, labelled at
        // its start (as KiKit writes it).
        let line = r#"(gr_line (start -1 22.5) (end 21 22.5) (stroke (width 0.4) (type default)) (layer "Cmts.User"))"#;
        let label = r#"(gr_text "V-CUT" (at -4 22.5 0) (layer "Cmts.User"))"#;
        let scored = two_pieces(6.0, 20.0, &format!("{line} {label}"));
        assert_eq!(Cuts::of_board(&scored).lines.len(), 1);
        let pieces = Pieces::of_board(&scored).unwrap().expect("pieces");
        assert_eq!(pieces.count(), 2);
        assert_eq!(pieces.cut_by, vec!["V-cuts"]);
        assert!(pieces.between([10.0, 22.5]));
        // Without its label the line is a drawing.
        assert!(
            Pieces::of_board(&two_pieces(6.0, 20.0, line))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn a_piece_without_parts_is_a_frame() {
        // The board above, a 5 mm rail below, joined by mouse bites.
        let pcb = two_pieces(6.0, 5.0, &bites(7.25, 8, 0.75, 0.5));
        let pieces = Pieces::of_board(&pcb).unwrap().expect("pieces");
        let board = pieces.piece_at([10.0, 10.0]).unwrap();
        let rail = pieces.piece_at([10.0, 28.0]).unwrap();
        assert_eq!(
            pieces.frames(&BTreeSet::from([board])),
            (0..2).map(|piece| piece == rail).collect::<Vec<_>>()
        );
        // Nothing known: no frame.
        assert_eq!(pieces.frames(&BTreeSet::new()), vec![false, false]);
        // A piece larger than half of those with parts is a board whose
        // parts are all movable, not a frame.
        let pcb = two_pieces(6.0, 15.0, &bites(7.25, 8, 0.75, 0.5));
        let pieces = Pieces::of_board(&pcb).unwrap().expect("pieces");
        let board = pieces.piece_at([10.0, 10.0]).unwrap();
        assert_eq!(pieces.frames(&BTreeSet::from([board])), vec![false, false]);
    }

    #[test]
    fn the_placeable_area_leaves_out_frames_and_tabs() {
        // The board above, a rail below; two placed parts on the board.
        let part = |name: &str, x: f64, y: f64| {
            format!(
                r#"(footprint "{name}" (at {x} {y}) (property "Reference" "{name}") (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "GND")) (pad "2" smd rect (at 2 0) (size 1 1) (layers "F.Cu") (net "SIG")))"#
            )
        };
        let extra = format!(
            "{} {} {}",
            bites(7.25, 8, 0.75, 0.5),
            part("R1", 4.0, 5.0),
            part("R2", 12.0, 15.0)
        );
        let pcb = two_pieces(6.0, 5.0, &extra);
        let config = crate::board_placer::KiCadBoardPlacerConfig::default();
        let mut lowered = crate::board_placer::lower_placement(&pcb, &config).unwrap();
        let footprints = 3;
        let sub = detect_for_placer(&pcb, &lowered.problem, footprints, None)
            .unwrap()
            .expect("pieces");
        assert_eq!(sub.count(&lowered.problem), 1);
        assert!(sub.has_frames());
        let components = lowered.problem.components.len();
        let applied = apply_to_placer(
            &sub,
            &mut lowered.problem,
            &mut lowered.references,
            &mut lowered.source_at,
            &mut lowered.poured,
        );
        assert_eq!(applied.held, 0, "one sub-board: nothing to hold");
        assert!(applied.tabs > 0);
        let keepouts: Vec<[f64; 4]> = (components..lowered.problem.components.len())
            .map(|index| {
                body_box(
                    &lowered.problem.components[index],
                    lowered.problem.poses[index],
                )
            })
            .collect();
        let kept = |point: [f64; 2]| {
            keepouts.iter().any(|b| {
                point[0] >= b[0] && point[0] <= b[2] && point[1] >= b[1] && point[1] <= b[3]
            })
        };
        // The rail, the tab and a margin beside the tab are kept out; the
        // board is not.
        assert!(kept([10.0, 28.0]) && kept([2.0, 28.0]));
        assert!(kept([10.0, 22.5]));
        assert!(kept([10.0, 21.0]), "the margin above the tab: {keepouts:?}");
        assert!(!kept([10.0, 10.0]) && !kept([2.0, 18.0]));
        let mut placer = pcb_placer::Config::new();
        placer.global.seed = 1;
        placer.work_limit = Some((2.0 * pcb_placer::work_per_second()) as u64);
        let placement = pcb_placer::place(&lowered.problem, &placer);
        assert!(placement.unplaced.is_empty() && placement.illegal.is_empty());
        for name in ["R1", "R2"] {
            let part = lowered.references.iter().position(|r| r == name).unwrap();
            assert!(!lowered.problem.components[part].fixed);
            let b = body_box(&lowered.problem.components[part], placement.poses[part]);
            assert!(b[3] < 20.0, "{name} off the board: {b:?}");
        }
    }

    #[test]
    fn mouse_bites_cut_the_outline_into_pieces() {
        let pieces = Pieces::of_board(&panel(true, "")).unwrap().expect("pieces");
        assert_eq!(pieces.count(), 2);
        assert_eq!(pieces.tabs.len(), 1, "{:?} {:?}", pieces.tabs, pieces.boxes);
        let top = pieces.piece_at([10.0, 10.0]).unwrap();
        let bottom = pieces.piece_at([10.0, 35.0]).unwrap();
        assert_ne!(top, bottom);
        assert!(pieces.between([9.6, 22.5]));
        assert!(!pieces.between([30.0, 22.5]));
        // A solid 4 mm tab is part of the board: one piece.
        assert!(Pieces::of_board(&panel(false, "")).unwrap().is_none());
    }

    #[test]
    fn nets_of_one_name_on_two_pieces_are_split_unless_copper_joins_them() {
        let pcb = panel(true, "");
        let pieces = Pieces::of_board(&pcb).unwrap().unwrap();
        assert_eq!(
            board_split_nets(&pcb, &pieces),
            BTreeMap::from([("GND".to_string(), 2), ("SIG".to_string(), 2)])
        );
        let config = crate::board_router::KiCadBoardRouterConfig {
            default_rules: Some(crate::KiCadConnectionRoutingRules {
                trace_width_mm: 0.2,
                clearance_mm: 0.2,
                via_size_mm: 0.6,
                via_drill_mm: 0.3,
            }),
            ..Default::default()
        };
        let board = crate::board_router::lower(&pcb, &config, true)
            .unwrap()
            .board;
        let names: Vec<&str> = board.nets.iter().map(|net| net.name.as_str()).collect();
        assert_eq!(names.iter().filter(|name| **name == "GND").count(), 2);
        assert_eq!(names.iter().filter(|name| **name == "SIG").count(), 2);
        for (index, net) in board.nets.iter().enumerate() {
            assert_eq!(net.terminals.len(), 2, "{}", net.name);
            let ends: BTreeSet<Option<usize>> = net
                .terminals
                .iter()
                .map(|terminal| pieces.piece_at(terminal.anchor))
                .collect();
            assert_eq!(ends.len(), 1, "{}", net.name);
            for terminal in &net.terminals {
                assert_eq!(
                    board.obstacles[terminal.pad].net,
                    Some(index as core::NetId)
                );
            }
        }
        // A track of SIG over the tab joins it: SIG stays one net.
        let joined = panel(
            true,
            r#"(segment (start 10 10) (end 10 40) (width 0.2) (layer "B.Cu") (net "SIG"))"#,
        );
        assert_eq!(
            board_split_nets(&joined, &pieces),
            BTreeMap::from([("GND".to_string(), 2)])
        );
        let board = crate::board_router::lower(&joined, &config, true)
            .unwrap()
            .board;
        assert_eq!(board.nets.iter().filter(|net| net.name == "SIG").count(), 1);
        // Switched off: one net each.
        let off = crate::board_router::KiCadBoardRouterConfig {
            sub_board_nets: Some(false),
            ..config.clone()
        };
        let board = crate::board_router::lower(&pcb, &off, true).unwrap().board;
        assert_eq!(board.nets.iter().filter(|net| net.name == "GND").count(), 1);
    }

    #[test]
    fn kicad_items_across_two_sub_boards_are_not_counted() {
        let pcb = panel(true, "");
        let pieces = Pieces::of_board(&pcb).unwrap().unwrap();
        let item = |net: &str, a: [f64; 2], b: [f64; 2]| {
            serde_json::json!({"type": "unconnected_items", "items": [
                {"description": format!("Pad 1 [{net}] of R1 on F.Cu"), "pos": {"x": a[0], "y": a[1]}},
                {"description": format!("Pad 1 [{net}] of R2 on F.Cu"), "pos": {"x": b[0], "y": b[1]}}]})
        };
        let mut report = serde_json::json!({"unconnected_items": [
            item("GND", [3.0, 5.0], [3.0, 35.0]),
            // A second GND edge across: an island within a piece left open.
            item("GND", [13.0, 5.0], [13.0, 35.0]),
            item("SIG", [5.0, 5.0], [15.0, 5.0]),
        ]});
        let mut only_sig = report.clone();
        assert_eq!(excuse_across(&mut report, &pcb, &pieces, None), 1);
        assert_eq!(report["unconnected_items"].as_array().unwrap().len(), 2);
        assert_eq!(report[ACROSS_KEY].as_array().unwrap().len(), 1);
        // A placement that knew GND not to be on both boards by design:
        // its items across count.
        let designed = BTreeSet::from(["SIG".to_string()]);
        assert_eq!(
            excuse_across(&mut only_sig, &pcb, &pieces, Some(&designed)),
            0
        );
    }

    #[test]
    fn a_placed_panel_holds_each_part_on_its_board() {
        // A hierarchical name: the placer's connection drops the slash,
        // KiCad's report keeps it.
        let pcb = panel_with(true, "", "/S/SIG");
        let config = crate::board_placer::KiCadBoardPlacerConfig::default();
        let mut lowered = crate::board_placer::lower_placement(&pcb, &config).unwrap();
        let sub = detect_for_placer(&pcb, &lowered.problem, 4, None)
            .unwrap()
            .expect("pieces");
        assert_eq!(sub.count(&lowered.problem), 2);
        assert_eq!(sub.from, vec!["placed input"]);
        assert_eq!(sub.split.len(), 2, "GND and SIG on both boards");
        assert_ne!(sub.part_piece[0], sub.part_piece[2], "the two R1 apart");
        let designed = designed_nets(&pcb, &sub, &lowered.net_names);
        assert_eq!(
            designed,
            BTreeSet::from(["/S/SIG".to_string(), "GND".to_string()])
        );
        let applied = apply_to_placer(
            &sub,
            &mut lowered.problem,
            &mut lowered.references,
            &mut lowered.source_at,
            &mut lowered.poured,
        );
        assert_eq!((applied.held, applied.nets_split), (4, 2));
        assert!(applied.tabs >= 1);
        // The two boards' R1 no longer share a net.
        let nets = |part: usize| -> Vec<usize> {
            lowered.problem.components[part]
                .pins
                .iter()
                .map(|pin| pin.net)
                .collect()
        };
        assert!(nets(0).iter().all(|net| !nets(2).contains(net)));
        let mut placer = pcb_placer::Config::new();
        placer.global.seed = 1;
        placer.work_limit = Some((2.0 * pcb_placer::work_per_second()) as u64);
        let placement = place_sub_boards(&lowered.problem, &sub, &placer);
        assert!(placement.unplaced.is_empty() && placement.illegal.is_empty());
        for part in 0..4 {
            let piece = sub.part_piece[part].unwrap();
            let at = placement.poses[part].position;
            assert_eq!(
                sub.pieces.piece_at(at),
                Some(piece),
                "part {part} at {at:?}"
            );
        }
    }

    #[test]
    fn a_panels_cells_split_only_the_nets_named_by_design() {
        // One outline piece (a solid tab): no pieces; the cells a
        // placement found split the nets it names, GND here, alone.
        let pcb = panel(false, "");
        let config = crate::board_router::KiCadBoardRouterConfig {
            default_rules: Some(crate::KiCadConnectionRoutingRules {
                trace_width_mm: 0.2,
                clearance_mm: 0.2,
                via_size_mm: 0.6,
                via_drill_mm: 0.3,
            }),
            ..Default::default()
        };
        let mut board = crate::board_router::lower(&pcb, &config, true)
            .unwrap()
            .board;
        assert_eq!(board.nets.len(), 2);
        let cells = Pieces::of_boxes(vec![[0.0, 0.0, 20.0, 22.0], [0.0, 23.0, 20.0, 45.0]]);
        assert_eq!(cells.piece_at([5.0, 30.0]), Some(1));
        let only = BTreeSet::from(["GND".to_string()]);
        let split = split_router_nets(&mut board, &cells, Some(&only));
        assert_eq!(split.nets, vec![("GND".to_string(), 2)]);
        assert_eq!(board.nets.iter().filter(|net| net.name == "GND").count(), 2);
        assert_eq!(board.nets.iter().filter(|net| net.name == "SIG").count(), 1);
    }
}
