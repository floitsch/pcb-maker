// Copyright (C) 2026 Toit contributors.

//! The outline's pieces: a panel of different boards (the OpenESC 4in1
//! panel: a 30 x 30 and a 20 x 20 ESC joined by tabs with mouse bites)
//! shares net names between its boards (GND, +BATT, the ESC sheets' nets),
//! but no copper joins them: KiCad reports one unconnected item per such
//! net on the designer's own board (130 on the 4in1 panel), and after the
//! tabs are broken they are different nets. Placed and routed as one
//! board, the parts interleave and the router tries to join the same-named
//! nets across the tabs (docs/placer.md, "Sub-boards in a panel").
//! `sub_boards` places them; this module finds them.
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

use super::*;
use crate::kicad_sexpr::{Expr, form_at};
use pcb_router as core;

/// A panel of copies as the keepouts see it (`Pieces::copies_keepouts`):
/// per copy its transform from the first copy (a turn by `angle`
/// degrees, as `pcb_placer::problem::rotate` turns, then `offset`), its
/// surroundings (`within`: where its own parts may lie) and the bands
/// where the other copies' keepouts are not carried (`held`: along a
/// board edge one of its parts is held at, as deep as the part and its
/// distance: OpenRX's second copy holds its crystal X301 within 1.25 mm
/// of the right edge, where the first copy's frame would come).
#[derive(Clone, Debug, Default)]
pub(crate) struct CopyFrames {
    pub transforms: Vec<(f64, [f64; 2])>,
    pub within: Vec<[f64; 4]>,
    pub held: Vec<Vec<[f64; 4]>>,
}

/// `a` less `b`: up to four boxes.
fn subtract_box(a: [f64; 4], b: [f64; 4]) -> Vec<[f64; 4]> {
    let cut = clip_box(a, b);
    if !(cut[0] < cut[2] && cut[1] < cut[3]) {
        return vec![a];
    }
    [
        [a[0], a[1], a[2], cut[1]],
        [a[0], cut[3], a[2], a[3]],
        [a[0], cut[1], cut[0], cut[3]],
        [cut[2], cut[1], a[2], cut[3]],
    ]
    .into_iter()
    .filter(|b| b[0] < b[2] && b[1] < b[3])
    .collect()
}

impl CopyFrames {
    /// A point of copy `copy` in the first copy's frame.
    fn carried_back(&self, copy: usize, point: [f64; 2]) -> [f64; 2] {
        let (angle, offset) = self.transforms[copy];
        pcb_placer::problem::rotate([point[0] - offset[0], point[1] - offset[1]], angle)
    }

    /// A point of the first copy in copy `copy`'s frame.
    fn carried_out(&self, copy: usize, point: [f64; 2]) -> [f64; 2] {
        let (angle, offset) = self.transforms[copy];
        let turned = pcb_placer::problem::rotate(point, -angle);
        [turned[0] + offset[0], turned[1] + offset[1]]
    }
}

pub(crate) fn clip_box(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    [
        a[0].max(b[0]),
        a[1].max(b[1]),
        a[2].min(b[2]),
        a[3].min(b[3]),
    ]
}

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

/// The blocks of the frames and tabs themselves (mm, `Pieces::copper`):
/// a connector reaching over them keeps its copper off these.
const COPPER_BLOCK_MM: f64 = 0.25;

/// How far parts keep from a tab beyond the board the opening took from
/// it (mm): a row of mouse bites breaks along its holes, its edge rough
/// (the keepout then reaches about 1.6 mm from the row's centre line,
/// 1.4 mm from 0.5 mm holes' edges).
pub(crate) const TAB_MARGIN_MM: f64 = 1.5;

/// Frames and rails together take at most this share of the area of the
/// pieces parts lie on (the ULK panels' frames: about a sixth).
const MOST_FRAME_SHARE: f64 = 0.5;

/// A piece spanning this share of the panel along a side, holding at most
/// `RAIL_AREA_SHARE` of its area, is a rail or frame by the outline alone
/// (`Pieces::outline_frames`).
const RAIL_SPAN: f64 = 0.9;
const RAIL_AREA_SHARE: f64 = 0.2;

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

