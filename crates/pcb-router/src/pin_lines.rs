// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Lattice lines where the pins are (docs/router.md, "Lattice lines at
//! pin rows (2026-11-07)"). The regular lattice has one pitch and one
//! phase per axis (`Grid::choose`); a pin row whose rules are an exact
//! fit (a 0.4 mm QFN row at 0.2 mm track and clearance) can only be left
//! along a line through its pads' centres, and a channel between two pads
//! that holds two necked tracks only along two lines far enough apart.
//! Where the regular lattice has no line within the slack such a channel
//! leaves, the row is a wall.
//!
//! `pin_lines` finds the lines the pins ask for: per pad of a row its
//! straight exit's band (between the neighbours' edges, minus the
//! clearances and the half track, inside the pad) and per gap between two
//! facing pads the band a track passing between them keeps (as many
//! tracks as the band holds, spread evenly). A line is asked for only
//! where its slack is under half a pitch: any lattice of that pitch has a
//! line within a wider slack. `missing` says which the regular lattice
//! misses.

use crate::board::{Board, ObstacleKind};
use crate::geometry::Aabb;

/// Two pads face each other across a gap of at most this much, mm, to
/// make a row or a channel.
pub const ROW_GAP: f64 = 0.8;

/// A pad wider than this on both axes is no pin (an exposed pad, a
/// connector's shell): it walls a row but asks for no exit line.
pub const PIN_SIZE: f64 = 1.2;

/// A part of fewer copper pads has no row (a passive's two pads, a
/// SOT-23): its pads wall rows but ask for no line (the fan-out plan's
/// `MIN_PART_PADS`).
pub const MIN_PART_PADS: usize = 4;

/// The part of a pad's label ("U19.12" -> "U19").
pub fn part_of(label: &str) -> &str {
    label.rsplit_once('.').map_or(label, |(part, _)| part)
}

/// Positions are whole nanometres: a line within this of a needed
/// position is on it.
pub const ON_LINE: f64 = 0.5e-6;

/// What a line is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    /// A pad's straight exit out of its row.
    Exit,
    /// A track passing between two facing pads.
    Channel,
}

/// A lattice line a pin asks for: on `axis` (0: a line of constant x,
/// along which tracks run in y), at `at`, any line within `slack` of it
/// will do. `span` is the band's extent along the other axis.
#[derive(Clone, Debug)]
pub struct PinLine {
    pub axis: usize,
    pub at: f64,
    pub slack: f64,
    pub span: [f64; 2],
    pub kind: LineKind,
    /// The pad (its obstacle's label) the line is for.
    pub label: String,
}

struct Pad {
    bounds: Aabb,
    /// Its part has `MIN_PART_PADS` or more: it may ask for lines.
    row: bool,
    /// The track's half width and its clearance to the neighbours, per
    /// width the pad's net may leave with (the class's, then the neck).
    widths: Vec<f64>,
    clearance: f64,
    layers: u32,
    pin: bool,
    exits: bool,
    centre: [f64; 2],
    label: String,
}

