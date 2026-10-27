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

/// No piece: the cell's label where the opened area does not reach.
const NONE: u16 = u16::MAX;

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
}

impl Pieces {
    /// The pieces of a board's outline, or None where it has fewer than
    /// two.
    pub(crate) fn of_board(pcb: &Expr) -> Result<Option<Pieces>, String> {
        let loops = outline::board_loops(pcb)?;
        Ok(Pieces::of_loops(&loops.piece_loops, &loops.cutouts))
    }

    /// The pieces of the area inside `loops` (even-odd with `cutouts`).
    pub(crate) fn of_loops(loops: &[Vec<[f64; 2]>], cutouts: &[Vec<[f64; 2]>]) -> Option<Pieces> {
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
        // Distance to the outside (chamfer 3-4, in thirds of a cell).
        let far = u32::MAX / 2;
        let mut distance: Vec<u32> = inside.iter().map(|&i| if i { far } else { 0 }).collect();
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
            if !inside[index] || grown[index] > radius + 3 {
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
            seen[start] = true;
            stack.push(start);
            while let Some(at) = stack.pop() {
                let (row, column) = (at / columns, at % columns);
                b = [b[0].min(column), b[1].min(row), b[2].max(column), b[3].max(row)];
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
            if touching.len() >= 2 {
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
            loop {
                let Some(index) = merged.iter().position(|other| near(other, &tab)) else {
                    break;
                };
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
        Some(Pieces {
            origin,
            cell,
            columns,
            rows,
            labels,
            inside,
            boxes,
            tabs,
        })
    }

    pub(crate) fn count(&self) -> usize {
        self.boxes.len()
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
        let cell = self.cell_of(point)?;
        (self.labels[cell] != NONE).then_some(self.labels[cell] as usize)
    }

    /// The piece `point` lies on or, off the board, the nearest piece
    /// within `reach` mm (a zone's corner beyond the edge, as KiCad places
    /// a zone's unconnected item); None on a tab or a neck.
    pub(crate) fn piece_near(&self, point: [f64; 2], reach: f64) -> Option<usize> {
        if let Some(piece) = self.piece_at(point) {
            return Some(piece);
        }
        if self.between(point) {
            return None;
        }
        let steps = (reach / self.cell).ceil() as i64;
        let mut best: Option<(f64, usize)> = None;
        for dy in -steps..=steps {
            for dx in -steps..=steps {
                let at = [
                    point[0] + dx as f64 * self.cell,
                    point[1] + dy as f64 * self.cell,
                ];
                let distance = ((dx * dx + dy * dy) as f64).sqrt() * self.cell;
                if distance > reach || best.is_some_and(|(d, _)| d <= distance) {
                    continue;
                }
                if let Some(piece) = self.piece_at(at) {
                    best = Some((distance, piece));
                }
            }
        }
        best.map(|(_, piece)| piece)
    }

    /// Whether `point` lies on the board but on no piece (a tab, a neck).
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
        |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])],
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
pub(crate) fn split_router_nets(board: &mut core::Board, pieces: &Pieces) -> NetSplit {
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
        if board.nets[net].terminals.len() < 2 {
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
                board.obstacles[*index].net = Some(id_of_piece.get(piece).copied().unwrap_or(net as core::NetId));
            }
        }
        for merged in board.merged_pads.iter_mut().filter(|m| m.net as usize == net) {
            if let Some(&(id, index)) = moved.get(merged.host) {
                merged.net = id;
                merged.host = index;
                if let Some(obstacle) = board.obstacles.get_mut(merged.pad) {
                    obstacle.net = Some(id);
                }
            }
        }
        for copper in board.terminal_copper.iter_mut().filter(|c| c.net as usize == net) {
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
                for pad in item.children().iter().filter(|child| child.head() == Some("pad")) {
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
        .map(|(net, pieces)| (net, pieces.len()))
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
pub(crate) fn excuse_across(report: &mut serde_json::Value, pcb: &Expr, pieces: &Pieces) -> usize {
    let Some(items) = report["unconnected_items"].as_array() else {
        return 0;
    };
    if items.is_empty() {
        return 0;
    }
    let split = board_split_nets(pcb, pieces);
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
                    [Some(x), Some(y)] => pieces.piece_near([x, y], NECK_RADIUS_MM),
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
    let moved = across.len();
    if moved > 0 {
        report["unconnected_items"] = serde_json::Value::Array(kept);
        report[ACROSS_KEY] = serde_json::Value::Array(across);
    }
    moved
}

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
    let Some(pieces) = Pieces::of_board(&pcb)? else {
        return Ok(0);
    };
    let moved = excuse_across(&mut value, &pcb, &pieces);
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
    ) -> Option<std::sync::Arc<Pieces>> {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for polygon in loops.iter().chain(std::iter::once(&Vec::new())).chain(cutouts) {
            polygon.len().hash(&mut hasher);
            for point in polygon {
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
        let pieces = Pieces::of_loops(loops, cutouts).map(std::sync::Arc::new);
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
    let Some(pieces) = Pieces::of_loops(&loops.piece_loops, &loops.cutouts) else {
        return Ok(serde_json::json!({"pieces": 1, "outline_loops": loops.piece_loops.len()}));
    };
    let split = board_split_nets(&pcb, &pieces);
    Ok(serde_json::json!({
        "pieces": pieces.count(),
        "outline_loops": loops.piece_loops.len(),
        "boxes": pieces.boxes.iter().map(round).collect::<Vec<_>>(),
        "tabs": pieces.tabs.iter().map(round).collect::<Vec<_>>(),
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
    let Some(pieces) = Pieces::of_loops_cached(&loops.piece_loops, &loops.cutouts) else {
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
    for part in 0..footprints {
        let component = &problem.components[part];
        if component.fixed
            && let Some(Some(piece)) = pieces.piece_of_box(body_box(component, problem.poses[part]))
        {
            part_piece[part] = Some(piece);
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
            .filter_map(|item| Some((crate::kicad_nets::footprint_uuid(item)?, form_at(item).ok()?)))
            .collect();
        for part in 0..footprints {
            if part_piece[part].is_none()
                && let Some(Some(uuid)) = uuids.get(part)
                && let Some(at) = places.get(uuid)
                && let Some(piece) = pieces.piece_at([at[0], at[1]])
            {
                part_piece[part] = Some(piece);
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
                .entry(((at[0] * 100.0).round() as i64, (at[1] * 100.0).round() as i64))
                .or_default() += 1;
        }
        let alone = movable
            .iter()
            .filter(|part| {
                let at = problem.poses[**part].position;
                seen[&((at[0] * 100.0).round() as i64, (at[1] * 100.0).round() as i64)] == 1
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
    // The nets: on two pieces by the parts placed so far, split.
    let net_count = problem.net_weights.len();
    let mut on: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); net_count];
    for part in 0..footprints {
        if let Some(piece) = part_piece[part] {
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
    for part in 0..footprints {
        if let Some(piece) = part_piece[part] {
            let group = root(&mut parent, part);
            group_pieces.entry(group).or_default().insert(piece);
        }
    }
    let conflicts = group_pieces.values().filter(|pieces| pieces.len() > 1).count();
    let mut by_nets = false;
    for part in 0..footprints {
        if part_piece[part].is_none() && with_pins(part) {
            let group = root(&mut parent, part);
            if let Some(pieces) = group_pieces.get(&group)
                && pieces.len() == 1
            {
                part_piece[part] = pieces.iter().next().copied();
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
        from: order.into_iter().filter(|kind| from.contains(kind)).collect(),
        conflicts,
    }))
}

impl PlacerSubBoards {
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
    /// Tab keepouts added.
    pub tabs: usize,
}

/// The sub-boards in the placement problem (on a panel of different
/// boards: two sub-boards or more, nets on several of them):
///
/// - no part's body on a tab: each tab a keepout (an obstacle of the
///   board, as a rule area's);
/// - where at most `MOST_UNASSIGNED_SHARE` of the movable parts have no
///   sub-board: each other movable part held in its sub-board's region
///   (unless a constraint holds it in one already) and started at its
///   centre, and each split net one net per sub-board (the parts without
///   one stay on the first's), its weight as the lowering gives a net of
///   that many pins.
pub(crate) fn apply_to_placer(
    sub: &PlacerSubBoards,
    problem: &mut pcb_placer::Problem,
    references: &mut Vec<String>,
    source_at: &mut Vec<[f64; 3]>,
    poured: &mut Vec<bool>,
) -> PlacerApplied {
    let mut applied = PlacerApplied::default();
    if sub.split.is_empty() || sub.count(problem) < 2 {
        return applied;
    }
    for tab in &sub.pieces.tabs {
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
        });
        problem.poses.push(pcb_placer::Pose {
            position: [(tab[0] + tab[2]) / 2.0, (tab[1] + tab[3]) / 2.0],
            angle: 0.0,
        });
        references.push("tab".into());
        source_at.push([0.0; 3]);
        applied.tabs += 1;
    }
    let (movable, unassigned) = sub.unassigned(problem);
    if unassigned.len() as f64 > MOST_UNASSIGNED_SHARE * movable as f64 {
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
            problem
                .constraints
                .starts
                .push((part, [(region[0] + region[2]) / 2.0, (region[1] + region[3]) / 2.0]));
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
    /// a tenth of the movable parts without one), and the tabs it kept
    /// parts off.
    pub held: bool,
    pub tabs: Vec<[f64; 4]>,
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
                .map(|part| names.get(*part).or(references.get(*part)).cloned().unwrap_or_default())
                .collect(),
            conflicts: sub.conflicts,
            held: applied.held > 0,
            tabs: sub.pieces.tabs.iter().map(round).collect(),
            sub_boards,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two 20 x 20 boards one above the other, joined by a 4 mm tab (with
    /// a row of mouse bites when `bites`); two parts on each, on GND and
    /// SIG; `extra` more items.
    fn panel(bites: bool, extra: &str) -> Expr {
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
                r#"(footprint "{name}" (at {x} {y}) (property "Reference" "{name}") (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "GND")) (pad "2" smd rect (at 2 0) (size 1 1) (layers "F.Cu") (net "SIG")))"#
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
        let board = crate::board_router::lower(&pcb, &config, true).unwrap().board;
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
                assert_eq!(board.obstacles[terminal.pad].net, Some(index as core::NetId));
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
        let board = crate::board_router::lower(&joined, &config, true).unwrap().board;
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
        assert_eq!(excuse_across(&mut report, &pcb, &pieces), 1);
        assert_eq!(report["unconnected_items"].as_array().unwrap().len(), 2);
        assert_eq!(report[ACROSS_KEY].as_array().unwrap().len(), 1);
    }
}