/// The tabs of V-cuts (`members`: the cells of the tab components with a
/// scored cell, of a raster `columns` wide at `origin` with cells of
/// `cell` mm): on a panel a grid of cut lines, whose components' boxes
/// reach over the boards (trial 5's SmartSpin2k: one tab `[15, 15, 204.5,
/// 219.9]`). The tabs are the cut lines instead: per line of `lines` that
/// passes them, the box of their cells within `NECK_RADIUS_MM` of the line
/// (the line and the erosion's fringe beside it). `None` where no line
/// passes them.
fn cut_line_tabs(
    members: &[usize],
    columns: usize,
    origin: [f64; 2],
    cell: f64,
    lines: &[[[f64; 2]; 2]],
) -> Option<Vec<[f64; 4]>> {
    let centre = |at: usize| {
        [
            origin[0] + ((at % columns) as f64 + 0.5) * cell,
            origin[1] + ((at / columns) as f64 + 0.5) * cell,
        ]
    };
    let mut boxes = Vec::new();
    for [a, b] in lines {
        let along = [b[0] - a[0], b[1] - a[1]];
        let length = along[0].hypot(along[1]);
        if length <= 0.0 {
            continue;
        }
        let mut found = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for at in members {
            let point = centre(*at);
            let t = (((point[0] - a[0]) * along[0] + (point[1] - a[1]) * along[1])
                / (length * length))
                .clamp(0.0, 1.0);
            let near = [a[0] + t * along[0], a[1] + t * along[1]];
            if (point[0] - near[0]).hypot(point[1] - near[1]) <= NECK_RADIUS_MM {
                found = [
                    found[0].min(point[0] - cell / 2.0),
                    found[1].min(point[1] - cell / 2.0),
                    found[2].max(point[0] + cell / 2.0),
                    found[3].max(point[1] + cell / 2.0),
                ];
            }
        }
        if found[0].is_finite() {
            boxes.push(found);
        }
    }
    (!boxes.is_empty()).then_some(boxes)
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
        // The cells of the tabs V-cuts make, which the cut lines report
        // (`cut_line_tabs`).
        let mut scored_members: Vec<usize> = Vec::new();
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
                let to_mm = |b: [usize; 4]| {
                    [
                        origin[0] + b[0] as f64 * cell,
                        origin[1] + b[1] as f64 * cell,
                        origin[0] + (b[2] + 1) as f64 * cell,
                        origin[1] + (b[3] + 1) as f64 * cell,
                    ]
                };
                // A grid of V-cuts makes components whose boxes reach
                // over the boards (trial 5's SmartSpin2k: one tab [15,
                // 15, 204.5, 219.9]): their cut lines are the tabs
                // (`cut_line_tabs`), one each, not merged below.
                if members.iter().any(|at| scored[*at]) {
                    scored_members.extend(members);
                } else {
                    tabs.push(to_mm(b));
                }
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
        merged.extend(
            cut_line_tabs(&scored_members, columns, origin, cell, &cuts.lines).unwrap_or_default(),
        );
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
        self.mask_boxes(&self.kept_mask(frames, margin), KEEPOUT_BLOCK_MM)
    }

    /// The tabs and the frames themselves (`keepouts` not grown), in
    /// blocks of `COPPER_BLOCK_MM`: what a connector reaching over the
    /// keepouts keeps its copper off.
    pub(crate) fn copper(&self, frames: &[bool]) -> Vec<[f64; 4]> {
        self.mask_boxes(&self.kept_mask(frames, 0.0), COPPER_BLOCK_MM)
    }

    /// `keepouts` and `copper` on a panel of copies (`CopyFrames`): each
    /// copy keeps off what every copy keeps off, carried into its own
    /// frame, so that one placement of the first copy is legal in all
    /// (a top copy's rail is a strip along the top of every copy). The
    /// first copy's cells within its surroundings are kept where the
    /// point, carried to any copy, lies on a kept cell within that copy's;
    /// those cells in blocks make the first copy's boxes, and each other
    /// copy takes their exact images (cut to its surroundings). The kept
    /// cells within no copy's surroundings (the frames beyond) are boxes
    /// of their own. Within a copy's held bands (`CopyFrames::held`) it
    /// keeps its own keepouts only.
    pub(crate) fn copies_keepouts(
        &self,
        frames: &[bool],
        copies: &CopyFrames,
        margin: f64,
    ) -> (Vec<[f64; 4]>, Vec<[f64; 4]>) {
        (
            self.spread(&self.kept_mask(frames, margin), KEEPOUT_BLOCK_MM, copies),
            self.spread(&self.kept_mask(frames, 0.0), COPPER_BLOCK_MM, copies),
        )
    }

    fn spread(&self, mask: &[bool], block: f64, copies: &CopyFrames) -> Vec<[f64; 4]> {
        if self.boxed() || copies.within.is_empty() {
            return self.mask_boxes(mask, block);
        }
        let inside = |point: [f64; 2], b: &[f64; 4]| {
            point[0] >= b[0] && point[0] <= b[2] && point[1] >= b[1] && point[1] <= b[3]
        };
        let kept_at = |point: [f64; 2]| self.cell_of(point).is_some_and(|cell| mask[cell]);
        let mut first = vec![false; mask.len()];
        let mut rest = vec![false; mask.len()];
        for row in 0..self.rows {
            for column in 0..self.columns {
                let at = row * self.columns + column;
                let point = [
                    self.origin[0] + (column as f64 + 0.5) * self.cell,
                    self.origin[1] + (row as f64 + 0.5) * self.cell,
                ];
                if mask[at] && !copies.within.iter().any(|b| inside(point, b)) {
                    rest[at] = true;
                }
                if !inside(point, &copies.within[0]) {
                    continue;
                }
                let local = copies.carried_back(0, point);
                first[at] = (0..copies.within.len()).any(|copy| {
                    let there = copies.carried_out(copy, local);
                    inside(there, &copies.within[copy]) && kept_at(there)
                });
            }
        }
        let mut boxes = self.mask_boxes(&rest, block);
        // Within a copy's held bands only its own keepouts.
        let own = self.mask_boxes(mask, block);
        for (copy, bands) in copies.held.iter().enumerate() {
            for band in bands {
                let band = clip_box(*band, copies.within[copy]);
                for b in &own {
                    let cut = clip_box(*b, band);
                    if cut[0] < cut[2] && cut[1] < cut[3] {
                        boxes.push(cut);
                    }
                }
            }
        }
        let first_boxes: Vec<[f64; 4]> = self
            .mask_boxes(&first, block)
            .into_iter()
            .map(|b| clip_box(b, copies.within[0]))
            .filter(|b| b[0] < b[2] && b[1] < b[3])
            .collect();
        for copy in 0..copies.within.len() {
            for b in &first_boxes {
                let image = if copy == 0 {
                    *b
                } else {
                    let corners = [[b[0], b[1]], [b[2], b[1]], [b[2], b[3]], [b[0], b[3]]]
                        .map(|corner| copies.carried_out(copy, copies.carried_back(0, corner)));
                    corners.iter().fold(
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
                };
                let mut pieces = vec![clip_box(image, copies.within[copy])];
                for band in copies.held.get(copy).into_iter().flatten() {
                    pieces = pieces
                        .into_iter()
                        .flat_map(|piece| subtract_box(piece, *band))
                        .collect();
                }
                boxes.extend(
                    pieces
                        .into_iter()
                        .filter(|image| image[0] < image[2] && image[1] < image[3]),
                );
            }
        }
        boxes
    }

    /// Per cell: the board on no piece (a tab) or on a piece `frames`
    /// marks, or within `margin` of such a cell, on the board.
    fn kept_mask(&self, frames: &[bool], margin: f64) -> Vec<bool> {
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
        distance
            .iter()
            .zip(&self.inside)
            .map(|(near, inside)| *inside && *near <= reach)
            .collect()
    }

    /// The cells of `mask` as boxes: in blocks of `block` mm (a block any
    /// such cell lies in is kept whole), the blocks of one row joined into
    /// runs and equal runs of consecutive rows into one box.
    fn mask_boxes(&self, mask: &[bool], block: f64) -> Vec<[f64; 4]> {
        if self.boxed() {
            return Vec::new();
        }
        let (columns, rows, cell) = (self.columns, self.rows, self.cell);
        let per = ((block / cell).round() as usize).max(1);
        let (block_columns, block_rows) = (columns.div_ceil(per), rows.div_ceil(per));
        let mut blocked = vec![false; block_columns * block_rows];
        for (at, kept) in mask.iter().enumerate() {
            if *kept {
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

    /// The pieces that look like a frame or rails by the outline alone,
    /// where no part says which pieces hold boards (a netlist import's
    /// stack): a piece whose box spans at least `RAIL_SPAN` of the
    /// panel's along one side and that holds at most `RAIL_AREA_SHARE`
    /// of its area (Goga64 ULK's top and bottom rails: 0.07 and 0.17 of
    /// the area, the full width; its halves 0.38 each). Reported, not
    /// kept off: the placer keeps parts off frames only where parts tell
    /// them (`frames`).
    pub(crate) fn outline_frames(&self) -> Vec<bool> {
        let none = vec![false; self.count()];
        if self.count() < 2 || self.boxed() {
            return none;
        }
        let all = self.boxes.iter().fold(
            [
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ],
            |a, b| {
                [
                    a[0].min(b[0]),
                    a[1].min(b[1]),
                    a[2].max(b[2]),
                    a[3].max(b[3]),
                ]
            },
        );
        let total: f64 = self.areas.iter().sum();
        let frames: Vec<bool> = self
            .boxes
            .iter()
            .zip(&self.areas)
            .map(|(b, area)| {
                let spans = (b[2] - b[0]) >= RAIL_SPAN * (all[2] - all[0])
                    || (b[3] - b[1]) >= RAIL_SPAN * (all[3] - all[1]);
                spans && *area <= RAIL_AREA_SHARE * total
            })
            .collect();
        // Some piece must be a board.
        if frames.iter().all(|frame| *frame) {
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
pub(crate) fn clip(polygon: &[[f64; 2]], region: [f64; 4]) -> Vec<[f64; 2]> {
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

/// A net's name as KiCad's reports write it: the board file's string
/// escapes a quote and a backslash (`unescape`), and its text escapes `/`
/// as `{slash}` and so on (KiCad's EscapeString); the DRC report does
/// neither.
fn unescaped(name: &str) -> String {
    let name = unescape(name);
    if !name.contains('{') {
        return name.into_owned();
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
        crate::fs_util::write_atomically(report, text.as_bytes())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rails_are_frames_by_their_shape_alone() {
        // Two boards between a top and a bottom rail spanning the panel.
        let pieces = Pieces {
            origin: [0.0, 0.0],
            cell: 1.0,
            columns: 1,
            rows: 1,
            labels: vec![0],
            inside: vec![true],
            boxes: vec![
                [0.0, 0.0, 100.0, 8.0],
                [0.0, 10.0, 48.0, 60.0],
                [52.0, 10.0, 100.0, 60.0],
                [0.0, 62.0, 100.0, 70.0],
            ],
            tabs: Vec::new(),
            areas: vec![800.0, 2400.0, 2400.0, 800.0],
            cut_by: vec!["mouse bites"],
        };
        assert_eq!(pieces.outline_frames(), vec![true, false, false, true]);
        // Two boards side by side, one spanning the panel's height: no
        // frame (each holds half the area).
        let pieces = Pieces {
            boxes: vec![[0.0, 0.0, 48.0, 60.0], [52.0, 0.0, 100.0, 60.0]],
            areas: vec![2880.0, 2880.0],
            ..pieces
        };
        assert_eq!(pieces.outline_frames(), vec![false, false]);
    }
}