/// The lines the board's pin rows and pad gaps ask for at `pitch`: those
/// whose slack is under half a pitch.
pub fn pin_lines(board: &Board, pitch: f64) -> Vec<PinLine> {
    let default_clearance = board
        .classes
        .iter()
        .map(|class| class.clearance)
        .fold(f64::INFINITY, f64::min);
    if !default_clearance.is_finite() {
        return Vec::new();
    }
    let neck = (board.neck_width > 0.0).then_some(board.neck_width);
    // Which obstacle is which net's pad.
    let mut pad_net: Vec<Option<(usize, [f64; 2])>> = vec![None; board.obstacles.len()];
    for (net, description) in board.nets.iter().enumerate() {
        for terminal in &description.terminals {
            if let Some(slot) = pad_net.get_mut(terminal.pad) {
                *slot = Some((net, terminal.anchor));
            }
        }
    }
    let mut part_pads: std::collections::HashMap<&str, usize> = Default::default();
    for obstacle in &board.obstacles {
        if obstacle.kind == ObstacleKind::Copper && obstacle.label.contains('.') {
            *part_pads.entry(part_of(&obstacle.label)).or_default() += 1;
        }
    }
    let pads: Vec<Pad> = board
        .obstacles
        .iter()
        .enumerate()
        .filter(|(_, obstacle)| obstacle.kind == ObstacleKind::Copper && obstacle.layers != 0)
        .filter_map(|(index, obstacle)| {
            let bounds = obstacle.shape.aabb();
            let size = [
                bounds.maximum[0] - bounds.minimum[0],
                bounds.maximum[1] - bounds.minimum[1],
            ];
            // Board copper that is no pad (a pour's outline, a large
            // graphic) walls nothing a row cares about here.
            if size[0] > 2.0 * PIN_SIZE && size[1] > 2.0 * PIN_SIZE {
                return None;
            }
            let class_clearance = obstacle
                .net
                .and_then(|net| board.nets.get(net as usize))
                .map_or(default_clearance, |net| board.classes[net.class].clearance);
            let clearance = obstacle
                .clearance_override
                .unwrap_or(class_clearance.max(obstacle.clearance));
            let (widths, centre, exits) = match pad_net[index] {
                Some((net, anchor)) => {
                    let class = &board.classes[board.nets[net].class];
                    let mut widths = vec![class.trace_width];
                    if let Some(neck) = neck.filter(|neck| *neck < class.trace_width) {
                        widths.push(neck);
                    }
                    (widths, anchor, true)
                }
                None => (
                    Vec::new(),
                    [
                        (bounds.minimum[0] + bounds.maximum[0]) / 2.0,
                        (bounds.minimum[1] + bounds.maximum[1]) / 2.0,
                    ],
                    false,
                ),
            };
            Some(Pad {
                bounds,
                row: obstacle.label.contains('.')
                    && part_pads
                        .get(part_of(&obstacle.label))
                        .is_some_and(|count| *count >= MIN_PART_PADS),
                widths,
                clearance,
                layers: obstacle.layers,
                pin: size[0] <= PIN_SIZE || size[1] <= PIN_SIZE,
                exits,
                centre,
                label: obstacle.label.clone(),
            })
        })
        .collect();
    // A bucket grid of 1 mm over the pads' boxes.
    let cell = 1.0;
    let mut buckets: std::collections::HashMap<(i64, i64), Vec<usize>> =
        std::collections::HashMap::new();
    for (index, pad) in pads.iter().enumerate() {
        let low = [
            ((pad.bounds.minimum[0] - ROW_GAP) / cell).floor() as i64,
            ((pad.bounds.minimum[1] - ROW_GAP) / cell).floor() as i64,
        ];
        let high = [
            ((pad.bounds.maximum[0] + ROW_GAP) / cell).floor() as i64,
            ((pad.bounds.maximum[1] + ROW_GAP) / cell).floor() as i64,
        ];
        for x in low[0]..=high[0] {
            for y in low[1]..=high[1] {
                buckets.entry((x, y)).or_default().push(index);
            }
        }
    }
    let mut lines = Vec::new();
    for (index, pad) in pads.iter().enumerate() {
        if !pad.pin || !pad.row {
            continue;
        }
        let (bx, by) = (
            ((pad.bounds.minimum[0] + pad.bounds.maximum[0]) / 2.0 / cell).floor() as i64,
            ((pad.bounds.minimum[1] + pad.bounds.maximum[1]) / 2.0 / cell).floor() as i64,
        );
        let mut candidates: Vec<usize> = Vec::new();
        for x in bx - 1..=bx + 1 {
            for y in by - 1..=by + 1 {
                if let Some(found) = buckets.get(&(x, y)) {
                    candidates.extend(found.iter().copied());
                }
            }
        }
        candidates.sort_unstable();
        candidates.dedup();
        for axis in 0..2 {
            let other = 1 - axis;
            // The nearest pad facing this one across `axis` on each side:
            // sharing a layer, overlapping it along the other axis, its
            // near edge beyond this pad's.
            let mut sides: [Option<(usize, f64)>; 2] = [None, None];
            for &candidate in &candidates {
                if candidate == index {
                    continue;
                }
                let them = &pads[candidate];
                if them.layers & pad.layers == 0 {
                    continue;
                }
                let overlap = pad.bounds.maximum[other].min(them.bounds.maximum[other])
                    - pad.bounds.minimum[other].max(them.bounds.minimum[other]);
                if overlap <= 0.0 {
                    continue;
                }
                for (side, gap) in [
                    (0, pad.bounds.minimum[axis] - them.bounds.maximum[axis]),
                    (1, them.bounds.minimum[axis] - pad.bounds.maximum[axis]),
                ] {
                    if (0.0..=ROW_GAP).contains(&gap)
                        && sides[side].is_none_or(|(_, best)| gap < best)
                    {
                        sides[side] = Some((candidate, gap));
                    }
                }
            }
            // The pad's straight exit along `other`: between its
            // neighbours, inside the pad.
            if pad.exits && sides.iter().any(Option::is_some) {
                for &width in &pad.widths {
                    let half = width / 2.0;
                    let low = sides[0].map_or(f64::NEG_INFINITY, |(them, _)| {
                        pads[them].bounds.maximum[axis]
                            + pads[them].clearance.max(pad.clearance)
                            + half
                    });
                    let high = sides[1].map_or(f64::INFINITY, |(them, _)| {
                        pads[them].bounds.minimum[axis]
                            - pads[them].clearance.max(pad.clearance)
                            - half
                    });
                    let low = low.max(pad.bounds.minimum[axis]);
                    let high = high.min(pad.bounds.maximum[axis]);
                    if high < low - ON_LINE {
                        continue;
                    }
                    // The pad's centre where the band holds it (its anchor
                    // is where the router's terminal ends), else the band's
                    // middle.
                    let at = if (low - ON_LINE..=high + ON_LINE).contains(&pad.centre[axis]) {
                        pad.centre[axis]
                    } else {
                        (low + high) / 2.0
                    };
                    let slack = (at - low).min(high - at).max(0.0);
                    if slack < pitch / 2.0 {
                        lines.push(PinLine {
                            axis,
                            at,
                            slack,
                            span: [pad.bounds.minimum[other], pad.bounds.maximum[other]],
                            kind: LineKind::Exit,
                            label: pad.label.clone(),
                        });
                    }
                    break;
                }
            }
            // The gap to the neighbour on the high side: the tracks that
            // pass between the two, of the narrower width either pad's
            // net leaves with.
            if let Some((them, _)) = sides[1] {
                let them = &pads[them];
                let Some(width) = pad
                    .widths
                    .iter()
                    .chain(them.widths.iter())
                    .copied()
                    .reduce(f64::min)
                    .or(neck)
                else {
                    continue;
                };
                let half = width / 2.0;
                let clearance = pad.clearance.max(them.clearance);
                let low = pad.bounds.maximum[axis] + pad.clearance + half;
                let high = them.bounds.minimum[axis] - them.clearance - half;
                if high < low - ON_LINE {
                    continue;
                }
                let room = (high - low).max(0.0);
                let tracks = (room / (width + clearance) + 1.0e-9).floor() as usize + 1;
                let span = [
                    pad.bounds.minimum[other].max(them.bounds.minimum[other]),
                    pad.bounds.maximum[other].min(them.bounds.maximum[other]),
                ];
                // The play the tracks leave, shared evenly: track `i` may
                // lie anywhere in its share, and any choice keeps the
                // tracks' spacing (a lone track: half the band either way).
                let play = room - (tracks - 1) as f64 * (width + clearance);
                let slack = play / (2 * tracks) as f64;
                if slack >= pitch / 2.0 {
                    continue;
                }
                for track in 0..tracks {
                    let at =
                        low + track as f64 * (width + clearance) + (2 * track + 1) as f64 * slack;
                    lines.push(PinLine {
                        axis,
                        at,
                        slack,
                        span,
                        kind: LineKind::Channel,
                        label: pad.label.clone(),
                    });
                }
            }
        }
    }
    lines
}

/// Whether the regular lattice line set `origin + k * pitch` has a line
/// within `slack` of `at`.
pub fn regular_has(origin: f64, pitch: f64, at: f64, slack: f64) -> bool {
    let offset = (at - origin).rem_euclid(pitch);
    offset.min(pitch - offset) <= slack + ON_LINE
}

/// The lines the regular lattice of `grid` misses.
pub fn missing<'a>(lines: &'a [PinLine], grid: &crate::grid::Grid) -> Vec<&'a PinLine> {
    lines
        .iter()
        .filter(|line| !regular_has(grid.origin[line.axis], grid.pitch, line.at, line.slack))
        .collect()
}

/// The positions to add per axis for `lines`: one per group of lines
/// whose slack intervals meet (the middle of their common part), groups
/// closer than `min_gap` merged. Sorted.
pub fn positions(lines: &[&PinLine], min_gap: f64) -> [Vec<f64>; 2] {
    let mut result = [Vec::new(), Vec::new()];
    for (axis, out) in result.iter_mut().enumerate() {
        let mut intervals: Vec<(f64, f64)> = lines
            .iter()
            .filter(|line| line.axis == axis)
            .map(|line| (line.at - line.slack, line.at + line.slack))
            .collect();
        intervals.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
        // Greedy stabbing: the fewest points with one in every interval.
        let mut group: Option<(f64, f64)> = None;
        for (low, high) in intervals {
            group = match group {
                Some((group_low, group_high)) if low <= group_high + ON_LINE => {
                    Some((group_low.max(low), group_high.min(high)))
                }
                Some(done) => {
                    out.push(round_nm((done.0 + done.1) / 2.0));
                    Some((low, high))
                }
                None => Some((low, high)),
            };
        }
        if let Some(done) = group {
            out.push(round_nm((done.0 + done.1) / 2.0));
        }
        out.dedup_by(|later, earlier| *later - *earlier < min_gap);
    }
    result
}

fn round_nm(value: f64) -> f64 {
    (value * 1.0e6).round() / 1.0e6
}

/// `PCB_ROUTER_PIN_LINES_PROBE`: what lines the pins of `board` ask for on
/// `grid`, which the regular lattice misses and what adding them costs,
/// on stderr (once per pitch and board size in a process).
pub fn probe(board: &Board, grid: &crate::grid::Grid) {
    use std::sync::Mutex;
    static SEEN: Mutex<Vec<(i64, usize, usize)>> = Mutex::new(Vec::new());
    let key = ((grid.pitch * 1.0e6).round() as i64, grid.nx, grid.ny);
    {
        let mut seen = SEEN.lock().unwrap_or_else(|poison| poison.into_inner());
        if seen.contains(&key) {
            return;
        }
        seen.push(key);
    }
    let lines = pin_lines(board, grid.pitch);
    let missed = missing(&lines, grid);
    let added = positions(&missed, 0.005);
    let count = |kind: LineKind, axis: usize, of: &[&PinLine]| {
        of.iter()
            .filter(|line| line.kind == kind && line.axis == axis)
            .count()
    };
    let all: Vec<&PinLine> = lines.iter().collect();
    let extra_nodes =
        added[0].len() * grid.ny + added[1].len() * grid.nx + added[0].len() * added[1].len();
    eprintln!(
        "pin lines: pitch {} lattice {} x {}: asked x {} exit / {} channel, y {} / {}; missed x {} / {}, y {} / {}; added x {} y {} lines, {} nodes a layer more ({:.1} %)",
        grid.pitch,
        grid.nx,
        grid.ny,
        count(LineKind::Exit, 0, &all),
        count(LineKind::Channel, 0, &all),
        count(LineKind::Exit, 1, &all),
        count(LineKind::Channel, 1, &all),
        count(LineKind::Exit, 0, &missed),
        count(LineKind::Channel, 0, &missed),
        count(LineKind::Exit, 1, &missed),
        count(LineKind::Channel, 1, &missed),
        added[0].len(),
        added[1].len(),
        extra_nodes,
        100.0 * extra_nodes as f64 / grid.cells().max(1) as f64,
    );
    // Per part: how many of its lines the lattice misses.
    let mut parts: std::collections::BTreeMap<&str, (usize, usize, f64)> = Default::default();
    for line in &lines {
        let part = part_of(&line.label);
        let entry = parts.entry(part).or_insert((0, 0, f64::INFINITY));
        entry.0 += 1;
        entry.2 = entry.2.min(line.slack);
    }
    for line in &missed {
        let part = part_of(&line.label);
        parts.entry(part).or_insert((0, 0, f64::INFINITY)).1 += 1;
    }
    for (part, (asked, missed, slack)) in parts {
        if missed > 0 {
            // The shift per axis that puts the part's exits on the regular
            // lattice, where every exit asks the same one.
            let shift = |axis: usize| {
                let shifts: Vec<f64> = lines
                    .iter()
                    .filter(|line| {
                        line.axis == axis
                            && line.kind == LineKind::Exit
                            && part_of(&line.label) == part
                    })
                    .map(|line| {
                        let steps = ((line.at - grid.origin[axis]) / grid.pitch).round();
                        round_nm(grid.origin[axis] + steps * grid.pitch - line.at)
                    })
                    .collect();
                match shifts.first() {
                    None => "-".to_string(),
                    Some(first) if shifts.iter().all(|shift| (shift - first).abs() < 2.0e-6) => {
                        format!("{first:.6}")
                    }
                    Some(_) => "mixed".to_string(),
                }
            };
            eprintln!(
                "pin lines: pitch {}: {part}: {missed} of {asked} lines missed (least slack {:.4} mm), shift {} {}",
                grid.pitch,
                slack,
                shift(0),
                shift(1)
            );
        }
    }
    if std::env::var("PCB_ROUTER_PIN_LINES_PROBE").as_deref() == Ok("2") {
        for line in &missed {
            eprintln!(
                "pin lines: pitch {}: missed {:?} {} at {:.6} slack {:.4} ({:.3}..{:.3}) {}",
                grid.pitch,
                line.kind,
                if line.axis == 0 { "x" } else { "y" },
                line.at,
                line.slack,
                line.span[0],
                line.span[1],
                line.label
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{Net, Obstacle, RuleClass, Terminal};
    use crate::geometry::Shape;

    fn rect(centre: [f64; 2], size: [f64; 2]) -> Shape {
        Shape::Polygon {
            points: vec![
                [centre[0] - size[0] / 2.0, centre[1] - size[1] / 2.0],
                [centre[0] + size[0] / 2.0, centre[1] - size[1] / 2.0],
                [centre[0] + size[0] / 2.0, centre[1] + size[1] / 2.0],
                [centre[0] - size[0] / 2.0, centre[1] + size[1] / 2.0],
            ],
        }
    }

    /// A row of `count` pads of `size` at `pitch` along x from `start`,
    /// each its own net, the class's track `width` and `clearance`.
    pub(crate) fn row_board(
        start: [f64; 2],
        count: usize,
        pitch: f64,
        size: [f64; 2],
        width: f64,
        clearance: f64,
    ) -> Board {
        let mut board = Board {
            layer_count: 2,
            outline: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
            edge_clearance: 0.1,
            hole_clearance: 0.1,
            hole_to_hole: 0.1,
            classes: vec![RuleClass {
                trace_width: width,
                clearance,
                via_diameter: 0.6,
                via_drill: 0.3,
                layers: 0,
            }],
            neck_width: 0.0,
            obstacles: Vec::new(),
            nets: Vec::new(),
            planes: Vec::new(),
            solid_pads: Vec::new(),
            isolated_pads: Vec::new(),
            solid_pad_obstacles: Vec::new(),
            isolated_pad_obstacles: Vec::new(),
            pad_spokes: Default::default(),
            merged_pads: Vec::new(),
            no_via_pads: Vec::new(),
            fill_passes: Vec::new(),
            pin_row_gaps: Vec::new(),
            terminal_copper: Vec::new(),
            attachment: Default::default(),
            via_spans: Vec::new(),
        };
        for index in 0..count {
            let centre = [start[0] + index as f64 * pitch, start[1]];
            board.obstacles.push(Obstacle {
                shape: rect(centre, size),
                layers: 1,
                kind: ObstacleKind::Copper,
                net: Some(index as u32),
                clearance: 0.0,
                clearance_override: None,
                plated: false,
                blocks_tracks: true,
                blocks_vias: true,
                label: format!("U1.{}", index + 1),
            });
            board.nets.push(Net {
                name: format!("N{index}"),
                class: 0,
                terminals: vec![Terminal {
                    anchor: centre,
                    layers: 1,
                    pad: index,
                    contact: None,
                    label: format!("U1.{}", index + 1),
                }],
            });
        }
        board
    }

    #[test]
    fn an_exact_fit_row_asks_for_its_pad_centres() {
        // 0.4 mm pitch, 0.2 mm pads, 0.2 mm track and clearance: the
        // exit's band is the pad's centre alone.
        let board = row_board([2.03, 3.0], 5, 0.4, [0.2, 0.8], 0.2, 0.2);
        let lines = pin_lines(&board, 0.1);
        let exits: Vec<&PinLine> = lines
            .iter()
            .filter(|line| line.kind == LineKind::Exit && line.axis == 0)
            .collect();
        assert_eq!(exits.len(), 5);
        for (index, line) in exits.iter().enumerate() {
            assert!((line.at - (2.03 + 0.4 * index as f64)).abs() < 1.0e-9);
            assert!(line.slack < 1.0e-6, "{line:?}");
        }
        // No track passes between the pads (0.2 mm of copper-free gap).
        assert!(lines.iter().all(|line| line.kind != LineKind::Channel));
        // A lattice at phase 0 misses every one; one at phase 0.03 none.
        let grid = |origin: f64| crate::grid::Grid {
            origin: [origin, 0.0],
            pitch: 0.1,
            nx: 100,
            ny: 100,
        };
        assert_eq!(missing(&lines, &grid(0.0)).len(), 5);
        assert!(missing(&lines, &grid(0.03)).is_empty());
    }

    #[test]
    fn a_channel_of_two_necked_tracks_asks_for_two_lines() {
        // The laptop keyboard's F23: a gap of 0.75 mm between two pads
        // holds two 0.127 mm tracks at 0.127 mm clearance, 0.254 mm apart
        // in a band of 0.369 mm, which a 0.125 mm lattice cannot hold.
        let mut board = row_board([3.0, 3.0], 2, 0.75 + 0.3, [0.3, 0.3], 0.2, 0.127);
        board.neck_width = 0.127;
        let lines = pin_lines(&board, 0.125);
        let channel: Vec<&PinLine> = lines
            .iter()
            .filter(|line| line.kind == LineKind::Channel && line.axis == 0)
            .collect();
        assert_eq!(channel.len(), 2, "{lines:?}");
        // The band between the pads' edges at 3.15 and 3.9, 0.369 mm wide:
        // each track has a quarter of the 0.115 mm play, so any two lines
        // within the slacks keep 0.254 mm.
        let low = 3.15 + 0.127 + 0.0635;
        assert!(
            (channel[0].at - (low + 0.02875)).abs() < 1.0e-9,
            "{lines:?}"
        );
        assert!((channel[1].at - (low + 0.369 - 0.02875)).abs() < 1.0e-9);
        assert!((channel[0].slack - 0.02875).abs() < 1.0e-9);
        let gap = (channel[1].at - channel[1].slack) - (channel[0].at + channel[0].slack);
        assert!(gap >= 0.254 - 1.0e-9, "{gap}");
        // A 0.125 mm lattice misses one of them whatever its phase.
        let grid = |origin: f64| crate::grid::Grid {
            origin: [origin, 0.0],
            pitch: 0.125,
            nx: 100,
            ny: 100,
        };
        for step in 0..25 {
            let channel: Vec<PinLine> = channel.iter().map(|line| (*line).clone()).collect();
            assert!(!missing(&channel, &grid(step as f64 * 0.005)).is_empty());
        }
    }

    #[test]
    fn positions_merge_lines_whose_slack_meets() {
        let line = |at: f64, slack: f64| PinLine {
            axis: 1,
            at,
            slack,
            span: [0.0, 1.0],
            kind: LineKind::Exit,
            label: String::new(),
        };
        let lines = [
            line(1.0, 0.01),
            line(1.015, 0.01),
            line(1.5, 0.0),
            line(1.502, 0.0),
        ];
        let refs: Vec<&PinLine> = lines.iter().collect();
        let [x, y] = positions(&refs, 0.005);
        assert!(x.is_empty());
        // 1.0 and 1.015 meet in [1.005, 1.01]; 1.5 and 1.502 do not meet
        // but lie closer than the minimum gap: one line.
        assert_eq!(y.len(), 2, "{y:?}");
        assert!((y[0] - 1.0075).abs() < 1.0e-9);
        assert!((y[1] - 1.5).abs() < 1.0e-9);
    }
}
